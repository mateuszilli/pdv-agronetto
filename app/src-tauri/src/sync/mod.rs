//! Outbox sync worker (plan sections 1.3 and 3.5).
//!
//! Reads pending rows from the local `outbox_events` table and pushes them
//! to the Go backend's `POST /sync/push`, then pulls remote changes via
//! `GET /sync/pull?since=<cursor>`. Idempotent via each event's UUID
//! `event_id`. Conflict resolution: last-write-wins per field (guarded by
//! an `updated_at` comparison for `product`/`product_unit`/`customer`),
//! except stock and store-credit balances, which sync as deltas and are
//! never overwritten wholesale.
//!
//! Every command that mutates a syncable row (see `commands::{inventory,
//! customers, sales, cash}`) writes an `outbox_events` row in the same
//! local transaction — this module only reads that table and the backend,
//! it never decides what belongs in it.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::{Pool, Sqlite};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::config::backend_url;
use crate::db;

const SYNC_INTERVAL: Duration = Duration::from_secs(15);
const PUSH_BATCH_SIZE: i64 = 200;

const AUTH_STORE_FILE: &str = "auth-session.json";
const SYNC_STATE_STORE_FILE: &str = "sync-state.json";
const CURSOR_KEY: &str = "cursor";
const EPOCH: &str = "1970-01-01T00:00:00Z";

#[derive(Debug, Deserialize)]
struct StoredSession {
    token: String,
}

fn load_session<R: Runtime>(app: &AppHandle<R>) -> Option<StoredSession> {
    let store = app.store(AUTH_STORE_FILE).ok()?;
    let value = store.get("session")?;
    serde_json::from_value(value).ok()
}

fn load_cursor<R: Runtime>(app: &AppHandle<R>) -> String {
    app.store(SYNC_STATE_STORE_FILE)
        .ok()
        .and_then(|s| s.get(CURSOR_KEY))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| EPOCH.to_string())
}

fn save_cursor<R: Runtime>(app: &AppHandle<R>, cursor: &str) {
    if let Ok(store) = app.store(SYNC_STATE_STORE_FILE) {
        store.set(CURSOR_KEY, JsonValue::String(cursor.to_string()));
        let _ = store.save();
    }
}

#[derive(Debug, Serialize, Default, Clone, Copy)]
pub struct SyncSummary {
    pub pushed: i64,
    pub pulled: i64,
}

/// One push/pull cycle. Returns `Err` (logged by the caller, never fatal)
/// if the device isn't signed in yet or either leg failed — whatever did
/// succeed is still reflected in the returned counts on a partial failure.
pub async fn run_once<R: Runtime>(app: &AppHandle<R>) -> Result<SyncSummary, String> {
    let session = load_session(app).ok_or_else(|| "not signed in yet".to_string())?;
    let pool = db::pool(app).await?;

    let mut errors = Vec::new();

    let pushed = match push_pending(&pool, &session).await {
        Ok(n) => n,
        Err(e) => {
            errors.push(format!("push: {e}"));
            0
        }
    };
    let pulled = match pull_remote(app, &pool, &session).await {
        Ok(n) => n,
        Err(e) => {
            errors.push(format!("pull: {e}"));
            0
        }
    };

    if errors.is_empty() {
        Ok(SyncSummary { pushed, pulled })
    } else {
        Err(errors.join("; "))
    }
}

/// Manual "sync now" entry point for a status indicator in the shell —
/// the background worker (`spawn_worker`) already runs this on an
/// interval, but reconnection testing (plan section 9) is a lot easier
/// with a button that doesn't wait up to `SYNC_INTERVAL`.
#[tauri::command]
pub async fn sync_now<R: Runtime>(app: AppHandle<R>) -> Result<SyncSummary, String> {
    run_once(&app).await
}

/// Spawns the background sync loop. Call once from `lib.rs`'s `.setup()`.
pub fn spawn_worker<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = run_once(&app).await {
                eprintln!("sync: {e}");
            }
            tokio::time::sleep(SYNC_INTERVAL).await;
        }
    });
}

// --- Push ---

#[derive(Debug, sqlx::FromRow)]
struct OutboxRow {
    id: String,
    entity_type: String,
    entity_id: String,
    payload_json: String,
    device_id: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct PushEvent {
    id: String,
    entity_type: String,
    entity_id: String,
    payload_json: JsonValue,
    device_id: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct PushRequest {
    events: Vec<PushEvent>,
}

#[derive(Debug, Deserialize)]
struct PushResponse {
    accepted: Vec<String>,
}

/// Tables that also carry their own `synced_at` column (plan section 3),
/// kept in lockstep with the outbox entry so a UI could show per-row sync
/// status without joining `outbox_events` — keyed by the row's entity_type.
fn mirrored_table(entity_type: &str) -> Option<&'static str> {
    match entity_type {
        "sale" => Some("sales"),
        "stock_movement" => Some("stock_movements"),
        "customer_ledger" => Some("customer_ledger"),
        "return" => Some("returns"),
        _ => None,
    }
}

async fn push_pending(pool: &Pool<Sqlite>, session: &StoredSession) -> Result<i64, String> {
    let rows: Vec<OutboxRow> = sqlx::query_as(
        "SELECT id, entity_type, entity_id, payload_json, device_id, created_at
         FROM outbox_events WHERE synced_at IS NULL ORDER BY created_at LIMIT ?",
    )
    .bind(PUSH_BATCH_SIZE)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(0);
    }

    let events = rows
        .iter()
        .map(|r| {
            let payload_json: JsonValue =
                serde_json::from_str(&r.payload_json).map_err(|e| e.to_string())?;
            Ok(PushEvent {
                id: r.id.clone(),
                entity_type: r.entity_type.clone(),
                entity_id: r.entity_id.clone(),
                payload_json,
                device_id: r.device_id.clone(),
                created_at: r.created_at.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let client = reqwest::Client::new();
    let url = format!("{}/sync/push", backend_url());
    let response = client
        .post(&url)
        .bearer_auth(&session.token)
        .json(&PushRequest { events })
        .send()
        .await
        .map_err(|e| format!("failed to reach backend at {url}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("push rejected ({status}): {body}"));
    }

    let parsed: PushResponse = response.json().await.map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    for row in &rows {
        if !parsed.accepted.contains(&row.id) {
            continue;
        }
        sqlx::query("UPDATE outbox_events SET synced_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&row.id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;

        if let Some(table) = mirrored_table(&row.entity_type) {
            sqlx::query(&format!("UPDATE {table} SET synced_at = ? WHERE id = ?"))
                .bind(&now)
                .bind(&row.entity_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(parsed.accepted.len() as i64)
}

// --- Pull ---

#[derive(Debug, Deserialize)]
struct PulledEvent {
    id: String,
    entity_type: String,
    payload_json: JsonValue,
}

#[derive(Debug, Deserialize)]
struct PullResponse {
    events: Vec<PulledEvent>,
    cursor: String,
}

async fn pull_remote<R: Runtime>(
    app: &AppHandle<R>,
    pool: &Pool<Sqlite>,
    session: &StoredSession,
) -> Result<i64, String> {
    let cursor = load_cursor(app);

    let client = reqwest::Client::new();
    let url = format!("{}/sync/pull", backend_url());
    let response = client
        .get(&url)
        .bearer_auth(&session.token)
        .query(&[("since", cursor.as_str())])
        .send()
        .await
        .map_err(|e| format!("failed to reach backend at {url}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("pull rejected ({status}): {body}"));
    }

    let parsed: PullResponse = response.json().await.map_err(|e| e.to_string())?;

    for event in &parsed.events {
        if let Err(e) = apply_event(pool, event).await {
            eprintln!(
                "sync: failed to apply pulled {} event {}: {e}",
                event.entity_type, event.id
            );
        }
    }

    let count = parsed.events.len() as i64;
    save_cursor(app, &parsed.cursor);

    Ok(count)
}

async fn apply_event(pool: &Pool<Sqlite>, event: &PulledEvent) -> Result<(), String> {
    match event.entity_type.as_str() {
        "product" => apply_product(pool, &event.payload_json).await,
        "product_unit" => apply_product_unit(pool, &event.payload_json).await,
        "customer" => apply_customer(pool, &event.payload_json).await,
        "cash_session" => apply_cash_session(pool, &event.payload_json).await,
        "cash_movement" => apply_cash_movement(pool, &event.payload_json).await,
        "stock_movement" => apply_stock_movement(pool, &event.payload_json).await,
        "customer_ledger" => apply_customer_ledger(pool, &event.payload_json).await,
        "sale" => apply_sale(pool, &event.payload_json).await,
        "return" => apply_return(pool, &event.payload_json).await,
        other => {
            eprintln!("sync: unknown entity_type '{other}', skipping");
            Ok(())
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProductPayload {
    id: String,
    sku: Option<String>,
    barcode: Option<String>,
    name: String,
    ncm: String,
    cfop_default: Option<String>,
    cst_csosn: Option<String>,
    stock_unit: String,
    cost_price: f64,
    active: bool,
    updated_at: String,
}

async fn apply_product(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let p: ProductPayload = serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO products
            (id, sku, barcode, name, ncm, cfop_default, cst_csosn, stock_unit, cost_price,
             active, updated_at, deleted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
         ON CONFLICT(id) DO UPDATE SET
            sku = excluded.sku, barcode = excluded.barcode, name = excluded.name,
            ncm = excluded.ncm, cfop_default = excluded.cfop_default,
            cst_csosn = excluded.cst_csosn, stock_unit = excluded.stock_unit,
            cost_price = excluded.cost_price, active = excluded.active,
            updated_at = excluded.updated_at
         WHERE excluded.updated_at >= products.updated_at",
    )
    .bind(&p.id)
    .bind(&p.sku)
    .bind(&p.barcode)
    .bind(&p.name)
    .bind(&p.ncm)
    .bind(&p.cfop_default)
    .bind(&p.cst_csosn)
    .bind(&p.stock_unit)
    .bind(p.cost_price)
    .bind(p.active)
    .bind(&p.updated_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT OR IGNORE INTO stock_levels (product_id, quantity, min_quantity, updated_at)
         VALUES (?, 0, 0, ?)",
    )
    .bind(&p.id)
    .bind(&p.updated_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct ProductUnitPayload {
    id: String,
    product_id: String,
    unit_label: String,
    factor_to_stock: f64,
    sell_price: f64,
    is_default_sale: bool,
    active: bool,
    updated_at: String,
}

async fn apply_product_unit(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let u: ProductUnitPayload =
        serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO product_units
            (id, product_id, unit_label, factor_to_stock, sell_price, is_default_sale, active,
             updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            unit_label = excluded.unit_label, factor_to_stock = excluded.factor_to_stock,
            sell_price = excluded.sell_price, is_default_sale = excluded.is_default_sale,
            active = excluded.active, updated_at = excluded.updated_at
         WHERE excluded.updated_at >= product_units.updated_at",
    )
    .bind(&u.id)
    .bind(&u.product_id)
    .bind(&u.unit_label)
    .bind(u.factor_to_stock)
    .bind(u.sell_price)
    .bind(u.is_default_sale)
    .bind(u.active)
    .bind(&u.updated_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct CustomerPayload {
    id: String,
    doc_type: String,
    doc_number: String,
    name: String,
    phone: Option<String>,
    address_json: Option<String>,
    credit_limit: f64,
    default_due_days: i64,
    active: bool,
    created_at: String,
    updated_at: String,
}

async fn apply_customer(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let c: CustomerPayload = serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO customers
            (id, doc_type, doc_number, name, phone, address_json, credit_limit,
             default_due_days, active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            doc_type = excluded.doc_type, doc_number = excluded.doc_number,
            name = excluded.name, phone = excluded.phone, address_json = excluded.address_json,
            credit_limit = excluded.credit_limit, default_due_days = excluded.default_due_days,
            active = excluded.active, updated_at = excluded.updated_at
         WHERE excluded.updated_at >= customers.updated_at",
    )
    .bind(&c.id)
    .bind(&c.doc_type)
    .bind(&c.doc_number)
    .bind(&c.name)
    .bind(&c.phone)
    .bind(&c.address_json)
    .bind(c.credit_limit)
    .bind(c.default_due_days)
    .bind(c.active)
    .bind(&c.created_at)
    .bind(&c.updated_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct CashSessionPayload {
    id: String,
    device_id: String,
    operator_id: String,
    opening_amount: f64,
    closing_amount: Option<f64>,
    expected_amount: Option<f64>,
    status: String,
    opened_at: String,
    closed_at: Option<String>,
}

/// Unlike `product`/`customer`, a session only ever has one owning
/// device — this device never opens/closes a session it pulls in, so
/// there's no concurrent-edit race to guard against with a timestamp.
async fn apply_cash_session(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let s: CashSessionPayload =
        serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO cash_sessions
            (id, device_id, operator_id, opening_amount, closing_amount, expected_amount,
             status, opened_at, closed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            closing_amount = excluded.closing_amount, expected_amount = excluded.expected_amount,
            status = excluded.status, closed_at = excluded.closed_at",
    )
    .bind(&s.id)
    .bind(&s.device_id)
    .bind(&s.operator_id)
    .bind(s.opening_amount)
    .bind(s.closing_amount)
    .bind(s.expected_amount)
    .bind(&s.status)
    .bind(&s.opened_at)
    .bind(&s.closed_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct CashMovementPayload {
    id: String,
    cash_session_id: String,
    #[serde(rename = "type")]
    movement_type: String,
    amount: f64,
    reason: Option<String>,
    created_at: String,
}

async fn apply_cash_movement(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let m: CashMovementPayload =
        serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT OR IGNORE INTO cash_movements
            (id, cash_session_id, type, amount, reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.id)
    .bind(&m.cash_session_id)
    .bind(&m.movement_type)
    .bind(m.amount)
    .bind(&m.reason)
    .bind(&m.created_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct StockMovementPayload {
    id: String,
    product_id: String,
    delta_quantity: f64,
    reason: String,
    reference_id: Option<String>,
    device_id: String,
    created_at: String,
}

/// Applies the delta to `stock_levels` only the first time this movement
/// is seen locally (`rows_affected() == 1` on the `INSERT OR IGNORE`) —
/// re-applying an already-known movement would double-count the delta.
async fn apply_stock_movement(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let m: StockMovementPayload =
        serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let result = sqlx::query(
        "INSERT OR IGNORE INTO stock_movements
            (id, product_id, delta_quantity, reason, reference_id, device_id, created_at,
             synced_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.id)
    .bind(&m.product_id)
    .bind(m.delta_quantity)
    .bind(&m.reason)
    .bind(&m.reference_id)
    .bind(&m.device_id)
    .bind(&m.created_at)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if result.rows_affected() == 1 {
        sqlx::query(
            "INSERT INTO stock_levels (product_id, quantity, min_quantity, updated_at)
             VALUES (?, ?, 0, ?)
             ON CONFLICT(product_id) DO UPDATE SET
                quantity = stock_levels.quantity + excluded.quantity,
                updated_at = excluded.updated_at",
        )
        .bind(&m.product_id)
        .bind(m.delta_quantity)
        .bind(&m.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct CustomerLedgerPayload {
    id: String,
    customer_id: String,
    entry_type: String,
    amount: f64,
    due_date: Option<String>,
    reference_sale_id: Option<String>,
    notes: Option<String>,
    device_id: String,
    created_at: String,
}

/// Append-only — the customer's balance is always `SUM(debit) -
/// SUM(credit)` over this table (plan section 3.2), never a stored value,
/// so there's nothing else to update here.
async fn apply_customer_ledger(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let l: CustomerLedgerPayload =
        serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT OR IGNORE INTO customer_ledger
            (id, customer_id, entry_type, amount, due_date, reference_sale_id, notes,
             device_id, created_at, synced_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&l.id)
    .bind(&l.customer_id)
    .bind(&l.entry_type)
    .bind(l.amount)
    .bind(&l.due_date)
    .bind(&l.reference_sale_id)
    .bind(&l.notes)
    .bind(&l.device_id)
    .bind(&l.created_at)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct SaleItemPayload {
    id: String,
    product_id: String,
    unit_id: String,
    quantity: f64,
    quantity_stock: f64,
    unit_price: f64,
    discount: f64,
    line_total: f64,
}

#[derive(Debug, Deserialize)]
struct SalePaymentPayload {
    id: String,
    method: String,
    amount: f64,
    installments: i32,
}

#[derive(Debug, Deserialize)]
struct SalePayload {
    id: String,
    cash_session_id: String,
    customer_id: Option<String>,
    status: String,
    subtotal: f64,
    discount_total: f64,
    total: f64,
    device_id: String,
    operator_id: String,
    created_at: String,
    items: Vec<SaleItemPayload>,
    payments: Vec<SalePaymentPayload>,
}

/// Sales, their items, and their payments are immutable once created in
/// this MVP (no edit path — only status changes via Phase 7's returns
/// flow, see `apply_return` below), so every insert here is a plain
/// append, `OR IGNORE`d for idempotency against redelivery.
async fn apply_sale(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let s: SalePayload = serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT OR IGNORE INTO sales
            (id, cash_session_id, customer_id, status, subtotal, discount_total, total,
             device_id, operator_id, created_at, synced_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&s.id)
    .bind(&s.cash_session_id)
    .bind(&s.customer_id)
    .bind(&s.status)
    .bind(s.subtotal)
    .bind(s.discount_total)
    .bind(s.total)
    .bind(&s.device_id)
    .bind(&s.operator_id)
    .bind(&s.created_at)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    for item in &s.items {
        sqlx::query(
            "INSERT OR IGNORE INTO sale_items
                (id, sale_id, product_id, unit_id, quantity, quantity_stock, unit_price,
                 discount, line_total)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&item.id)
        .bind(&s.id)
        .bind(&item.product_id)
        .bind(&item.unit_id)
        .bind(item.quantity)
        .bind(item.quantity_stock)
        .bind(item.unit_price)
        .bind(item.discount)
        .bind(item.line_total)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    for payment in &s.payments {
        sqlx::query(
            "INSERT OR IGNORE INTO sale_payments
                (id, sale_id, method, amount, installments, gateway_ref)
             VALUES (?, ?, ?, ?, ?, NULL)",
        )
        .bind(&payment.id)
        .bind(&s.id)
        .bind(&payment.method)
        .bind(payment.amount)
        .bind(payment.installments)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ReturnItemPayload {
    id: String,
    sale_item_id: String,
    quantity: f64,
}

#[derive(Debug, Deserialize)]
struct ReturnPayload {
    id: String,
    original_sale_id: String,
    reason: Option<String>,
    total_refunded: f64,
    refund_method: String,
    operator_id: String,
    device_id: String,
    created_at: String,
    sale_status: String,
    items: Vec<ReturnItemPayload>,
}

/// A return pulled from another device at the same store: append the
/// return + its items (idempotent, `OR IGNORE`d against redelivery) and
/// apply the same `sales.status` transition the originating device already
/// computed — mirrors `apply_sale`'s idempotent-append shape. The
/// accompanying `stock_movement`/`customer_ledger` events for this same
/// return arrive as their own separate pulled events and are handled by
/// `apply_stock_movement`/`apply_customer_ledger` above, unchanged.
async fn apply_return(pool: &Pool<Sqlite>, payload: &JsonValue) -> Result<(), String> {
    let r: ReturnPayload = serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT OR IGNORE INTO returns
            (id, original_sale_id, reason, total_refunded, refund_method, operator_id,
             device_id, created_at, synced_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&r.id)
    .bind(&r.original_sale_id)
    .bind(&r.reason)
    .bind(r.total_refunded)
    .bind(&r.refund_method)
    .bind(&r.operator_id)
    .bind(&r.device_id)
    .bind(&r.created_at)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    for item in &r.items {
        sqlx::query(
            "INSERT OR IGNORE INTO return_items (id, return_id, sale_item_id, quantity)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&item.id)
        .bind(&r.id)
        .bind(&item.sale_item_id)
        .bind(item.quantity)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    sqlx::query("UPDATE sales SET status = ? WHERE id = ?")
        .bind(&r.sale_status)
        .bind(&r.original_sale_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}
