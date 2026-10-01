-- Customers and store credit ("fiado") — plan section 3.2, adapted to Postgres types.

CREATE TABLE customers (
    id              UUID PRIMARY KEY,
    doc_type        TEXT NOT NULL,
    doc_number      TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    phone           TEXT,
    address_json    TEXT,
    credit_limit    NUMERIC(12,2) NOT NULL DEFAULT 0,
    active          BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL
);

-- customer balance owed = SUM(debit) - SUM(credit)
CREATE TABLE customer_ledger (
    id                  UUID PRIMARY KEY,
    customer_id         UUID NOT NULL REFERENCES customers(id),
    entry_type          TEXT NOT NULL,
    amount              NUMERIC(12,2) NOT NULL,
    due_date            TIMESTAMPTZ,
    reference_sale_id   UUID,
    notes               TEXT,
    device_id           TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    synced_at           TIMESTAMPTZ
);
