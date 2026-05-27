use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Recipe {
    pub url: String,
    pub title: String,
    pub total_time: i32,
    pub ingredients: Vec<String>,
    pub instructions: Vec<String>,
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub recipe: Recipe,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QueueStats {
    pub pending: i64,
    pub in_progress: i64,
    pub done: i64,
    pub error: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub total: i64,
    pub offset: i32,
    pub limit: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchParams {
    pub q: String,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct RecipeQuery {
    pub url: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScrapeRequest {
    pub url: String,
}
