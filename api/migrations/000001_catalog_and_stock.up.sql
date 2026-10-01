-- Catalog and stock tables — plan section 3.1, adapted to Postgres types:
-- TEXT-as-UUID -> UUID, TEXT-as-timestamp -> TIMESTAMPTZ, INTEGER-as-bool -> BOOLEAN.

CREATE TABLE products (
    id              UUID PRIMARY KEY,
    sku             TEXT UNIQUE,
    barcode         TEXT,
    name            TEXT NOT NULL,
    ncm             TEXT NOT NULL,
    cfop_default    TEXT,
    cst_csosn       TEXT,
    stock_unit      TEXT NOT NULL,
    cost_price      NUMERIC(12,4) NOT NULL,
    active          BOOLEAN NOT NULL DEFAULT true,
    updated_at      TIMESTAMPTZ NOT NULL,
    deleted_at      TIMESTAMPTZ
);

CREATE TABLE product_units (
    id              UUID PRIMARY KEY,
    product_id      UUID NOT NULL REFERENCES products(id),
    unit_label      TEXT NOT NULL,
    factor_to_stock NUMERIC(14,6) NOT NULL,
    sell_price      NUMERIC(12,4) NOT NULL,
    is_default_sale BOOLEAN NOT NULL DEFAULT false,
    active          BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE product_barcodes (
    id          UUID PRIMARY KEY,
    product_id  UUID NOT NULL REFERENCES products(id),
    barcode     TEXT NOT NULL UNIQUE,
    unit_id     UUID NOT NULL REFERENCES product_units(id)
);

CREATE TABLE stock_levels (
    product_id      UUID PRIMARY KEY REFERENCES products(id),
    quantity        NUMERIC(14,4) NOT NULL DEFAULT 0,
    min_quantity    NUMERIC(14,4) NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ NOT NULL
);

CREATE TABLE stock_movements (
    id              UUID PRIMARY KEY,
    product_id      UUID NOT NULL REFERENCES products(id),
    delta_quantity  NUMERIC(14,4) NOT NULL,
    reason          TEXT NOT NULL,
    reference_id    TEXT,
    device_id       TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    synced_at       TIMESTAMPTZ
);

CREATE TABLE purchase_orders (
    id              UUID PRIMARY KEY,
    supplier_name   TEXT NOT NULL,
    status          TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL
);

CREATE TABLE purchase_order_items (
    id                  UUID PRIMARY KEY,
    purchase_order_id   UUID NOT NULL REFERENCES purchase_orders(id),
    product_id          UUID NOT NULL REFERENCES products(id),
    quantity_ordered    NUMERIC(14,4) NOT NULL,
    quantity_received   NUMERIC(14,4) NOT NULL DEFAULT 0,
    unit_cost           NUMERIC(12,4) NOT NULL
);
