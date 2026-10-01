-- Mirrors app/src-tauri/migrations/0009_returns_sync_columns.sql — see
-- that file for why these columns exist beyond plan section 3.3's schema.

ALTER TABLE returns ADD COLUMN device_id TEXT;
ALTER TABLE returns ADD COLUMN synced_at TIMESTAMPTZ;
