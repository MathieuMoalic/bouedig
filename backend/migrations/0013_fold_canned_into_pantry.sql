-- The Canned category is retired: everything folds into Pantry. Item rows
-- keep their list position; Jev cache rows move too (pinned manual choices
-- stay pinned, now on Pantry).
UPDATE grocery_items SET category = 'Pantry' WHERE category = 'Canned';

UPDATE ingredient_categories SET category = 'Pantry' WHERE category = 'Canned';
