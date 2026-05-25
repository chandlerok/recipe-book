use std::{collections::HashSet, sync::atomic::AtomicUsize, time::Duration};

use anyhow::{Context, Result};
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, CONNECTION, DNT, REFERER, USER_AGENT,
};
use scraper::{Html, Selector};
use tokio::{sync::mpsc, time::sleep};
use tracing::{info, warn};
use url::Url;

const REQUEST_DELAY: Duration = Duration::from_secs(3);
const MAX_RETRIES: u32 = 2;
const BASE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

static USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
];

static UA_INDEX: AtomicUsize = AtomicUsize::new(0);

fn next_user_agent() -> &'static str {
    let idx = UA_INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    USER_AGENTS[idx % USER_AGENTS.len()]
}

fn is_retryable(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS
            | reqwest::StatusCode::INTERNAL_SERVER_ERROR
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

pub struct SiteConfig {
    pub name: &'static str,
    pub base_url: &'static str,
    pub seed_paths: &'static [&'static str],
    pub recipe_url_test: fn(&str) -> bool,
}

pub static ALLRECIPES: SiteConfig = SiteConfig {
    name: "allrecipes",
    base_url: "https://www.allrecipes.com",
    seed_paths: &[
        "/recipes/",
        "/recipes/meat-and-poultry/",
        "/recipes/seafood/",
        "/recipes/pasta-and-noodles/",
        "/recipes/salad/",
        "/recipes/soups-stews-and-chili/",
        "/recipes/appetizers-and-snacks/",
        "/recipes/bread/",
        "/recipes/desserts/",
        "/recipes/breakfast-and-brunch/",
        "/recipes/drinks/",
        "/recipes/side-dish/",
        "/recipes/bbq-and-grilling/",
        "/recipes/holidays-and-events/",
        "/recipes/healthy-recipes/",
        "/recipes/quick-and-easy/",
    ],
    recipe_url_test: |url| {
        url.contains("allrecipes.com/recipe/")
            && !url.contains("/photo/")
            && !url.contains("/video/")
            && !url.contains('#')
    },
};

pub static BONAPPETIT: SiteConfig = SiteConfig {
    name: "bonappetit",
    base_url: "https://www.bonappetit.com",
    seed_paths: &[
        "/recipes/",
        "/recipes/quick-and-easy/",
        "/recipes/healthyish/",
        "/recipes/vegetarian/",
        "/recipes/vegan/",
        "/recipes/desserts/",
        "/recipes/drinks/",
        "/recipes/pasta/",
        "/recipes/chicken/",
        "/recipes/breakfast/",
    ],
    recipe_url_test: |url| {
        url.contains("bonappetit.com/recipe/") && !url.contains("#") && !url.contains("/gallery/")
    },
};

pub async fn crawl(
    site: &SiteConfig,
    client: &reqwest::Client,
    tx: mpsc::Sender<String>,
) -> Result<()> {
    let recipe_link_sel = Selector::parse("a[href*=\"/recipe/\"]").unwrap();
    let next_page_sel =
        Selector::parse("a[rel=\"next\"], a.pagination__next, a[href*=\"?page=\"]").unwrap();

    let mut visited_pages: HashSet<String> = HashSet::new();
    let mut seen_recipes: HashSet<String> = HashSet::new();

    let mut page_queue: Vec<String> = site
        .seed_paths
        .iter()
        .map(|p| format!("{}{}", site.base_url, p))
        .collect();

    info!(
        site = site.name,
        seeds = site.seed_paths.len(),
        "starting crawl",
    );

    while let Some(page_url) = page_queue.pop() {
        if visited_pages.contains(&page_url) {
            continue;
        }
        visited_pages.insert(page_url.clone());

        info!(site = site.name, url = %page_url, "fetching page");

        let response = match fetch_page(client, &page_url, site.base_url).await {
            Ok(r) => r,
            Err(e) => {
                warn!(site = site.name, url = %page_url, error = %e, "fetch failed");
                sleep(REQUEST_DELAY).await;
                continue;
            }
        };

        let page_for_parse = page_url.clone();
        let recipe_sel = recipe_link_sel.clone();
        let next_sel = next_page_sel.clone();
        let recipe_test = site.recipe_url_test;
        let base_url = site.base_url;

        let (raw_recipes, raw_next): (Vec<String>, Vec<String>) =
            tokio::task::spawn_blocking(move || {
                let document = Html::parse_document(&response);

                let raw_recipes: Vec<String> = document
                    .select(&recipe_sel)
                    .filter_map(|el| el.value().attr("href"))
                    .map(|href| resolve_url(&page_for_parse, href, base_url))
                    .filter(|url| recipe_test(url))
                    .collect();

                let raw_next: Vec<String> = document
                    .select(&next_sel)
                    .filter_map(|el| el.value().attr("href"))
                    .map(|href| resolve_url(&page_for_parse, href, base_url))
                    .collect();

                (raw_recipes, raw_next)
            })
            .await
            .context("spawn_blocking failed")?;

        let recipe_urls: Vec<String> = raw_recipes
            .into_iter()
            .filter(|url| seen_recipes.insert(url.clone()))
            .collect();

        let count = recipe_urls.len();
        for url in recipe_urls {
            if tx.send(url).await.is_err() {
                return Ok(());
            }
        }

        for next in raw_next {
            if !visited_pages.contains(&next) {
                page_queue.push(next);
            }
        }

        info!(
            site = site.name,
            url = %page_url,
            new_recipes = count,
            total_seen = seen_recipes.len(),
            "page processed",
        );

        sleep(REQUEST_DELAY).await;
    }

    info!(
        site = site.name,
        total = seen_recipes.len(),
        "crawl complete"
    );
    Ok(())
}

async fn fetch_page(client: &reqwest::Client, url: &str, site_base_url: &str) -> Result<String> {
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=MAX_RETRIES {
        let ua = next_user_agent();

        let result = client
            .get(url)
            .header(USER_AGENT, ua)
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
            )
            .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .header(ACCEPT_ENCODING, "gzip, deflate, br")
            .header(CACHE_CONTROL, "no-cache")
            .header(DNT, "1")
            .header(CONNECTION, "keep-alive")
            .header(REFERER, site_base_url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => {
                return response
                    .text()
                    .await
                    .context("failed to read response body");
            }
            Ok(response) => {
                let status = response.status();
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let retry_after = response
                        .headers()
                        .get("Retry-After")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(5);
                    warn!(url, retry_after, "rate limited");
                    sleep(Duration::from_secs(retry_after)).await;
                    last_error = Some(anyhow::anyhow!("HTTP 429"));
                    continue;
                }
                if status.is_client_error() && !is_retryable(status) {
                    anyhow::bail!("HTTP {status}");
                }
                last_error = Some(anyhow::anyhow!("HTTP {status}"));
            }
            Err(e) => {
                last_error = Some(anyhow::anyhow!(e));
            }
        }

        if attempt < MAX_RETRIES {
            let backoff = BASE_BACKOFF * 2u32.pow(attempt);
            let delay = backoff.min(MAX_BACKOFF);
            warn!(
                url,
                attempt = attempt + 1,
                delay_ms = delay.as_millis(),
                "retrying"
            );
            sleep(delay).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("fetch failed")))
}

fn resolve_url(base: &str, href: &str, base_url: &str) -> String {
    if href.starts_with("http") {
        return href.to_string();
    }
    if let Ok(base_parsed) = Url::parse(base)
        && let Ok(full) = base_parsed.join(href)
    {
        return full.to_string();
    }
    format!("{}{}", base_url, href.trim_start_matches('/'))
}
