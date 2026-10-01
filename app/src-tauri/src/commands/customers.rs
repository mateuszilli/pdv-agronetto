//! Customer registry and store-credit ("fiado") commands (plan section 4.3).
//!
//! `default_due_days` (see migrations/0006_customer_due_days.sql) closes a
//! gap between section 3.2's schema, which has no column for it, and
//! section 4.3's "due_date (default: 30 days, configurable per customer)".
//!
//! Printing a receipt for a recorded payment (plan section 4.3) is
//! deferred to Phase 6, alongside the rest of the ESC/POS pipeline.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::{AppHandle, Runtime};

use crate::db;
use crate::money::{amount_to_cents, cents_to_amount};
use crate::outbox;

/// Mirrors the `customers` row shape the backend's apply logic upserts
/// into central Postgres (plan section 3.2), keyed by `updated_at` for
/// last-write-wins (plan section 1.3) — see migrations/0007_sync_lww_columns.sql.
#[derive(Debug, Serialize)]
struct CustomerEventPayload<'a> {
    id: &'a str,
    doc_type: &'a str,
    doc_number: &'a str,
    name: &'a str,
    phone: &'a Option<String>,
    address_json: &'a Option<String>,
    credit_limit: f64,
    default_due_days: i64,
    active: bool,
    created_at: &'a str,
    updated_at: &'a str,
}

/// Mirrors `customer_ledger` (plan section 3.2) — append-only, applied as
/// a delta on the central `stock_levels`-style projection (plan section
/// 1.3: balances sync as deltas, never absolute values).
#[derive(Debug, Serialize)]
struct CustomerLedgerEventPayload<'a> {
    id: &'a str,
    customer_id: &'a str,
    entry_type: &'a str,
    amount: f64,
    due_date: Option<&'a str>,
    reference_sale_id: Option<&'a str>,
    notes: &'a Option<String>,
    device_id: &'a str,
    created_at: &'a str,
}

const DOC_TYPES: &[&str] = &["CPF", "CNPJ"];

/// Every customer-balance query needs this same "debits minus credits"
/// computation (plan section 3.2) — kept as one constant so the list,
/// detail, and receivables queries can't drift from each other.
const BALANCE_SUBQUERY: &str = "COALESCE((
    SELECT SUM(CASE WHEN entry_type = 'debit' THEN amount ELSE -amount END)
    FROM customer_ledger WHERE customer_id = c.id
), 0)";

#[derive(Debug, Serialize)]
pub struct CustomerSummary {
    pub id: String,
    pub doc_type: String,
    pub doc_number: String,
    pub name: String,
    pub phone: Option<String>,
    pub credit_limit_cents: i64,
    pub balance_owed_cents: i64,
    pub active: bool,
}

#[derive(Debug, Serialize)]
pub struct CustomerDetail {
    pub id: String,
    pub doc_type: String,
    pub doc_number: String,
    pub name: String,
    pub phone: Option<String>,
    pub address_json: Option<String>,
    pub credit_limit_cents: i64,
    pub default_due_days: i64,
    pub balance_owed_cents: i64,
    pub active: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CustomerInput {
    pub doc_type: String,
    pub doc_number: String,
    pub name: String,
    pub phone: Option<String>,
    pub address_json: Option<String>,
    pub credit_limit_cents: i64,
    pub default_due_days: i64,
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

/// Strips everything but digits and checks the expected length for the
/// declared `doc_type` (11 for CPF, 14 for CNPJ) — enough to catch typos
/// without implementing the full mod-11 check-digit algorithm; the fiscal
/// provider validates it for real once notes are actually emitted (Phase 6).
fn validate_doc(doc_type: &str, doc_number: &str) -> Result<String, String> {
    if !DOC_TYPES.contains(&doc_type) {
        return Err("doc_type must be 'CPF' or 'CNPJ'".into());
    }
    let digits: String = doc_number.chars().filter(|c| c.is_ascii_digit()).collect();
    let expected_len = if doc_type == "CPF" { 11 } else { 14 };
    if digits.len() != expected_len {
        return Err(format!("{doc_type} must have {expected_len} digits"));
    }
    Ok(digits)
}

fn row_to_summary(row: &sqlx::sqlite::SqliteRow) -> Result<CustomerSummary, sqlx::Error> {
    let credit_limit: f64 = row.try_get("credit_limit")?;
    let balance_owed: f64 = row.try_get("balance_owed")?;
    Ok(CustomerSummary {
        id: row.try_get("id")?,
        doc_type: row.try_get("doc_type")?,
        doc_number: row.try_get("doc_number")?,
        name: row.try_get("name")?,
        phone: row.try_get("phone")?,
        credit_limit_cents: amount_to_cents(credit_limit),
        balance_owed_cents: amount_to_cents(balance_owed),
        active: row.try_get::<i64, _>("active")? != 0,
    })
}

/// Searches customers by name or doc number. Used both by the customer
/// registry screen and by the sale screen's F3 "identify customer" search
/// (plan section 4.1) — `query: None` returns the first 50 alphabetically.
#[tauri::command]
pub async fn list_customers<R: Runtime>(
    app: AppHandle<R>,
    query: Option<String>,
) -> Result<Vec<CustomerSummary>, String> {
    let pool = db::pool(&app).await?;
    let query = non_empty(query);
    let like = query.as_ref().map(|q| format!("%{q}%")).unwrap_or_default();
    let has_query = query.is_some();

    let sql = format!(
        "SELECT c.id, c.doc_type, c.doc_number, c.name, c.phone, c.credit_limit, c.active,
                {BALANCE_SUBQUERY} as balance_owed
         FROM customers c
         WHERE (? = 0 OR c.name LIKE ? OR c.doc_number LIKE ?)
         ORDER BY c.name
         LIMIT 50"
    );

    let rows = sqlx::query(&sql)
        .bind(!has_query)
        .bind(&like)
        .bind(&like)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;

    rows.iter()
        .map(row_to_summary)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Customers with a positive outstanding balance (plan section 4.3's
/// accounts-receivable screen), highest balance first. `balance_owed` is a
/// derived-table column here (not an outer-query alias), since SQLite's
/// WHERE can't reference a sibling SELECT alias directly.
#[tauri::command]
pub async fn list_receivables<R: Runtime>(app: AppHandle<R>) -> Result<Vec<CustomerSummary>, String> {
    let pool = db::pool(&app).await?;
    let sql = format!(
        "SELECT * FROM (
            SELECT c.id, c.doc_type, c.doc_number, c.name, c.phone, c.credit_limit, c.active,
                   {BALANCE_SUBQUERY} as balance_owed
            FROM customers c
         ) WHERE balance_owed > 0
         ORDER BY balance_owed DESC"
    );

    let rows = sqlx::query(&sql).fetch_all(&pool).await.map_err(|e| e.to_string())?;

    rows.iter()
        .map(row_to_summary)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_customer<R: Runtime>(app: AppHandle<R>, id: String) -> Result<CustomerDetail, String> {
    let pool = db::pool(&app).await?;
    let sql = format!(
        "SELECT c.id, c.doc_type, c.doc_number, c.name, c.phone, c.address_json,
                c.credit_limit, c.default_due_days, c.active, c.created_at,
                {BALANCE_SUBQUERY} as balance_owed
         FROM customers c WHERE c.id = ?"
    );

    let row = sqlx::query(&sql)
        .bind(&id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("customer {id} not found"))?;

    let credit_limit: f64 = row.try_get("credit_limit").map_err(|e| e.to_string())?;
    let balance_owed: f64 = row.try_get("balance_owed").map_err(|e| e.to_string())?;

    Ok(CustomerDetail {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        doc_type: row.try_get("doc_type").map_err(|e| e.to_string())?,
        doc_number: row.try_get("doc_number").map_err(|e| e.to_string())?,
        name: row.try_get("name").map_err(|e| e.to_string())?,
        phone: row.try_get("phone").map_err(|e| e.to_string())?,
        address_json: row.try_get("address_json").map_err(|e| e.to_string())?,
        credit_limit_cents: amount_to_cents(credit_limit),
        default_due_days: row.try_get("default_due_days").map_err(|e| e.to_string())?,
        balance_owed_cents: amount_to_cents(balance_owed),
        active: row.try_get::<i64, _>("active").map_err(|e| e.to_string())? != 0,
        created_at: row.try_get("created_at").map_err(|e| e.to_string())?,
    })
}

#[tauri::command]
pub async fn create_customer<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
    input: CustomerInput,
) -> Result<CustomerDetail, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let doc_number = validate_doc(&input.doc_type, &input.doc_number)?;
    if input.credit_limit_cents < 0 {
        return Err("credit limit cannot be negative".into());
    }
    if input.default_due_days < 0 {
        return Err("due days cannot be negative".into());
    }

    let pool = db::pool(&app).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let phone = non_empty(input.phone);
    let address_json = non_empty(input.address_json);
    let credit_limit = cents_to_amount(input.credit_limit_cents);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO customers
            (id, doc_type, doc_number, name, phone, address_json, credit_limit,
             default_due_days, active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.doc_type)
    .bind(&doc_number)
    .bind(&name)
    .bind(&phone)
    .bind(&address_json)
    .bind(credit_limit)
    .bind(input.default_due_days)
    .bind(input.active)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("failed to create customer (doc number already registered?): {e}"))?;

    let payload = CustomerEventPayload {
        id: &id,
        doc_type: &input.doc_type,
        doc_number: &doc_number,
        name: &name,
        phone: &phone,
        address_json: &address_json,
        credit_limit,
        default_due_days: input.default_due_days,
        active: input.active,
        created_at: &now,
        updated_at: &now,
    };
    outbox::record(&mut tx, "customer", &id, &device_id, &payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    get_customer(app, id).await
}

#[tauri::command]
pub async fn update_customer<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
    id: String,
    input: CustomerInput,
) -> Result<CustomerDetail, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let doc_number = validate_doc(&input.doc_type, &input.doc_number)?;
    if input.credit_limit_cents < 0 {
        return Err("credit limit cannot be negative".into());
    }
    if input.default_due_days < 0 {
        return Err("due days cannot be negative".into());
    }

    let pool = db::pool(&app).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let phone = non_empty(input.phone);
    let address_json = non_empty(input.address_json);
    let credit_limit = cents_to_amount(input.credit_limit_cents);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let result = sqlx::query(
        "UPDATE customers
         SET doc_type = ?, doc_number = ?, name = ?, phone = ?, address_json = ?,
             credit_limit = ?, default_due_days = ?, active = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&input.doc_type)
    .bind(&doc_number)
    .bind(&name)
    .bind(&phone)
    .bind(&address_json)
    .bind(credit_limit)
    .bind(input.default_due_days)
    .bind(input.active)
    .bind(&now)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("failed to update customer (doc number already registered?): {e}"))?;

    if result.rows_affected() == 0 {
        return Err(format!("customer {id} not found"));
    }

    let (created_at,): (String,) = sqlx::query_as("SELECT created_at FROM customers WHERE id = ?")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    let payload = CustomerEventPayload {
        id: &id,
        doc_type: &input.doc_type,
        doc_number: &doc_number,
        name: &name,
        phone: &phone,
        address_json: &address_json,
        credit_limit,
        default_due_days: input.default_due_days,
        active: input.active,
        created_at: &created_at,
        updated_at: &now,
    };
    outbox::record(&mut tx, "customer", &id, &device_id, &payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    get_customer(app, id).await
}

#[derive(Debug, Serialize)]
pub struct LedgerEntry {
    pub id: String,
    pub entry_type: String,
    pub amount_cents: i64,
    pub due_date: Option<String>,
    pub reference_sale_id: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}

fn row_to_ledger_entry(row: &sqlx::sqlite::SqliteRow) -> Result<LedgerEntry, sqlx::Error> {
    let amount: f64 = row.try_get("amount")?;
    Ok(LedgerEntry {
        id: row.try_get("id")?,
        entry_type: row.try_get("entry_type")?,
        amount_cents: amount_to_cents(amount),
        due_date: row.try_get("due_date")?,
        reference_sale_id: row.try_get("reference_sale_id")?,
        notes: row.try_get("notes")?,
        created_at: row.try_get("created_at")?,
    })
}

/// A customer's ledger, most recent first — the detail view behind the
/// accounts-receivable list (plan section 4.3).
#[tauri::command]
pub async fn list_ledger_entries<R: Runtime>(
    app: AppHandle<R>,
    customer_id: String,
) -> Result<Vec<LedgerEntry>, String> {
    let pool = db::pool(&app).await?;
    let rows = sqlx::query(
        "SELECT id, entry_type, amount, due_date, reference_sale_id, notes, created_at
         FROM customer_ledger WHERE customer_id = ? ORDER BY created_at DESC",
    )
    .bind(&customer_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter()
        .map(row_to_ledger_entry)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Records a partial (or full) payment against a customer's outstanding
/// balance — a `credit` ledger entry (plan section 4.3's accounts-
/// receivable screen). Printing the receipt is deferred to Phase 6.
#[tauri::command]
pub async fn record_payment<R: Runtime>(
    app: AppHandle<R>,
    customer_id: String,
    device_id: String,
    amount_cents: i64,
    notes: Option<String>,
) -> Result<LedgerEntry, String> {
    if amount_cents <= 0 {
        return Err("amount must be greater than zero".into());
    }

    let pool = db::pool(&app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM customers WHERE id = ?")
        .bind(&customer_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err(format!("customer {customer_id} not found"));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let notes = non_empty(notes);
    let amount = cents_to_amount(amount_cents);

    sqlx::query(
        "INSERT INTO customer_ledger
            (id, customer_id, entry_type, amount, due_date, reference_sale_id, notes,
             device_id, created_at, synced_at)
         VALUES (?, ?, 'credit', ?, NULL, NULL, ?, ?, ?, NULL)",
    )
    .bind(&id)
    .bind(&customer_id)
    .bind(amount)
    .bind(&notes)
    .bind(&device_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let payload = CustomerLedgerEventPayload {
        id: &id,
        customer_id: &customer_id,
        entry_type: "credit",
        amount,
        due_date: None,
        reference_sale_id: None,
        notes: &notes,
        device_id: &device_id,
        created_at: &now,
    };
    outbox::record(&mut tx, "customer_ledger", &id, &device_id, &payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(LedgerEntry {
        id,
        entry_type: "credit".to_string(),
        amount_cents,
        due_date: None,
        reference_sale_id: None,
        notes,
        created_at: now,
    })
}
