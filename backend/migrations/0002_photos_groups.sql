-- Bouedig schema, version 0002: recipe photos + grocery categories.

ALTER TABLE recipes ADD COLUMN image_path TEXT;
ALTER TABLE recipes ADD COLUMN thumb_path TEXT;
ALTER TABLE grocery_items ADD COLUMN category TEXT NOT NULL DEFAULT 'Groceries';
