-- Auto-categorization: "Groceries" is retired in favour of "Other" (blaz's
-- default), and a name → category cache remembers past classifications so
-- repeat adds are instant and API calls are only needed once per name.
UPDATE grocery_items SET category = 'Other' WHERE category = 'Groceries';

CREATE TABLE ingredient_categories (
    name TEXT PRIMARY KEY,
    category TEXT NOT NULL
);
