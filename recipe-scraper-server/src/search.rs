use anyhow::{Context, Result};
use sqlx::PgPool;
use tantivy::{
    Index, IndexReader, ReloadPolicy, Term,
    collector::{Count, TopDocs},
    doc,
    query::{BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, TermQuery},
    schema::*,
};
use tracing::info;

use crate::models::{Recipe, SearchHit, SearchResults};
use crate::scraper::ScrapedRecipe;

pub struct RecipeIndex {
    #[allow(dead_code)]
    schema: Schema,
    url: Field,
    title: Field,
    ingredients: Field,
    instructions: Field,
    publication: Field,
    total_time: Field,
    image: Field,
    index: Index,
    reader: IndexReader,
    pool: PgPool,
}

fn extract_url(
    searcher: &tantivy::Searcher,
    addr: tantivy::DocAddress,
    field: Field,
) -> Option<String> {
    let doc: tantivy::TantivyDocument = searcher.doc(addr).ok()?;
    if let Some(value) = doc.get_first(field) {
        return value.as_str().map(|s| s.to_string());
    }
    None
}

impl RecipeIndex {
    pub async fn build(pool: PgPool) -> Result<Self> {
        info!("building search index from database");

        let mut sb = Schema::builder();
        let url = sb.add_field(FieldEntry::new(
            "url".to_string(),
            FieldType::Str(STRING | STORED),
        ));
        let title = sb.add_field(FieldEntry::new_text("title".to_string(), TEXT | STORED));
        let ingredients = sb.add_field(FieldEntry::new_text(
            "ingredients".to_string(),
            TEXT | STORED,
        ));
        let instructions = sb.add_field(FieldEntry::new_text(
            "instructions".to_string(),
            TEXT | STORED,
        ));
        let publication = sb.add_field(FieldEntry::new(
            "publication".to_string(),
            FieldType::Str(STRING | STORED),
        ));
        let total_time = sb.add_field(FieldEntry::new_i64(
            "total_time".to_string(),
            NumericOptions::default().set_stored(),
        ));
        let image = sb.add_field(FieldEntry::new(
            "image".to_string(),
            FieldType::Str(STRING | STORED),
        ));
        let schema = sb.build();

        let index = Index::create_in_ram(schema.clone());

        #[derive(sqlx::FromRow)]
        struct RecipeRow {
            url: String,
            title: Option<String>,
            ingredients: Option<String>,
            instructions: Option<String>,
            publication: Option<String>,
            total_time: Option<i32>,
            image: Option<String>,
        }

        let rows: Vec<RecipeRow> = sqlx::query_as(
            r#"
            SELECT url, title, ingredients, instructions, publication, total_time, image
            FROM recipes
            "#,
        )
        .fetch_all(&pool)
        .await
        .context("failed to load recipes for index build")?;

        let mut writer = index.writer(100_000_000)?;
        for row in &rows {
            let title_str = row.title.as_deref().unwrap_or("");
            let ingredients_str = row.ingredients.as_deref().unwrap_or("[]");
            let instructions_str = row.instructions.as_deref().unwrap_or("[]");
            let pub_str = row.publication.as_deref().unwrap_or("");
            let time_val = row.total_time.unwrap_or(0) as i64;
            let image_str = row.image.as_deref().unwrap_or("");

            let ing_text: Vec<String> = serde_json::from_str(ingredients_str).unwrap_or_default();
            let instr_text: Vec<String> =
                serde_json::from_str(instructions_str).unwrap_or_default();

            writer.add_document(doc!(
                url => row.url.as_str(),
                title => title_str,
                ingredients => ing_text.join(" ").as_str(),
                instructions => instr_text.join(" ").as_str(),
                publication => pub_str,
                total_time => time_val,
                image => image_str,
            ))?;
        }
        writer.commit()?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        info!("search index built: {} recipes indexed", rows.len());

        Ok(Self {
            schema,
            url,
            title,
            ingredients,
            instructions,
            publication,
            total_time,
            image,
            index,
            reader,
            pool,
        })
    }

    pub fn upsert(&self, recipe: &ScrapedRecipe) -> Result<()> {
        let mut writer = self.index.writer(50_000_000)?;

        writer.delete_term(Term::from_field_text(self.url, &recipe.url));

        writer.add_document(doc!(
            self.url => recipe.url.as_str(),
            self.title => recipe.title.as_str(),
            self.ingredients => recipe.ingredients.join(" ").as_str(),
            self.instructions => recipe.instructions.join(" ").as_str(),
            self.publication => recipe.publication.as_str(),
            self.total_time => recipe.total_time as i64,
            self.image => recipe.image.as_str(),
        ))?;

        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub async fn search(&self, query_str: &str, limit: i32, offset: i32) -> SearchResults {
        if query_str.len() < 2 {
            return SearchResults {
                hits: Vec::new(),
                total: 0,
                offset,
                limit,
            };
        }

        let (limit_u, offset_u) = (limit.max(0) as usize, offset.max(0) as usize);
        let searcher = self.reader.searcher();

        let mut subqueries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.title, self.ingredients, self.instructions],
        );

        if let Ok(parsed) = query_parser.parse_query(query_str) {
            let boosted = BoostQuery::new(Box::new(parsed), 2.0);
            subqueries.push((Occur::Should, Box::new(boosted)));
        }

        let fuzzy_title =
            FuzzyTermQuery::new(Term::from_field_text(self.title, query_str), 1, true);
        subqueries.push((Occur::Should, Box::new(fuzzy_title)));

        let fuzzy_ingredients =
            FuzzyTermQuery::new(Term::from_field_text(self.ingredients, query_str), 1, true);
        subqueries.push((Occur::Should, Box::new(fuzzy_ingredients)));

        let boosted_pubs = ["Bon Appétit", "NYT Cooking", "Epicurious"];
        for pub_name in &boosted_pubs {
            let term = Term::from_field_text(self.publication, pub_name);
            let term_query = TermQuery::new(term, IndexRecordOption::Basic);
            let boosted = BoostQuery::new(Box::new(term_query), 0.5);
            subqueries.push((Occur::Should, Box::new(boosted)));
        }

        let bool_query = BooleanQuery::new(subqueries);

        let (top_docs, total) = match searcher.search(
            &bool_query,
            &(TopDocs::with_limit(limit_u + offset_u), Count),
        ) {
            Ok((docs, count)) => (docs, count as i64),
            Err(_) => {
                return SearchResults {
                    hits: Vec::new(),
                    total: 0,
                    offset,
                    limit,
                };
            }
        };

        let urls: Vec<String> = top_docs
            .into_iter()
            .skip(offset_u)
            .take(limit_u)
            .filter_map(|(_score, addr)| extract_url(&searcher, addr, self.url))
            .collect();

        if urls.is_empty() {
            return SearchResults {
                hits: Vec::new(),
                total,
                offset,
                limit,
            };
        }

        let hits = match self.fetch_recipes(&urls).await {
            Ok(recipes) => recipes
                .into_iter()
                .map(|r| SearchHit {
                    recipe: r,
                    score: 0.0,
                })
                .collect(),
            Err(e) => {
                tracing::warn!("failed to fetch recipe details: {e}");
                Vec::new()
            }
        };

        SearchResults {
            hits,
            total,
            offset,
            limit,
        }
    }

    async fn fetch_recipes(&self, urls: &[String]) -> Result<Vec<Recipe>> {
        let params: Vec<String> = urls
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect();
        let sql = format!(
            "SELECT url, title, total_time, ingredients, instructions, image, publication FROM recipes WHERE url IN ({})",
            params.join(", ")
        );

        let mut query = sqlx::query_as::<_, RecipeDbRow>(&sql);
        for url in urls {
            query = query.bind(url);
        }

        let rows: Vec<RecipeDbRow> = query.fetch_all(&self.pool).await?;

        Ok(rows
            .into_iter()
            .map(|r| {
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
                }
            })
            .collect())
    }
}

#[derive(sqlx::FromRow)]
struct RecipeDbRow {
    url: String,
    title: Option<String>,
    total_time: Option<i32>,
    ingredients: Option<String>,
    instructions: Option<String>,
    image: Option<String>,
    publication: Option<String>,
}
