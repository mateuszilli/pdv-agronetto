-- Sync / outbox — plan section 3.5, adapted to Postgres types.

CREATE TABLE outbox_events (
    id              UUID PRIMARY KEY,
    entity_type     TEXT NOT NULL,
    entity_id       TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    device_id       TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    synced_at       TIMESTAMPTZ
);
