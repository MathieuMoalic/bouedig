-- Bouedig schema, version 0003: structured ingredients + instructions.

CREATE TABLE recipe_ingredients (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    quantity  REAL,
    unit      TEXT,
    name      TEXT NOT NULL,
    prep      TEXT
);

ALTER TABLE recipes ADD COLUMN instructions TEXT;
