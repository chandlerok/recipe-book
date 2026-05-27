use anyhow::{Context, Result};
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use tracing::info;

use crate::models::{QueueStats, Recipe, SearchHit};
use crate::scraper::ScrapedRecipe;

#[derive(Clone)]
pub struct RecipeDb {
    pool: sqlx::PgPool,
}

impl RecipeDb {
    pub async fn new(dsn: &str) -> Result<Self> {
        info!("connecting to postgres: dsn={dsn}");
        let pool = PgPoolOptions::new()
            .min_connections(1)
            .max_connections(20)
            .connect(dsn)
            .await
            .context("failed to connect to PostgreSQL")?;

        let db = Self { pool };
        db.init_db().await?;
        Ok(db)
    }

    async fn init_db(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS scrape_queue (
                id SERIAL PRIMARY KEY,
                url VARCHAR NOT NULL UNIQUE,
                status VARCHAR NOT NULL DEFAULT 'pending',
                error_message VARCHAR,
                added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                scraped_at TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS recipes (
                url VARCHAR PRIMARY KEY,
                title TEXT,
                total_time INTEGER,
                ingredients TEXT,
                instructions TEXT,
                image TEXT,
                scraped_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("ALTER TABLE recipes ADD COLUMN IF NOT EXISTS search_vector tsvector")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_recipes_search ON recipes USING GIN(search_vector)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_trgm")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_recipes_title_trgm
               ON recipes USING GIN (title gin_trgm_ops)"#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_recipes_ingredients_trgm
               ON recipes USING GIN (ingredients gin_trgm_ops)"#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE OR REPLACE FUNCTION recipes_search_update() RETURNS trigger AS $$
            BEGIN
                NEW.search_vector :=
                    setweight(to_tsvector('english', COALESCE(NEW.title, '')), 'A') ||
                    setweight(to_tsvector('english', COALESCE(NEW.ingredients, '')), 'B') ||
                    setweight(to_tsvector('english', COALESCE(NEW.instructions, '')), 'C');
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("DROP TRIGGER IF EXISTS trg_recipes_search ON recipes")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TRIGGER trg_recipes_search
                BEFORE INSERT OR UPDATE ON recipes
                FOR EACH ROW EXECUTE FUNCTION recipes_search_update()
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            UPDATE recipes SET search_vector =
                setweight(to_tsvector('english', COALESCE(title, '')), 'A') ||
                setweight(to_tsvector('english', COALESCE(ingredients, '')), 'B') ||
                setweight(to_tsvector('english', COALESCE(instructions, '')), 'C')
            WHERE search_vector IS NULL
            "#,
        )
        .execute(&self.pool)
        .await?;

        info!("database initialized");
        Ok(())
    }

    pub async fn enqueue_url(&self, url: &str) -> Result<String> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT status FROM scrape_queue WHERE url = $1")
                .bind(url)
                .fetch_optional(&self.pool)
                .await?;

        if let Some(status) = existing {
            return Ok(status);
        }

        sqlx::query("INSERT INTO scrape_queue (url) VALUES ($1)")
            .bind(url)
            .execute(&self.pool)
            .await?;

        Ok("pending".to_string())
    }

    pub async fn next_pending(&self) -> Result<Option<(i32, String)>> {
        let row = sqlx::query(
            r#"
            UPDATE scrape_queue SET status = 'in_progress'
            WHERE id = (
                SELECT id FROM scrape_queue
                WHERE status = 'pending'
                ORDER BY added_at ASC
                LIMIT 1
                FOR UPDATE SKIP LOCKED
            )
            RETURNING id, url
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| (r.get("id"), r.get("url"))))
    }

    #[allow(dead_code)]
    pub async fn mark_done(&self, job_id: i32) -> Result<()> {
        sqlx::query(
            "UPDATE scrape_queue SET status = 'done', scraped_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_error(&self, job_id: i32, error: &str) -> Result<()> {
        let error = &error[..error.len().min(500)];
        sqlx::query("UPDATE scrape_queue SET status = 'error', error_message = $1 WHERE id = $2")
            .bind(error)
            .bind(job_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn save_recipe(&self, recipe: &ScrapedRecipe) -> Result<()> {
        let ingredients = serde_json::to_string(&recipe.ingredients)?;
        let instructions = serde_json::to_string(&recipe.instructions)?;

        sqlx::query(
            r#"
            INSERT INTO recipes (url, title, total_time, ingredients, instructions, image)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (url) DO UPDATE SET
                title = EXCLUDED.title,
                total_time = EXCLUDED.total_time,
                ingredients = EXCLUDED.ingredients,
                instructions = EXCLUDED.instructions,
                image = EXCLUDED.image,
                scraped_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&recipe.url)
        .bind(&recipe.title)
        .bind(recipe.total_time)
        .bind(&ingredients)
        .bind(&instructions)
        .bind(&recipe.image)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn save_recipe_and_mark_done(
        &self,
        recipe: &ScrapedRecipe,
        job_id: i32,
    ) -> Result<()> {
        let ingredients = serde_json::to_string(&recipe.ingredients)?;
        let instructions = serde_json::to_string(&recipe.instructions)?;

        let mut tx = self.pool.begin().await?;

        sqlx::query(
            r#"
            INSERT INTO recipes (url, title, total_time, ingredients, instructions, image)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (url) DO UPDATE SET
                title = EXCLUDED.title,
                total_time = EXCLUDED.total_time,
                ingredients = EXCLUDED.ingredients,
                instructions = EXCLUDED.instructions,
                image = EXCLUDED.image,
                scraped_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&recipe.url)
        .bind(&recipe.title)
        .bind(recipe.total_time)
        .bind(&ingredients)
        .bind(&instructions)
        .bind(&recipe.image)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE scrape_queue SET status = 'done', scraped_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(job_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_recipe(&self, url: &str) -> Result<Option<Recipe>> {
        let row = sqlx::query(
            r#"
            SELECT url, title, total_time, ingredients, instructions, image
            FROM recipes WHERE url = $1
            "#,
        )
        .bind(url)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let ingredients_str: String = r.get("ingredients");
            let instructions_str: String = r.get("instructions");

            let ingredients: Vec<String> =
                serde_json::from_str(&ingredients_str).unwrap_or_default();
            let instructions: Vec<String> =
                serde_json::from_str(&instructions_str).unwrap_or_default();

            Recipe {
                url: r.get("url"),
                title: r.get("title"),
                total_time: r.get("total_time"),
                ingredients,
                instructions,
                image: r.get("image"),
            }
        }))
    }

    pub async fn search(&self, query: &str, limit: i32) -> Result<Vec<SearchHit>> {
        let pattern = format!("%{}%", query);
        let rows = sqlx::query(
            r#"
            SELECT url, title, total_time, ingredients, instructions, image,
                   COALESCE(ts_rank(search_vector, websearch_to_tsquery('english', $1)), 0)
                   + CASE WHEN title ILIKE $3 THEN 0.3 ELSE 0 END
                   + CASE WHEN ingredients ILIKE $3 THEN 0.1 ELSE 0 END AS score
            FROM recipes
            WHERE search_vector @@ websearch_to_tsquery('english', $1)
               OR title ILIKE $3
               OR ingredients ILIKE $3
            ORDER BY score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(limit)
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await?;

        let hits = rows
            .into_iter()
            .map(|r| {
                let ingredients_str: String = r.get("ingredients");
                let instructions_str: String = r.get("instructions");
                let score: f64 = r.get("score");

                let ingredients: Vec<String> =
                    serde_json::from_str(&ingredients_str).unwrap_or_default();
                let instructions: Vec<String> =
                    serde_json::from_str(&instructions_str).unwrap_or_default();

                SearchHit {
                    recipe: Recipe {
                        url: r.get("url"),
                        title: r.get("title"),
                        total_time: r.get("total_time"),
                        ingredients,
                        instructions,
                        image: r.get("image"),
                    },
                    score,
                }
            })
            .collect();

        Ok(hits)
    }

    pub async fn queue_stats(&self) -> Result<QueueStats> {
        let rows =
            sqlx::query("SELECT status, COUNT(*) as count FROM scrape_queue GROUP BY status")
                .fetch_all(&self.pool)
                .await?;

        let mut stats = QueueStats {
            pending: 0,
            in_progress: 0,
            done: 0,
            error: 0,
        };

        for row in rows {
            let status: String = row.get("status");
            let count: i64 = row.get("count");
            match status.as_str() {
                "pending" => stats.pending = count,
                "in_progress" => stats.in_progress = count,
                "done" => stats.done = count,
                "error" => stats.error = count,
                _ => {}
            }
        }

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;

    const ADMIN_DSN: &str = "postgresql:///postgres";
    const TEST_DSN: &str = "postgresql:///recipe_book_test";

    static DB_MUTEX: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    static DB_CREATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn db_mutex() -> &'static tokio::sync::Mutex<()> {
        DB_MUTEX.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    async fn setup_db() -> RecipeDb {
        if !DB_CREATED.load(std::sync::atomic::Ordering::Acquire) {
            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(ADMIN_DSN)
                .await
                .unwrap();
            sqlx::query("DROP DATABASE IF EXISTS recipe_book_test WITH (FORCE)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("CREATE DATABASE recipe_book_test")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
            DB_CREATED.store(true, std::sync::atomic::Ordering::Release);
        }

        let db = RecipeDb::new(TEST_DSN).await.unwrap();
        sqlx::query("DELETE FROM scrape_queue")
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM recipes")
            .execute(&db.pool)
            .await
            .unwrap();
        db
    }

    fn sample_recipe(url: &str) -> ScrapedRecipe {
        ScrapedRecipe {
            url: url.to_string(),
            title: "Test Recipe".to_string(),
            total_time: 30,
            ingredients: vec!["item1".to_string(), "item2".to_string()],
            instructions: vec!["step 1".to_string(), "step 2".to_string()],
            image: String::new(),
        }
    }

    #[tokio::test]
    async fn test_enqueue_new_url_returns_pending() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let result = db.enqueue_url("https://example.com/new").await.unwrap();
        assert_eq!(result, "pending");
    }

    #[tokio::test]
    async fn test_enqueue_duplicate_returns_existing_status() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        db.enqueue_url("https://example.com/dup").await.unwrap();
        let result = db.enqueue_url("https://example.com/dup").await.unwrap();
        assert_eq!(result, "pending");
    }

    #[tokio::test]
    async fn test_next_pending_returns_oldest_in_fifo_order() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        db.enqueue_url("https://example.com/first").await.unwrap();
        db.enqueue_url("https://example.com/second").await.unwrap();

        let job = db.next_pending().await.unwrap();
        assert!(job.is_some());
        let (_, url) = job.unwrap();
        assert_eq!(url, "https://example.com/first");
    }

    #[tokio::test]
    async fn test_next_pending_marks_in_progress() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        db.enqueue_url("https://example.com/test").await.unwrap();

        let job = db.next_pending().await.unwrap();
        assert!(job.is_some());
        let (id, _) = job.unwrap();

        let row = sqlx::query("SELECT status FROM scrape_queue WHERE id = $1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let status: String = row.get("status");
        assert_eq!(status, "in_progress");
    }

    #[tokio::test]
    async fn test_next_pending_returns_none_when_empty() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let result = db.next_pending().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_mark_done_updates_status() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        db.enqueue_url("https://example.com/done").await.unwrap();
        let job = db.next_pending().await.unwrap();
        assert!(job.is_some());
        let (id, _) = job.unwrap();
        db.mark_done(id).await.unwrap();

        let row = sqlx::query("SELECT status FROM scrape_queue WHERE id = $1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let status: String = row.get("status");
        assert_eq!(status, "done");
    }

    #[tokio::test]
    async fn test_mark_error_updates_status_and_message() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        db.enqueue_url("https://example.com/error").await.unwrap();
        let job = db.next_pending().await.unwrap();
        assert!(job.is_some());
        let (id, _) = job.unwrap();
        db.mark_error(id, "something broke").await.unwrap();

        let row = sqlx::query("SELECT status, error_message FROM scrape_queue WHERE id = $1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let status: String = row.get("status");
        let message: String = row.get("error_message");
        assert_eq!(status, "error");
        assert_eq!(message, "something broke");
    }

    #[tokio::test]
    async fn test_mark_error_truncates_long_messages() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        db.enqueue_url("https://example.com/long").await.unwrap();
        let job = db.next_pending().await.unwrap();
        assert!(job.is_some());
        let (id, _) = job.unwrap();
        db.mark_error(id, &"x".repeat(1000)).await.unwrap();

        let row = sqlx::query("SELECT error_message FROM scrape_queue WHERE id = $1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let message: String = row.get("error_message");
        assert_eq!(message.len(), 500);
    }

    #[tokio::test]
    async fn test_save_and_get_roundtrip() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let recipe = sample_recipe("https://example.com/test");
        db.save_recipe(&recipe).await.unwrap();

        let result = db.get_recipe("https://example.com/test").await.unwrap();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.title, "Test Recipe");
        assert_eq!(r.total_time, 30);
        assert_eq!(r.ingredients, vec!["item1", "item2"]);
        assert_eq!(r.instructions, vec!["step 1", "step 2"]);
    }

    #[tokio::test]
    async fn test_get_nonexistent_returns_none() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let result = db.get_recipe("https://example.com/nope").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_save_overwrites_existing() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let old = ScrapedRecipe {
            title: "Old".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        let new = ScrapedRecipe {
            title: "New".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&old).await.unwrap();
        db.save_recipe(&new).await.unwrap();

        let result = db.get_recipe("https://example.com/test").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().title, "New");
    }

    #[tokio::test]
    async fn test_search_finds_matching_recipe() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let recipe = ScrapedRecipe {
            title: "Chicken Parmesan".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&recipe).await.unwrap();

        let results = db.search("chicken", 20).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].recipe.title, "Chicken Parmesan");
    }

    #[tokio::test]
    async fn test_search_returns_empty_for_no_match() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let recipe = ScrapedRecipe {
            title: "Chicken Parmesan".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&recipe).await.unwrap();

        let results = db.search("zucchini", 20).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_respects_limit() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        for i in 0..5 {
            let recipe = ScrapedRecipe {
                url: format!("https://example.com/test{i}"),
                title: format!("Chicken Dish {i}"),
                ..sample_recipe("https://example.com/test0")
            };
            db.save_recipe(&recipe).await.unwrap();
        }

        let results = db.search("chicken", 2).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_fts_incremental_updates_after_save() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let alpha = ScrapedRecipe {
            url: "https://example.com/alpha".to_string(),
            title: "Alpha Dish".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&alpha).await.unwrap();
        assert_eq!(db.search("alpha", 20).await.unwrap().len(), 1);

        let beta = ScrapedRecipe {
            url: "https://example.com/beta".to_string(),
            title: "Beta Dish".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&beta).await.unwrap();
        assert_eq!(db.search("beta", 20).await.unwrap().len(), 1);
        assert_eq!(db.search("alpha", 20).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_search_partial_word_match() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let recipe = ScrapedRecipe {
            title: "Chicken Parmesan".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&recipe).await.unwrap();

        let results = db.search("chick", 20).await.unwrap();
        assert!(!results.is_empty(), "partial word 'chick' should match 'Chicken'");
        assert_eq!(results[0].recipe.title, "Chicken Parmesan");
    }

    #[tokio::test]
    async fn test_counts_by_status() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        db.enqueue_url("https://example.com/p1").await.unwrap();
        db.enqueue_url("https://example.com/p2").await.unwrap();
        db.enqueue_url("https://example.com/e1").await.unwrap();

        let job = db.next_pending().await.unwrap();
        assert!(job.is_some());
        db.mark_error(job.unwrap().0, "fail").await.unwrap();

        let stats = db.queue_stats().await.unwrap();
        assert_eq!(stats.pending, 2);
        assert_eq!(stats.in_progress, 0);
        assert_eq!(stats.error, 1);
        assert_eq!(stats.done, 0);
    }
}
