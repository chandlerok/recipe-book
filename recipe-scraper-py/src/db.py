import json
from collections.abc import Iterator

import psycopg
import structlog
from psycopg_pool import ConnectionPool

log = structlog.get_logger()


class RecipeDB:
    def __init__(self, dsn: str) -> None:
        self._dsn = dsn
        log.info("connecting to postgres", dsn=dsn)
        self._pool = ConnectionPool(dsn, min_size=1, max_size=10, open=True)
        self._init_db()

    def _init_db(self) -> None:
        with self._pool.connection() as conn, conn.transaction():
            conn.execute("""
                CREATE TABLE IF NOT EXISTS scrape_queue (
                    id SERIAL PRIMARY KEY,
                    url VARCHAR NOT NULL UNIQUE,
                    status VARCHAR NOT NULL DEFAULT 'pending',
                    error_message VARCHAR,
                    added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    scraped_at TIMESTAMP
                )
            """)
            conn.execute("""
                CREATE TABLE IF NOT EXISTS recipes (
                    url VARCHAR PRIMARY KEY,
                    title TEXT,
                    total_time INTEGER,
                    ingredients TEXT,
                    instructions TEXT,
                    image TEXT,
                    scraped_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                )
            """)
            conn.execute("""
                ALTER TABLE recipes ADD COLUMN IF NOT EXISTS search_vector tsvector
            """)
            conn.execute("""
                CREATE INDEX IF NOT EXISTS idx_recipes_search
                ON recipes USING GIN(search_vector)
            """)
            conn.execute("""
                CREATE OR REPLACE FUNCTION recipes_search_update() RETURNS trigger AS $$
                BEGIN
                    NEW.search_vector :=
                        setweight(to_tsvector('english', COALESCE(NEW.title, '')), 'A') ||
                        setweight(to_tsvector('english', COALESCE(NEW.ingredients, '')), 'B') ||
                        setweight(to_tsvector('english', COALESCE(NEW.instructions, '')), 'C');
                    RETURN NEW;
                END;
                $$ LANGUAGE plpgsql
            """)
            conn.execute("DROP TRIGGER IF EXISTS trg_recipes_search ON recipes")
            conn.execute("""
                CREATE TRIGGER trg_recipes_search
                    BEFORE INSERT OR UPDATE ON recipes
                    FOR EACH ROW EXECUTE FUNCTION recipes_search_update()
            """)
            conn.execute("""
                UPDATE recipes SET search_vector =
                    setweight(to_tsvector('english', COALESCE(title, '')), 'A') ||
                    setweight(to_tsvector('english', COALESCE(ingredients, '')), 'B') ||
                    setweight(to_tsvector('english', COALESCE(instructions, '')), 'C')
                WHERE search_vector IS NULL
            """)

    def enqueue_url(self, url: str) -> str:
        with self._pool.connection() as conn, conn.transaction():
            existing = conn.execute(
                "SELECT status FROM scrape_queue WHERE url = %s", [url]
            ).fetchone()
            if existing:
                return existing[0]

            conn.execute("INSERT INTO scrape_queue (url) VALUES (%s)", [url])
            return "pending"

    def next_pending(self) -> tuple[int, str] | None:
        with self._pool.connection() as conn, conn.transaction():
            row = conn.execute("""
                UPDATE scrape_queue SET status = 'in_progress'
                WHERE id = (
                    SELECT id FROM scrape_queue
                    WHERE status = 'pending'
                    ORDER BY added_at ASC
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                )
                RETURNING id, url
            """).fetchone()
            if row is None:
                return None
            return (row[0], row[1])

    def mark_done(self, job_id: int) -> None:
        with self._pool.connection() as conn, conn.transaction():
            conn.execute(
                "UPDATE scrape_queue SET status = 'done', scraped_at = CURRENT_TIMESTAMP WHERE id = %s",
                [job_id],
            )

    def mark_error(self, job_id: int, error: str) -> None:
        with self._pool.connection() as conn, conn.transaction():
            conn.execute(
                "UPDATE scrape_queue SET status = 'error', error_message = %s WHERE id = %s",
                [error[:500], job_id],
            )

    def save_recipe(self, recipe: dict) -> None:
        with self._pool.connection() as conn, conn.transaction():
            conn.execute(
                """
                INSERT INTO recipes (url, title, total_time, ingredients, instructions, image)
                VALUES (%s, %s, %s, %s, %s, %s)
                ON CONFLICT (url) DO UPDATE SET
                    title = EXCLUDED.title,
                    total_time = EXCLUDED.total_time,
                    ingredients = EXCLUDED.ingredients,
                    instructions = EXCLUDED.instructions,
                    image = EXCLUDED.image,
                    scraped_at = CURRENT_TIMESTAMP
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

    def get_recipe(self, url: str) -> dict | None:
        with self._pool.connection() as conn:
            row = conn.execute(
                "SELECT url, title, total_time, ingredients, instructions, image FROM recipes WHERE url = %s",
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
        with self._pool.connection() as conn:
            rows = conn.execute(
                """
                SELECT url, title, total_time, ingredients, instructions, image,
                       ts_rank(search_vector, plainto_tsquery('english', %s)) AS score
                FROM recipes
                WHERE search_vector @@ plainto_tsquery('english', %s)
                ORDER BY score DESC
                LIMIT %s
            """,
                [query, query, limit],
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
        with self._pool.connection() as conn:
            rows = conn.execute(
                "SELECT status, COUNT(*) FROM scrape_queue GROUP BY status"
            ).fetchall()
        stats = {"pending": 0, "in_progress": 0, "done": 0, "error": 0}
        for status, count in rows:
            if status in stats:
                stats[status] = count
        return stats

    def close(self) -> None:
        self._pool.close()
        log.info("closed connection pool", dsn=self._dsn)
