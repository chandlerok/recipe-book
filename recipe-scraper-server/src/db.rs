use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use tracing::info;

use crate::models::{QueueStats, Recipe};
use crate::scraper::ScrapedRecipe;

pub const CRAWL_LOCK_ID: i64 = 42;

pub struct RecipeDb {
    pool: sqlx::PgPool,
}

impl Clone for RecipeDb {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
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

        sqlx::migrate!().run(&pool).await?;
        info!("database initialized");

        let db = Self { pool };
        Ok(db)
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    pub async fn enqueue_url(&self, url: &str) -> Result<String> {
        let existing: Option<String> =
            sqlx::query_scalar!("SELECT status FROM scrape_queue WHERE url = $1", url)
                .fetch_optional(&self.pool)
                .await?;

        if let Some(status) = existing {
            return Ok(status);
        }

        sqlx::query!("INSERT INTO scrape_queue (url) VALUES ($1)", url)
            .execute(&self.pool)
            .await?;

        Ok("pending".to_string())
    }

    pub async fn next_pending(&self) -> Result<Option<(i32, String)>> {
        let row = sqlx::query!(
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

        Ok(row.map(|r| (r.id, r.url)))
    }

    #[allow(dead_code)]
    pub async fn mark_done(&self, job_id: i32) -> Result<()> {
        sqlx::query!(
            "UPDATE scrape_queue SET status = 'done', scraped_at = CURRENT_TIMESTAMP WHERE id = $1",
            job_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_error(&self, job_id: i32, error: &str) -> Result<()> {
        let error = &error[..error.len().min(500)];
        sqlx::query!(
            "UPDATE scrape_queue SET status = 'error', error_message = $1 WHERE id = $2",
            error,
            job_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn save_recipe(&self, recipe: &ScrapedRecipe) -> Result<()> {
        let ingredients = serde_json::to_string(&recipe.ingredients)?;
        let instructions = serde_json::to_string(&recipe.instructions)?;

        sqlx::query!(
            r#"
            INSERT INTO recipes (url, title, total_time, ingredients, instructions, image, publication, description, json_ld)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (url) DO UPDATE SET
                title = EXCLUDED.title,
                total_time = EXCLUDED.total_time,
                ingredients = EXCLUDED.ingredients,
                instructions = EXCLUDED.instructions,
                image = EXCLUDED.image,
                publication = EXCLUDED.publication,
                description = EXCLUDED.description,
                json_ld = EXCLUDED.json_ld,
                scraped_at = CURRENT_TIMESTAMP
            "#,
            recipe.url,
            recipe.title,
            recipe.total_time,
            ingredients,
            instructions,
            recipe.image,
            recipe.publication,
            recipe.description,
            recipe.json_ld,
        )
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

        sqlx::query!(
            r#"
            INSERT INTO recipes (url, title, total_time, ingredients, instructions, image, publication, description, json_ld)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (url) DO UPDATE SET
                title = EXCLUDED.title,
                total_time = EXCLUDED.total_time,
                ingredients = EXCLUDED.ingredients,
                instructions = EXCLUDED.instructions,
                image = EXCLUDED.image,
                publication = EXCLUDED.publication,
                description = EXCLUDED.description,
                json_ld = EXCLUDED.json_ld,
                scraped_at = CURRENT_TIMESTAMP
            "#,
            recipe.url,
            recipe.title,
            recipe.total_time,
            ingredients,
            instructions,
            recipe.image,
            recipe.publication,
            recipe.description,
            recipe.json_ld,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "UPDATE scrape_queue SET status = 'done', scraped_at = CURRENT_TIMESTAMP WHERE id = $1",
            job_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_recipe(&self, url: &str) -> Result<Option<Recipe>> {
        let row = sqlx::query!(
            r#"
            SELECT url, title, total_time, ingredients, instructions, image, publication, description
            FROM recipes WHERE url = $1
            "#,
            url
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let ingredients_str = r.ingredients.unwrap_or_default();
            let instructions_str = r.instructions.unwrap_or_default();

            let ingredients: Vec<String> =
                serde_json::from_str(&ingredients_str).unwrap_or_default();
            let instructions: Vec<String> =
                serde_json::from_str(&instructions_str).unwrap_or_default();

            Recipe {
                url: r.url,
                title: r.title.unwrap_or_default(),
                total_time: r.total_time.unwrap_or(0),
                ingredients,
                instructions,
                image: r.image.unwrap_or_default(),
                publication: r.publication.unwrap_or_default(),
                description: r.description.unwrap_or_default(),
            }
        }))
    }

    pub async fn queue_stats(&self) -> Result<QueueStats> {
        let rows =
            sqlx::query!("SELECT status, COUNT(*) as count FROM scrape_queue GROUP BY status")
                .fetch_all(&self.pool)
                .await?;

        let mut stats = QueueStats {
            pending: 0,
            in_progress: 0,
            done: 0,
            error: 0,
        };

        for row in rows {
            match row.status.as_str() {
                "pending" => stats.pending = row.count.unwrap_or(0),
                "in_progress" => stats.in_progress = row.count.unwrap_or(0),
                "done" => stats.done = row.count.unwrap_or(0),
                "error" => stats.error = row.count.unwrap_or(0),
                _ => {}
            }
        }

        Ok(stats)
    }

    pub async fn enqueue_backfill(&self, limit: i64) -> Result<i64> {
        let urls: Vec<String> = sqlx::query_scalar!(
            r#"
            SELECT url FROM recipes
            WHERE description IS NULL
            AND url NOT IN (SELECT url FROM scrape_queue WHERE status = 'pending')
            ORDER BY scraped_at ASC
            LIMIT $1
            "#,
            limit
        )
        .fetch_all(&self.pool)
        .await?;

        if urls.is_empty() {
            return Ok(0);
        }

        sqlx::query!(
            "DELETE FROM scrape_queue WHERE url = ANY($1) AND status IN ('done', 'error')",
            &urls
        )
        .execute(&self.pool)
        .await?;

        let mut count = 0i64;
        for url in &urls {
            match self.enqueue_url(url).await {
                Ok(status) if status == "pending" => count += 1,
                _ => {}
            }
        }

        Ok(count)
    }

    pub async fn try_acquire_crawl_lock(&self) -> Result<bool> {
        let locked: Option<bool> =
            sqlx::query_scalar!("SELECT pg_try_advisory_lock($1)", CRAWL_LOCK_ID)
                .fetch_one(&self.pool)
                .await?;
        Ok(locked.unwrap_or(false))
    }

    pub async fn release_crawl_lock(&self) -> Result<()> {
        sqlx::query!("SELECT pg_advisory_unlock($1)", CRAWL_LOCK_ID)
            .fetch_optional(&self.pool)
            .await?;
        Ok(())
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
            sqlx::query!("DROP DATABASE IF EXISTS recipe_book_test WITH (FORCE)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query!("CREATE DATABASE recipe_book_test")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
            DB_CREATED.store(true, std::sync::atomic::Ordering::Release);
        }

        let db = RecipeDb::new(TEST_DSN).await.unwrap();
        sqlx::query!("DELETE FROM scrape_queue")
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query!("DELETE FROM recipes")
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
            publication: "Test".to_string(),
            description: "A test recipe".to_string(),
            json_ld: String::new(),
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

        let row = sqlx::query!("SELECT status FROM scrape_queue WHERE id = $1", id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(row.status, "in_progress");
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

        let row = sqlx::query!("SELECT status FROM scrape_queue WHERE id = $1", id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(row.status, "done");
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

        let row = sqlx::query!(
            "SELECT status, error_message FROM scrape_queue WHERE id = $1",
            id
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(row.status, "error");
        assert_eq!(row.error_message, Some("something broke".to_string()));
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

        let row = sqlx::query!("SELECT error_message FROM scrape_queue WHERE id = $1", id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(row.error_message.unwrap().len(), 500);
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
