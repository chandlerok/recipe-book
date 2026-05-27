use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use rand::Rng;
use recipe_scraper::{Extract, SchemaOrgEntry, Scrape};
use tracing::{info, warn};
use url::Url;

pub struct ProxyPool {
    proxies: Vec<wreq::Proxy>,
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

        let proxies: Vec<wreq::Proxy> = urls
            .into_iter()
            .filter_map(|u| wreq::Proxy::all(&u).ok())
            .collect();

        if proxies.is_empty() {
            return None;
        }

        info!("proxy pool initialized: count={}", proxies.len());
        Some(Arc::new(Self { proxies }))
    }

    pub fn next(&self) -> Option<&wreq::Proxy> {
        if self.proxies.is_empty() {
            return None;
        }
        let idx = rand::thread_rng().gen_range(0..self.proxies.len());
        Some(&self.proxies[idx])
    }
}

const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
];

const MAX_RETRIES: u32 = 3;
const BASE_BACKOFF: f64 = 1.0;
const MAX_BACKOFF: f64 = 30.0;
pub const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

static UA_INDEX: AtomicUsize = AtomicUsize::new(0);

fn next_user_agent() -> &'static str {
    let idx = UA_INDEX.fetch_add(1, Ordering::Relaxed);
    USER_AGENTS[idx % USER_AGENTS.len()]
}

#[derive(Debug, Clone)]
pub struct ScrapedRecipe {
    pub url: String,
    pub title: String,
    pub total_time: i32,
    pub ingredients: Vec<String>,
    pub instructions: Vec<String>,
    pub image: String,
}

fn is_retryable_status(status: wreq::StatusCode) -> bool {
    matches!(
        status,
        wreq::StatusCode::TOO_MANY_REQUESTS
            | wreq::StatusCode::INTERNAL_SERVER_ERROR
            | wreq::StatusCode::BAD_GATEWAY
            | wreq::StatusCode::SERVICE_UNAVAILABLE
            | wreq::StatusCode::GATEWAY_TIMEOUT
    )
}

pub async fn fetch_page(
    client: &wreq::Client,
    url: &str,
    proxy: Option<&wreq::Proxy>,
) -> Result<String> {
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=MAX_RETRIES {
        let ua = next_user_agent();
        let parsed = Url::parse(url).context("invalid URL")?;
        let domain = parsed.host_str().map(|h| h.to_string()).unwrap_or_default();
        let referer = format!("https://{}/", domain);

        let mut req = client
            .get(url)
            .header("User-Agent", ua)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Accept-Encoding", "gzip, deflate")
            .header("Cache-Control", "no-cache")
            .header("DNT", "1")
            .header("Connection", "keep-alive")
            .header("Referer", &referer)
            .timeout(HTTP_TIMEOUT);

        if let Some(proxy) = proxy {
            req = req.proxy(proxy.clone());
        }

        let result = req.send().await;

        match result {
            Ok(response) if response.status().is_success() => {
                return response
                    .text()
                    .await
                    .context("failed to read response body");
            }
            Ok(response) => {
                let status = response.status();
                if status == wreq::StatusCode::TOO_MANY_REQUESTS {
                    let retry_after = response
                        .headers()
                        .get("Retry-After")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(5);
                    warn!(url, retry_after, "rate limited");
                    tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;
                    last_error = Some(anyhow::anyhow!("HTTP 429"));
                    continue;
                }
                if status.is_client_error() && !is_retryable_status(status) {
                    anyhow::bail!("HTTP {status}");
                }
                last_error = Some(anyhow::anyhow!("HTTP {status}"));
            }
            Err(e) => {
                last_error = Some(anyhow::anyhow!(e));
            }
        }

        if attempt < MAX_RETRIES {
            let backoff = BASE_BACKOFF * 2.0f64.powi(attempt as i32);
            let delay = backoff.min(MAX_BACKOFF);
            let jitter: f64 = rand::thread_rng().gen_range(0.0..delay * 0.1);
            let sleep_dur = std::time::Duration::from_secs_f64(delay + jitter);
            warn!(
                url,
                attempt = attempt + 1,
                delay_ms = sleep_dur.as_millis(),
                "retrying"
            );
            tokio::time::sleep(sleep_dur).await;
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("fetch failed after {} attempts", MAX_RETRIES + 1)))
}

pub fn parse_recipe(html: &str, url: &str) -> Result<ScrapedRecipe> {
    let entries = SchemaOrgEntry::scrape_html(html);
    let recipes: Vec<recipe_scraper::SchemaOrgRecipe> =
        entries.iter().flat_map(|e| e.extract_recipes()).collect();

    let recipe = recipes
        .first()
        .ok_or_else(|| anyhow::anyhow!("no recipe found in HTML"))?;

    let title = recipe.name().to_string();

    let ingredients: Vec<String> = recipe.ingredients().clone().into_iter().collect();

    let instructions: Vec<String> = match recipe.directions() {
        Some(list) => match list.directions() {
            Some(dirs) => {
                let all: Vec<String> = dirs.iter().map(|d| d.to_string()).collect();
                if all.len() == 1 && all[0].contains('\n') {
                    all[0].split('\n').map(|s| s.to_string()).collect()
                } else {
                    all
                }
            }
            None => Vec::new(),
        },
        None => Vec::new(),
    };

    let total_time = recipe
        .total_time()
        .as_ref()
        .and_then(|m| m.duration())
        .map(|d| {
            let minutes = d.hour * 60.0 + d.minute + d.second / 60.0;
            minutes.round() as i32
        })
        .unwrap_or(0);

    let doc = scraper::Html::parse_document(html);
    let og_selector =
        scraper::Selector::parse(r#"meta[property="og:image"], meta[name="og:image"]"#).unwrap();
    let image = doc
        .select(&og_selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(String::from)
        .unwrap_or_default();

    info!("scraped recipe: url={url} title={title}");

    Ok(ScrapedRecipe {
        url: url.to_string(),
        title,
        total_time,
        ingredients,
        instructions,
        image,
    })
}

pub async fn scrape_recipe(
    client: &wreq::Client,
    url: &str,
    proxy: Option<&wreq::Proxy>,
) -> Result<ScrapedRecipe> {
    info!(url, "scraping recipe");
    let html = fetch_page(client, url, proxy).await?;
    parse_recipe(&html, url)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const RECIPE_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<meta property="og:image" content="https://example.com/image.jpg" />
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "Recipe",
  "name": "Test Recipe",
  "description": "A test recipe",
  "totalTime": "PT30M",
  "recipeIngredient": ["item1", "item2"],
  "recipeInstructions": "step 1\nstep 2"
}
</script>
</head>
<body></body>
</html>"#;

    const RECIPE_HTML_SINGLE_INSTRUCTION: &str = r#"<!DOCTYPE html>
<html>
<head>
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "Recipe",
  "name": "Simple Recipe",
  "description": "Simple",
  "totalTime": "PT15M",
  "recipeIngredient": ["salt"],
  "recipeInstructions": "Just do it"
}
</script>
</head>
<body></body>
</html>"#;

    const RECIPE_HTML_COMPLEX_INSTRUCTIONS: &str = r#"<!DOCTYPE html>
<html>
<head>
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "Recipe",
  "name": "Complex Dish",
  "description": "A complex dish",
  "recipeIngredient": "Single ingredient line",
  "recipeInstructions": [
    {"@type": "HowToStep", "text": "First step"},
    {"@type": "HowToStep", "text": "Second step"}
  ]
}
</script>
</head>
<body></body>
</html>"#;

    #[test]
    fn test_is_retryable_true() {
        for code in [
            wreq::StatusCode::TOO_MANY_REQUESTS,
            wreq::StatusCode::INTERNAL_SERVER_ERROR,
            wreq::StatusCode::BAD_GATEWAY,
            wreq::StatusCode::SERVICE_UNAVAILABLE,
            wreq::StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(is_retryable_status(code), "expected {code} to be retryable");
        }
    }

    #[test]
    fn test_is_retryable_false() {
        for code in [
            wreq::StatusCode::OK,
            wreq::StatusCode::NOT_FOUND,
            wreq::StatusCode::FORBIDDEN,
            wreq::StatusCode::UNAUTHORIZED,
            wreq::StatusCode::MOVED_PERMANENTLY,
        ] {
            assert!(
                !is_retryable_status(code),
                "expected {code} to NOT be retryable"
            );
        }
    }

    #[test]
    fn test_parse_recipe_success() {
        let result = parse_recipe(RECIPE_HTML, "https://example.com/test");
        assert!(result.is_ok());
        let recipe = result.unwrap();
        assert_eq!(recipe.title, "Test Recipe");
        assert_eq!(recipe.total_time, 30);
        assert_eq!(recipe.ingredients, vec!["item1", "item2"]);
        assert_eq!(recipe.instructions, vec!["step 1", "step 2"]);
        assert_eq!(recipe.image, "https://example.com/image.jpg");
    }

    #[test]
    fn test_parse_recipe_no_ld_json() {
        let result = parse_recipe("<html></html>", "https://example.com/nope");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no recipe found"));
    }

    #[test]
    fn test_parse_recipe_single_instruction() {
        let result = parse_recipe(RECIPE_HTML_SINGLE_INSTRUCTION, "https://example.com/simple");
        assert!(result.is_ok());
        let recipe = result.unwrap();
        assert_eq!(recipe.title, "Simple Recipe");
        assert_eq!(recipe.total_time, 15);
        assert_eq!(recipe.ingredients, vec!["salt"]);
        assert_eq!(recipe.instructions, vec!["Just do it"]);
        assert_eq!(recipe.image, "");
    }

    #[test]
    fn test_parse_recipe_no_image() {
        let result = parse_recipe(RECIPE_HTML_SINGLE_INSTRUCTION, "https://example.com/noimg");
        assert!(result.is_ok());
        let recipe = result.unwrap();
        assert_eq!(recipe.image, "");
    }

    #[test]
    fn test_parse_recipe_complex_instructions() {
        let result = parse_recipe(
            RECIPE_HTML_COMPLEX_INSTRUCTIONS,
            "https://example.com/complex",
        );
        assert!(result.is_ok());
        let recipe = result.unwrap();
        assert_eq!(recipe.title, "Complex Dish");
        assert_eq!(recipe.instructions, vec!["First step", "Second step"]);
    }

    #[tokio::test]
    async fn test_fetch_page_succeeds_on_200() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("recipe content"))
            .mount(&mock_server)
            .await;

        let client = wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome110)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), None).await;
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

        let client = wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome110)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_page_retries_then_fails() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome110)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_page_succeeds_after_retries() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string("success after retry"),
            )
            .mount(&mock_server)
            .await;

        let client = wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome110)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success after retry");
    }

    #[tokio::test]
    async fn test_fetch_page_retries_on_429() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .mount(&mock_server)
            .await;

        let client = wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome110)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let result = fetch_page(&client, &mock_server.uri(), None).await;
        assert!(result.is_err());
    }
}
