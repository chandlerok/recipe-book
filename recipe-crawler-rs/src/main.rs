mod crawler;
mod recipe;

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tonic::transport::Channel;
use tracing::{info, warn};

use recipe::{AddScrapeJobRequest, recipe_service_client::RecipeServiceClient};

const GRPC_ADDR: &str = "http://[::1]:50051";
const GRPC_SUBMIT_DELAY: Duration = Duration::from_millis(500);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("failed to build HTTP client")?;

    info!(grpc_addr = GRPC_ADDR, "connecting to gRPC server");
    let channel = Channel::from_static(GRPC_ADDR)
        .connect()
        .await
        .context("failed to connect to gRPC server")?;
    let mut grpc = RecipeServiceClient::new(channel);

    let (tx, mut rx) = mpsc::channel::<String>(512);

    let client_a = http_client.clone();
    let tx_a = tx.clone();
    let allrecipes_handle = tokio::spawn(async move {
        if let Err(e) = crawler::crawl(&crawler::ALLRECIPES, &client_a, tx_a).await {
            warn!(error = %e, "allrecipes crawl error");
        }
    });

    let client_b = http_client.clone();
    let tx_b = tx.clone();
    let bonappetit_handle = tokio::spawn(async move {
        if let Err(e) = crawler::crawl(&crawler::BONAPPETIT, &client_b, tx_b).await {
            warn!(error = %e, "bonappetit crawl error");
        }
    });

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

    let _ = tokio::join!(allrecipes_handle, bonappetit_handle);
    Ok(())
}
