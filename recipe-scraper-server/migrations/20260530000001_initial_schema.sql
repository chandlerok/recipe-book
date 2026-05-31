CREATE TABLE IF NOT EXISTS scrape_queue (
    id SERIAL PRIMARY KEY,
    url VARCHAR NOT NULL UNIQUE,
    status VARCHAR NOT NULL DEFAULT 'pending',
    error_message VARCHAR,
    added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    scraped_at TIMESTAMP
);

CREATE TABLE IF NOT EXISTS recipes (
    url VARCHAR PRIMARY KEY,
    title TEXT,
    total_time INTEGER,
    ingredients TEXT,
    instructions TEXT,
    image TEXT,
    scraped_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE recipes ADD COLUMN IF NOT EXISTS search_vector tsvector;

CREATE INDEX IF NOT EXISTS idx_recipes_search ON recipes USING GIN(search_vector);

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX IF NOT EXISTS idx_recipes_title_trgm
    ON recipes USING GIN (title gin_trgm_ops);

CREATE INDEX IF NOT EXISTS idx_recipes_ingredients_trgm
    ON recipes USING GIN (ingredients gin_trgm_ops);

CREATE OR REPLACE FUNCTION recipes_search_update() RETURNS trigger AS $$
BEGIN
    NEW.search_vector :=
        setweight(to_tsvector('english', COALESCE(NEW.title, '')), 'A') ||
        setweight(to_tsvector('english', COALESCE(NEW.ingredients, '')), 'B') ||
        setweight(to_tsvector('english', COALESCE(NEW.instructions, '')), 'C');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_recipes_search ON recipes;

CREATE TRIGGER trg_recipes_search
    BEFORE INSERT OR UPDATE ON recipes
    FOR EACH ROW EXECUTE FUNCTION recipes_search_update();

UPDATE recipes SET search_vector =
    setweight(to_tsvector('english', COALESCE(title, '')), 'A') ||
    setweight(to_tsvector('english', COALESCE(ingredients, '')), 'B') ||
    setweight(to_tsvector('english', COALESCE(instructions, '')), 'C')
WHERE search_vector IS NULL;
