use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing_subscriber::EnvFilter;

mod crawler;
mod db;
mod models;
mod scraper;
mod search;
mod server;

#[derive(Parser)]
#[command(name = "recipe-scraper-server")]
struct Cli {
    #[arg(long, env = "HOST", default_value = "127.0.0.1")]
    host: String,

    #[arg(long, env = "PORT", default_value = "3000")]
    port: u16,

    #[arg(long, env = "PG_DSN", default_value = "postgresql:///recipe_book")]
    pg_dsn: String,

    #[arg(long, env = "WORKERS", default_value = "5")]
    workers: usize,

    #[arg(long, env = "WEB_DIST", default_value = "../recipe-scraper-web/dist")]
    web_dist: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,recipe_scraper_server=debug")),
        )
        .init();

    let cli = Cli::parse();

    let db = db::RecipeDb::new(&cli.pg_dsn).await?;

    let search_index = Arc::new(search::RecipeIndex::build(db.pool().clone()).await?);

    let crawler_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let crawler_proxy_pool = crawler::ProxyPool::from_env();

    let sched = JobScheduler::new().await?;
    sched
        .add(Job::new_async("0 0 0 * * *", {
            let db = db.clone();
            let client = crawler_client.clone();
            let proxy_pool = crawler_proxy_pool.clone();
            move |_uuid, _l| {
                let db = db.clone();
                let client = client.clone();
                let proxy_pool = proxy_pool.clone();
                Box::pin(async move {
                    tracing::info!("starting daily crawl");
                    crawler::crawl_all_sites(&db, &client, proxy_pool).await;
                    tracing::info!("daily crawl complete");
                })
            }
        })?)
        .await?;

    sched.start().await?;

    server::serve(
        cli.host,
        cli.port,
        cli.pg_dsn,
        cli.workers,
        cli.web_dist,
        search_index,
    )
    .await
}
