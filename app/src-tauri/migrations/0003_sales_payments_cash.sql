-- Sales, payments and cash register — plan section 3.3
-- Money/quantity columns use REAL, not the plan's NUMERIC(p,s) — see the
-- note in 0001_catalog_and_stock.sql on SQLite's NUMERIC affinity/sqlx
-- decode bug.

CREATE TABLE cash_sessions (
    id              TEXT PRIMARY KEY,
    device_id       TEXT NOT NULL,
    operator_id     TEXT NOT NULL,
    opening_amount  REAL NOT NULL,
    closing_amount  REAL,          -- filled in at close
    expected_amount REAL,           -- calculated: opening + cash sales + cash_in - cash_out
    status          TEXT NOT NULL,           -- 'open','closed'
    opened_at       TEXT NOT NULL,
    closed_at       TEXT
);

CREATE TABLE cash_movements (
    id              TEXT PRIMARY KEY,
    cash_session_id TEXT NOT NULL REFERENCES cash_sessions(id),
    type            TEXT NOT NULL,           -- 'cash_out' (sangria) or 'cash_in' (suprimento)
    amount          REAL NOT NULL,
    reason          TEXT,
    created_at      TEXT NOT NULL
);

CREATE TABLE sales (
    id              TEXT PRIMARY KEY,        -- UUID = event_id
    cash_session_id TEXT NOT NULL REFERENCES cash_sessions(id),
    customer_id     TEXT REFERENCES customers(id),  -- nullable = unidentified consumer
    status          TEXT NOT NULL,            -- 'completed','cancelled','returned','partially_returned'
    subtotal        REAL NOT NULL,
    discount_total  REAL NOT NULL DEFAULT 0,
    total           REAL NOT NULL,
    device_id       TEXT NOT NULL,
    operator_id     TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    synced_at       TEXT
);

CREATE TABLE sale_items (
    id              TEXT PRIMARY KEY,
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    product_id      TEXT NOT NULL REFERENCES products(id),
    unit_id         TEXT NOT NULL REFERENCES product_units(id), -- unit chosen at sale time
    quantity        REAL NOT NULL,   -- in the chosen unit_id
    quantity_stock  REAL NOT NULL,   -- converted to stock_unit (for auditing)
    unit_price      REAL NOT NULL,
    discount        REAL NOT NULL DEFAULT 0,
    line_total      REAL NOT NULL
);

CREATE TABLE sale_payments (
    id              TEXT PRIMARY KEY,
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    method          TEXT NOT NULL,            -- 'cash','debit','credit','pix','boleto','store_credit'
    amount          REAL NOT NULL,
    installments    INTEGER DEFAULT 1,        -- installment credit purchases
    gateway_ref      TEXT                      -- card-terminal/Pix reference, if any, post-MVP
);

CREATE TABLE returns (
    id                  TEXT PRIMARY KEY,
    original_sale_id    TEXT NOT NULL REFERENCES sales(id),
    reason              TEXT,
    total_refunded       REAL NOT NULL,
    refund_method        TEXT NOT NULL,        -- may differ from the original payment (e.g. store credit)
    operator_id           TEXT NOT NULL,
    created_at             TEXT NOT NULL
);

CREATE TABLE return_items (
    id              TEXT PRIMARY KEY,
    return_id       TEXT NOT NULL REFERENCES returns(id),
    sale_item_id    TEXT NOT NULL REFERENCES sale_items(id),
    quantity        REAL NOT NULL
);
