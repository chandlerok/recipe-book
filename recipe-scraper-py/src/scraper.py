from __future__ import annotations

import os
import random
import time
from typing import Any
from urllib.parse import urlparse

import structlog
from curl_cffi import requests as curl_requests
from curl_cffi.requests.exceptions import ConnectionError, HTTPError, Timeout
from recipe_scrapers import scrape_html

log = structlog.get_logger()

USER_AGENTS = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
]

RETRYABLE_STATUSES = {429, 500, 502, 503, 504}
MAX_RETRIES = 3
BACKOFF_FACTOR = 2.0
MIN_BACKOFF = 1.0
MAX_BACKOFF = 30.0
HTTP_TIMEOUT = 30


class ScrapeError(Exception):
    pass


def _random_user_agent() -> str:
    return random.choice(USER_AGENTS)


IMPERSONATE = "chrome110"


def _build_session() -> curl_requests.Session:
    session = curl_requests.Session(timeout=HTTP_TIMEOUT)

    proxy_url = os.getenv("PROXY_URL", "")
    if proxy_url:
        session.proxies = {"http": proxy_url, "https": proxy_url}
        log.info("proxy configured", proxy=proxy_url)

    return session


def _build_headers(url: str) -> dict[str, str]:
    domain = urlparse(url).netloc
    return {
        "User-Agent": _random_user_agent(),
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
        "Accept-Language": "en-US,en;q=0.9",
        "Accept-Encoding": "gzip, deflate",
        "Referer": f"https://{domain}/",
        "DNT": "1",
        "Connection": "keep-alive",
    }


_session: curl_requests.Session | None = None


def _get_session() -> curl_requests.Session:
    global _session
    if _session is None:
        _session = _build_session()
    return _session


def _fetch(url: str) -> curl_requests.Response:
    session = _get_session()
    headers = _build_headers(url)

    last_exception: BaseException | None = None
    for attempt in range(1 + MAX_RETRIES):
        try:
            response = session.get(
                url, headers=headers, timeout=HTTP_TIMEOUT, impersonate=IMPERSONATE
            )
            response.raise_for_status()
            return response
        except HTTPError as e:
            status = e.response.status_code if e.response is not None else None
            if status == 429:
                retry_after = (
                    e.response.headers.get("Retry-After", "5")
                    if e.response is not None
                    else "5"
                )
                try:
                    wait = int(retry_after)
                except ValueError:
                    wait = 5
                log.warning("rate limited", url=url, retry_after=wait)
                time.sleep(wait)
                continue
            if (
                status
                and status >= 400
                and status < 500
                and status not in RETRYABLE_STATUSES
            ):
                raise ScrapeError(f"HTTP {status}") from e
            last_exception = e
        except (ConnectionError, Timeout) as e:
            last_exception = e

        if attempt < MAX_RETRIES:
            sleep = min(MAX_BACKOFF, MIN_BACKOFF * (BACKOFF_FACTOR**attempt))
            jitter = random.uniform(0, sleep * 0.1)
            log.warning(
                "retry", url=url, attempt=attempt + 1, sleep=round(sleep + jitter, 1)
            )
            time.sleep(sleep + jitter)

    raise ScrapeError(
        f"HTTP error after {MAX_RETRIES + 1} attempts: {last_exception}"
    ) from last_exception


def scrape_recipe(url: str) -> dict[str, Any]:
    log.info("scraping recipe", url=url)

    response = _fetch(url)

    try:
        scraper = scrape_html(html=response.text, org_url=url)
    except Exception as e:
        log.warning("parser init error", url=url, error=str(e))
        raise ScrapeError(f"Parser init error: {e}") from e

    try:
        total_time = scraper.total_time() or 0
        if total_time == 0:
            try:
                total_time = scraper.schema.total_time() or 0
            except Exception:
                pass

        recipe: dict[str, Any] = {
            "url": url,
            "title": scraper.title(),
            "total_time": total_time,
            "ingredients": scraper.ingredients(),
            "instructions": scraper.instructions().split("\n"),
            "image": scraper.image() or "",
        }
        log.info("scraped recipe", url=url, title=recipe["title"])
        return recipe
    except Exception as e:
        log.warning("data extraction error", url=url, error=str(e))
        raise ScrapeError(f"Data extraction error: {e}") from e
