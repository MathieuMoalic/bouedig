-- Provenance: which recipe a grocery item was added from (via the
-- "add to shopping list" sheet). Cleared when the recipe is deleted;
-- manually added items stay NULL.
ALTER TABLE grocery_items
    ADD COLUMN recipe_id INTEGER REFERENCES recipes(id) ON DELETE SET NULL;
