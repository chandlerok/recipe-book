use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use askama::Template;
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde_json::json;
use tokio::sync::Mutex;
use tower_http::services::ServeDir;
use tracing::{info, warn};
use url::Url;

use crate::db::RecipeDb;
use crate::models::{RecipeQuery, SearchParams};
use crate::scraper::{self, ProxyPool};
use crate::templates::{
    HitView, IndexTemplate, RecipeDetailTemplate, SearchResultsTemplate,
};

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
pub struct AppState {
    pub db: RecipeDb,
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

async fn index_handler(State(state): State<AppState>, Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let query = params.q.unwrap_or_default().trim().to_string();
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    let (results, total) = if !query.is_empty() {
        match state.db.search(&query, limit, offset).await {
            Ok(r) => (r.hits.into_iter().map(HitView::from).collect(), r.total),
            Err(e) => {
                warn!("search error: {}", e);
                (Vec::new(), 0)
            }
        }
    } else {
        (Vec::new(), 0)
    };

    let tmpl = IndexTemplate {
        query: &query,
        results,
        total,
        offset: offset as i64,
        limit: limit as i64,
    };

    match tmpl.render() {
        Ok(html) => (StatusCode::OK, [("content-type", "text/html")], html),
        Err(e) => {
            warn!("template error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, [("content-type", "text/plain")], "Internal error".to_string())
        }
    }
}

async fn search_handler(State(state): State<AppState>, Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let query = params.q.unwrap_or_default().trim().to_string();
    if query.is_empty() {
        return (StatusCode::OK, [("content-type", "text/html")], String::new());
    }

    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    let (results, total) = match state.db.search(&query, limit, offset).await {
        Ok(r) => (r.hits.into_iter().map(HitView::from).collect(), r.total),
        Err(e) => {
            warn!("search error: {}", e);
            (Vec::new(), 0)
        }
    };

    if results.is_empty() {
        let html = format!(
            r#"<div class="empty-state"><p class="empty-title">No recipes found</p><p class="empty-subtitle">Try adjusting your search term</p></div>"#
        );
        return (StatusCode::OK, [("content-type", "text/html")], html);
    }

    let tmpl = SearchResultsTemplate {
        query,
        results,
        total,
        offset: offset as i64,
        limit: limit as i64,
    };

    match tmpl.render() {
        Ok(html) => (StatusCode::OK, [("content-type", "text/html")], html),
        Err(e) => {
            warn!("template error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, [("content-type", "text/plain")], "Internal error".to_string())
        }
    }
}

async fn recipe_handler(State(state): State<AppState>, Query(params): Query<RecipeQuery>) -> impl IntoResponse {
    match state.db.get_recipe(&params.url).await {
        Ok(Some(recipe)) => {
            let tmpl = RecipeDetailTemplate { recipe };
            match tmpl.render() {
                Ok(html) => (StatusCode::OK, [("content-type", "text/html")], html),
                Err(e) => {
                    warn!("template error: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, [("content-type", "text/plain")], "Internal error".to_string())
                }
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, [("content-type", "text/plain")], "Recipe not found".to_string()),
        Err(e) => {
            warn!("get recipe error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, [("content-type", "text/plain")], e.to_string())
        }
    }
}

pub async fn serve(
    host: String,
    port: u16,
    dsn: String,
    workers: usize,
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

    let state = AppState { db: db.clone() };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/search", get(search_handler))
        .route("/recipe", get(recipe_handler))
        .route("/api/recipes/search", get(search_recipes_handler))
        .route("/api/recipes", get(get_recipe_handler))
        .route("/api/queue/status", get(queue_status_handler))
        .nest_service("/static", ServeDir::new("static"))
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

// JSON API handlers (kept for programmatic access)

async fn search_recipes_handler(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> impl axum::response::IntoResponse {
    if params.q.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            json!({"error": "query parameter 'q' is required"}).to_string(),
        );
    }

    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    match state.db.search(params.q.trim(), limit, offset).await {
        Ok(results) => {
            info!("search: query={} total={}", params.q, results.total);
            (StatusCode::OK, serde_json::to_string(&results).unwrap())
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

async fn get_recipe_handler(
    State(state): State<AppState>,
    Query(params): Query<RecipeQuery>,
) -> impl axum::response::IntoResponse {
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

async fn queue_status_handler(State(state): State<AppState>) -> impl axum::response::IntoResponse {
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
