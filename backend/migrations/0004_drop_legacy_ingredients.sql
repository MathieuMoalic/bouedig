-- Bouedig schema, version 0004: structured ingredients now live in the
-- recipe_ingredients table, so the legacy free-text column goes away.
ALTER TABLE recipes DROP COLUMN ingredients;
