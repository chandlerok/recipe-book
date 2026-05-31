use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::RwLock;
use tracing::info;

use crate::models::{QueueStats, Recipe, SearchHit, SearchResults};
use crate::scraper::ScrapedRecipe;

pub const CRAWL_LOCK_ID: i64 = 42;

struct CacheEntry {
    hits: SearchResults,
    at: Instant,
}

pub struct RecipeDb {
    pool: sqlx::PgPool,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
}

impl Clone for RecipeDb {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            cache: self.cache.clone(),
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

        let db = Self {
            pool,
            cache: Arc::new(RwLock::new(HashMap::new())),
        };
        Ok(db)
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
            recipe.url,
            recipe.title,
            recipe.total_time,
            ingredients,
            instructions,
            recipe.image,
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
            recipe.url,
            recipe.title,
            recipe.total_time,
            ingredients,
            instructions,
            recipe.image,
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
            SELECT url, title, total_time, ingredients, instructions, image
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
            }
        }))
    }

    pub async fn search(&self, query: &str, limit: i32, offset: i32) -> Result<SearchResults> {
        if query.len() < 2 {
            return Ok(SearchResults {
                hits: Vec::new(),
                total: 0,
                offset,
                limit,
            });
        }

        let cache_key = format!("{}:{}:{}", query, limit, offset);
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(&cache_key)
                && entry.at.elapsed() < Duration::from_secs(30)
            {
                return Ok(entry.hits.clone());
            }
        }

        let pattern = format!("%{}%", query);
        let mut tx = self.pool.begin().await?;

        sqlx::query!("SELECT set_limit(0.18)")
            .fetch_optional(&mut *tx)
            .await?;

        let rows = sqlx::query!(
            r#"
            WITH matched AS (
                SELECT url FROM recipes
                WHERE search_vector @@ websearch_to_tsquery('english', $1)
                UNION
                SELECT url FROM recipes WHERE title ILIKE $3
                UNION
                SELECT url FROM recipes WHERE ingredients ILIKE $3
                UNION
                SELECT url FROM recipes WHERE title % $1
            )
            SELECT r.url, r.title, r.total_time, r.ingredients, r.instructions, r.image,
                   COALESCE(ts_rank(r.search_vector, websearch_to_tsquery('english', $1)), 0)
                   + CASE WHEN r.title ILIKE $3 THEN 0.3 ELSE 0 END
                   + CASE WHEN r.ingredients ILIKE $3 THEN 0.1 ELSE 0 END
                   + GREATEST(word_similarity($1, COALESCE(r.title, '')), 0) * 0.4
                   + GREATEST(word_similarity($1, COALESCE(r.ingredients, '')), 0) * 0.15 AS score,
                   COUNT(*) OVER() AS total
            FROM recipes r
            JOIN matched m ON r.url = m.url
            ORDER BY score DESC
            LIMIT $2 OFFSET $4
            "#,
            query,
            i64::from(limit),
            pattern,
            i64::from(offset),
        )
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

        let total: i64 = rows.first().and_then(|r| r.total).unwrap_or(0);

        let hits: Vec<SearchHit> = rows
            .into_iter()
            .map(|r| {
                let ingredients_str = r.ingredients.unwrap_or_default();
                let instructions_str = r.instructions.unwrap_or_default();

                let ingredients: Vec<String> =
                    serde_json::from_str(&ingredients_str).unwrap_or_default();
                let instructions: Vec<String> =
                    serde_json::from_str(&instructions_str).unwrap_or_default();

                SearchHit {
                    recipe: Recipe {
                        url: r.url,
                        title: r.title.unwrap_or_default(),
                        total_time: r.total_time.unwrap_or(0),
                        ingredients,
                        instructions,
                        image: r.image.unwrap_or_default(),
                    },
                    score: r.score.unwrap_or(0.0),
                }
            })
            .collect();

        let results = SearchResults {
            hits: hits.clone(),
            total,
            offset,
            limit,
        };

        {
            let mut cache = self.cache.write().await;
            cache.retain(|_, e| e.at.elapsed() < Duration::from_secs(30));
            cache.insert(
                cache_key,
                CacheEntry {
                    hits: results.clone(),
                    at: Instant::now(),
                },
            );
        }

        Ok(results)
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
    async fn test_search_finds_matching_recipe() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let recipe = ScrapedRecipe {
            title: "Chicken Parmesan".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&recipe).await.unwrap();

        let results = db.search("chicken", 20, 0).await.unwrap();
        assert!(!results.hits.is_empty());
        assert_eq!(results.hits[0].recipe.title, "Chicken Parmesan");
        assert!(results.total > 0);
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

        let results = db.search("zucchini", 20, 0).await.unwrap();
        assert!(results.hits.is_empty());
        assert_eq!(results.total, 0);
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

        let results = db.search("chicken", 2, 0).await.unwrap();
        assert_eq!(results.hits.len(), 2);
        assert_eq!(results.total, 5);
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
        assert_eq!(db.search("alpha", 20, 0).await.unwrap().hits.len(), 1);

        let beta = ScrapedRecipe {
            url: "https://example.com/beta".to_string(),
            title: "Beta Dish".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&beta).await.unwrap();
        assert_eq!(db.search("beta", 20, 0).await.unwrap().hits.len(), 1);
        assert_eq!(db.search("alpha", 20, 0).await.unwrap().hits.len(), 1);
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

        let results = db.search("chick", 20, 0).await.unwrap();
        assert!(
            !results.hits.is_empty(),
            "partial word 'chick' should match 'Chicken'"
        );
        assert_eq!(results.hits[0].recipe.title, "Chicken Parmesan");
    }

    #[tokio::test]
    async fn test_search_misspelled_word() {
        let _lock = db_mutex().lock().await;
        let db = setup_db().await;
        let recipe = ScrapedRecipe {
            title: "Chicken Parmesan".to_string(),
            ..sample_recipe("https://example.com/test")
        };
        db.save_recipe(&recipe).await.unwrap();

        let results = db.search("chikcen", 20, 0).await.unwrap();
        assert!(
            !results.hits.is_empty(),
            "misspelled 'chikcen' should match 'Chicken' via trigram similarity"
        );
        assert_eq!(results.hits[0].recipe.title, "Chicken Parmesan");
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
