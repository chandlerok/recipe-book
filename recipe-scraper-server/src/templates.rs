use askama::Template;

use crate::models::{Recipe, SearchHit};

pub struct HitView {
    pub url_encoded: String,
    pub title: String,
    pub total_time: i32,
    pub ingredients: Vec<String>,
    pub instructions: Vec<String>,
    pub image: String,
    pub score: f64,
}

impl From<SearchHit> for HitView {
    fn from(hit: SearchHit) -> Self {
        Self {
            url_encoded: urlencode(&hit.recipe.url),
            title: hit.recipe.title,
            total_time: hit.recipe.total_time,
            ingredients: hit.recipe.ingredients,
            instructions: hit.recipe.instructions,
            image: hit.recipe.image,
            score: hit.score,
        }
    }
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate<'a> {
    pub query: &'a str,
    pub results: Vec<HitView>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Template)]
#[template(path = "search_results.html")]
pub struct SearchResultsTemplate {
    pub query: String,
    pub results: Vec<HitView>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Template)]
#[template(path = "recipe_detail.html")]
pub struct RecipeDetailTemplate {
    pub recipe: Recipe,
}
