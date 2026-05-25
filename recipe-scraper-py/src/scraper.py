from __future__ import annotations

import structlog

import requests
from recipe_scrapers import scrape_html

log = structlog.get_logger()

HEADERS = {
    "User-Agent": (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/125.0.0.0 Safari/537.36"
    )
}


class ScrapeError(Exception):
    pass


def scrape_recipe(url: str) -> dict:
    log.info("scraping recipe", url=url)

    try:
        response = requests.get(url, headers=HEADERS, timeout=15)
        response.raise_for_status()
    except requests.RequestException as e:
        log.warning("http error", url=url, error=str(e))
        raise ScrapeError(f"HTTP error: {e}") from e

    try:
        scraper = scrape_html(html=response.text, org_url=url)
    except Exception as e:
        log.warning("parser init error", url=url, error=str(e))
        raise ScrapeError(f"Parser init error: {e}") from e

    try:
        recipe = {
            "url": url,
            "title": scraper.title(),
            "total_time": scraper.total_time() or 0,
            "ingredients": scraper.ingredients(),
            "instructions": scraper.instructions().split("\n"),
            "image": scraper.image() or "",
        }
        log.info("scraped recipe", url=url, title=recipe["title"])
        return recipe
    except Exception as e:
        log.warning("data extraction error", url=url, error=str(e))
        raise ScrapeError(f"Data extraction error: {e}") from e
