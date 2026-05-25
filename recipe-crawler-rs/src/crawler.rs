use std::{collections::HashSet, sync::atomic::AtomicUsize, time::Duration};

use anyhow::{Context, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
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
    pub recipe_link_selector: &'static str,
    pub use_sitemap: bool,
}

pub static ALLRECIPES: SiteConfig = SiteConfig {
    name: "allrecipes",
    base_url: "https://www.allrecipes.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.contains("allrecipes.com/recipe/")
            && !url.contains("/photo/")
            && !url.contains("/video/")
            && !url.contains('#')
    },
    recipe_link_selector: "a[href*=\"/recipe/\"]",
    use_sitemap: true,
};

pub static BONAPPETIT: SiteConfig = SiteConfig {
    name: "bonappetit",
    base_url: "https://www.bonappetit.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.contains("bonappetit.com/recipe/") && !url.contains("#") && !url.contains("/gallery/")
    },
    recipe_link_selector: "a[href*=\"/recipe/\"]",
    use_sitemap: true,
};

pub static FOOD52: SiteConfig = SiteConfig {
    name: "food52",
    base_url: "https://food52.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| url.contains("food52.com/recipes/") && !url.contains('#'),
    recipe_link_selector: "a[href*=\"/recipe\"]",
    use_sitemap: true,
};

pub static SIMPLY_RECIPES: SiteConfig = SiteConfig {
    name: "simplyrecipes",
    base_url: "https://www.simplyrecipes.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.contains("simplyrecipes.com/") && !url.contains("/how-to/") && !url.contains('#')
    },
    recipe_link_selector: "a[href*=\"/recipe\"]",
    use_sitemap: true,
};

pub static PIONEER_WOMAN: SiteConfig = SiteConfig {
    name: "pioneerwoman",
    base_url: "https://www.thepioneerwoman.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.contains("thepioneerwoman.com/food-cooking/recipes/") && !url.contains('#')
    },
    recipe_link_selector: "a[href*=\"/recipe\"]",
    use_sitemap: true,
};

pub static TASTE_OF_HOME: SiteConfig = SiteConfig {
    name: "tasteofhome",
    base_url: "https://www.tasteofhome.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| url.contains("tasteofhome.com/recipes/") && !url.contains('#'),
    recipe_link_selector: "a[href*=\"/recipe\"]",
    use_sitemap: true,
};

pub static SERIOUS_EATS: SiteConfig = SiteConfig {
    name: "seriouseats",
    base_url: "https://www.seriouseats.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| url.contains("seriouseats.com/"),
    recipe_link_selector: "a[href*=\"/recipe\"]",
    use_sitemap: true,
};

pub static COOKIE_AND_KATE: SiteConfig = SiteConfig {
    name: "cookieandkate",
    base_url: "https://cookieandkate.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.starts_with("https://cookieandkate.com/")
            && url != "https://cookieandkate.com/"
            && !url.starts_with("https://cookieandkate.com/about")
            && !url.starts_with("https://cookieandkate.com/cookbook")
            && !url.starts_with("https://cookieandkate.com/favorite")
            && !url.starts_with("https://cookieandkate.com/shop")
            && !url.starts_with("https://cookieandkate.com/contact")
            && !url.starts_with("https://cookieandkate.com/recipe-index")
            && !url.starts_with("https://cookieandkate.com/privacy")
            && !url.starts_with("https://cookieandkate.com/category")
            && !url.contains('#')
    },
    recipe_link_selector: "article a[href], .post a[href], .archive a[href], h2 a[href], h3 a[href]",
    use_sitemap: true,
};

pub static PINCH_OF_YUM: SiteConfig = SiteConfig {
    name: "pinchofyum",
    base_url: "https://pinchofyum.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.starts_with("https://pinchofyum.com/")
            && url != "https://pinchofyum.com/"
            && !url.starts_with("https://pinchofyum.com/about")
            && !url.starts_with("https://pinchofyum.com/shop")
            && !url.starts_with("https://pinchofyum.com/resources")
            && !url.starts_with("https://pinchofyum.com/pinch")
            && !url.starts_with("https://pinchofyum.com/contact")
            && !url.starts_with("https://pinchofyum.com/recipe-index")
            && !url.starts_with("https://pinchofyum.com/category")
            && !url.contains('#')
    },
    recipe_link_selector: "article a[href], .post a[href], .archive a[href], h2 a[href], h3 a[href]",
    use_sitemap: true,
};

pub static HALF_BAKED_HARVEST: SiteConfig = SiteConfig {
    name: "halfbakedharvest",
    base_url: "https://www.halfbakedharvest.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.starts_with("https://www.halfbakedharvest.com/")
            && url != "https://www.halfbakedharvest.com/"
            && !url.starts_with("https://www.halfbakedharvest.com/about")
            && !url.starts_with("https://www.halfbakedharvest.com/cookbook")
            && !url.starts_with("https://www.halfbakedharvest.com/shop")
            && !url.starts_with("https://www.halfbakedharvest.com/contact")
            && !url.starts_with("https://www.halfbakedharvest.com/recipe-index")
            && !url.starts_with("https://www.halfbakedharvest.com/privacy")
            && !url.starts_with("https://www.halfbakedharvest.com/category")
            && !url.contains('#')
    },
    recipe_link_selector: "article a[href], .post a[href], .archive a[href], h2 a[href], h3 a[href]",
    use_sitemap: true,
};

pub static LOVE_AND_LEMONS: SiteConfig = SiteConfig {
    name: "loveandlemons",
    base_url: "https://www.loveandlemons.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.starts_with("https://www.loveandlemons.com/")
            && url != "https://www.loveandlemons.com/"
            && !url.starts_with("https://www.loveandlemons.com/about")
            && !url.starts_with("https://www.loveandlemons.com/cookbook")
            && !url.starts_with("https://www.loveandlemons.com/shop")
            && !url.starts_with("https://www.loveandlemons.com/contact")
            && !url.starts_with("https://www.loveandlemons.com/recipes")
            && !url.starts_with("https://www.loveandlemons.com/category")
            && !url.contains('#')
    },
    recipe_link_selector: "article a[href], .post a[href], .archive a[href], h2 a[href], h3 a[href]",
    use_sitemap: true,
};

pub static RECIPE_TIN_EATS: SiteConfig = SiteConfig {
    name: "recipetineats",
    base_url: "https://www.recipetineats.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.starts_with("https://www.recipetineats.com/")
            && url != "https://www.recipetineats.com/"
            && !url.starts_with("https://www.recipetineats.com/about")
            && !url.starts_with("https://www.recipetineats.com/cookbook")
            && !url.starts_with("https://www.recipetineats.com/shop")
            && !url.starts_with("https://www.recipetineats.com/contact")
            && !url.starts_with("https://www.recipetineats.com/recipes")
            && !url.starts_with("https://www.recipetineats.com/recipe-index")
            && !url.starts_with("https://www.recipetineats.com/category")
            && !url.contains('#')
    },
    recipe_link_selector: "article a[href], .post a[href], .archive a[href], h2 a[href], h3 a[href]",
    use_sitemap: true,
};

pub static MINIMALIST_BAKER: SiteConfig = SiteConfig {
    name: "minimalistbaker",
    base_url: "https://minimalistbaker.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.starts_with("https://minimalistbaker.com/")
            && url != "https://minimalistbaker.com/"
            && !url.starts_with("https://minimalistbaker.com/about")
            && !url.starts_with("https://minimalistbaker.com/cookbook")
            && !url.starts_with("https://minimalistbaker.com/shop")
            && !url.starts_with("https://minimalistbaker.com/contact")
            && !url.starts_with("https://minimalistbaker.com/recipes")
            && !url.starts_with("https://minimalistbaker.com/recipe-index")
            && !url.starts_with("https://minimalistbaker.com/category")
            && !url.contains('#')
    },
    recipe_link_selector: "article a[href], .post a[href], .archive a[href], h2 a[href], h3 a[href]",
    use_sitemap: true,
};

pub static BUDGET_BYTES: SiteConfig = SiteConfig {
    name: "budgetbytes",
    base_url: "https://www.budgetbytes.com",
    seed_paths: &["/sitemap.xml"],
    recipe_url_test: |url| {
        url.starts_with("https://www.budgetbytes.com/")
            && url != "https://www.budgetbytes.com/"
            && !url.starts_with("https://www.budgetbytes.com/about")
            && !url.starts_with("https://www.budgetbytes.com/shop")
            && !url.starts_with("https://www.budgetbytes.com/contact")
            && !url.starts_with("https://www.budgetbytes.com/how-to")
            && !url.starts_with("https://www.budgetbytes.com/extra-bytes")
            && !url.starts_with("https://www.budgetbytes.com/recipes")
            && !url.starts_with("https://www.budgetbytes.com/meal-plans")
            && !url.starts_with("https://www.budgetbytes.com/category")
            && !url.contains('#')
    },
    recipe_link_selector: "article a[href], .post a[href], .archive a[href], h2 a[href], h3 a[href]",
    use_sitemap: true,
};

pub static ALL_SITES: &[&SiteConfig] = &[
    &ALLRECIPES,
    &BONAPPETIT,
    &FOOD52,
    &SIMPLY_RECIPES,
    &PIONEER_WOMAN,
    &TASTE_OF_HOME,
    &SERIOUS_EATS,
    &COOKIE_AND_KATE,
    &PINCH_OF_YUM,
    &HALF_BAKED_HARVEST,
    &LOVE_AND_LEMONS,
    &RECIPE_TIN_EATS,
    &MINIMALIST_BAKER,
    &BUDGET_BYTES,
];

pub async fn crawl(
    site: &SiteConfig,
    client: &reqwest::Client,
    tx: mpsc::Sender<String>,
) -> Result<()> {
    if site.use_sitemap {
        return crawl_sitemap(site, client, tx).await;
    }
    crawl_pages(site, client, tx).await
}

async fn crawl_sitemap(
    site: &SiteConfig,
    client: &reqwest::Client,
    tx: mpsc::Sender<String>,
) -> Result<()> {
    info!(site = site.name, "starting sitemap crawl");

    let mut seen: HashSet<String> = HashSet::new();
    let mut sitemap_queue: Vec<String> = site
        .seed_paths
        .iter()
        .map(|p| format!("{}{}", site.base_url, p))
        .collect();

    while let Some(sitemap_url) = sitemap_queue.pop() {
        info!(site = site.name, url = %sitemap_url, "fetching sitemap");

        let body = fetch_page(client, &sitemap_url, site.base_url).await?;

        let (sitemap_urls, recipe_urls) = {
            let recipe_test = site.recipe_url_test;
            tokio::task::spawn_blocking(move || parse_sitemap(&body, &recipe_test))
                .await
                .context("spawn_blocking failed")??
        };

        info!(
            site = site.name,
            sitemap = %sitemap_url,
            sub_sitemaps = sitemap_urls.len(),
            recipes = recipe_urls.len(),
            "sitemap parsed"
        );

        for next in sitemap_urls {
            if seen.insert(next.clone()) {
                sitemap_queue.push(next);
            }
        }

        for url in recipe_urls {
            if tx.send(url).await.is_err() {
                return Ok(());
            }
        }

        sleep(REQUEST_DELAY).await;
    }

    info!(
        site = site.name,
        total = seen.len(),
        "sitemap crawl complete"
    );
    Ok(())
}

fn local_name(name: &[u8]) -> &[u8] {
    if let Some(pos) = name.iter().position(|&b| b == b'}') {
        &name[pos + 1..]
    } else {
        name
    }
}

fn parse_sitemap(
    body: &str,
    recipe_test: &(dyn Fn(&str) -> bool + Send + Sync),
) -> Result<(Vec<String>, Vec<String>)> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);

    let mut sitemap_urls = Vec::new();
    let mut recipe_urls = Vec::new();
    let mut in_sitemap = false;
    let mut in_loc = false;
    let mut current_loc = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => match local_name(e.name().as_ref()) {
                b"sitemap" => in_sitemap = true,
                b"loc" => in_loc = true,
                _ => {}
            },
            Ok(Event::End(ref e)) => match local_name(e.name().as_ref()) {
                b"sitemap" => {
                    in_sitemap = false;
                    if !current_loc.is_empty() {
                        current_loc.clear();
                    }
                }
                b"url" => {
                    if !current_loc.is_empty() {
                        if recipe_test(&current_loc) {
                            recipe_urls.push(current_loc.clone());
                        }
                        current_loc.clear();
                    }
                }
                b"loc" => in_loc = false,
                _ => {}
            },
            Ok(Event::Text(ref e)) => {
                if in_loc {
                    let text = e.unescape()?;
                    if in_sitemap {
                        sitemap_urls.push(text.to_string());
                    } else {
                        current_loc = text.to_string();
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                warn!("xml parse error: {e}");
                break;
            }
            _ => {}
        }
    }

    Ok((sitemap_urls, recipe_urls))
}

async fn crawl_pages(
    site: &SiteConfig,
    client: &reqwest::Client,
    tx: mpsc::Sender<String>,
) -> Result<()> {
    let recipe_link_sel = Selector::parse(site.recipe_link_selector).unwrap();
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
        "starting page crawl",
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
        "page crawl complete"
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
            .header(ACCEPT_ENCODING, "gzip, deflate")
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
