mod db;
mod recipe;
mod scraper;
mod server;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Recipe scraper gRPC service")]
struct Cli {
    #[arg(long, default_value = "[::]")]
    host: String,

    #[arg(long, default_value_t = 50051)]
    port: u16,

    #[arg(long, default_value = "postgresql:///recipe_book")]
    pg_dsn: String,

    #[arg(
        long,
        default_value_t = 5,
        help = "Number of concurrent scrape workers"
    )]
    workers: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    server::serve(cli.host, cli.port, cli.pg_dsn, cli.workers).await
}
