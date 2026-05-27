use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::json;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::services::fs::ServeFile;
use tracing::{info, warn};
use url::Url;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::db::RecipeDb;
use crate::models::{QueueStats, Recipe, RecipeQuery, ScrapeRequest, SearchHit, SearchParams};
use crate::scraper::{self, ProxyPool};

pub const SCRAPE_DELAY: Duration = Duration::from_secs(2);

pub struct DomainRateLimiter {
    min_delay: Duration,
    last_request: Mutex<HashMap<String, Instant>>,
}

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

#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub db: RecipeDb,
    pub scrape_client: wreq::Client,
    pub rate_limiter: Arc<DomainRateLimiter>,
    pub proxy_pool: Option<Arc<ProxyPool>>,
    pub shutdown: Arc<AtomicBool>,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        search_recipes_handler,
        get_recipe_handler,
        submit_scrape_handler,
        queue_status_handler,
    ),
    components(
        schemas(Recipe, SearchHit, QueueStats, ScrapeRequest, SearchParams)
    ),
    tags(
        (name = "recipes", description = "Recipe search and retrieval"),
        (name = "scrape", description = "Recipe scraping jobs"),
    )
)]
struct ApiDoc;

#[utoipa::path(
    get,
    path = "/api/recipes/search",
    params(
        ("q" = String, Query, description = "Search query"),
        ("limit" = Option<i32>, Query, description = "Maximum results"),
    ),
    responses(
        (status = 200, description = "Search results", body = Vec<SearchHit>),
        (status = 400, description = "Missing query parameter"),
    ),
    tag = "recipes"
)]
async fn search_recipes_handler(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    if params.q.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            json!({"error": "query parameter 'q' is required"}).to_string(),
        );
    }

    let limit = params.limit.unwrap_or(20).clamp(1, 100);

    match state.db.search(params.q.trim(), limit).await {
        Ok(hits) => {
            info!("search: query={} hits={}", params.q, hits.len());
            (StatusCode::OK, serde_json::to_string(&hits).unwrap())
        }
        Err(e) => {
            warn!("search error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": e.to_string()}).to_string(),
            )
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/recipes",
    params(
        ("url" = String, Query, description = "Recipe URL (URL-encoded)"),
    ),
    responses(
        (status = 200, description = "Recipe found", body = Recipe),
        (status = 404, description = "Recipe not found"),
    ),
    tag = "recipes"
)]
async fn get_recipe_handler(
    State(state): State<AppState>,
    Query(params): Query<RecipeQuery>,
) -> impl IntoResponse {
    match state.db.get_recipe(&params.url).await {
        Ok(Some(recipe)) => (StatusCode::OK, serde_json::to_string(&recipe).unwrap()),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            json!({"error": "recipe not found"}).to_string(),
        ),
        Err(e) => {
            warn!("get recipe error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": e.to_string()}).to_string(),
            )
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/scrape",
    request_body = ScrapeRequest,
    responses(
        (status = 200, description = "URL enqueued for scraping"),
        (status = 400, description = "Invalid request"),
    ),
    tag = "scrape"
)]
async fn submit_scrape_handler(
    State(state): State<AppState>,
    axum::extract::Json(params): axum::extract::Json<ScrapeRequest>,
) -> impl IntoResponse {
    let url = params.url.trim().to_string();
    if url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            json!({"error": "url is required"}).to_string(),
        );
    }

    match state.db.enqueue_url(&url).await {
        Ok(status) => {
            info!("job queued: url={url} status={status}");
            (
                StatusCode::OK,
                json!({"status": status, "url": url}).to_string(),
            )
        }
        Err(e) => {
            warn!("enqueue error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": e.to_string()}).to_string(),
            )
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/queue/status",
    responses(
        (status = 200, description = "Queue statistics", body = QueueStats),
    ),
    tag = "scrape"
)]
async fn queue_status_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.db.queue_stats().await {
        Ok(stats) => (StatusCode::OK, serde_json::to_string(&stats).unwrap()),
        Err(e) => {
            warn!("queue stats error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": e.to_string()}).to_string(),
            )
        }
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

pub async fn serve(
    host: String,
    port: u16,
    dsn: String,
    workers: usize,
    web_dist: String,
) -> Result<()> {
    let db = RecipeDb::new(&dsn).await?;
    let shutdown = Arc::new(AtomicBool::new(false));

    let proxy_pool = ProxyPool::from_env();

    let scrape_client = wreq::Client::builder()
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
        let client = scrape_client.clone();
        let rate_limiter = rate_limiter.clone();
        let shutdown = shutdown.clone();
        let proxy_pool = proxy_pool.clone();
        tokio::spawn(async move {
            run_worker(db, client, rate_limiter, shutdown, proxy_pool, i + 1).await;
        });
    }

    let state = AppState {
        db: db.clone(),
        scrape_client,
        rate_limiter,
        proxy_pool,
        shutdown: shutdown.clone(),
    };

    let app = Router::new()
        .route("/api/recipes/search", get(search_recipes_handler))
        .route("/api/recipes", get(get_recipe_handler))
        .route("/api/scrape", post(submit_scrape_handler))
        .route("/api/queue/status", get(queue_status_handler))
        .merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .fallback_service(
            ServeDir::new(&web_dist).fallback(ServeFile::new(format!("{web_dist}/index.html"))),
        )
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: std::net::SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?;

    info!("server starting: host={host} port={port} workers={workers}");

    let shutdown_signal = shutdown.clone();
    axum::serve(
        tokio::net::TcpListener::bind(addr).await?,
        app.into_make_service(),
    )
    .with_graceful_shutdown(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("shutting down");
        shutdown_signal.store(true, Ordering::Release);
    })
    .await?;

    Ok(())
}
