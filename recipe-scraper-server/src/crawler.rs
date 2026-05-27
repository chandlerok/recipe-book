use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
use rand::Rng;
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, CONNECTION, DNT, REFERER, USER_AGENT,
};
use scraper::{Html, Selector};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{info, warn};
use url::Url;

use crate::db::RecipeDb;

pub struct ProxyPool {
    clients: Vec<reqwest::Client>,
}

impl ProxyPool {
    pub fn from_env() -> Option<Arc<Self>> {
        let proxy_list = std::env::var("PROXY_LIST").ok();
        let proxy_url = std::env::var("PROXY_URL").ok();

        let urls: Vec<String> = match (proxy_list, proxy_url) {
            (Some(list), _) if !list.is_empty() => list
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            (_, Some(url)) if !url.is_empty() => vec![url],
            _ => return None,
        };

        if urls.is_empty() {
            return None;
        }

        let clients: Vec<reqwest::Client> = urls
            .into_iter()
            .filter_map(|u| {
                let proxy = reqwest::Proxy::all(&u).ok()?;
                reqwest::Client::builder()
                    .proxy(proxy)
                    .timeout(Duration::from_secs(30))
                    .build()
                    .ok()
            })
            .collect();

        if clients.is_empty() {
            return None;
        }

        info!("proxy pool initialized: count={}", clients.len());
        Some(Arc::new(Self { clients }))
    }

    pub fn next(&self) -> Option<&reqwest::Client> {
        if self.clients.is_empty() {
            return None;
        }
        let idx = rand::thread_rng().gen_range(0..self.clients.len());
        Some(&self.clients[idx])
    }
}

const REQUEST_DELAY: Duration = Duration::from_secs(3);
const MAX_RETRIES: u32 = 2;
const BASE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn jitter_delay(base: Duration) -> Duration {
    let extra_ms: u64 = rand::thread_rng().gen_range(0..=1000);
    base + Duration::from_millis(extra_ms)
}

static USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
];

static UA_INDEX: AtomicUsize = AtomicUsize::new(0);

fn next_user_agent() -> &'static str {
    let idx = UA_INDEX.fetch_add(1, Ordering::Relaxed);
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
        url.contains("simplyrecipes.com/")
            && !url.contains("/how-to/")
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/terms")
            && !url.contains("/privacy")
            && !url.contains('#')
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
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/feed/")
            && !url.contains("/wp-")
            && !url.contains("/comment-page-")
            && !url.contains("/terms")
            && !url.contains("/tos")
            && !url.contains("/meal-plan")
            && !url.contains("/newsletter")
            && !url.contains("/policy")
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
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/feed/")
            && !url.contains("/wp-")
            && !url.contains("/comment-page-")
            && !url.contains("/terms")
            && !url.contains("/tos")
            && !url.contains("/meal-plan")
            && !url.contains("/newsletter")
            && !url.contains("/policy")
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
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/feed/")
            && !url.contains("/wp-")
            && !url.contains("/comment-page-")
            && !url.contains("/terms")
            && !url.contains("/tos")
            && !url.contains("/meal-plan")
            && !url.contains("/newsletter")
            && !url.contains("/policy")
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
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/feed/")
            && !url.contains("/wp-")
            && !url.contains("/comment-page-")
            && !url.contains("/terms")
            && !url.contains("/tos")
            && !url.contains("/meal-plan")
            && !url.contains("/newsletter")
            && !url.contains("/policy")
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
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/feed/")
            && !url.contains("/wp-")
            && !url.contains("/comment-page-")
            && !url.contains("/terms")
            && !url.contains("/tos")
            && !url.contains("/meal-plan")
            && !url.contains("/newsletter")
            && !url.contains("/policy")
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
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/feed/")
            && !url.contains("/wp-")
            && !url.contains("/comment-page-")
            && !url.contains("/terms")
            && !url.contains("/tos")
            && !url.contains("/meal-plan")
            && !url.contains("/newsletter")
            && !url.contains("/policy")
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
            && !url.contains("/author/")
            && !url.contains("/tag/")
            && !url.contains("/page/")
            && !url.contains("/feed/")
            && !url.contains("/wp-")
            && !url.contains("/comment-page-")
            && !url.contains("/terms")
            && !url.contains("/tos")
            && !url.contains("/meal-plan")
            && !url.contains("/newsletter")
            && !url.contains("/policy")
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

pub async fn crawl_all_sites(
    db: &RecipeDb,
    client: &reqwest::Client,
    proxy_pool: Option<Arc<ProxyPool>>,
) {
    match db.try_acquire_crawl_lock().await {
        Ok(true) => info!("acquired crawl lock"),
        Ok(false) => {
            info!("crawl lock held by another instance, skipping");
            return;
        }
        Err(e) => {
            warn!(error = %e, "failed to acquire crawl lock, skipping");
            return;
        }
    }

    info!("starting crawl of {} sites", ALL_SITES.len());

    let (tx, mut rx) = mpsc::channel(1024);
    let mut handles = Vec::new();

    for site in ALL_SITES {
        let tx = tx.clone();
        let client = client.clone();
        let proxy_pool = proxy_pool.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = crawl(site, &client, tx, proxy_pool).await {
                warn!(site = site.name, error = %e, "crawl failed");
            }
        }));
    }

    drop(tx);

    while let Some(url) = rx.recv().await {
        if let Err(e) = db.enqueue_url(&url).await {
            warn!(url, error = %e, "failed to enqueue discovered url");
        }
    }

    for handle in handles {
        let _ = handle.await;
    }

    info!("crawl_all_sites complete");

    if let Err(e) = db.release_crawl_lock().await {
        warn!(error = %e, "failed to release crawl lock");
    }
}

async fn crawl(
    site: &SiteConfig,
    client: &reqwest::Client,
    tx: mpsc::Sender<String>,
    proxy_pool: Option<Arc<ProxyPool>>,
) -> Result<()> {
    if site.use_sitemap {
        return crawl_sitemap(site, client, tx, proxy_pool).await;
    }
    crawl_pages(site, client, tx, proxy_pool).await
}

async fn crawl_sitemap(
    site: &SiteConfig,
    client: &reqwest::Client,
    tx: mpsc::Sender<String>,
    proxy_pool: Option<Arc<ProxyPool>>,
) -> Result<()> {
    let site_name = site.name;
    info!(site = site_name, "starting sitemap crawl");

    let queue: Arc<tokio::sync::Mutex<Vec<String>>> = Arc::new(tokio::sync::Mutex::new(
        site.seed_paths
            .iter()
            .map(|p| format!("{}{}", site.base_url, p))
            .collect(),
    ));
    let seen: Arc<tokio::sync::Mutex<HashSet<String>>> =
        Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let last_req: Arc<tokio::sync::Mutex<Instant>> =
        Arc::new(tokio::sync::Mutex::new(Instant::now() - REQUEST_DELAY));

    loop {
        let url = {
            let mut q = queue.lock().await;
            q.pop()
        };

        let url = match url {
            Some(u) => u,
            None => {
                if in_flight.load(Ordering::Acquire) == 0 {
                    break;
                }
                sleep(Duration::from_millis(100)).await;
                continue;
            }
        };

        in_flight.fetch_add(1, Ordering::Release);

        let client = client.clone();
        let base_url = site.base_url;
        let recipe_test = site.recipe_url_test;
        let tx = tx.clone();
        let queue = queue.clone();
        let seen = seen.clone();
        let in_flight = in_flight.clone();
        let last_req = last_req.clone();
        let proxy_pool = proxy_pool.clone();

        tokio::spawn(async move {
            {
                let mut last = last_req.lock().await;
                let elapsed = last.elapsed();
                if elapsed < REQUEST_DELAY {
                    sleep(jitter_delay(REQUEST_DELAY - elapsed)).await;
                }
                *last = Instant::now();
            }

            match fetch_page(&client, &url, base_url, proxy_pool.as_deref()).await {
                Ok(body) => {
                    if let Ok(Ok((new_sitemaps, recipes))) =
                        tokio::task::spawn_blocking(move || parse_sitemap(&body, &recipe_test))
                            .await
                    {
                        info!(
                            site = site_name,
                            url = %url,
                            sub_sitemaps = new_sitemaps.len(),
                            recipes = recipes.len(),
                            "sitemap parsed"
                        );

                        {
                            let mut s = seen.lock().await;
                            let mut q = queue.lock().await;
                            for next in new_sitemaps {
                                if s.insert(next.clone()) {
                                    q.push(next);
                                }
                            }
                        }

                        for recipe_url in recipes {
                            if tx.send(recipe_url).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(site = site_name, url = %url, error = %e, "sitemap fetch failed");
                }
            }

            in_flight.fetch_sub(1, Ordering::Release);
        });
    }

    info!(site = site_name, "sitemap crawl complete");
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
    proxy_pool: Option<Arc<ProxyPool>>,
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

        let response =
            match fetch_page(client, &page_url, site.base_url, proxy_pool.as_deref()).await {
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

async fn fetch_page(
    client: &reqwest::Client,
    url: &str,
    site_base_url: &str,
    proxy_pool: Option<&ProxyPool>,
) -> Result<String> {
    let effective_client = proxy_pool.and_then(|p| p.next()).unwrap_or(client);
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=MAX_RETRIES {
        let ua = next_user_agent();

        let result = effective_client
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable_true() {
        for code in [
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            reqwest::StatusCode::BAD_GATEWAY,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            reqwest::StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(is_retryable(code), "expected {code} to be retryable");
        }
    }

    #[test]
    fn test_is_retryable_false() {
        for code in [
            reqwest::StatusCode::OK,
            reqwest::StatusCode::NOT_FOUND,
            reqwest::StatusCode::FORBIDDEN,
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::MOVED_PERMANENTLY,
        ] {
            assert!(!is_retryable(code), "expected {code} to NOT be retryable");
        }
    }

    #[test]
    fn test_local_name_strips_namespace() {
        assert_eq!(local_name(b"{http://sitemaps.org}url"), b"url");
        assert_eq!(local_name(b"{http://sitemaps.org}sitemap"), b"sitemap");
        assert_eq!(local_name(b"{ns}loc"), b"loc");
    }

    #[test]
    fn test_local_name_no_namespace() {
        assert_eq!(local_name(b"url"), b"url");
        assert_eq!(local_name(b"loc"), b"loc");
        assert_eq!(local_name(b"tag"), b"tag");
    }

    #[test]
    fn test_local_name_empty_namespace() {
        assert_eq!(local_name(b"{}tag"), b"tag");
    }

    #[test]
    fn test_local_name_empty_input() {
        assert!(local_name(b"").is_empty());
    }

    #[test]
    fn test_resolve_url_absolute() {
        assert_eq!(
            resolve_url(
                "https://example.com/page",
                "https://other.com/recipe",
                "https://example.com"
            ),
            "https://other.com/recipe"
        );
    }

    #[test]
    fn test_resolve_url_relative() {
        assert_eq!(
            resolve_url(
                "https://example.com/page/",
                "recipe/123",
                "https://example.com"
            ),
            "https://example.com/page/recipe/123"
        );
    }

    #[test]
    fn test_resolve_url_root_relative() {
        assert_eq!(
            resolve_url(
                "https://example.com/page",
                "/recipe/123",
                "https://example.com"
            ),
            "https://example.com/recipe/123"
        );
    }

    #[test]
    fn test_resolve_url_fallback() {
        let result = resolve_url("not-a-url", "/recipe/123", "https://example.com");
        assert_eq!(result, "https://example.comrecipe/123");
    }

    #[test]
    fn test_resolve_url_fallback_missing_slash() {
        let result = resolve_url("not-a-url", "recipe/123", "https://example.com");
        assert_eq!(result, "https://example.comrecipe/123");
    }

    #[test]
    fn test_resolve_url_trailing_slash() {
        assert_eq!(
            resolve_url(
                "https://example.com/",
                "//other.com/recipe",
                "https://example.com"
            ),
            "https://other.com/recipe"
        );
    }

    #[test]
    fn test_parse_sitemap_empty() {
        let (sitemaps, recipes) = parse_sitemap("", &|_| true).unwrap();
        assert!(sitemaps.is_empty());
        assert!(recipes.is_empty());
    }

    #[test]
    fn test_parse_sitemap_recipe_urls() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
            <url><loc>https://example.com/recipe/pasta</loc></url>
            <url><loc>https://example.com/recipe/pizza</loc></url>
        </urlset>"#;

        let (sitemaps, recipes) = parse_sitemap(xml, &|url| url.contains("/recipe/")).unwrap();
        assert!(sitemaps.is_empty());
        assert_eq!(recipes.len(), 2);
        assert!(recipes.contains(&"https://example.com/recipe/pasta".to_string()));
        assert!(recipes.contains(&"https://example.com/recipe/pizza".to_string()));
    }

    #[test]
    fn test_parse_sitemap_filters_by_recipe_test() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <urlset>
            <url><loc>https://example.com/recipe/pasta</loc></url>
            <url><loc>https://example.com/about</loc></url>
        </urlset>"#;

        let (_, recipes) = parse_sitemap(xml, &|url| url.contains("/recipe/")).unwrap();
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0], "https://example.com/recipe/pasta");
    }

    #[test]
    fn test_parse_sitemap_index() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <sitemapindex>
            <sitemap><loc>https://example.com/sitemap1.xml</loc></sitemap>
            <sitemap><loc>https://example.com/sitemap2.xml</loc></sitemap>
        </sitemapindex>"#;

        let (sitemaps, recipes) = parse_sitemap(xml, &|_| true).unwrap();
        assert_eq!(sitemaps.len(), 2);
        assert!(recipes.is_empty());
    }

    #[test]
    fn test_parse_sitemap_default_namespace() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
            <url><loc>https://example.com/recipe/pasta</loc></url>
        </urlset>"#;

        let (_, recipes) = parse_sitemap(xml, &|_| true).unwrap();
        assert_eq!(recipes.len(), 1);
    }

    #[test]
    fn test_parse_sitemap_mixed_index_and_urls() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <sitemapindex>
            <sitemap><loc>https://example.com/sitemap1.xml</loc></sitemap>
            <url><loc>https://example.com/recipe/pasta</loc></url>
        </sitemapindex>"#;

        let (sitemaps, recipes) = parse_sitemap(xml, &|_| true).unwrap();
        assert_eq!(sitemaps.len(), 1);
        assert_eq!(recipes.len(), 1);
    }

    #[test]
    fn test_parse_sitemap_malformed() {
        let xml = "not valid xml at all <<<>>>";
        let (sitemaps, recipes) = parse_sitemap(xml, &|_| true).unwrap();
        assert!(sitemaps.is_empty());
        assert!(recipes.is_empty());
    }

    #[test]
    fn test_allrecipes_recipe_url_test() {
        let test = ALLRECIPES.recipe_url_test;
        assert!(test("https://www.allrecipes.com/recipe/123/"));
        assert!(!test("https://www.allrecipes.com/photo/123/"));
        assert!(!test("https://www.allrecipes.com/recipe/123/#comment"));
        assert!(!test("https://www.allrecipes.com/video/123/"));
    }

    #[test]
    fn test_bonappetit_recipe_url_test() {
        let test = BONAPPETIT.recipe_url_test;
        assert!(test("https://www.bonappetit.com/recipe/pasta"));
        assert!(!test("https://www.bonappetit.com/recipe/pasta#comment"));
        assert!(!test("https://www.bonappetit.com/gallery/photos"));
    }

    #[test]
    fn test_food52_recipe_url_test() {
        let test = FOOD52.recipe_url_test;
        assert!(test("https://food52.com/recipes/123-pasta"));
        assert!(!test("https://food52.com/recipes/123#comment"));
        assert!(!test("https://food52.com/about"));
    }

    #[test]
    fn test_simplyrecipes_recipe_url_test() {
        let test = SIMPLY_RECIPES.recipe_url_test;
        assert!(test("https://www.simplyrecipes.com/pasta-recipe"));
        assert!(!test("https://www.simplyrecipes.com/how-to/cut-onions"));
        assert!(!test("https://www.simplyrecipes.com/author/jane"));
        assert!(!test("https://www.simplyrecipes.com/tag/pasta"));
        assert!(!test("https://www.simplyrecipes.com/terms"));
    }

    #[test]
    fn test_cookieandkate_recipe_url_test() {
        let test = COOKIE_AND_KATE.recipe_url_test;
        assert!(test("https://cookieandkate.com/chocolate-chip-cookies"));
        assert!(!test("https://cookieandkate.com/"));
        assert!(!test("https://cookieandkate.com/about"));
        assert!(!test("https://cookieandkate.com/author/mary"));
        assert!(!test("https://cookieandkate.com/tag/vegan"));
        assert!(!test("https://cookieandkate.com/privacy-policy"));
    }

    #[test]
    fn test_pinchofyum_recipe_url_test() {
        let test = PINCH_OF_YUM.recipe_url_test;
        assert!(test("https://pinchofyum.com/chicken-tacos"));
        assert!(!test("https://pinchofyum.com/"));
        assert!(!test("https://pinchofyum.com/about"));
        assert!(!test("https://pinchofyum.com/author/mary"));
        assert!(!test("https://pinchofyum.com/tag/vegan"));
    }

    #[test]
    fn test_halfbakedharvest_recipe_url_test() {
        let test = HALF_BAKED_HARVEST.recipe_url_test;
        assert!(test("https://www.halfbakedharvest.com/one-pot-pasta"));
        assert!(!test("https://www.halfbakedharvest.com/"));
        assert!(!test("https://www.halfbakedharvest.com/about"));
        assert!(!test("https://www.halfbakedharvest.com/cookbook"));
    }

    #[test]
    fn test_budgetbytes_recipe_url_test() {
        let test = BUDGET_BYTES.recipe_url_test;
        assert!(test("https://www.budgetbytes.com/black-bean-tacos"));
        assert!(!test("https://www.budgetbytes.com/"));
        assert!(!test("https://www.budgetbytes.com/about"));
        assert!(!test("https://www.budgetbytes.com/how-to/cook-rice"));
        assert!(!test("https://www.budgetbytes.com/meal-plans/week1"));
    }

    #[test]
    fn test_resolve_url_handles_empty_href() {
        let result = resolve_url("https://example.com/page", "", "https://example.com");
        assert_eq!(result, "https://example.com/page");
    }

    #[test]
    fn test_resolve_url_preserves_query_params() {
        assert_eq!(
            resolve_url("https://example.com/page", "?page=2", "https://example.com"),
            "https://example.com/page?page=2"
        );
    }

    #[tokio::test]
    async fn test_fetch_page_retries_on_429() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .expect(1..)
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), &mock_server.uri(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_page_succeeds_on_200() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("recipe content"))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), &mock_server.uri(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "recipe content");
    }

    #[tokio::test]
    async fn test_fetch_page_bails_on_404() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), &mock_server.uri(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_page_retries_on_500_then_succeeds() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("success"))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), &mock_server.uri(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }
}
