-- Bouedig schema, version 0006: instruction sections, notes/yield/source.

-- One sections table for both kinds (ingredients + instructions).
ALTER TABLE recipe_sections ADD COLUMN kind TEXT NOT NULL DEFAULT 'ingredient';

-- Steps move out of the JSON blob into a real table.
CREATE TABLE recipe_instructions (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    section   TEXT,
    text      TEXT NOT NULL
);

INSERT INTO recipe_instructions (recipe_id, position, section, text)
SELECT r.id, je.key, NULL, je.value
FROM recipes r, json_each(COALESCE(r.instructions, '[]')) je;

ALTER TABLE recipes DROP COLUMN instructions;

ALTER TABLE recipes ADD COLUMN notes TEXT NOT NULL DEFAULT '';
ALTER TABLE recipes ADD COLUMN yield_amount TEXT NOT NULL DEFAULT '';
ALTER TABLE recipes ADD COLUMN source TEXT NOT NULL DEFAULT '';
