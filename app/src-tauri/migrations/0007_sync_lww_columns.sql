-- Phase 5 (synchronization) needs a per-field "last-write-wins" timestamp
-- (plan section 1.3) for every entity that can be edited from more than
-- one device. `products` already has `updated_at` (section 3.1); section
-- 3.2's `customers` and section 3.1's `product_units` do not, so this
-- closes that gap the same way 0006_customer_due_days did for
-- `default_due_days`.

ALTER TABLE customers ADD COLUMN updated_at TEXT;
ALTER TABLE product_units ADD COLUMN updated_at TEXT;

UPDATE customers SET updated_at = created_at WHERE updated_at IS NULL;
UPDATE product_units SET updated_at = (SELECT updated_at FROM products WHERE products.id = product_units.product_id)
WHERE updated_at IS NULL;
