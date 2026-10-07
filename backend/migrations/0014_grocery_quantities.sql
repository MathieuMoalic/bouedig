-- Optional amounts on grocery items: a numeric quantity plus a short unit
-- typed by the user ("500" + "g" oats). NULL quantity = no amount; the
-- name never carries the amount (recipe pushes are split on insert).
ALTER TABLE grocery_items ADD COLUMN quantity REAL;

ALTER TABLE grocery_items ADD COLUMN unit TEXT NOT NULL DEFAULT '';
