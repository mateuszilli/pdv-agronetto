# PDV for Agricultural Supplies & Construction Materials Stores — MVP Implementation Plan

**Scope:** offline-first point-of-sale system for agricultural supplies and construction materials stores in Brazil, with NFC-e fiscal emission, desktop client (Tauri), and cloud sync backend (Go + Postgres).

**Language note:** all code, schema, and internal naming are in English. Brazilian fiscal/regulatory terms (NFC-e, NF-e, SEFAZ, CFOP, NCM, CST, CSOSN, CPF, CNPJ, DANFCE, Pix, Boleto, Simples Nacional) are kept in Portuguese/as-is because they are legal terms or literal fields expected by government and fiscal-provider APIs — translating them would break both compliance and integration. Brazilian retail operational jargon without a clean English equivalent (e.g. "fiado") is translated in code, with the original term noted for anyone working with local store staff.

---

## 1. Overall Architecture

### 1.1 High-level view

```
┌─────────────────────────────────────────────────────────────┐
│                     STORE (local device)                       │
│                                                                 │
│   ┌──────────────┐      ┌──────────────────┐                 │
│   │  Tauri App    │◄────►│  SQLite (local)   │                │
│   │  (Rust+WebView)│      │  offline source    │                │
│   │                │      │  of truth          │                │
│   └───────┬────────┘      └──────────────────┘                │
│           │                                                    │
│           ├──► ESC/POS thermal printer (USB/Serial) — DANFCE  │
│           │      / non-fiscal receipt                         │
│           ├──► Barcode scanner (HID)                           │
│           ├──► Cash drawer (kicked via printer)                │
│           └──► Scale (Serial, post-MVP)                        │
│                                                                 │
└──────────────────┬──────────────────────────────┬─────────────┘
                    │ sync (when online)            │
                    ▼                               ▼
        ┌───────────────────────┐      ┌────────────────────────┐
        │   Go Backend (Sync)     │      │  Fiscal Provider API    │
        │   + Postgres             │      │  (e.g. Focus NFe)       │
        │   - event queue          │      │  - NFC-e emission        │
        │   - push/pull sync       │◄────►│  - SEFAZ authorization    │
        │   - multi-store dashboard │      │  - contingency mode       │
        └───────────────────────┘      └────────────────────────┘
```

### 1.2 Core principle: local is the operational source of truth

The Tauri app **never blocks a sale for lack of internet**. Every sale is written to local SQLite first. Sync with the backend and fiscal emission happen asynchronously, with a retry queue.

### 1.3 Sync pattern: Outbox + Event Log

Each device (store/terminal) keeps a local `outbox_events` table. Every relevant mutation (sale, payment, cash movement, stock adjustment) generates an outbox event. A background worker:

1. Reads pending events from the outbox, in order.
2. Sends them in batches to the Go backend via `POST /sync/push`.
3. The backend validates, applies (idempotent via UUID `event_id`), and persists to central Postgres.
4. The backend returns a confirmation → the device marks the events as `synced`.
5. The device also runs `GET /sync/pull?since=<cursor>` to receive changes from other terminals at the same store (e.g. a catalog update made by another terminal or by the back office).

**Conflict resolution:** for the MVP, use "last-write-wins" per field with a server timestamp, EXCEPT for stock balances and store-credit ("fiado") balances, which must be treated as **incremental events (deltas)**, never as absolute values — this prevents two offline terminals from stomping on each other's balance when they sync.

**Idempotency:** every event has a client-generated `event_id` UUID created at the moment it's produced. The backend uses this as the deduplication key — resending the same event (e.g. after a network timeout) never duplicates its effect.

---

## 2. Tech Stack

| Layer | Technology | Notes |
|---|---|---|
| Desktop client | **Tauri v2** (Rust core) | Frontend in React + TypeScript (recommended for the maturity of its keyboard-first component ecosystem) |
| Local database | **SQLite** via `tauri-plugin-sql` (sqlx) | Offline source of truth |
| Sync backend | **Go** (Gin or Echo for HTTP) | Simple, performant, easy to operate |
| Central database | **PostgreSQL** | Multi-store, reporting, dashboard |
| Fiscal | **Provider API (Focus NFe)** | Async HTTP calls, with retry queue |
| Printing | ESC/POS via a Rust crate (`escpos` or similar), invoked from Tauri commands | Same pipeline for non-fiscal receipts and DANFCE |
| Auth | JWT per device + operator (cashier PIN) | Each terminal has its own identity |

---

## 3. Data Model

Conceptual schema below. These tables exist both in local SQLite (operational source of truth) and central Postgres (consolidation), with minor type differences (`TEXT`/`UUID`, `REAL`/`NUMERIC`).

### 3.1 Catalog and Stock

```sql
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

-- Multiple barcodes per product (common case: the bulk pallet has one EAN, the single unit has another)
CREATE TABLE product_barcodes (
    id          TEXT PRIMARY KEY,
    product_id  TEXT NOT NULL REFERENCES products(id),
    barcode     TEXT NOT NULL UNIQUE,
    unit_id     TEXT NOT NULL REFERENCES product_units(id)
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
```

### 3.2 Customers and Store Credit ("Fiado")

```sql
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
```

### 3.3 Sales, Payments and Cash Register

```sql
CREATE TABLE cash_sessions (
    id              TEXT PRIMARY KEY,
    device_id       TEXT NOT NULL,
    operator_id     TEXT NOT NULL,
    opening_amount  NUMERIC(12,2) NOT NULL,
    closing_amount  NUMERIC(12,2),          -- filled in at close
    expected_amount NUMERIC(12,2),           -- calculated: opening + cash sales + cash_in - cash_out
    status          TEXT NOT NULL,           -- 'open','closed'
    opened_at       TEXT NOT NULL,
    closed_at       TEXT
);

CREATE TABLE cash_movements (
    id              TEXT PRIMARY KEY,
    cash_session_id TEXT NOT NULL REFERENCES cash_sessions(id),
    type            TEXT NOT NULL,           -- 'cash_out' (sangria) or 'cash_in' (suprimento)
    amount          NUMERIC(12,2) NOT NULL,
    reason          TEXT,
    created_at      TEXT NOT NULL
);

CREATE TABLE sales (
    id              TEXT PRIMARY KEY,        -- UUID = event_id
    cash_session_id TEXT NOT NULL REFERENCES cash_sessions(id),
    customer_id     TEXT REFERENCES customers(id),  -- nullable = unidentified consumer
    status          TEXT NOT NULL,            -- 'completed','cancelled','returned','partially_returned'
    subtotal        NUMERIC(12,2) NOT NULL,
    discount_total  NUMERIC(12,2) NOT NULL DEFAULT 0,
    total           NUMERIC(12,2) NOT NULL,
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
    quantity        NUMERIC(14,4) NOT NULL,   -- in the chosen unit_id
    quantity_stock  NUMERIC(14,4) NOT NULL,   -- converted to stock_unit (for auditing)
    unit_price      NUMERIC(12,4) NOT NULL,
    discount        NUMERIC(12,2) NOT NULL DEFAULT 0,
    line_total      NUMERIC(12,2) NOT NULL
);

CREATE TABLE sale_payments (
    id              TEXT PRIMARY KEY,
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    method          TEXT NOT NULL,            -- 'cash','debit','credit','pix','boleto','store_credit'
    amount          NUMERIC(12,2) NOT NULL,
    installments    INTEGER DEFAULT 1,        -- installment credit purchases
    gateway_ref      TEXT                      -- card-terminal/Pix reference, if any, post-MVP
);

CREATE TABLE returns (
    id                  TEXT PRIMARY KEY,
    original_sale_id    TEXT NOT NULL REFERENCES sales(id),
    reason              TEXT,
    total_refunded       NUMERIC(12,2) NOT NULL,
    refund_method        TEXT NOT NULL,        -- may differ from the original payment (e.g. store credit)
    operator_id           TEXT NOT NULL,
    created_at             TEXT NOT NULL
);

CREATE TABLE return_items (
    id              TEXT PRIMARY KEY,
    return_id       TEXT NOT NULL REFERENCES returns(id),
    sale_item_id    TEXT NOT NULL REFERENCES sale_items(id),
    quantity        NUMERIC(14,4) NOT NULL
);
```

### 3.4 Fiscal

```sql
CREATE TABLE fiscal_documents (
    id                  TEXT PRIMARY KEY,
    sale_id             TEXT NOT NULL REFERENCES sales(id),
    doc_type            TEXT NOT NULL,         -- 'NFCE','NFE' (future B2B)
    status              TEXT NOT NULL,         -- 'pending','processing','authorized','rejected','contingency','cancelled'
    provider_ref        TEXT,                  -- external id at the fiscal provider
    access_key          TEXT,                  -- 44-digit access key, once authorized
    xml_url             TEXT,
    danfe_url           TEXT,
    rejection_reason    TEXT,
    attempts            INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

-- local emission queue — anything not yet confirmed as authorized
CREATE TABLE fiscal_queue (
    id                  TEXT PRIMARY KEY,
    fiscal_document_id  TEXT NOT NULL REFERENCES fiscal_documents(id),
    payload_json        TEXT NOT NULL,        -- sent to the fiscal provider using its required Portuguese field names
    next_retry_at       TEXT,
    retry_count         INTEGER NOT NULL DEFAULT 0
);
```

> **Note on `payload_json`:** the JSON body sent to the fiscal provider's API (Focus NFe or similar) must use the field names and Portuguese values defined by that provider's contract — e.g. `natureza_operacao`, `cfop`, `cliente`, `produtos`, `impostos`. Do not translate these keys; they are part of an external API contract, not your codebase's naming.

### 3.5 Sync / Outbox

```sql
CREATE TABLE outbox_events (
    id              TEXT PRIMARY KEY,        -- = event_id, used for idempotency in the backend
    entity_type     TEXT NOT NULL,            -- 'sale','stock_movement','customer_ledger','cash_movement', etc.
    entity_id       TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    device_id       TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    synced_at       TEXT                      -- NULL = pending
);
```

---

## 4. Detailed Functional Flows

### 4.1 Fast sale screen (keyboard-first)

**Goal:** the operator never needs to touch the mouse during a normal sale.

- Search field is always focused when the screen opens.
- The barcode scanner works as a "keyboard" (HID) — the app listens for fast input followed by Enter and treats it as a barcode read, with no special field required.
- Text search: partial name, SKU, barcode — results appear in a list navigable with arrow keys (↑↓), Enter confirms.
- Fixed keyboard shortcuts, shown discreetly on screen (footer):
  - `F2` – search product
  - `F3` – identify customer (CPF/CNPJ)
  - `F4` – apply discount
  - `F5` – change the item's selling unit (e.g. bag → fractional kg)
  - `F8` – finalize sale / go to payment
  - `F9` – cancel item
  - `ESC` – cancel sale in progress
- When adding an item that has more than one active `product_units` entry, a fast selector opens (numeric, 1-2-3) with no mouse required.
- The payment screen allows multiple methods on the same sale (writes multiple `sale_payments` rows), validating that the sum matches the total.

### 4.2 Fractional / bulk sales and unit conversion

Flow: product registration defines a `stock_unit` (the stock "truth") and one or more `product_units` with a conversion factor.

Practical example — Cement:
- `stock_unit = KG`
- `product_units`:
  - "50kg bag" → `factor_to_stock = 50`, `sell_price` per bag
  - "Loose kg" → `factor_to_stock = 1`, `sell_price` per kg (fractional)

At sale time, the operator picks the unit; the system:
1. Writes `sale_items.quantity` in the chosen unit and `sale_items.quantity_stock = quantity * factor_to_stock`.
2. Generates a `stock_movements` row with `delta_quantity = -quantity_stock`.
3. NFC-e is emitted with the correct commercial unit per the fiscal provider's rules (most accept a custom unit in the payload).

Rebar cut by the meter follows the same pattern: `stock_unit = M`, selling unit "12m bar" with factor 12, plus a fractional per-meter sale option.

### 4.3 Store credit / running tab ("fiado")

- When closing a sale, if the payment method is "store_credit" (fiado), an identified customer is required (CPF/CNPJ mandatory).
- The system checks the customer's current balance owed (`SUM(debit) - SUM(credit)` in `customer_ledger`) plus the new sale amount against `credit_limit`. If it's exceeded, the operator gets a warning (not blocked by default in the MVP — just a warning; whether to hard-block is a business decision for the store owner).
- Creates a `debit` entry in the ledger with a configurable `due_date` (default: 30 days, configurable per customer).
- A separate "Accounts receivable" screen lists customers with an outstanding balance, allows recording a partial payment (`credit` entry), with the option to print a simple (non-fiscal) receipt.

### 4.4 Cash register (open/close, cash-out/cash-in)

- Sales cannot start without an open `cash_sessions` on the device.
- Opening: operator enters the starting cash amount → creates `cash_sessions` with `status='open'`.
- Cash-out/cash-in ("sangria"/"suprimento"): free-form entries at any time during an open session, always with a required reason.
- Closing: the system calculates `expected_amount` (opening + cash sales + cash_in − cash_out), the operator enters the physically counted amount, the system shows the difference (cash discrepancy) and records both values for auditing — closing is not blocked even if there's a discrepancy.

### 4.5 Returns / exchanges

- Always tied to an original sale (`original_sale_id`) — searchable by sale number, NFC-e access key, or customer CPF + date.
- Item-by-item selection (returning the whole sale is not required).
- Generates a positive (inbound) `stock_movements` entry for returned items.
- Generates a new fiscal document of the appropriate type (return/reversal note) via the fiscal provider, referencing the original note's access key.
- Refund can be in cash, card reversal (outside automation scope — record only), or store credit (`customer_ledger` type `credit`, useful for fiado customers).

### 4.6 Stock and purchasing

- Every stock change goes through `stock_movements` (never a direct update to `stock_levels.quantity` — the value in `stock_levels` is a **materialized projection** recalculated from the movements, to preserve auditability and enable delta-based sync).
- Low-stock alert: the dashboard and product screen flag items where `stock_levels.quantity <= min_quantity`.
- Simple purchase orders: creates `purchase_orders` + `purchase_order_items`; on receiving goods (full or partial), generates positive `stock_movements` and updates `quantity_received`.

### 4.7 Reports (what the owner actually looks at)

All calculated from data already synced to central Postgres (a simple web dashboard, outside the Tauri app — can be a lightweight web page served by the Go backend itself, with no need for a separate frontend in the MVP):

- **Margin per product**: `(sell_price - cost_price) / sell_price`, aggregated by period.
- **Dead stock**: products with no `stock_movements` of type `sale` in the last N days (configurable, default 60).
- **Average ticket**: `SUM(sales.total) / COUNT(sales.id)` by period.
- **ABC curve**: product ranking by cumulative revenue — class A = top products making up 80% of revenue, B = next 15%, C = last 5%.

All of these queries run directly against `sales`, `sale_items`, `products` in Postgres — no BI engine needed, just parameterized views/queries exposed through simple endpoints (`GET /reports/margin`, `/reports/dead-stock`, `/reports/avg-ticket`, `/reports/abc-curve`).

---

## 5. Fiscal Integration (NFC-e Provider)

### 5.1 Emission flow

1. The sale is finalized locally (`sales.status = 'completed'`) — **this already counts as a valid sale operationally**, regardless of fiscal status.
2. The system creates a `fiscal_documents` row with `status='pending'` and enqueues it in `fiscal_queue`.
3. The sync worker (same process that syncs sales, or a dedicated worker) attempts emission via the provider's API:
   - If authorized → updates `fiscal_documents` with `access_key`, `xml_url`, `danfe_url`, `status='authorized'` → triggers DANFCE printing.
   - If rejected (validation error, e.g. invalid NCM) → `status='rejected'`, a visible alert lets the operator fix the product registration and re-emit manually.
   - If a connectivity/SEFAZ-unavailable error → stays in `fiscal_queue`, retried with exponential backoff (`next_retry_at`).

### 5.2 Contingency mode

- If there's no connection to the fiscal API at the moment of sale, the receipt is printed immediately as a **non-fiscal proof of purchase** (clearly labeled "PROVISIONAL — AWAITING FISCAL EMISSION"), the sale proceeds normally, and the fiscal document stays in the queue.
- When connectivity returns, actual emission happens in the background, and the app can optionally reprint the final DANFCE, or simply keep the XML/access key available on the sale screen for on-demand reprinting.
- **Important:** confirm with your chosen provider the maximum time allowed for contingency emission (typically up to 24h) — this should become an alert on the back-office dashboard if fiscal documents remain pending past that window.

### 5.3 Digital certificate

- A1 digital certificate (.pfx) registered with the fiscal provider — it doesn't need to live on the client device, which greatly simplifies things (single upload in the provider's dashboard).
- Set up an expiration alert at least 30 days in advance (most providers already expose this via API/dashboard).

---

## 6. Printing (ESC/POS)

- ESC/POS commands sent via Rust (the `escpos` crate, or a direct implementation of the commands over `serialport`/USB), triggered by a Tauri command invoked from the frontend.
- The same "receipt" generation pipeline serves both:
  - Non-fiscal receipts (e.g. store-credit proof, partial-payment receipt)
  - DANFCE (the printed representation of the NFC-e, 58mm or 80mm depending on the printer)
- Cash drawer opening: an ESC/POS "kick" (pulse) command sent through the printer itself (standard RJ11/RJ12 connector) — no extra hardware needed.
- Recommend abstracting the printer behind a dedicated Rust interface (`trait Printer`) from day one, even if the MVP only supports one model — this makes it much easier to add other models/protocols later without touching business logic.

---

## 7. Suggested Project Structure

### 7.1 Client (Tauri)

```
pdv-client/
├── src/                      # React + TypeScript frontend
│   ├── screens/
│   │   ├── Sale/              # fast sale screen
│   │   ├── CashRegister/      # open/close
│   │   ├── Customers/         # registry + store credit
│   │   ├── Inventory/         # products, stock, purchasing
│   │   └── Returns/
│   ├── hooks/
│   ├── lib/api.ts             # calls into Tauri commands (invoke)
│   └── state/                 # state management (Zustand/Redux)
├── src-tauri/
│   ├── src/
│   │   ├── db/                 # SQLite layer (sqlx), migrations
│   │   ├── commands/            # commands exposed to the frontend (invoke handlers)
│   │   │   ├── sales.rs
│   │   │   ├── inventory.rs
│   │   │   ├── customers.rs
│   │   │   ├── cash.rs
│   │   │   └── fiscal.rs
│   │   ├── printing/            # ESC/POS abstraction
│   │   ├── sync/                 # sync worker (outbox)
│   │   ├── fiscal_client/        # fiscal provider HTTP client
│   │   └── main.rs
│   └── tauri.conf.json
```

### 7.2 Backend (Go)

```
pdv-backend/
├── cmd/api/main.go
├── internal/
│   ├── sync/               # push/pull endpoints, event resolution
│   ├── sales/
│   ├── customers/
│   ├── inventory/
│   ├── reports/            # margin, ABC curve, avg ticket, dead-stock queries
│   ├── fiscal/             # orchestration/proxy with the provider (if needed)
│   └── auth/                # JWT per device + operator
├── migrations/               # SQL migrations (e.g. golang-migrate)
└── go.mod
```

---

## 8. Development Roadmap (MVP)

| Phase | Estimated duration | Deliverables |
|---|---|---|
| **0. Foundation** | 1–2 weeks | Tauri + local SQLite setup, Go + Postgres setup, initial schema, basic device/operator auth |
| **1. Basic offline sale** | 2–3 weeks | Keyboard-first sale screen, product catalog, cart, total calculation, cash register (open/close/cash-out/cash-in) |
| **2. Fractional units** | 1 week | `product_units` model, stock conversion, unit selection at sale time |
| **3. Multiple payments** | 1 week | `sale_payments`, split payment, store credit as a payment method |
| **4. Customers and store credit** | 1–2 weeks | CPF/CNPJ registry, `customer_ledger`, accounts-receivable screen, credit limit |
| **5. Synchronization** | 2 weeks | Local outbox, push/pull endpoints on the Go backend, delta-based conflict resolution, reconnection testing |
| **6. Fiscal integration** | 2–3 weeks | Provider integration (homologation/sandbox), emission queue, contingency mode, DANFCE printing |
| **7. Returns** | 1 week | Return flow tied to the original sale + fiscal reversal note |
| **8. Stock and purchasing** | 1 week | Low-stock alerts, simple purchase orders, receiving |
| **9. Reporting** | 1 week | Simple web dashboard (margin, dead stock, average ticket, ABC curve) |
| **10. Pilot at a real store** | 2–4 weeks | Fiscal homologation → production, close support for the first customer, adjustments |

**Total estimate: ~16–22 weeks** for a solo developer working full-time, or proportionally less with a small team (2–3 people) working in parallel across client/backend/fiscal.

---

## 9. Testing and Fiscal Homologation

- Every emission must first be tested in **SEFAZ's homologation (sandbox) environment** — every fiscal provider offers this — never test in production with a real CNPJ.
- Checklist before going to production:
  - [ ] NFC-e emission successfully authorized in homologation, for at least 3 different taxation scenarios (products with distinct CST/CSOSN codes, if the customer has a mixed tax regime)
  - [ ] Return/reversal note emission tested and correctly linked to the original note
  - [ ] Contingency mode tested (simulate an internet outage mid-sale)
  - [ ] DANFCE reprint tested
  - [ ] Sync tested with the device fully offline for an extended period (e.g. 24h) and then reconnected
  - [ ] Cash-register closing with a discrepancy (over/short) tested
  - [ ] A1 digital certificate valid and registered with the provider, with expiration alert configured

---

## 10. Assumptions and Open Questions

- **Pilot customer's tax regime**: confirm whether it's Simples Nacional (most likely for a small store) — this determines CSOSN vs CST and significantly simplifies the MVP's tax logic.
- **TEF (card-terminal integration)**: out of scope for the MVP — debit/credit payments are logged manually by the operator (amount + method), with no automatic integration with the card machine. This is acceptable for the pilot and can be added later via integration with Stone, Cielo, Rede, etc.
- **Pix**: in the MVP, treated as just another manual payment method (operator confirms receipt, optionally with a static QR code shown on screen — dynamic QR code generation via a bank API is left for a later phase).
- **Integrated scale**: mentioned as post-MVP — in the MVP, weight is entered manually by the operator during a fractional sale.
- **B2B NF-e (not NFC-e)**: if the pilot customer sells to farmers/contractors with a CNPJ that require a formal NF-e (not just NFC-e), this should be validated early — most fiscal providers support both, but confirm the specific NF-e payload rather than assuming it's identical to NFC-e.

---

## 11. Immediate Next Steps

1. Validate the pilot customer's tax regime and product profile (Simples Nacional? Which NCMs are most common?).
2. Create an account in the fiscal provider's homologation (sandbox) environment (Focus NFe) and emit a test NFC-e end-to-end before writing any PDV code.
3. Set up the Tauri project + initial SQLite schema (Roadmap Phase 0).
4. Set up the Go backend + Postgres with the sync schema (Roadmap Phase 0).
