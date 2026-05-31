DROP TRIGGER IF EXISTS trg_recipes_search ON recipes;
DROP FUNCTION IF EXISTS recipes_search_update;
ALTER TABLE recipes DROP COLUMN IF EXISTS search_vector;
DROP INDEX IF EXISTS idx_recipes_search;
DROP INDEX IF EXISTS idx_recipes_title_trgm;
DROP INDEX IF EXISTS idx_recipes_ingredients_trgm;
