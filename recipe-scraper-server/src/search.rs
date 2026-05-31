use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;
use tantivy::{
    Index, IndexReader, ReloadPolicy, Term,
    collector::{Count, TopDocs},
    doc,
    query::{BooleanQuery, BoostQuery, Occur, PhraseQuery, Query, QueryParser, TermQuery},
    schema::*,
    tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer},
};
use tracing::info;

use crate::models::{Recipe, SearchHit, SearchResults};
use crate::scraper::ScrapedRecipe;

pub struct RecipeIndex {
    #[allow(dead_code)]
    schema: Schema,
    url: Field,
    title: Field,
    title_ngram: Field,
    ingredients: Field,
    ingredients_ngram: Field,
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

fn ngram_field(name: &str) -> FieldEntry {
    FieldEntry::new_text(
        name.to_string(),
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_index_option(IndexRecordOption::WithFreqs),
        ),
    )
}

fn word_ngrams(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .flat_map(|word| {
            let chars: Vec<char> = word.chars().collect();
            let max_len = chars.len().min(20);
            (2..=max_len).map(move |len| chars[..len].iter().collect::<String>())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn query_ngrams(text: &str) -> Vec<String> {
    let text = text.to_lowercase();
    let mut ngrams: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() < 2 {
            continue;
        }
        for len in 2..=chars.len().min(20) {
            ngrams.push(chars[..len].iter().collect());
        }
    }
    ngrams
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
        let title_ngram = sb.add_field(ngram_field("title_ngram"));
        let ingredients = sb.add_field(FieldEntry::new_text(
            "ingredients".to_string(),
            TEXT | STORED,
        ));
        let ingredients_ngram = sb.add_field(ngram_field("ingredients_ngram"));
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

            let ing_joined = ing_text.join(" ");
            let instr_joined = instr_text.join(" ");

            let title_ngrams = word_ngrams(title_str);
            let ing_ngrams = word_ngrams(&ing_joined);

            writer.add_document(doc!(
                url => row.url.as_str(),
                title => title_str,
                title_ngram => title_ngrams.as_str(),
                ingredients => ing_joined.as_str(),
                ingredients_ngram => ing_ngrams.as_str(),
                instructions => instr_joined.as_str(),
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
            title_ngram,
            ingredients,
            ingredients_ngram,
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

        let ing_joined = recipe.ingredients.join(" ");
        let instr_joined = recipe.instructions.join(" ");
        let title_ngrams = word_ngrams(&recipe.title);
        let ing_ngrams = word_ngrams(&ing_joined);

        writer.add_document(doc!(
            self.url => recipe.url.as_str(),
            self.title => recipe.title.as_str(),
            self.title_ngram => title_ngrams.as_str(),
            self.ingredients => ing_joined.as_str(),
            self.ingredients_ngram => ing_ngrams.as_str(),
            self.instructions => instr_joined.as_str(),
            self.publication => recipe.publication.as_str(),
            self.total_time => recipe.total_time as i64,
            self.image => recipe.image.as_str(),
        ))?;

        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub async fn search(&self, query_str: &str, limit: i32, offset: i32) -> SearchResults {
        if query_str.chars().count() < 2 {
            return SearchResults {
                hits: Vec::new(),
                total: 0,
                offset,
                limit,
            };
        }

        let (limit_u, offset_u) = (limit.max(0) as usize, offset.max(0) as usize);
        let searcher = self.reader.searcher();

        let words: Vec<&str> = query_str.split_whitespace().collect();
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

<<<<<<< HEAD
        // Filter using ngram fields so partial words like "chick" still match
        // "chicken". The QueryParser with conjunction_by_default requires every
        // query term to appear in at least one ngram field.
        let ngram_fields = vec![self.title_ngram, self.ingredients_ngram];
        let mut parser = QueryParser::for_index(&self.index, ngram_fields);
        parser.set_conjunction_by_default();
        if let Ok(parsed) = parser.parse_query(query_str) {
            outer.push((Occur::Must, Box::new(parsed)));
=======
        let ngram_tokens = query_ngrams(query_str);

        if ngram_tokens.is_empty() {
            return SearchResults {
                hits: Vec::new(),
                total: 0,
                offset,
                limit,
            };
        }

        for token in &ngram_tokens {
            let mut field_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            for field in [self.title_ngram, self.ingredients_ngram] {
                let term = Term::from_field_text(field, token);
                field_clauses.push((
                    Occur::Should,
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
                ));
            }
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(field_clauses))));
>>>>>>> 8d52035 (update ngram logic)
        }

        // Score-only boosts (don't affect filtering)
        if words.len() >= 2 {
            // Phrase query in standard title field for exact consecutive word match
            let phrase_terms: Vec<(usize, Term)> = words
                .iter()
                .enumerate()
                .map(|(i, w)| (i, Term::from_field_text(self.title, w)))
                .collect();
            let phrase = PhraseQuery::new_with_offset_and_slop(phrase_terms, 1);
<<<<<<< HEAD
            outer.push((
=======
            clauses.push((
>>>>>>> 8d52035 (update ngram logic)
                Occur::Should,
                Box::new(BoostQuery::new(Box::new(phrase), 5.0)),
            ));
        }

<<<<<<< HEAD
        // Publication boost
        for pub_name in &["Bon Appétit", "NYT Cooking", "Epicurious"] {
            let term = Term::from_field_text(self.publication, pub_name);
            let term_query = TermQuery::new(term, IndexRecordOption::Basic);
            outer.push((
                Occur::Should,
                Box::new(BoostQuery::new(Box::new(term_query), 0.01)),
            ));
        }

        let bool_query = BooleanQuery::new(outer);
=======
        let bool_query = BooleanQuery::new(clauses);
>>>>>>> 8d52035 (update ngram logic)

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

        let mut rows: Vec<RecipeDbRow> = query.fetch_all(&self.pool).await?;

        let order: HashMap<&str, usize> =
            urls.iter().enumerate().map(|(i, u)| (u.as_str(), i)).collect();
        rows.sort_by_key(|r| order.get(r.url.as_str()).copied().unwrap_or(usize::MAX));

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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestIndex {
        url: Field,
        title_ngram: Field,
        ingredients_ngram: Field,
        index: Index,
        reader: IndexReader,
    }

    fn ngram_field(name: &str) -> FieldEntry {
        FieldEntry::new_text(
            name.to_string(),
            TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_index_option(IndexRecordOption::WithFreqs),
            ),
        )
    }

    impl TestIndex {
        fn build() -> Self {
            let mut sb = Schema::builder();
            let url = sb.add_field(FieldEntry::new("url".to_string(), FieldType::Str(STRING | STORED)));
            let title_ngram = sb.add_field(ngram_field("title_ngram"));
            let ingredients_ngram = sb.add_field(ngram_field("ingredients_ngram"));
            let _other = sb.add_field(FieldEntry::new_text("title".to_string(), TEXT | STORED));
            let _other2 = sb.add_field(FieldEntry::new_text("instructions".to_string(), TEXT | STORED));
            let schema = sb.build();

            let index = Index::create_in_ram(schema.clone());

            let reader = index
                .reader_builder()
                .reload_policy(ReloadPolicy::OnCommitWithDelay)
                .try_into()
                .unwrap();

            Self {
                url,
                title_ngram,
                ingredients_ngram,
                index,
                reader,
            }
        }

        fn add(&self, id: &str, title: &str, ingredients: &str) {
            let mut writer = self.index.writer(50_000_000).unwrap();
            let title_ngrams = word_ngrams(title);
            let ing_ngrams = word_ngrams(ingredients);
            writer
                .add_document(doc!(
                    self.url => id,
                    self.title_ngram => title_ngrams.as_str(),
                    self.ingredients_ngram => ing_ngrams.as_str(),
                ))
                .unwrap();
            writer.commit().unwrap();
            self.reader.reload().unwrap();
        }

        fn search_urls(&self, query_str: &str) -> Vec<String> {
            let searcher = self.reader.searcher();
            let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

            let ngram_tokens = query_ngrams(query_str);

            if ngram_tokens.is_empty() {
                return vec![];
            }

            for token in &ngram_tokens {
                let mut field_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
                for field in [self.title_ngram, self.ingredients_ngram] {
                    let term = Term::from_field_text(field, token);
                    field_clauses.push((
                        Occur::Should,
                        Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
                    ));
                }
                clauses.push((Occur::Must, Box::new(BooleanQuery::new(field_clauses))));
            }

            let bool_query = BooleanQuery::new(clauses);
            let (top_docs, _count) = searcher
                .search(&bool_query, &(TopDocs::with_limit(10), Count))
                .unwrap();

            top_docs
                .into_iter()
                .filter_map(|(_score, addr)| {
                    let doc: tantivy::TantivyDocument = searcher.doc(addr).ok()?;
                    doc.get_first(self.url)?.as_str().map(String::from)
                })
                .collect()
        }
    }

    #[test]
    fn single_term_soup_matches_all_soups() {
        let index = TestIndex::build();
        index.add("fos", "French Onion Soup", "beef broth onions cheese bread");
        index.add("cns", "Chicken Noodle Soup", "chicken noodles broth carrots");
        index.add("ts", "Tomato Soup", "tomatoes cream basil");
        index.add("ft", "French Toast", "bread eggs milk cinnamon");
        let results = index.search_urls("soup");
        assert_eq!(results.len(), 3, "Expected all 3 soups, got: {:?}", results);
    }

    #[test]
    fn matches_via_ingredients() {
        let index = TestIndex::build();
        index.add("fos", "French Onion Soup", "beef broth onions cheese bread");
        index.add("ft", "French Toast", "bread eggs milk cinnamon");
        let results = index.search_urls("broth");
        assert!(
            results.contains(&"fos".to_string()),
            "'broth' should match via ingredients, got: {:?}",
            results
        );
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn prefix_steps_produce_same_results() {
        let index = TestIndex::build();
        index.add("fos", "French Onion Soup", "beef broth onions cheese bread");
        for prefix in &["fr", "fre", "fren", "frenc", "french"] {
            let results = index.search_urls(prefix);
            assert!(
                results.contains(&"fos".to_string()),
                "'{prefix}' should match French Onion Soup, got empty"
            );
        }
    }

    #[test]
    fn chicken_soup_does_not_match_beef() {
        let index = TestIndex::build();
        index.add("fos", "French Onion Soup", "beef broth onions cheese bread");
        index.add("cns", "Chicken Noodle Soup", "chicken noodles broth carrots");
        let results = index.search_urls("chicken soup");
        assert!(results.contains(&"cns".to_string()));
        assert!(!results.contains(&"fos".to_string()));
    }
}
