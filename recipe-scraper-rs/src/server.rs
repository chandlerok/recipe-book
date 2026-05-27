use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tracing::{info, warn};
use url::Url;

use crate::db::RecipeDb;
use crate::recipe::{
    AddScrapeJobRequest, AddScrapeJobResponse, GetRecipeRequest, GetRecipeResponse,
    QueueStatusRequest, QueueStatusResponse, RecipeHit, SearchRecipesRequest,
    SearchRecipesResponse,
    recipe_service_server::{RecipeService, RecipeServiceServer},
};
use crate::scraper::{self, ProxyPool};

pub const SCRAPE_DELAY: Duration = Duration::from_secs(2);

pub struct DomainRateLimiter {
    min_delay: Duration,
    last_request: Mutex<HashMap<String, Instant>>,
}

#[allow(dead_code)]
impl DomainRateLimiter {
    pub fn new(min_delay: Duration) -> Self {
        Self {
            min_delay,
            last_request: Mutex::new(HashMap::new()),
        }
    }

    pub async fn wait_if_needed(&self, url: &str) {
        let domain = match Url::parse(url) {
            Ok(parsed) => parsed.host_str().map(|h| h.to_string()).unwrap_or_default(),
            Err(_) => return,
        };

        let mut last = self.last_request.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(*last.get(&domain).unwrap_or(&(now - self.min_delay)));
        if elapsed < self.min_delay {
            tokio::time::sleep(self.min_delay - elapsed).await;
        }
        last.insert(domain, Instant::now());
    }
}

#[derive(Debug, Clone)]
pub struct RecipeResponse {
    pub url: String,
    pub title: String,
    pub total_time: i32,
    pub ingredients: Vec<String>,
    pub instructions: Vec<String>,
    pub image: String,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub recipe: RecipeResponse,
    pub score: f64,
}

fn recipe_to_proto(r: RecipeResponse) -> GetRecipeResponse {
    GetRecipeResponse {
        url: r.url,
        title: r.title,
        total_time: r.total_time,
        ingredients: r.ingredients,
        instructions: r.instructions,
        image: r.image,
    }
}

pub struct RecipeServiceImpl {
    db: RecipeDb,
}

impl RecipeServiceImpl {
    pub fn new(db: RecipeDb) -> Self {
        Self { db }
    }
}

#[tonic::async_trait]
impl RecipeService for RecipeServiceImpl {
    async fn add_scrape_job(
        &self,
        request: Request<AddScrapeJobRequest>,
    ) -> std::result::Result<Response<AddScrapeJobResponse>, Status> {
        let req = request.into_inner();
        let url = req.url.trim().to_string();

        if url.is_empty() {
            return Err(Status::invalid_argument("url is required"));
        }

        let status = self
            .db
            .enqueue_url(&url)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if !req.background {
            let client = wreq::Client::builder()
                .emulation(wreq_util::Emulation::Chrome110)
                .timeout(scraper::HTTP_TIMEOUT)
                .build()
                .map_err(|e| Status::internal(e.to_string()))?;
            match scraper::scrape_recipe(&client, &url, None).await {
                Ok(recipe) => {
                    self.db
                        .save_recipe(&recipe)
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?;
                    info!("sync scrape done: url={url}");
                    let r = RecipeResponse {
                        url: recipe.url,
                        title: recipe.title,
                        total_time: recipe.total_time,
                        ingredients: recipe.ingredients,
                        instructions: recipe.instructions,
                        image: recipe.image,
                    };
                    return Ok(Response::new(AddScrapeJobResponse {
                        status: "done".to_string(),
                        recipe: Some(recipe_to_proto(r)),
                    }));
                }
                Err(e) => {
                    warn!("sync scrape failed: url={url} error={e}");
                    return Err(Status::internal(e.to_string()));
                }
            }
        }

        info!("job queued: url={url}");
        Ok(Response::new(AddScrapeJobResponse {
            status,
            recipe: None,
        }))
    }

    async fn search_recipes(
        &self,
        request: Request<SearchRecipesRequest>,
    ) -> std::result::Result<Response<SearchRecipesResponse>, Status> {
        let req = request.into_inner();
        if req.query.trim().is_empty() {
            return Err(Status::invalid_argument("query is required"));
        }

        let limit = if req.limit > 0 { req.limit } else { 20 };
        let hits = self
            .db
            .search(req.query.trim(), limit)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let proto_hits: Vec<RecipeHit> = hits
            .into_iter()
            .map(|h| RecipeHit {
                recipe: Some(recipe_to_proto(h.recipe)),
                score: h.score,
            })
            .collect();

        info!("search: query={} hits={}", req.query, proto_hits.len());
        Ok(Response::new(SearchRecipesResponse { hits: proto_hits }))
    }

    async fn get_recipe(
        &self,
        request: Request<GetRecipeRequest>,
    ) -> std::result::Result<Response<GetRecipeResponse>, Status> {
        let url = request.into_inner().url;
        match self.db.get_recipe(&url).await {
            Ok(Some(recipe)) => Ok(Response::new(recipe_to_proto(recipe))),
            Ok(None) => Err(Status::not_found("recipe not found")),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn queue_status(
        &self,
        _request: Request<QueueStatusRequest>,
    ) -> std::result::Result<Response<QueueStatusResponse>, Status> {
        let stats = self
            .db
            .queue_stats()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(QueueStatusResponse {
            pending: stats.get("pending").copied().unwrap_or(0) as i32,
            in_progress: stats.get("in_progress").copied().unwrap_or(0) as i32,
            done: stats.get("done").copied().unwrap_or(0) as i32,
            error: stats.get("error").copied().unwrap_or(0) as i32,
        }))
    }
}

async fn run_worker(
    db: RecipeDb,
    client: wreq::Client,
    rate_limiter: Arc<DomainRateLimiter>,
    shutdown: Arc<AtomicBool>,
    proxy_pool: Option<Arc<ProxyPool>>,
    worker_id: usize,
) {
    info!("worker started: worker_id={worker_id}");

    while !shutdown.load(Ordering::Acquire) {
        match db.next_pending().await {
            Ok(Some((job_id, url))) => {
                let proxy = proxy_pool.as_ref().and_then(|p| p.next());

                // Rate limit per domain unless using a proxy (different IP)
                if proxy.is_none() {
                    rate_limiter.wait_if_needed(&url).await;
                }

                info!("scraping job: worker_id={worker_id} job_id={job_id} url={url}");

                match scraper::scrape_recipe(&client, &url, proxy).await {
                    Ok(recipe) => match db.save_recipe_and_mark_done(&recipe, job_id).await {
                        Ok(_) => info!(
                            "job done: worker_id={worker_id} job_id={job_id} url={url} title={}",
                            recipe.title
                        ),
                        Err(e) => warn!(
                            "failed to save and mark done: url={url} job_id={job_id} error={e}"
                        ),
                    },
                    Err(e) => {
                        warn!(
                            "job failed: worker_id={worker_id} job_id={job_id} url={url} error={e}"
                        );
                        if let Err(db_err) = db.mark_error(job_id, &e.to_string()).await {
                            warn!("failed to mark error: job_id={job_id} error={db_err}");
                        }
                    }
                }
            }
            Ok(None) => {
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
            Err(e) => {
                warn!("db error in worker: worker_id={worker_id} error={e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }

    info!("worker stopped: worker_id={worker_id}");
}

pub async fn serve(host: String, port: u16, dsn: String, workers: usize) -> Result<()> {
    let db = RecipeDb::new(&dsn).await?;
    let shutdown = Arc::new(AtomicBool::new(false));

    let proxy_pool = ProxyPool::from_env();

    let client = wreq::Client::builder()
        .emulation(wreq_util::Emulation::Chrome110)
        .timeout(scraper::HTTP_TIMEOUT)
        .build()?;

    if proxy_pool.is_some() {
        info!("proxy pool configured: each request uses a random proxy");
    } else {
        info!("no proxy configured: requests will use direct connection");
    }

    let rate_limiter = Arc::new(DomainRateLimiter::new(SCRAPE_DELAY));

    for i in 0..workers {
        let db = db.clone();
        let client = client.clone();
        let rate_limiter = rate_limiter.clone();
        let shutdown = shutdown.clone();
        let proxy_pool = proxy_pool.clone();
        tokio::spawn(async move {
            run_worker(db, client, rate_limiter, shutdown, proxy_pool, i + 1).await;
        });
    }

    let addr: std::net::SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?;

    let service = RecipeServiceImpl::new(db);
    let shutdown_clone = shutdown.clone();

    info!("gRPC server starting: host={host} port={port} workers={workers}");

    tonic::transport::Server::builder()
        .add_service(RecipeServiceServer::new(service))
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c().await.ok();
            info!("shutting down");
            shutdown_clone.store(true, Ordering::Release);
        })
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_recipe() -> RecipeResponse {
        RecipeResponse {
            url: "https://example.com/test".to_string(),
            title: "Test Dish".to_string(),
            total_time: 45,
            ingredients: vec!["a".to_string(), "b".to_string()],
            instructions: vec!["step 1".to_string(), "step 2".to_string()],
            image: "https://img.example.com/test.jpg".to_string(),
        }
    }

    #[test]
    fn test_recipe_to_proto_converts_fields() {
        let r = sample_recipe();
        let proto = recipe_to_proto(r);

        assert_eq!(proto.url, "https://example.com/test");
        assert_eq!(proto.title, "Test Dish");
        assert_eq!(proto.total_time, 45);
        let ings: Vec<&str> = proto.ingredients.iter().map(|s| s.as_str()).collect();
        assert_eq!(ings, vec!["a", "b"]);
        let insts: Vec<&str> = proto.instructions.iter().map(|s| s.as_str()).collect();
        assert_eq!(insts, vec!["step 1", "step 2"]);
        assert_eq!(proto.image, "https://img.example.com/test.jpg");
    }

    #[test]
    fn test_recipe_to_proto_defaults_empty() {
        let r = RecipeResponse {
            url: "https://example.com/minimal".to_string(),
            title: String::new(),
            total_time: 0,
            ingredients: vec![],
            instructions: vec![],
            image: String::new(),
        };
        let proto = recipe_to_proto(r);

        assert_eq!(proto.url, "https://example.com/minimal");
        assert_eq!(proto.title, "");
        assert_eq!(proto.total_time, 0);
        assert!(proto.ingredients.is_empty());
        assert!(proto.instructions.is_empty());
        assert_eq!(proto.image, "");
    }
}
