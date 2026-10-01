-- Phase 7 (returns) needs `returns` to carry the same sync bookkeeping
-- every other syncable table already has (device_id + synced_at, see
-- `sales`/`stock_movements`/`customer_ledger`) — `returns`/`return_items`
-- were created early (section 3.3) but never given these columns since
-- nothing wrote to them until now. Mirrors
-- api/migrations/000009_returns_sync_columns.up.sql.

ALTER TABLE returns ADD COLUMN device_id TEXT;
ALTER TABLE returns ADD COLUMN synced_at TEXT;
