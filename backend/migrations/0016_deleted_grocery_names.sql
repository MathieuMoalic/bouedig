-- Names explicitly deleted from the shopping list (edit-sheet Delete) are
-- hidden from the /api/grocery/names suggestion history until the name is
-- added again. Ticking an item off keeps its name in the history — only an
-- explicit delete means "never suggest this again". Case-insensitive so
-- "Oat Milk" and "oat milk" hide together.
CREATE TABLE deleted_grocery_names (
    name TEXT PRIMARY KEY COLLATE NOCASE
);
