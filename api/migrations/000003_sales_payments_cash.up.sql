-- Sales, payments and cash register — plan section 3.3, adapted to Postgres types.

CREATE TABLE cash_sessions (
    id              UUID PRIMARY KEY,
    device_id       TEXT NOT NULL,
    operator_id     UUID NOT NULL,
    opening_amount  NUMERIC(12,2) NOT NULL,
    closing_amount  NUMERIC(12,2),
    expected_amount NUMERIC(12,2),
    status          TEXT NOT NULL,
    opened_at       TIMESTAMPTZ NOT NULL,
    closed_at       TIMESTAMPTZ
);

CREATE TABLE cash_movements (
    id              UUID PRIMARY KEY,
    cash_session_id UUID NOT NULL REFERENCES cash_sessions(id),
    type            TEXT NOT NULL,
    amount          NUMERIC(12,2) NOT NULL,
    reason          TEXT,
    created_at      TIMESTAMPTZ NOT NULL
);

CREATE TABLE sales (
    id              UUID PRIMARY KEY,
    cash_session_id UUID NOT NULL REFERENCES cash_sessions(id),
    customer_id     UUID REFERENCES customers(id),
    status          TEXT NOT NULL,
    subtotal        NUMERIC(12,2) NOT NULL,
    discount_total  NUMERIC(12,2) NOT NULL DEFAULT 0,
    total           NUMERIC(12,2) NOT NULL,
    device_id       TEXT NOT NULL,
    operator_id     UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    synced_at       TIMESTAMPTZ
);

CREATE TABLE sale_items (
    id              UUID PRIMARY KEY,
    sale_id         UUID NOT NULL REFERENCES sales(id),
    product_id      UUID NOT NULL REFERENCES products(id),
    unit_id         UUID NOT NULL REFERENCES product_units(id),
    quantity        NUMERIC(14,4) NOT NULL,
    quantity_stock  NUMERIC(14,4) NOT NULL,
    unit_price      NUMERIC(12,4) NOT NULL,
    discount        NUMERIC(12,2) NOT NULL DEFAULT 0,
    line_total      NUMERIC(12,2) NOT NULL
);

CREATE TABLE sale_payments (
    id              UUID PRIMARY KEY,
    sale_id         UUID NOT NULL REFERENCES sales(id),
    method          TEXT NOT NULL,
    amount          NUMERIC(12,2) NOT NULL,
    installments    INTEGER DEFAULT 1,
    gateway_ref     TEXT
);

CREATE TABLE returns (
    id                  UUID PRIMARY KEY,
    original_sale_id    UUID NOT NULL REFERENCES sales(id),
    reason              TEXT,
    total_refunded      NUMERIC(12,2) NOT NULL,
    refund_method       TEXT NOT NULL,
    operator_id         UUID NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL
);

CREATE TABLE return_items (
    id              UUID PRIMARY KEY,
    return_id       UUID NOT NULL REFERENCES returns(id),
    sale_item_id    UUID NOT NULL REFERENCES sale_items(id),
    quantity        NUMERIC(14,4) NOT NULL
);
