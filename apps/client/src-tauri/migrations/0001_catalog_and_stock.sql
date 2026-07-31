-- Catalog and stock tables — plan section 3.1
-- Ported near-verbatim from docs/PDV-MVP-Implementation-Plan.md section 3.1.
-- Table order adjusted (product_units before product_barcodes) purely for
-- readability — SQLite does not enforce FK constraints at CREATE TABLE
-- time, so declaration order has no functional effect.

-- Base product — the real stock-keeping unit
CREATE TABLE products (
    id              TEXT PRIMARY KEY,      -- UUID
    sku             TEXT UNIQUE,
    barcode         TEXT,                  -- can have multiple, see product_barcodes
    name            TEXT NOT NULL,
    ncm             TEXT NOT NULL,          -- required for NFC-e (Mercosur tax classification code)
    cfop_default    TEXT,                  -- default CFOP (fiscal operation code)
    cst_csosn       TEXT,                  -- item's tax regime code (CST or CSOSN)
    stock_unit      TEXT NOT NULL,          -- stock-keeping unit: 'KG','UN','M','SC' etc
    cost_price      NUMERIC(12,4) NOT NULL,
    active          INTEGER NOT NULL DEFAULT 1,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT
);

-- Alternative selling units + conversion factor to the stock unit
-- Example: product "Cement" has stock_unit = KG
--          unit "50kg bag" has factor = 50 (1 bag = 50 kg)
--          unit "Ton" has factor = 1000
CREATE TABLE product_units (
    id              TEXT PRIMARY KEY,
    product_id      TEXT NOT NULL REFERENCES products(id),
    unit_label      TEXT NOT NULL,       -- '50kg Bag', 'Ton', 'Unit', 'Meter'
    factor_to_stock NUMERIC(14,6) NOT NULL, -- how many stock_units equal 1 of this unit
    sell_price      NUMERIC(12,4) NOT NULL,
    is_default_sale INTEGER NOT NULL DEFAULT 0,
    active          INTEGER NOT NULL DEFAULT 1
);

-- Multiple barcodes per product (common case: the bulk pallet has one EAN, the single unit has another)
CREATE TABLE product_barcodes (
    id          TEXT PRIMARY KEY,
    product_id  TEXT NOT NULL REFERENCES products(id),
    barcode     TEXT NOT NULL UNIQUE,
    unit_id     TEXT NOT NULL REFERENCES product_units(id)
);

CREATE TABLE stock_levels (
    product_id      TEXT PRIMARY KEY REFERENCES products(id),
    quantity        NUMERIC(14,4) NOT NULL DEFAULT 0,  -- always in stock_unit
    min_quantity    NUMERIC(14,4) NOT NULL DEFAULT 0,  -- low-stock trigger
    updated_at      TEXT NOT NULL
);

-- Stock movements are events, not direct edits — enables auditing and delta-based sync
CREATE TABLE stock_movements (
    id              TEXT PRIMARY KEY,      -- UUID = event_id
    product_id      TEXT NOT NULL REFERENCES products(id),
    delta_quantity  NUMERIC(14,4) NOT NULL, -- positive (inbound) or negative (outbound), in stock_unit
    reason          TEXT NOT NULL,          -- 'sale','return','purchase','adjustment','loss'
    reference_id    TEXT,                   -- id of the sale, the return, the purchase order
    device_id       TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    synced_at       TEXT
);

CREATE TABLE purchase_orders (
    id              TEXT PRIMARY KEY,
    supplier_name   TEXT NOT NULL,
    status          TEXT NOT NULL,          -- 'draft','sent','partially_received','received'
    created_at      TEXT NOT NULL
);

CREATE TABLE purchase_order_items (
    id                  TEXT PRIMARY KEY,
    purchase_order_id   TEXT NOT NULL REFERENCES purchase_orders(id),
    product_id          TEXT NOT NULL REFERENCES products(id),
    quantity_ordered    NUMERIC(14,4) NOT NULL,
    quantity_received   NUMERIC(14,4) NOT NULL DEFAULT 0,
    unit_cost           NUMERIC(12,4) NOT NULL
);
