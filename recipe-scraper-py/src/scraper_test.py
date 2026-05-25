from unittest.mock import MagicMock, patch

import pytest
import requests

from src.scraper import ScrapeError, scrape_recipe


class TestScrapeRecipe:
    @patch("src.scraper.requests.get")
    def test_http_error_raises_scrape_error(self, mock_get: MagicMock) -> None:
        mock_get.side_effect = requests.ConnectionError("no network")
        with pytest.raises(ScrapeError, match="HTTP error"):
            scrape_recipe("https://example.com/recipe")

    @patch("src.scraper.requests.get")
    def test_http_404_raises_scrape_error(self, mock_get: MagicMock) -> None:
        mock_response = MagicMock()
        mock_response.raise_for_status.side_effect = requests.HTTPError("404")
        mock_get.return_value = mock_response
        with pytest.raises(ScrapeError, match="HTTP error"):
            scrape_recipe("https://example.com/recipe")

    @patch("src.scraper.requests.get")
    @patch("src.scraper.scrape_html")
    def test_parser_error_raises_scrape_error(
        self, mock_scrape_html: MagicMock, mock_get: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None
        mock_response.text = "<html></html>"
        mock_get.return_value = mock_response
        mock_scrape_html.side_effect = RuntimeError("parse fail")
        with pytest.raises(ScrapeError, match="Parser init error"):
            scrape_recipe("https://example.com/recipe")

    @patch("src.scraper.requests.get")
    @patch("src.scraper.scrape_html")
    def test_data_extraction_error_raises_scrape_error(
        self, mock_scrape_html: MagicMock, mock_get: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None
        mock_response.text = "<html></html>"
        mock_get.return_value = mock_response

        mock_scraper = MagicMock()
        mock_scraper.title.side_effect = AttributeError("no title")
        mock_scrape_html.return_value = mock_scraper
        with pytest.raises(ScrapeError, match="Data extraction error"):
            scrape_recipe("https://example.com/recipe")

    @patch("src.scraper.requests.get")
    @patch("src.scraper.scrape_html")
    def test_successful_scrape_returns_dict(
        self, mock_scrape_html: MagicMock, mock_get: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None
        mock_response.text = "<html></html>"
        mock_get.return_value = mock_response

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

    @patch("src.scraper.requests.get")
    @patch("src.scraper.scrape_html")
    def test_null_total_time_defaults_to_zero(
        self, mock_scrape_html: MagicMock, mock_get: MagicMock
    ) -> None:
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None
        mock_response.text = "<html></html>"
        mock_get.return_value = mock_response

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
