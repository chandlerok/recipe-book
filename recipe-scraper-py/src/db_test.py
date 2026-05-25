import json
import os
import tempfile
from collections.abc import Generator

import pytest

from src.db import RecipeDB


@pytest.fixture
def db() -> Generator[RecipeDB, None, None]:
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    os.unlink(path)
    database = RecipeDB(path)
    yield database
    database.close()
    try:
        os.unlink(path)
    except OSError:
        pass


def sample_recipe(url: str = "https://example.com/test", **overrides) -> dict:
    return {
        "url": url,
        "title": "Test Recipe",
        "total_time": 30,
        "ingredients": ["item1", "item2"],
        "instructions": ["step 1", "step 2"],
        "image": "",
        **overrides,
    }


class TestQueue:
    def test_enqueue_new_url_returns_pending(self, db: RecipeDB) -> None:
        assert db.enqueue_url("https://example.com/new") == "pending"

    def test_enqueue_duplicate_returns_existing_status(self, db: RecipeDB) -> None:
        db.enqueue_url("https://example.com/dup")
        assert db.enqueue_url("https://example.com/dup") == "pending"

    def test_next_pending_returns_oldest_in_fifo_order(self, db: RecipeDB) -> None:
        db.enqueue_url("https://example.com/first")
        db.enqueue_url("https://example.com/second")

        job = db.next_pending()
        assert job is not None
        job_id, url = job
        assert url == "https://example.com/first"

    def test_next_pending_marks_in_progress(self, db: RecipeDB) -> None:
        db.enqueue_url("https://example.com/test")
        job = db.next_pending()
        assert job is not None
        job_id, _ = job

        result = db._conn.execute(
            "SELECT status FROM scrape_queue WHERE id = ?", [job_id]
        ).fetchone()
        assert result is not None
        assert result[0] == "in_progress"

    def test_next_pending_returns_none_when_empty(self, db: RecipeDB) -> None:
        assert db.next_pending() is None

    def test_mark_done_updates_status(self, db: RecipeDB) -> None:
        db.enqueue_url("https://example.com/done")
        job = db.next_pending()
        assert job is not None
        db.mark_done(job[0])

        result = db._conn.execute(
            "SELECT status FROM scrape_queue WHERE id = ?", [job[0]]
        ).fetchone()
        assert result is not None
        assert result[0] == "done"

    def test_mark_error_updates_status_and_message(self, db: RecipeDB) -> None:
        db.enqueue_url("https://example.com/error")
        job = db.next_pending()
        assert job is not None
        db.mark_error(job[0], "something broke")

        row = db._conn.execute(
            "SELECT status, error_message FROM scrape_queue WHERE id = ?",
            [job[0]],
        ).fetchone()
        assert row is not None
        assert row[0] == "error"
        assert row[1] == "something broke"

    def test_mark_error_truncates_long_messages(self, db: RecipeDB) -> None:
        db.enqueue_url("https://example.com/long")
        job = db.next_pending()
        assert job is not None
        db.mark_error(job[0], "x" * 1000)

        row = db._conn.execute(
            "SELECT error_message FROM scrape_queue WHERE id = ?", [job[0]]
        ).fetchone()
        assert row is not None
        assert len(row[0]) == 500


class TestRecipes:
    def test_save_and_get_roundtrip(self, db: RecipeDB) -> None:
        db.save_recipe(sample_recipe())
        result = db.get_recipe("https://example.com/test")
        assert result is not None
        assert result["title"] == "Test Recipe"
        assert result["total_time"] == 30
        assert result["ingredients"] == ["item1", "item2"]
        assert result["instructions"] == ["step 1", "step 2"]

    def test_get_nonexistent_returns_none(self, db: RecipeDB) -> None:
        assert db.get_recipe("https://example.com/nope") is None

    def test_save_overwrites_existing(self, db: RecipeDB) -> None:
        db.save_recipe(sample_recipe(title="Old"))
        db.save_recipe(sample_recipe(title="New"))
        result = db.get_recipe("https://example.com/test")
        assert result is not None
        assert result["title"] == "New"


class TestSearch:
    def test_search_finds_matching_recipe(self, db: RecipeDB) -> None:
        db.save_recipe(sample_recipe(title="Chicken Parmesan"))
        results = db.search("chicken")
        assert len(results) > 0
        assert results[0]["recipe"]["title"] == "Chicken Parmesan"

    def test_search_returns_empty_for_no_match(self, db: RecipeDB) -> None:
        db.save_recipe(sample_recipe(title="Chicken Parmesan"))
        results = db.search("zucchini")
        assert len(results) == 0

    def test_search_respects_limit(self, db: RecipeDB) -> None:
        for i in range(5):
            db.save_recipe(
                sample_recipe(
                    url=f"https://example.com/test{i}", title=f"Chicken Dish {i}"
                )
            )
        results = db.search("chicken", limit=2)
        assert len(results) == 2

    def test_search_scores_relevance(self, db: RecipeDB) -> None:
        db.save_recipe(
            sample_recipe(
                title="Chicken Tacos", ingredients=["chicken", "tortilla", "lime"]
            )
        )
        db.save_recipe(
            sample_recipe(
                url="https://example.com/2",
                title="Beef Stew",
                ingredients=["beef", "carrots"],
            )
        )
        results = db.search("chicken")
        assert len(results) >= 1
        assert results[0]["recipe"]["title"] == "Chicken Tacos"
        assert results[0]["score"] > 0

    def test_fts_incremental_updates_after_save(self, db: RecipeDB) -> None:
        db.save_recipe(
            sample_recipe(title="Alpha Dish", url="https://example.com/alpha")
        )
        assert len(db.search("alpha")) == 1

        db.save_recipe(sample_recipe(title="Beta Dish", url="https://example.com/beta"))
        assert len(db.search("beta")) == 1
        assert len(db.search("alpha")) == 1


class TestQueueStats:
    def test_counts_by_status(self, db: RecipeDB) -> None:
        db.enqueue_url("https://example.com/p1")
        db.enqueue_url("https://example.com/p2")
        db.enqueue_url("https://example.com/e1")
        job = db.next_pending()
        assert job is not None
        db.mark_error(job[0], "fail")

        stats = db.queue_stats()
        assert stats["pending"] == 2
        assert stats["in_progress"] == 0
        assert stats["error"] == 1
        assert stats["done"] == 0
