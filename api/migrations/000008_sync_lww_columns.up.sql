-- Mirrors app/src-tauri/migrations/0007_sync_lww_columns.sql — see that
-- file for why these columns exist beyond plan section 3's schema.

ALTER TABLE customers ADD COLUMN updated_at TIMESTAMPTZ;
ALTER TABLE product_units ADD COLUMN updated_at TIMESTAMPTZ;

UPDATE customers SET updated_at = created_at WHERE updated_at IS NULL;
