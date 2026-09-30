-- Bouedig schema, version 0007: meal plan.

-- One row per recipe scheduled on one calendar day. Recipes cascade away
-- with their plan entries when deleted.
CREATE TABLE meal_plan_entries (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    date      TEXT NOT NULL,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE
);

CREATE INDEX idx_meal_plan_date ON meal_plan_entries(date);
