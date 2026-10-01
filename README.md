# PDV Monorepo

Offline-first point-of-sale system for agricultural supplies and construction
materials stores in Brazil. Desktop client (Tauri) + cloud sync backend
(Go + Postgres). See [docs/PDV-MVP-Implementation-Plan.md](docs/PDV-MVP-Implementation-Plan.md)
for the full architecture, data model and roadmap.

**Status:** Phase 9 (Reporting) — on top of Phase 1-5's offline, syncing
client (keyboard-first sale screen, fractional/bulk units, split
payments, cash register, product catalog, CPF/CNPJ customers with
store-credit, outbox-based sync with the Go backend) and Phase 6's fiscal
integration (finalizing a sale creates a `fiscal_documents`/`fiscal_queue`
pair, attempts one best-effort NFC-e emission against Focus NFe inline,
and falls back to a queued, backoff-retried contingency state on any
connectivity failure, with receipts printing through an ESC/POS pipeline
that defaults to a console driver in dev). Phase 7 adds item-by-item
returns tied to an original sale (inbound stock movements, an optional
store-credit refund, and the resulting `partially_returned`/`returned`
status), plus a best-effort whole-document NFC-e cancellation attempt
whenever a return brings the sale to fully `returned` — the closest
fiscal remedy Focus NFe's NFC-e contract actually offers; see the
deviations section below on why that's not the linked "reversal note"
the plan itself describes, and why partial returns stay bookkeeping-only.
Phase 8 adds
low-stock alerts (a configurable per-product `min_quantity` threshold,
flagged both on the product screen and in a dedicated purchasing view)
and simple purchase orders: create an order against a supplier, then
record goods received — full or across several partial deliveries —
which drives the same stock-movement pipeline sales and returns use.
Phase 9 adds the owner-facing back-office reports (margin per product,
dead stock, average ticket, ABC curve, all computed live from central
Postgres) behind `GET /reports/*` on the Go backend, plus a `GET
/dashboard` static page serving them with no separate frontend — see
"Backend dev notes" below. It also syncs `fiscal_documents` centrally
for the first time (previously local-only, see the deviations section)
so `/reports/fiscal-alerts` can flag a document stuck pending past the
plan's 24h contingency window.

## Layout

```
app/                 # Tauri v2 + React + TypeScript desktop app (local SQLite)
api/                  # Go (Gin) sync backend (Postgres)
docs/
└── PDV-MVP-Implementation-Plan.md
docker-compose.yml   # local Postgres for dev
Makefile             # common dev commands
```

## Prerequisites

- [Node.js](https://nodejs.org/) 18+ and npm
- [Rust](https://www.rust-lang.org/tools/install) (stable) — required by Tauri
- [Go](https://go.dev/dl/) 1.23+
- [Docker](https://www.docker.com/) (Postgres runs via `docker compose`)
- Tauri's platform dependencies for your OS — see the
  [Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/)
  (on macOS this is just Xcode Command Line Tools)

## First-time setup

```bash
cp .env.example .env   # adjust values if needed; defaults work for local dev
cd app && npm install && cd ..
```

## Running everything locally

In separate terminals, from the repo root:

```bash
make db-up            # starts Postgres via docker-compose
make migrate-backend   # applies Postgres migrations
make dev-backend        # starts the Go API on :8080
make dev-client          # starts the Tauri app (applies SQLite migrations on boot)
```

The client boots to a login screen. It ships with one seeded demo operator
(logged to the backend's console on first boot — PIN `1234`, printed as
`auth: seeded demo operator "Demo Operator" with PIN "1234" (dev only)`).
Signing in calls the backend's `POST /auth/login`, and the returned JWT is
stored locally via `tauri-plugin-store`.

### Client dev notes

- Local data lives in a SQLite database managed by `tauri-plugin-sql`;
  migrations under `app/src-tauri/migrations/` run automatically
  the first time the app opens the database — there's no separate CLI step
  (`make migrate-client` just prints this).
- The backend URL the client calls defaults to `http://localhost:8080`,
  overridable via the `PDV_BACKEND_URL` env var (see `.env.example`).
- Rust module layout under `src-tauri/src/`: `db/` (SQLite + migrations),
  `commands/` (Tauri command handlers invoked from the frontend, one file
  per screen — `sales`, `inventory`, `purchasing`, `returns`, `cash`,
  `customers`, `fiscal`, `auth`), `outbox.rs` (writes an `outbox_events`
  row from inside a command's own transaction), `sync/` (the push/pull
  worker), `printing/` (ESC/POS abstraction), `fiscal_client/` (Focus NFe
  HTTP client).
- The sync worker reads its JWT from the same `tauri-plugin-store` file
  the frontend's login flow writes (`auth-session.json`) and keeps its
  own pull cursor in `sync-state.json`; it does nothing until a device
  has signed in at least once.

### Backend dev notes

- Config is via env vars (`DATABASE_URL`, `JWT_SECRET`, `PORT`); `make`
  loads `.env` automatically for both `docker compose` and `go run`.
- `internal/auth` issues JWTs (backed by a seeded `operators` table) and
  also exposes `RequireAuth()`, a middleware gating `/sync/push`,
  `/sync/pull`, and now `/reports/*` on that same JWT — the plan's auth
  model (section 2) has no separate "owner" role, so Phase 9's reports
  reuse the one login every device already has rather than inventing one.
- `internal/sync` applies pushed events to central Postgres — one
  `apply*` function per outbox entity type, upserting with a
  last-write-wins guard for `product`/`product_unit`/`customer`/
  `fiscal_document` (the types genuinely edited/updated more than once —
  see below on why `fiscal_document` joined this list in Phase 9), a
  plain overwrite for `stock_level` (a configured threshold, not a
  balance — see the deviations section), and appending/delta-applying/
  status-upserting everything else (`sale`, `return`, `purchase_order`,
  ...) — and serves pull as "every other device's events since a
  cursor." `internal/sales`, `internal/customers`, `internal/inventory`,
  and `internal/fiscal` are still empty packages with doc comments
  describing their future responsibility — this project's actual
  sync/business logic lives in `internal/sync`'s `apply*` functions
  rather than being split out per domain package.
- `internal/reports` (Phase 9, plan section 4.7) exposes `GET
  /reports/margin`, `/dead-stock`, `/avg-ticket`, `/abc-curve`, and
  `/fiscal-alerts` (this last one a Phase 9 addition beyond the plan's
  named four — see the deviations section). All are plain parameterized
  SQL against `sales`/`sale_items`/`products`/`fiscal_documents`, no BI
  engine, per the plan's own instruction. Margin and ABC curve net out
  returned quantity per line (proportional to the line's own discount)
  rather than using the plan's literal `sell_price`/`cost_price` formula,
  since `sell_price` actually lives on `product_units` (plan section
  3.1), not `products` — see the package doc comment.
- `internal/dashboard` serves the actual "simple web dashboard" the plan
  asks for (section 4.7) at `GET /dashboard`: a single embedded static
  HTML+vanilla-JS page (no build step, no new frontend dependency) that
  logs in via the existing `POST /auth/login` and then calls
  `/reports/*` client-side with the returned bearer token, same as any
  other authenticated API caller — the Go side stays a plain static-file
  route, no server-side templating or session state.
- `GET /healthz` reports Postgres reachability.

### Fiscal & printing dev notes

- Fiscal emission runs entirely on the client (`app/src-tauri/src/fiscal_client/`)
  and talks directly to Focus NFe — matching the plan's own architecture
  diagram (section 1.1), which draws a separate arrow from the store
  straight to the Fiscal Provider API, not through the Go backend.
  `internal/fiscal` on the backend stays an empty stub for that reason,
  not because it was skipped.
- **No sandbox account required for local dev.** With
  `FISCAL_PROVIDER_TOKEN` unset, `fiscal_client` never calls the
  provider — every sale still gets a `fiscal_documents`/`fiscal_queue`
  pair and prints a provisional receipt, but emission just sits queued.
  The background worker logs a one-line reminder
  (`fiscal: no provider token configured, skipping emission (N
  document(s) waiting in the queue)`) instead of erroring. Set
  `FISCAL_PROVIDER_TOKEN` (and `FISCAL_ISSUER_CNPJ`) once you have a real
  Focus NFe homologation account to test emission end to end — see plan
  section 9's checklist before ever pointing `FISCAL_ENV=producao` at a
  real CNPJ.
- The NFC-e request/response contract (`fiscal_client::NfcePayload`/
  `NfceResponse`) was built against Focus NFe's published docs
  (`doc.focusnfe.com.br`), not verified against a live sandbox call —
  there was no account to test against yet (see above). Re-verify field
  names during homologation testing before production use.
- Receipt printing (`app/src-tauri/src/printing/`) wraps the `escpos`
  crate behind this project's own `Printer` trait (plan section 6's
  explicit recommendation). `PDV_PRINTER` selects the driver at runtime —
  `console` (default; prints to stdout, no hardware needed for dev),
  `network:<host>:<port>`, `serial:<path>[:baud]`, or `file:<path>` (a
  USB printer exposed as a raw device file, e.g. `/dev/usb/lp0` on
  Linux). True USB (vendor/product id) drivers need `escpos`'s
  `usb`/`native_usb` feature, not enabled here to avoid a system libusb
  build dependency — add it in `Cargo.toml` if a specific printer model
  needs it.

## Known deviations from the implementation plan

Section 3 of the plan doesn't include `operators` or `devices` tables —
every other table just carries a bare `device_id`/`operator_id` string.
Phase 0 needs somewhere to actually check a PIN against, so
`api/migrations/000006_auth.up.sql` adds minimal `operators` and
`devices` tables for this purpose. Devices auto-register on first
successful login; there's no separate device-provisioning flow yet.

Section 3.2's `customers` table has no column for the "configurable per
customer" due-date term section 4.3 describes for `customer_ledger.due_date`
("default: 30 days, configurable per customer"). Phase 4 adds
`customers.default_due_days` (migrations `0006_customer_due_days` on the
client, `000007_customer_due_days` on the backend, both defaulting to 30)
to close that gap.

Section 3.5 names four outbox entity types — `sale`, `stock_movement`,
`customer_ledger`, `cash_movement` — but central Postgres enforces foreign
keys from those tables onto `products`, `product_units`, `customers`, and
`cash_sessions`, none of which the plan lists as outbox entities. Without
syncing them too, the first sale pushed from a real device would violate a
foreign key centrally (its product/customer/cash_session would never have
arrived). Phase 5 therefore also syncs `product`, `product_unit`,
`customer`, and `cash_session` as outbox events — see
`api/internal/sync/sync.go`'s package comment and the mirrored emission
points in `app/src-tauri/src/commands/{inventory,customers,cash}.rs`.
This also needed a per-field last-write-wins timestamp (plan section 1.3)
on `product_units` and `customers`, which section 3's schema doesn't
define either — migrations `0007_sync_lww_columns` (client) and
`000008_sync_lww_columns` (backend) add `updated_at` to both, mirroring
`products.updated_at`, which the plan's schema already has.

Section 3 has no store/company table for the issuing CNPJ, address, etc.
Phase 6 doesn't add one — Focus NFe's own dashboard already ties one
account/token to one CNPJ and holds the rest of that registration data
(see plan section 5.3 on the digital certificate living with the
provider, not the client), so `FISCAL_ISSUER_CNPJ` is a plain env var
(`config::fiscal_issuer_cnpj`) rather than a new table.

Section 3.4's `fiscal_documents` columns have nowhere to keep the
ready-made QR-code URL Focus NFe's emission response returns
(`qrcode_url`), which an accurate DANFCE print needs (plan section 6).
Migration `0008_fiscal_qrcode` (client-only — this table isn't synced to
the backend, see below) adds one column for it.

`fiscal_documents`/`fiscal_queue` were **not** synced to the backend
through Phase 8 (not in `outbox_events`) — fiscal status was treated as a
local-device, provider-facing concern in the plan's own architecture
(section 1.1), and the back-office alert plan section 5.2 describes for a
fiscal document stuck pending past 24h was left as Phase 9 (reporting)
scope, not Phase 6.

**Phase 9 reverses this for `fiscal_documents` specifically** (per an
explicit decision, not a silent scope change): the plan does want that
24h-stuck alert, and it needs central visibility to compute, so every
status transition (`pending` at creation, then `authorized`/`rejected` —
`app/src-tauri/src/commands/sales.rs` and
`fiscal_client::mark_authorized`/`mark_rejected`) now also records a
`fiscal_document` outbox event (`internal/sync.applyFiscalDocument`).
`fiscal_queue` itself stays local-only — its retry bookkeeping
(`next_retry_at`, backoff state) has no back-office use. See
`migrations/000010_fiscal_documents.up.sql`'s comment; the central table
already existed from Phase 0's schema-parity migration
(`000004_fiscal.up.sql`), just unpopulated until now.

`GET /reports/fiscal-alerts` (`?hours=24`, matching the plan's own "up to
24h" language) is a Phase 9 addition beyond the plan's four explicitly
named report endpoints (margin/dead-stock/avg-ticket/abc-curve) — the
plan clearly wants this alert to exist (section 5.2) but never names its
endpoint.

Section 3.1's `stock_levels.min_quantity` (the low-stock trigger, plan
section 4.6) isn't in section 3.5's list of outbox entity types either,
for the same reason `product`/`customer`/etc. weren't (see above): a
multi-store dashboard needs it centrally too. Phase 8 syncs it as its own
`stock_level` event, applied as a plain overwrite rather than sharing
`stock_movement`'s delta-based path — it's a directly-set threshold, not
a balance, so overwriting it can't clobber a concurrent stock delta the
way overwriting `quantity` itself would.

Section 3.1's `purchase_orders.status` lists four values
(`draft`/`sent`/`partially_received`/`received`), but this MVP has no
edit-before-sending step, so `create_purchase_order` skips `draft`
entirely and creates orders already `sent` — see the doc comment on
`app/src-tauri/src/commands/purchasing.rs`.

The plan's returns flow (section 4.5) calls for "a new fiscal document of
the appropriate type (return/reversal note)" referencing the original
note's access key. Focus NFe's actual NFC-e contract has no such linked
reversal the way a formal NF-e devolução would — confirmed against
<https://doc.focusnfe.com.br/reference/cancelar_nfce> — a genuine mismatch
with the plan's wording. The only fiscal remedy it offers at all is
`DELETE /v2/nfce/{ref}` (same `ref` used at emission), cancelling the
*entire* original document, with a required 15-255 character
`justificativa`, and **only within 30 minutes of authorization** — tighter
than this project's own emission-side doc comment originally guessed
("same-day"). Phase 7 attempts exactly that, and only in the one case it
can actually represent: `create_return` fires a best-effort cancellation
whenever a return (a single call, or the last of several partial ones)
brings the sale to fully `returned`; a partial return has no equivalent in
Focus's contract and stays bookkeeping-only, as before. The return itself
is never blocked on the outcome (stock, ledger, and `sales.status` always
update) — on anything short of success, `fiscal_documents.status` just
stays `authorized` with a new `cancellation_note` column recording why
(past the window, rejected by SEFAZ, or a connectivity failure), synced
centrally like the rest of `fiscal_documents` since Phase 9, and visible
on the Returns screen with a manual retry
(`commands::fiscal::retry_fiscal_cancellation`) for as long as the window
stays open. See the doc comments on
`app/src-tauri/src/commands/returns.rs` and
`app/src-tauri/src/fiscal_client/mod.rs`'s cancellation section for the
full reasoning, including why this is a single inline attempt rather than
a retried background queue like emission's.

Two MVP simplifications inside `fiscal_client::build_payload`, both
flagged in code comments and worth revisiting before a real pilot:
`icms_origem` (origin of goods) is hardcoded `"0"` (nacional) since
section 3.1's `products` schema has no origin column, and
`local_destino` assumes the buyer is in the same state as the store,
since `customers.address_json` (section 3.2) isn't structured enough to
compute this per sale. A missing `cfop_default` on a product falls back
to `"5102"` rather than blocking the sale — the provider's own validation
is what actually catches a genuinely wrong tax setup (a `rejected`
fiscal document the operator fixes and re-emits, per section 5.1), not a
local check.

Section 4.7's margin formula, `(sell_price - cost_price) / sell_price`,
names `sell_price`, which lives on `product_units` (a product can have
several, section 3.1), not on `products` alongside `cost_price`. Phase 9
computes the same thing from realized transactions instead — net
revenue/cost per product from `sale_items`, returns-netted — rather than
a static per-unit price, which also makes it accurate across a product
with multiple selling units at different prices. `GET /reports/margin`
and `/reports/abc-curve` both default to the trailing 30 days when
`from`/`to` are omitted; the plan says "aggregated by period" without
naming a default window. ABC curve classification (section 4.7: "A = top
80%, B = next 15%, C = last 5%") is keyed off each product's cumulative
percentage *before* it, not including it — otherwise the single product
that actually crosses the 80% line lands in B or C instead of A, and a
one-product store (100% of revenue by definition) would always be C; see
the doc comment on `internal/reports.ABCCurve`.
