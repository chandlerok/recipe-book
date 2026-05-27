mod crawler;
mod recipe;

use std::time::Duration;

use anyhow::{Context, Result};
use rand::Rng;
use tokio::sync::mpsc;
use tonic::transport::Channel;
use tracing::{info, warn};

use recipe::{AddScrapeJobRequest, recipe_service_client::RecipeServiceClient};

const GRPC_ADDR: &str = "http://[::1]:50051";
const GRPC_SUBMIT_DELAY: Duration = Duration::from_millis(500);
const STARTUP_JITTER_MAX: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let proxy_pool = crawler::ProxyPool::from_env();

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("failed to build HTTP client")?;

    if proxy_pool.is_some() {
        info!("proxy pool configured: each request uses a random proxy");
    }

    info!(grpc_addr = GRPC_ADDR, "connecting to gRPC server");
    let channel = Channel::from_static(GRPC_ADDR)
        .connect()
        .await
        .context("failed to connect to gRPC server")?;
    let mut grpc = RecipeServiceClient::new(channel);

    let (tx, mut rx) = mpsc::channel::<String>(1024);

    let mut handles = Vec::new();
    for site in crawler::ALL_SITES {
        let client = http_client.clone();
        let tx = tx.clone();
        let name = site.name;
        let proxy_pool = proxy_pool.clone();

        let jitter_ms: u64 = rand::thread_rng().gen_range(0..STARTUP_JITTER_MAX.as_millis() as u64);
        handles.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
            if let Err(e) = crawler::crawl(site, &client, tx, proxy_pool).await {
                warn!(site = name, error = %e, "crawl error");
            }
        }));
    }

    drop(tx);

    let mut submitted = 0u64;
    let mut skipped = 0u64;

    while let Some(url) = rx.recv().await {
        let request = tonic::Request::new(AddScrapeJobRequest {
            url: url.clone(),
            background: true,
        });

        match grpc.add_scrape_job(request).await {
            Ok(response) => {
                let status = response.into_inner().status;
                if status == "pending" {
                    submitted += 1;
                    info!(url = %url, submitted, "queued recipe");
                } else {
                    skipped += 1;
                    info!(url = %url, status = %status, "skipped");
                }
            }
            Err(e) => {
                skipped += 1;
                warn!(url = %url, error = %e, "gRPC error submitting job");
            }
        }

        tokio::time::sleep(GRPC_SUBMIT_DELAY).await;
    }

    info!(submitted, skipped, "channel closed, crawl complete");

    for handle in handles {
        let _ = handle.await;
    }
    Ok(())
}
