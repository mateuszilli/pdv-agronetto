//! Cash register commands: open/close, cash-in/cash-out (plan section 4.4).
//!
//! "Sales cannot start without an open cash_sessions on the device" — the
//! sale screen checks `get_open_cash_session` before letting a sale start.
//! Closing is never blocked by a discrepancy; the difference between the
//! expected and counted amount is just reported back to the operator.

use serde::Serialize;
use sqlx::{sqlite::SqliteRow, Row};
use tauri::{AppHandle, Runtime};

use crate::db;
use crate::money::{amount_to_cents, cents_to_amount};
use crate::outbox;

/// Mirrors `cash_sessions` (plan section 3.3). A session only ever has one
/// owning device, so applying it centrally is a plain upsert-by-id — no
/// cross-device race to resolve (unlike `product`/`customer`).
#[derive(Debug, Serialize)]
struct CashSessionEventPayload<'a> {
    id: &'a str,
    device_id: &'a str,
    operator_id: &'a str,
    opening_amount: f64,
    closing_amount: Option<f64>,
    expected_amount: Option<f64>,
    status: &'a str,
    opened_at: &'a str,
    closed_at: Option<&'a str>,
}

/// Mirrors `cash_movements` (plan section 3.3) — immutable/append-only.
#[derive(Debug, Serialize)]
struct CashMovementEventPayload<'a> {
    id: &'a str,
    cash_session_id: &'a str,
    #[serde(rename = "type")]
    movement_type: &'a str,
    amount: f64,
    reason: Option<&'a str>,
    created_at: &'a str,
}

#[derive(Debug, Serialize)]
pub struct CashSession {
    pub id: String,
    pub device_id: String,
    pub operator_id: String,
    pub opening_amount_cents: i64,
    pub closing_amount_cents: Option<i64>,
    pub expected_amount_cents: Option<i64>,
    pub status: String,
    pub opened_at: String,
    pub closed_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CashMovement {
    pub id: String,
    pub cash_session_id: String,
    pub movement_type: String,
    pub amount_cents: i64,
    pub reason: Option<String>,
    pub created_at: String,
}

fn row_to_session(row: &SqliteRow) -> Result<CashSession, sqlx::Error> {
    let opening_amount: f64 = row.try_get("opening_amount")?;
    let closing_amount: Option<f64> = row.try_get("closing_amount")?;
    let expected_amount: Option<f64> = row.try_get("expected_amount")?;
    Ok(CashSession {
        id: row.try_get("id")?,
        device_id: row.try_get("device_id")?,
        operator_id: row.try_get("operator_id")?,
        opening_amount_cents: amount_to_cents(opening_amount),
        closing_amount_cents: closing_amount.map(amount_to_cents),
        expected_amount_cents: expected_amount.map(amount_to_cents),
        status: row.try_get("status")?,
        opened_at: row.try_get("opened_at")?,
        closed_at: row.try_get("closed_at")?,
    })
}

const SESSION_SELECT: &str = "SELECT id, device_id, operator_id, opening_amount,
    closing_amount, expected_amount, status, opened_at, closed_at FROM cash_sessions";

/// The device's currently open session, if any. `None` means the sale
/// screen must send the operator to open one before anything else works.
#[tauri::command]
pub async fn get_open_cash_session<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
) -> Result<Option<CashSession>, String> {
    let pool = db::pool(&app).await?;
    let row = sqlx::query(&format!(
        "{SESSION_SELECT} WHERE device_id = ? AND status = 'open' ORDER BY opened_at DESC LIMIT 1"
    ))
    .bind(&device_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;

    row.as_ref()
        .map(row_to_session)
        .transpose()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_cash_session<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
    operator_id: String,
    opening_amount_cents: i64,
) -> Result<CashSession, String> {
    if opening_amount_cents < 0 {
        return Err("opening amount cannot be negative".into());
    }
    let pool = db::pool(&app).await?;

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM cash_sessions WHERE device_id = ? AND status = 'open'")
            .bind(&device_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    if existing.is_some() {
        return Err("this device already has an open cash register".into());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let opening_amount = cents_to_amount(opening_amount_cents);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO cash_sessions
            (id, device_id, operator_id, opening_amount, closing_amount, expected_amount,
             status, opened_at, closed_at)
         VALUES (?, ?, ?, ?, NULL, NULL, 'open', ?, NULL)",
    )
    .bind(&id)
    .bind(&device_id)
    .bind(&operator_id)
    .bind(opening_amount)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let payload = CashSessionEventPayload {
        id: &id,
        device_id: &device_id,
        operator_id: &operator_id,
        opening_amount,
        closing_amount: None,
        expected_amount: None,
        status: "open",
        opened_at: &now,
        closed_at: None,
    };
    outbox::record(&mut tx, "cash_session", &id, &device_id, &payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    let row = sqlx::query(&format!("{SESSION_SELECT} WHERE id = ?"))
        .bind(&id)
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?;
    row_to_session(&row).map_err(|e| e.to_string())
}

/// Records a "sangria" (cash_out) or "suprimento" (cash_in). Always
/// requires a reason (plan section 4.4), even though the column itself
/// is nullable for schema-portability reasons.
#[tauri::command]
pub async fn record_cash_movement<R: Runtime>(
    app: AppHandle<R>,
    cash_session_id: String,
    movement_type: String,
    amount_cents: i64,
    reason: String,
) -> Result<CashMovement, String> {
    if movement_type != "cash_in" && movement_type != "cash_out" {
        return Err("movement_type must be 'cash_in' or 'cash_out'".into());
    }
    if amount_cents <= 0 {
        return Err("amount must be greater than zero".into());
    }
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err("reason is required".into());
    }

    let pool = db::pool(&app).await?;

    let session_row: Option<(String, String)> =
        sqlx::query_as("SELECT status, device_id FROM cash_sessions WHERE id = ?")
            .bind(&cash_session_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    let device_id = match session_row {
        Some((status, device_id)) if status == "open" => device_id,
        Some(_) => return Err("cash register is closed".into()),
        None => return Err("cash session not found".into()),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let amount = cents_to_amount(amount_cents);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO cash_movements (id, cash_session_id, type, amount, reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&cash_session_id)
    .bind(&movement_type)
    .bind(amount)
    .bind(&reason)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let payload = CashMovementEventPayload {
        id: &id,
        cash_session_id: &cash_session_id,
        movement_type: &movement_type,
        amount,
        reason: Some(&reason),
        created_at: &now,
    };
    outbox::record(&mut tx, "cash_movement", &id, &device_id, &payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(CashMovement {
        id,
        cash_session_id,
        movement_type,
        amount_cents,
        reason: Some(reason),
        created_at: now,
    })
}

#[tauri::command]
pub async fn list_cash_movements<R: Runtime>(
    app: AppHandle<R>,
    cash_session_id: String,
) -> Result<Vec<CashMovement>, String> {
    let pool = db::pool(&app).await?;
    let rows = sqlx::query(
        "SELECT id, cash_session_id, type, amount, reason, created_at
         FROM cash_movements WHERE cash_session_id = ? ORDER BY created_at",
    )
    .bind(&cash_session_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter()
        .map(|row| {
            let amount: f64 = row.try_get("amount")?;
            Ok(CashMovement {
                id: row.try_get("id")?,
                cash_session_id: row.try_get("cash_session_id")?,
                movement_type: row.try_get("type")?,
                amount_cents: amount_to_cents(amount),
                reason: row.try_get("reason")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|e| e.to_string())
}

/// Closes the session: computes `expected_amount` (opening + cash sales +
/// cash_in - cash_out) and records the operator's physically counted
/// amount. The discrepancy (if any) is only ever reported, never blocking.
#[tauri::command]
pub async fn close_cash_session<R: Runtime>(
    app: AppHandle<R>,
    cash_session_id: String,
    counted_amount_cents: i64,
) -> Result<CashSession, String> {
    let pool = db::pool(&app).await?;

    let session_row = sqlx::query(&format!("{SESSION_SELECT} WHERE id = ?"))
        .bind(&cash_session_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "cash session not found".to_string())?;
    let session = row_to_session(&session_row).map_err(|e| e.to_string())?;
    if session.status != "open" {
        return Err("cash register is already closed".into());
    }

    let cash_sales: (f64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(sp.amount), 0) FROM sale_payments sp
         JOIN sales s ON s.id = sp.sale_id
         WHERE s.cash_session_id = ? AND sp.method = 'cash' AND s.status != 'cancelled'",
    )
    .bind(&cash_session_id)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let cash_in: (f64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount), 0) FROM cash_movements
         WHERE cash_session_id = ? AND type = 'cash_in'",
    )
    .bind(&cash_session_id)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let cash_out: (f64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount), 0) FROM cash_movements
         WHERE cash_session_id = ? AND type = 'cash_out'",
    )
    .bind(&cash_session_id)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let expected_cents =
        session.opening_amount_cents + amount_to_cents(cash_sales.0) + amount_to_cents(cash_in.0)
            - amount_to_cents(cash_out.0);
    let now = chrono::Utc::now().to_rfc3339();
    let closing_amount = cents_to_amount(counted_amount_cents);
    let expected_amount = cents_to_amount(expected_cents);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE cash_sessions
         SET closing_amount = ?, expected_amount = ?, status = 'closed', closed_at = ?
         WHERE id = ?",
    )
    .bind(closing_amount)
    .bind(expected_amount)
    .bind(&now)
    .bind(&cash_session_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let payload = CashSessionEventPayload {
        id: &cash_session_id,
        device_id: &session.device_id,
        operator_id: &session.operator_id,
        opening_amount: cents_to_amount(session.opening_amount_cents),
        closing_amount: Some(closing_amount),
        expected_amount: Some(expected_amount),
        status: "closed",
        opened_at: &session.opened_at,
        closed_at: Some(&now),
    };
    outbox::record(&mut tx, "cash_session", &cash_session_id, &session.device_id, &payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    let row = sqlx::query(&format!("{SESSION_SELECT} WHERE id = ?"))
        .bind(&cash_session_id)
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?;
    row_to_session(&row).map_err(|e| e.to_string())
}
