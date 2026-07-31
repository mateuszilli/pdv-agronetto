-- Sync / outbox — plan section 3.5

CREATE TABLE outbox_events (
    id              TEXT PRIMARY KEY,        -- = event_id, used for idempotency in the backend
    entity_type     TEXT NOT NULL,            -- 'sale','stock_movement','customer_ledger','cash_movement', etc.
    entity_id       TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    device_id       TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    synced_at       TEXT                      -- NULL = pending
);
