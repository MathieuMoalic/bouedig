-- Bouedig schema, version 0005: ingredient sections.

-- Sections are first-class, ordered entities so that empty sections persist.
CREATE TABLE recipe_sections (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    name      TEXT NOT NULL
);

ALTER TABLE recipe_ingredients ADD COLUMN section TEXT;
