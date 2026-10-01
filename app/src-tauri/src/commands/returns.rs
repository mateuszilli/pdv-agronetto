//! Returns screen commands (plan section 4.5) — search for an original
//! sale, load its returnable items, and record a return.
//!
//! Item-by-item, partial returns are supported: a sale can be returned
//! across several separate `create_return` calls over time, each one
//! covering only some lines (or part of a line's quantity), as long as the
//! running total returned per `sale_items` row never exceeds what was
//! originally sold (see `quantity_returned`/`quantity_returnable` below).
//!
//! Fiscal note (plan section 4.5's "generate a new fiscal document ...
//! referencing the original note's access key"): Focus NFe's real NFC-e
//! contract has no such linked-reversal capability the way a formal NF-e
//! devolução would — a genuine mismatch with the plan's wording, confirmed
//! against <https://doc.focusnfe.com.br/reference/cancelar_nfce>. The only
//! fiscal remedy it offers at all is cancelling the *entire* original
//! document, and only within 30 minutes of authorization. `create_return`
//! therefore attempts that whole-document cancellation, but only when a
//! return brings the sale to fully `returned` — never for a partial
//! return, which has no equivalent in Focus's contract and stays
//! bookkeeping-only, exactly as before. See
//! `fiscal_client::try_cancel_for_return`'s doc comment for the full
//! reasoning (window handling, retry, and why a non-`Cancelled` outcome
//! never blocks the return itself: stock, ledger, and `sales.status`
//! always update regardless of what the fiscal call does).

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

use crate::commands::sales::line_total_cents;
use crate::db;
use crate::fiscal_client;
use crate::money::{amount_to_cents, cents_to_amount};
use crate::outbox;

/// Quantities are stored as `f64` (see money.rs's note on SQLite's REAL
/// affinity) — this tolerance absorbs float round-trip noise when
/// comparing a requested return quantity against what's still returnable.
const QUANTITY_EPSILON: f64 = 1e-6;

const REFUND_METHODS: &[&str] = &["cash", "debit", "credit", "pix", "store_credit"];
// "boleto" deliberately excluded: voiding an unpaid installment bill isn't
// a refund action this schema models (there's no money to give back yet).

/// Mirrors `return_items` (plan section 3.3), nested inside a `return`
/// outbox event rather than sent separately — same one-atomic-write
/// convention `sales.rs`'s `SaleItemEventPayload` uses.
#[derive(Debug, Serialize)]
struct ReturnItemEventPayload {
    id: String,
    sale_item_id: String,
    quantity: f64,
}

/// Mirrors `returns` (plan section 3.3) plus its nested items. Carries the
/// client-computed `sale_status` so the backend doesn't need to recompute
/// it — the device already has every `sale_items`/`return_items` row
/// loaded to build this return in the first place, the same trust model
/// `finalize_sale` uses for its own `status` field.
#[derive(Debug, Serialize)]
struct ReturnEventPayload {
    id: String,
    original_sale_id: String,
    reason: Option<String>,
    total_refunded: f64,
    refund_method: String,
    operator_id: String,
    device_id: String,
    created_at: String,
    sale_status: String,
    items: Vec<ReturnItemEventPayload>,
}

/// Mirrors `stock_movements` (plan section 3.1) — same shape as
/// `sales.rs`'s `StockMovementEventPayload`, duplicated per this
/// codebase's convention of not sharing small payload structs across
/// command modules.
#[derive(Debug, Serialize)]
struct StockMovementEventPayload {
    id: String,
    product_id: String,
    delta_quantity: f64,
    reason: String,
    reference_id: Option<String>,
    device_id: String,
    created_at: String,
}

/// Mirrors `customer_ledger` (plan section 3.2) — same shape as
/// `sales.rs`'s `CustomerLedgerEventPayload`.
#[derive(Debug, Serialize)]
struct CustomerLedgerEventPayload {
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

#[derive(Debug, Deserialize)]
pub struct SaleSearchInput {
    #[serde(default)]
    pub sale_id_query: Option<String>,
    #[serde(default)]
    pub access_key: Option<String>,
    #[serde(default)]
    pub customer_doc_number: Option<String>,
    #[serde(default)]
    pub date_from: Option<String>,
    #[serde(default)]
    pub date_to: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SaleSearchResult {
    pub id: String,
    pub status: String,
    pub total_cents: i64,
    pub created_at: String,
    pub customer_name: Option<String>,
    pub customer_doc_number: Option<String>,
    pub access_key: Option<String>,
}

fn non_empty(s: &Option<String>) -> bool {
    s.as_ref().is_some_and(|v| !v.trim().is_empty())
}

/// Searches for a sale by exactly one of: (a partial match on) sale id,
/// NFC-e access key, or customer document number (optionally narrowed by a
/// date range) — the three lookup modes plan section 4.5 lists for
/// locating the original sale.
#[tauri::command]
pub async fn search_sales_for_return<R: Runtime>(
    app: AppHandle<R>,
    input: SaleSearchInput,
) -> Result<Vec<SaleSearchResult>, String> {
    let modes_used = [
        non_empty(&input.sale_id_query),
        non_empty(&input.access_key),
        non_empty(&input.customer_doc_number),
    ]
    .iter()
    .filter(|used| **used)
    .count();
    if modes_used != 1 {
        return Err("search by exactly one of: sale id, access key, or customer document number"
            .into());
    }

    let pool = db::pool(&app).await?;

    const BASE_SELECT: &str = "
        SELECT s.id, s.status, s.total, s.created_at,
               c.name AS customer_name, c.doc_number AS customer_doc_number,
               (SELECT access_key FROM fiscal_documents fd
                WHERE fd.sale_id = s.id ORDER BY fd.created_at DESC LIMIT 1) AS access_key
        FROM sales s
        LEFT JOIN customers c ON c.id = s.customer_id
    ";

    let rows: Vec<(String, String, f64, String, Option<String>, Option<String>, Option<String>)> =
        if non_empty(&input.sale_id_query) {
            let query = format!("{BASE_SELECT} WHERE s.id LIKE ? ORDER BY s.created_at DESC LIMIT 50");
            let pattern = format!("%{}%", input.sale_id_query.unwrap().trim());
            sqlx::query_as(&query)
                .bind(pattern)
                .fetch_all(&pool)
                .await
                .map_err(|e| e.to_string())?
        } else if non_empty(&input.access_key) {
            let query = format!(
                "{BASE_SELECT}
                 WHERE (SELECT access_key FROM fiscal_documents fd
                        WHERE fd.sale_id = s.id ORDER BY fd.created_at DESC LIMIT 1) = ?
                 ORDER BY s.created_at DESC LIMIT 50"
            );
            sqlx::query_as(&query)
                .bind(input.access_key.unwrap().trim().to_string())
                .fetch_all(&pool)
                .await
                .map_err(|e| e.to_string())?
        } else {
            let now = chrono::Utc::now();
            let date_from = input
                .date_from
                .unwrap_or_else(|| (now - chrono::Duration::days(90)).to_rfc3339());
            let date_to = input.date_to.unwrap_or_else(|| now.to_rfc3339());
            let query = format!(
                "{BASE_SELECT}
                 WHERE c.doc_number = ? AND s.created_at >= ? AND s.created_at <= ?
                 ORDER BY s.created_at DESC LIMIT 50"
            );
            sqlx::query_as(&query)
                .bind(input.customer_doc_number.unwrap().trim().to_string())
                .bind(date_from)
                .bind(date_to)
                .fetch_all(&pool)
                .await
                .map_err(|e| e.to_string())?
        };

    Ok(rows
        .into_iter()
        .map(
            |(id, status, total, created_at, customer_name, customer_doc_number, access_key)| {
                SaleSearchResult {
                    id,
                    status,
                    total_cents: amount_to_cents(total),
                    created_at,
                    customer_name,
                    customer_doc_number,
                    access_key,
                }
            },
        )
        .collect())
}

#[derive(Debug, Serialize)]
pub struct ReturnableSaleItem {
    pub sale_item_id: String,
    pub product_id: String,
    pub product_name: String,
    pub unit_id: String,
    pub unit_label: String,
    pub quantity: f64,
    pub quantity_returned: f64,
    pub quantity_returnable: f64,
    pub unit_price_cents: i64,
    pub line_total_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct SaleForReturn {
    pub id: String,
    pub status: String,
    pub customer_id: Option<String>,
    pub customer_name: Option<String>,
    pub total_cents: i64,
    pub created_at: String,
    pub items: Vec<ReturnableSaleItem>,
}

/// Loads one sale's header plus every line item annotated with how much of
/// it has already been returned, so the item-selection screen can show
/// sold/returned/returnable side by side and cap the operator's input.
#[tauri::command]
pub async fn get_sale_for_return<R: Runtime>(
    app: AppHandle<R>,
    sale_id: String,
) -> Result<SaleForReturn, String> {
    let pool = db::pool(&app).await?;

    let header: Option<(String, Option<String>, Option<String>, f64, String)> = sqlx::query_as(
        "SELECT s.status, s.customer_id, c.name, s.total, s.created_at
         FROM sales s
         LEFT JOIN customers c ON c.id = s.customer_id
         WHERE s.id = ?",
    )
    .bind(&sale_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let (status, customer_id, customer_name, total, created_at) =
        header.ok_or_else(|| format!("sale {sale_id} not found"))?;

    let item_rows: Vec<(String, String, String, String, String, f64, f64, f64, f64)> =
        sqlx::query_as(
            "SELECT si.id, si.product_id, p.name, si.unit_id, pu.unit_label,
                    si.quantity, si.unit_price, si.line_total,
                    COALESCE((SELECT SUM(ri.quantity) FROM return_items ri
                              WHERE ri.sale_item_id = si.id), 0) AS quantity_returned
             FROM sale_items si
             JOIN products p ON p.id = si.product_id
             JOIN product_units pu ON pu.id = si.unit_id
             WHERE si.sale_id = ?",
        )
        .bind(&sale_id)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;

    let items = item_rows
        .into_iter()
        .map(
            |(
                sale_item_id,
                product_id,
                product_name,
                unit_id,
                unit_label,
                quantity,
                unit_price,
                line_total,
                quantity_returned,
            )| {
                ReturnableSaleItem {
                    sale_item_id,
                    product_id,
                    product_name,
                    unit_id,
                    unit_label,
                    quantity,
                    quantity_returned,
                    quantity_returnable: (quantity - quantity_returned).max(0.0),
                    unit_price_cents: amount_to_cents(unit_price),
                    line_total_cents: amount_to_cents(line_total),
                }
            },
        )
        .collect();

    Ok(SaleForReturn {
        id: sale_id,
        status,
        customer_id,
        customer_name,
        total_cents: amount_to_cents(total),
        created_at,
        items,
    })
}

#[derive(Debug, Deserialize)]
pub struct ReturnItemInput {
    pub sale_item_id: String,
    pub quantity: f64,
}

#[derive(Debug, Deserialize)]
pub struct CreateReturnInput {
    pub original_sale_id: String,
    pub device_id: String,
    pub operator_id: String,
    #[serde(default)]
    pub reason: Option<String>,
    pub refund_method: String,
    pub items: Vec<ReturnItemInput>,
}

#[derive(Debug, Serialize)]
pub struct ReturnResult {
    pub id: String,
    pub original_sale_id: String,
    pub total_refunded_cents: i64,
    pub refund_method: String,
    pub sale_status: String,
    pub created_at: String,
    /// `None` unless this return brought the sale to fully `returned` —
    /// only then is a whole-document NFC-e cancellation even attempted
    /// (see the module doc comment and `fiscal_client::try_cancel_for_return`).
    pub fiscal_cancellation: Option<fiscal_client::CancellationStatus>,
}

/// Records a return: one `returns` row, one `return_items` row per line, a
/// positive (inbound) `stock_movements` entry per line, an optional
/// `customer_ledger` credit entry (store-credit refund), and the sale's
/// resulting status — all in one transaction, mirroring
/// `sales::finalize_sale`'s shape (validate first, then one commit).
#[tauri::command]
pub async fn create_return<R: Runtime>(
    app: AppHandle<R>,
    input: CreateReturnInput,
) -> Result<ReturnResult, String> {
    if input.items.is_empty() {
        return Err("a return needs at least one item".into());
    }
    if input.items.iter().any(|i| i.quantity <= 0.0) {
        return Err("returned quantities must be greater than zero".into());
    }
    if !REFUND_METHODS.contains(&input.refund_method.as_str()) {
        return Err(format!("unsupported refund method '{}'", input.refund_method));
    }

    let pool = db::pool(&app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let sale_row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT status, customer_id FROM sales WHERE id = ?")
            .bind(&input.original_sale_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let (sale_status, customer_id) =
        sale_row.ok_or_else(|| format!("sale {} not found", input.original_sale_id))?;
    if sale_status == "returned" {
        return Err("this sale has already been fully returned".into());
    }

    if input.refund_method == "store_credit" && customer_id.is_none() {
        return Err(
            "store credit refund requires the original sale to have an identified customer".into(),
        );
    }

    let return_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let mut item_payloads = Vec::with_capacity(input.items.len());
    let mut total_refunded_cents: i64 = 0;

    for item in &input.items {
        let sale_item_row: Option<(String, String, f64, f64, f64)> = sqlx::query_as(
            "SELECT sale_id, product_id, quantity, quantity_stock, unit_price
             FROM sale_items WHERE id = ?",
        )
        .bind(&item.sale_item_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let (item_sale_id, product_id, original_quantity, quantity_stock, unit_price) =
            sale_item_row.ok_or_else(|| format!("sale item {} not found", item.sale_item_id))?;
        if item_sale_id != input.original_sale_id {
            return Err(format!(
                "sale item {} does not belong to sale {}",
                item.sale_item_id, input.original_sale_id
            ));
        }

        let already_returned: f64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(quantity), 0) FROM return_items WHERE sale_item_id = ?",
        )
        .bind(&item.sale_item_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let returnable = original_quantity - already_returned;
        if item.quantity > returnable + QUANTITY_EPSILON {
            return Err(format!(
                "only {returnable} remaining returnable for item {}",
                item.sale_item_id
            ));
        }

        let unit_price_cents = amount_to_cents(unit_price);
        let refund_cents = line_total_cents(unit_price_cents, item.quantity);
        total_refunded_cents += refund_cents;

        let delta_stock = quantity_stock * (item.quantity / original_quantity);

        let return_item_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO return_items (id, return_id, sale_item_id, quantity)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&return_item_id)
        .bind(&return_id)
        .bind(&item.sale_item_id)
        .bind(item.quantity)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        item_payloads.push(ReturnItemEventPayload {
            id: return_item_id,
            sale_item_id: item.sale_item_id.clone(),
            quantity: item.quantity,
        });

        let movement_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO stock_movements
                (id, product_id, delta_quantity, reason, reference_id, device_id, created_at,
                 synced_at)
             VALUES (?, ?, ?, 'return', ?, ?, ?, NULL)",
        )
        .bind(&movement_id)
        .bind(&product_id)
        .bind(delta_stock)
        .bind(&return_id)
        .bind(&input.device_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE stock_levels SET quantity = quantity + ?, updated_at = ? WHERE product_id = ?",
        )
        .bind(delta_stock)
        .bind(&now)
        .bind(&product_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let movement_payload = StockMovementEventPayload {
            id: movement_id.clone(),
            product_id: product_id.clone(),
            delta_quantity: delta_stock,
            reason: "return".to_string(),
            reference_id: Some(return_id.clone()),
            device_id: input.device_id.clone(),
            created_at: now.clone(),
        };
        outbox::record(&mut tx, "stock_movement", &movement_id, &input.device_id, &movement_payload)
            .await?;
    }

    let total_refunded = cents_to_amount(total_refunded_cents);

    sqlx::query(
        "INSERT INTO returns
            (id, original_sale_id, reason, total_refunded, refund_method, operator_id,
             device_id, created_at, synced_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&return_id)
    .bind(&input.original_sale_id)
    .bind(&input.reason)
    .bind(total_refunded)
    .bind(&input.refund_method)
    .bind(&input.operator_id)
    .bind(&input.device_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if input.refund_method == "store_credit" {
        // Validated above: store_credit requires customer_id.
        let customer_id = customer_id.as_ref().expect("validated above");
        let ledger_id = uuid::Uuid::new_v4().to_string();
        let notes = format!("Refund for return {return_id}");

        sqlx::query(
            "INSERT INTO customer_ledger
                (id, customer_id, entry_type, amount, due_date, reference_sale_id, notes,
                 device_id, created_at, synced_at)
             VALUES (?, ?, 'credit', ?, NULL, ?, ?, ?, ?, NULL)",
        )
        .bind(&ledger_id)
        .bind(customer_id)
        .bind(total_refunded)
        .bind(&input.original_sale_id)
        .bind(&notes)
        .bind(&input.device_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let ledger_payload = CustomerLedgerEventPayload {
            id: ledger_id.clone(),
            customer_id: customer_id.clone(),
            entry_type: "credit".to_string(),
            amount: total_refunded,
            due_date: None,
            reference_sale_id: Some(input.original_sale_id.clone()),
            notes: Some(notes),
            device_id: input.device_id.clone(),
            created_at: now.clone(),
        };
        outbox::record(&mut tx, "customer_ledger", &ledger_id, &input.device_id, &ledger_payload)
            .await?;
    }

    // Recompute the sale's status from every line's total-returned vs.
    // total-sold, inside this same transaction so it sees the
    // return_items rows just inserted above.
    let outstanding: Vec<(f64, f64)> = sqlx::query_as(
        "SELECT si.quantity,
                COALESCE((SELECT SUM(ri.quantity) FROM return_items ri
                          WHERE ri.sale_item_id = si.id), 0)
         FROM sale_items si WHERE si.sale_id = ?",
    )
    .bind(&input.original_sale_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let fully_returned = outstanding
        .iter()
        .all(|(quantity, returned)| quantity - returned <= QUANTITY_EPSILON);
    let new_sale_status = if fully_returned {
        "returned"
    } else {
        "partially_returned"
    };

    sqlx::query("UPDATE sales SET status = ? WHERE id = ?")
        .bind(new_sale_status)
        .bind(&input.original_sale_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    let return_payload = ReturnEventPayload {
        id: return_id.clone(),
        original_sale_id: input.original_sale_id.clone(),
        reason: input.reason.clone(),
        total_refunded,
        refund_method: input.refund_method.clone(),
        operator_id: input.operator_id.clone(),
        device_id: input.device_id.clone(),
        created_at: now.clone(),
        sale_status: new_sale_status.to_string(),
        items: item_payloads,
    };
    outbox::record(&mut tx, "return", &return_id, &input.device_id, &return_payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    // Best-effort whole-document NFC-e cancellation — only once this
    // return (possibly the last of several partial ones) brings the sale
    // to fully `returned`. Never fails the return itself: any outcome
    // short of `Cancelled` just leaves a trail on `fiscal_documents`
    // (see module doc comment / `fiscal_client::try_cancel_for_return`).
    let fiscal_cancellation = if new_sale_status == "returned" {
        match fiscal_client::try_cancel_for_return(&pool, &input.original_sale_id, input.reason.as_deref())
            .await
        {
            Ok(status) => Some(status),
            Err(e) => {
                eprintln!(
                    "fiscal: cancellation attempt for sale {} failed: {e}",
                    input.original_sale_id
                );
                None
            }
        }
    } else {
        None
    };

    Ok(ReturnResult {
        id: return_id,
        original_sale_id: input.original_sale_id,
        total_refunded_cents,
        refund_method: input.refund_method,
        sale_status: new_sale_status.to_string(),
        created_at: now,
        fiscal_cancellation,
    })
}
