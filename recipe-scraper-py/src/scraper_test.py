from unittest.mock import MagicMock, patch

import pytest
import requests

from src.scraper import RETRYABLE_STATUSES, ScrapeError, _fetch, scrape_recipe


@pytest.fixture(autouse=True)
def _reset_session() -> None:
    import src.scraper as mod

    mod._session = None


class TestFetch:
    def test_http_4xx_raises_scrape_error(self) -> None:
        err_response = MagicMock()
        err_response.status_code = 404
        err_response.raise_for_status.side_effect = requests.HTTPError(
            response=err_response
        )

        with (
            patch("src.scraper._get_session") as mock_session_fn,
            patch("src.scraper._build_headers", return_value={"User-Agent": "test"}),
        ):
            mock_session = MagicMock()
            mock_session.get.return_value = err_response
            mock_session_fn.return_value = mock_session

            with pytest.raises(ScrapeError, match="HTTP 404"):
                _fetch("https://example.com/recipe")

    def test_http_5xx_retries_then_raises(self) -> None:
        err_response = MagicMock()
        err_response.status_code = 500
        err_response.raise_for_status.side_effect = requests.HTTPError(
            response=err_response
        )

        with (
            patch("src.scraper._get_session") as mock_session_fn,
            patch("src.scraper._build_headers", return_value={"User-Agent": "test"}),
            patch("src.scraper.time.sleep") as mock_sleep,
        ):
            mock_session = MagicMock()
            mock_session.get.return_value = err_response
            mock_session_fn.return_value = mock_session

            with pytest.raises(ScrapeError, match="HTTP error after"):
                _fetch("https://example.com/recipe")

            assert mock_session.get.call_count == 1 + 3

    def test_http_429_retries_after_delay(self) -> None:
        err_response = MagicMock()
        err_response.status_code = 429
        err_response.headers = {"Retry-After": "2"}
        err_response.raise_for_status.side_effect = requests.HTTPError(
            response=err_response
        )

        with (
            patch("src.scraper._get_session") as mock_session_fn,
            patch("src.scraper._build_headers", return_value={"User-Agent": "test"}),
            patch("src.scraper.time.sleep") as mock_sleep,
        ):
            mock_session = MagicMock()
            mock_session.get.return_value = err_response
            mock_session_fn.return_value = mock_session

            with pytest.raises(ScrapeError, match="HTTP error after"):
                _fetch("https://example.com/recipe")

            mock_sleep.assert_any_call(2)

    def test_connection_error_retries_then_raises(self) -> None:
        with (
            patch("src.scraper._get_session") as mock_session_fn,
            patch("src.scraper._build_headers", return_value={"User-Agent": "test"}),
            patch("src.scraper.time.sleep") as mock_sleep,
        ):
            mock_session = MagicMock()
            mock_session.get.side_effect = requests.ConnectionError("no network")
            mock_session_fn.return_value = mock_session

            with pytest.raises(ScrapeError, match="HTTP error after"):
                _fetch("https://example.com/recipe")

            assert mock_session.get.call_count == 1 + 3

    def test_retries_exhaustive_on_all_retryable_statuses(self) -> None:
        retryable = {429, 500, 502, 503, 504}
        assert retryable == RETRYABLE_STATUSES

    def test_timeout_retries_then_raises(self) -> None:
        with (
            patch("src.scraper._get_session") as mock_session_fn,
            patch("src.scraper._build_headers", return_value={"User-Agent": "test"}),
            patch("src.scraper.time.sleep") as mock_sleep,
        ):
            mock_session = MagicMock()
            mock_session.get.side_effect = requests.Timeout("timed out")
            mock_session_fn.return_value = mock_session

            with pytest.raises(ScrapeError, match="HTTP error after"):
                _fetch("https://example.com/recipe")

            assert mock_session.get.call_count == 1 + 3

    def test_successful_fetch_returns_response(self) -> None:
        with (
            patch("src.scraper._get_session") as mock_session_fn,
            patch("src.scraper._build_headers", return_value={"User-Agent": "test"}),
        ):
            mock_response = MagicMock()
            mock_response.raise_for_status.return_value = None
            mock_session = MagicMock()
            mock_session.get.return_value = mock_response
            mock_session_fn.return_value = mock_session

            result = _fetch("https://example.com/recipe")
            assert result is mock_response
            mock_session.get.assert_called_once()

    def test_success_after_retries(self) -> None:
        err_response = MagicMock()
        err_response.status_code = 503
        err_response.raise_for_status.side_effect = requests.HTTPError(
            response=err_response
        )

        ok_response = MagicMock()
        ok_response.raise_for_status.return_value = None

        with (
            patch("src.scraper._get_session") as mock_session_fn,
            patch("src.scraper._build_headers", return_value={"User-Agent": "test"}),
            patch("src.scraper.time.sleep") as mock_sleep,
        ):
            mock_session = MagicMock()
            mock_session.get.side_effect = [
                err_response,
                ok_response,
            ]
            mock_session_fn.return_value = mock_session

            result = _fetch("https://example.com/recipe")
            assert result is ok_response
            assert mock_session.get.call_count == 2


class TestScrapeRecipe:
    @patch("src.scraper._fetch")
    def test_http_error_raises_scrape_error(self, mock_fetch: MagicMock) -> None:
        mock_fetch.side_effect = ScrapeError("HTTP error after 4 attempts")
        with pytest.raises(ScrapeError, match="HTTP error after 4 attempts"):
            scrape_recipe("https://example.com/recipe")

    @patch("src.scraper._fetch")
    @patch("src.scraper.scrape_html")
    def test_parser_error_raises_scrape_error(
        self, mock_scrape_html: MagicMock, mock_fetch: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.text = "<html></html>"
        mock_fetch.return_value = mock_response
        mock_scrape_html.side_effect = RuntimeError("parse fail")
        with pytest.raises(ScrapeError, match="Parser init error"):
            scrape_recipe("https://example.com/recipe")

    @patch("src.scraper._fetch")
    @patch("src.scraper.scrape_html")
    def test_data_extraction_error_raises_scrape_error(
        self, mock_scrape_html: MagicMock, mock_fetch: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.text = "<html></html>"
        mock_fetch.return_value = mock_response

        mock_scraper = MagicMock()
        mock_scraper.title.side_effect = AttributeError("no title")
        mock_scrape_html.return_value = mock_scraper
        with pytest.raises(ScrapeError, match="Data extraction error"):
            scrape_recipe("https://example.com/recipe")

    @patch("src.scraper._fetch")
    @patch("src.scraper.scrape_html")
    def test_successful_scrape_returns_dict(
        self, mock_scrape_html: MagicMock, mock_fetch: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.text = "<html></html>"
        mock_fetch.return_value = mock_response

        mock_scraper = MagicMock()
        mock_scraper.title.return_value = "Fish Tacos"
        mock_scraper.total_time.return_value = 30
        mock_scraper.ingredients.return_value = ["fish", "tortilla"]
        mock_scraper.instructions.return_value = "step 1\nstep 2"
        mock_scraper.image.return_value = "https://img.example.com/photo.jpg"
        mock_scraper.url = "https://example.com/fish-tacos"
        mock_scrape_html.return_value = mock_scraper

        result = scrape_recipe("https://example.com/fish-tacos")
        assert result["url"] == "https://example.com/fish-tacos"
        assert result["title"] == "Fish Tacos"
        assert result["total_time"] == 30
        assert result["ingredients"] == ["fish", "tortilla"]
        assert result["instructions"] == ["step 1", "step 2"]
        assert result["image"] == "https://img.example.com/photo.jpg"

    @patch("src.scraper._fetch")
    @patch("src.scraper.scrape_html")
    def test_null_total_time_defaults_to_zero(
        self, mock_scrape_html: MagicMock, mock_fetch: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.text = "<html></html>"
        mock_fetch.return_value = mock_response

        mock_scraper = MagicMock()
        mock_scraper.title.return_value = "Quick Dish"
        mock_scraper.total_time.return_value = None
        mock_scraper.ingredients.return_value = []
        mock_scraper.instructions.return_value = "do stuff"
        mock_scraper.image.return_value = None
        mock_scraper.url = "https://example.com/quick"
        mock_scrape_html.return_value = mock_scraper

        result = scrape_recipe("https://example.com/quick")
        assert result["total_time"] == 0
        assert result["image"] == ""


class TestUserAgentRotation:
    def test_each_request_uses_different_user_agent(self) -> None:
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None

        with (
            patch("src.scraper._get_session") as mock_session_fn,
        ):
            mock_session = MagicMock()
            mock_session.get.return_value = mock_response
            mock_session_fn.return_value = mock_session

            user_agents_seen = set()
            for _ in range(50):
                mock_session.get.reset_mock()
                _fetch("https://example.com/recipe")
                _, kwargs = mock_session.get.call_args
                ua = kwargs["headers"]["User-Agent"]
                user_agents_seen.add(ua)

        assert len(user_agents_seen) >= 2
