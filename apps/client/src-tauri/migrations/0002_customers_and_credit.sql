-- Customers and store credit ("fiado") — plan section 3.2

CREATE TABLE customers (
    id              TEXT PRIMARY KEY,
    doc_type        TEXT NOT NULL,          -- 'CPF' or 'CNPJ'
    doc_number      TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    phone           TEXT,
    address_json    TEXT,                   -- structured address (required if buyer data goes on the fiscal note)
    credit_limit    NUMERIC(12,2) NOT NULL DEFAULT 0,
    active          INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

-- Store-credit ledger ("fiado") — every row is an entry, balance is always computed, never stored directly
CREATE TABLE customer_ledger (
    id              TEXT PRIMARY KEY,       -- UUID = event_id
    customer_id     TEXT NOT NULL REFERENCES customers(id),
    entry_type      TEXT NOT NULL,          -- 'debit' (credit purchase) or 'credit' (payment received)
    amount          NUMERIC(12,2) NOT NULL, -- always positive, entry_type defines the sign
    due_date        TEXT,                   -- only for debit entries
    reference_sale_id TEXT,
    notes           TEXT,
    device_id       TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    synced_at       TEXT
);
-- customer balance owed = SUM(debit) - SUM(credit)
