import json
from pathlib import Path

import duckdb
import structlog


log = structlog.get_logger()


class RecipeDB:
    def __init__(self, db_path: str = "recipes.db") -> None:
        self.db_path = str(Path(db_path).resolve())
        log.info("opening database", path=self.db_path)
        self._conn = duckdb.connect(self.db_path)
        self._init_db()

    def _init_db(self) -> None:
        self._conn.execute("INSTALL fts;")
        self._conn.execute("LOAD fts;")

        self._conn.execute("""
            CREATE SEQUENCE IF NOT EXISTS scrape_queue_id_seq;
        """)
        self._conn.execute("""
            CREATE TABLE IF NOT EXISTS scrape_queue (
                id INTEGER PRIMARY KEY DEFAULT nextval('scrape_queue_id_seq'),
                url VARCHAR UNIQUE NOT NULL,
                status VARCHAR NOT NULL DEFAULT 'pending',
                error_message VARCHAR,
                added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                scraped_at TIMESTAMP,
            )
        """)

        self._conn.execute("""
            CREATE TABLE IF NOT EXISTS recipes (
                url VARCHAR PRIMARY KEY,
                title VARCHAR,
                total_time INTEGER,
                ingredients VARCHAR,
                instructions VARCHAR,
                image VARCHAR,
                scraped_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            )
        """)

        try:
            self._conn.execute(
                "PRAGMA create_fts_index('recipes', 'url', 'title', 'ingredients', 'instructions')"
            )
        except Exception:
            pass

    def enqueue_url(self, url: str) -> str:
        existing = self._conn.execute(
            "SELECT status FROM scrape_queue WHERE url = ?", [url]
        ).fetchone()
        if existing:
            return existing[0]

        self._conn.execute("INSERT INTO scrape_queue (url) VALUES (?)", [url])
        return "pending"

    def next_pending(self) -> tuple[int, str] | None:
        row = self._conn.execute("""
            SELECT id, url FROM scrape_queue
            WHERE status = 'pending'
            ORDER BY added_at ASC
            LIMIT 1
        """).fetchone()
        if row is None:
            return None
        job_id, url = row[0], row[1]
        self._conn.execute(
            "UPDATE scrape_queue SET status = 'in_progress' WHERE id = ?",
            [job_id],
        )
        return (job_id, url)

    def mark_done(self, job_id: int) -> None:
        self._conn.execute(
            "UPDATE scrape_queue SET status = 'done', scraped_at = CURRENT_TIMESTAMP WHERE id = ?",
            [job_id],
        )

    def mark_error(self, job_id: int, error: str) -> None:
        self._conn.execute(
            "UPDATE scrape_queue SET status = 'error', error_message = ? WHERE id = ?",
            [error[:500], job_id],
        )

    def save_recipe(self, recipe: dict) -> None:
        self._conn.execute(
            """
            INSERT OR REPLACE INTO recipes (url, title, total_time, ingredients, instructions, image)
            VALUES (?, ?, ?, ?, ?, ?)
        """,
            [
                recipe["url"],
                recipe.get("title", ""),
                recipe.get("total_time", 0),
                json.dumps(recipe.get("ingredients", [])),
                json.dumps(recipe.get("instructions", [])),
                recipe.get("image", ""),
            ],
        )
        self._conn.execute(
            "PRAGMA create_fts_index('recipes', 'url', 'title', 'ingredients', 'instructions', overwrite=1)",
        )

    def get_recipe(self, url: str) -> dict | None:
        row = self._conn.execute(
            "SELECT url, title, total_time, ingredients, instructions, image FROM recipes WHERE url = ?",
            [url],
        ).fetchone()
        if row is None:
            return None
        return {
            "url": row[0],
            "title": row[1],
            "total_time": row[2],
            "ingredients": json.loads(row[3]) if row[3] else [],
            "instructions": json.loads(row[4]) if row[4] else [],
            "image": row[5],
        }

    def search(self, query: str, limit: int = 20) -> list[dict]:
        rows = self._conn.execute(
            """
            SELECT r.url, r.title, r.total_time, r.ingredients, r.instructions, r.image, sq.score
            FROM (
                SELECT *, fts_main_recipes.match_bm25(url, ?) AS score
                FROM recipes
            ) sq
            JOIN recipes r ON r.url = sq.url
            WHERE sq.score IS NOT NULL
            ORDER BY sq.score DESC
            LIMIT ?
        """,
            [query, limit],
        ).fetchall()

        return [
            {
                "recipe": {
                    "url": r[0],
                    "title": r[1],
                    "total_time": r[2],
                    "ingredients": json.loads(r[3]) if r[3] else [],
                    "instructions": json.loads(r[4]) if r[4] else [],
                    "image": r[5],
                },
                "score": r[6],
            }
            for r in rows
        ]

    def queue_stats(self) -> dict:
        rows = self._conn.execute("""
            SELECT status, COUNT(*) FROM scrape_queue GROUP BY status
        """).fetchall()
        stats = {"pending": 0, "in_progress": 0, "done": 0, "error": 0}
        for status, count in rows:
            if status in stats:
                stats[status] = count
        return stats

    def close(self) -> None:
        self._conn.close()
        log.info("closed database", path=self.db_path)
