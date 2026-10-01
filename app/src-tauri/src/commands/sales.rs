//! Sale screen commands (plan section 4.1) — cart, totals, finalize sale.
//!
//! Phase 3 (plan section 8): a sale can be paid with multiple methods,
//! writing one `sale_payments` row per method-amount pair; the payment
//! screen validates the sum equals the sale total (plan section 4.1).
//!
//! Phase 4 (plan section 4.3): `store_credit` ("fiado") requires an
//! identified customer — enforced below — and writes a `debit` entry to
//! `customer_ledger` with `due_date = now + customers.default_due_days`.
//! The credit-limit check against that ledger is advisory only ("the
//! operator gets a warning ... not blocked by default"), so it lives in
//! the frontend before the operator confirms payment, not here.
//!
//! Scope cuts still deferred to later roadmap phases (not oversights):
//! only a sale-level discount is supported; per-line `sale_items.discount`
//! always writes 0 until a line-discount UI exists.
//!
//! `quantity` is a decimal (plan section 4.2 — fractional/bulk sales):
//! the operator picks a `unit_id` at sale time and this module looks up
//! that unit's `factor_to_stock` to derive `quantity_stock` for the stock
//! movement, exactly as section 4.2 describes for the cement bag/loose-kg
//! example. Money still crosses the boundary as integer cents; each line's
//! cents are rounded independently (`unit_price_cents * quantity`, rounded
//! once) and the subtotal is the sum of those already-rounded line totals,
//! so it never drifts from what the cart displayed per line.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

use crate::db;
use crate::fiscal_client::{self, FiscalBuyerInput, FiscalItemInput, FiscalPaymentInput};
use crate::money::cents_to_amount;
use crate::outbox;

/// Mirrors `sale_items` (plan section 3.3), nested inside a `sale` outbox
/// event rather than sent separately — a sale and its lines are one
/// atomic write, and the backend applies them together.
#[derive(Debug, Serialize)]
struct SaleItemEventPayload {
    id: String,
    product_id: String,
    unit_id: String,
    quantity: f64,
    quantity_stock: f64,
    unit_price: f64,
    discount: f64,
    line_total: f64,
}

/// Mirrors `sale_payments` (plan section 3.3), nested inside a `sale`
/// outbox event for the same reason as `SaleItemEventPayload`.
#[derive(Debug, Serialize)]
struct SalePaymentEventPayload {
    id: String,
    method: String,
    amount: f64,
    installments: i32,
}

/// Mirrors `sales` (plan section 3.3) plus its nested items/payments.
#[derive(Debug, Serialize)]
struct SaleEventPayload {
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
    items: Vec<SaleItemEventPayload>,
    payments: Vec<SalePaymentEventPayload>,
}

/// Mirrors `stock_movements` (plan section 3.1) — applied centrally as a
/// delta on `stock_levels`, never an absolute value (plan section 1.3).
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

/// Mirrors `customer_ledger` (plan section 3.2) — applied centrally as a
/// delta, matching the customer balance's "sum of entries" definition.
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

const PAYMENT_METHODS: &[&str] = &["cash", "debit", "credit", "pix", "boleto", "store_credit"];

#[derive(Debug, Deserialize)]
pub struct SaleItemInput {
    pub product_id: String,
    pub unit_id: String,
    pub quantity: f64,
    pub unit_price_cents: i64,
}

/// Rounds a line's cents once, the same way `money::amount_to_cents` does,
/// so cent totals never drift across repeated multiplication. `pub(crate)`
/// so `commands::returns` can reuse the exact same rounding rule for refund
/// line totals rather than re-deriving it.
pub(crate) fn line_total_cents(unit_price_cents: i64, quantity: f64) -> i64 {
    (unit_price_cents as f64 * quantity).round() as i64
}

#[derive(Debug, Deserialize)]
pub struct SalePaymentInput {
    pub method: String,
    pub amount_cents: i64,
    #[serde(default = "default_installments")]
    pub installments: i32,
}

fn default_installments() -> i32 {
    1
}

#[derive(Debug, Deserialize)]
pub struct FinalizeSaleInput {
    pub cash_session_id: String,
    pub device_id: String,
    pub operator_id: String,
    #[serde(default)]
    pub customer_id: Option<String>,
    pub items: Vec<SaleItemInput>,
    #[serde(default)]
    pub discount_total_cents: i64,
    pub payments: Vec<SalePaymentInput>,
}

#[derive(Debug, Serialize)]
pub struct SalePayment {
    pub id: String,
    pub method: String,
    pub amount_cents: i64,
    pub installments: i32,
}

#[derive(Debug, Serialize)]
pub struct Sale {
    pub id: String,
    pub cash_session_id: String,
    pub status: String,
    pub subtotal_cents: i64,
    pub discount_total_cents: i64,
    pub total_cents: i64,
    pub device_id: String,
    pub operator_id: String,
    pub customer_id: Option<String>,
    pub created_at: String,
    pub payments: Vec<SalePayment>,
    /// `fiscal_documents.id` created for this sale — used by
    /// `commands::fiscal::{get_fiscal_status, print_receipt}`.
    pub fiscal_document_id: String,
    /// `"authorized"`, `"rejected"`, or `"pending"` (queued — either a
    /// contingency-mode connectivity failure or no provider token
    /// configured yet; the frontend treats both the same: print the
    /// provisional receipt and let the background worker/manual retry
    /// resolve it later).
    pub fiscal_status: String,
}

#[tauri::command]
pub async fn finalize_sale<R: Runtime>(
    app: AppHandle<R>,
    input: FinalizeSaleInput,
) -> Result<Sale, String> {
    if input.items.is_empty() {
        return Err("a sale needs at least one item".into());
    }
    if input.items.iter().any(|i| i.quantity <= 0.0) {
        return Err("item quantities must be greater than zero".into());
    }
    if input.discount_total_cents < 0 {
        return Err("discount cannot be negative".into());
    }
    if input.payments.is_empty() {
        return Err("a sale needs at least one payment".into());
    }
    for payment in &input.payments {
        if !PAYMENT_METHODS.contains(&payment.method.as_str()) {
            return Err(format!("unsupported payment method '{}'", payment.method));
        }
        if payment.method == "store_credit" && input.customer_id.is_none() {
            return Err("store credit (fiado) requires an identified customer".into());
        }
        if payment.amount_cents <= 0 {
            return Err("each payment amount must be greater than zero".into());
        }
        if payment.installments < 1 {
            return Err("installments must be at least 1".into());
        }
    }

    let pool = db::pool(&app).await?;

    let session: Option<(String, String)> = sqlx::query_as(
        "SELECT status, device_id FROM cash_sessions WHERE id = ?",
    )
    .bind(&input.cash_session_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let (status, device_id) = session.ok_or_else(|| "cash session not found".to_string())?;
    if status != "open" {
        return Err("cash register is not open".into());
    }
    if device_id != input.device_id {
        return Err("cash session belongs to a different device".into());
    }

    let subtotal_cents: i64 = input
        .items
        .iter()
        .map(|i| line_total_cents(i.unit_price_cents, i.quantity))
        .sum();
    if input.discount_total_cents > subtotal_cents {
        return Err("discount cannot exceed the subtotal".into());
    }
    let total_cents = subtotal_cents - input.discount_total_cents;

    let paid_cents: i64 = input.payments.iter().map(|p| p.amount_cents).sum();
    if paid_cents != total_cents {
        return Err(format!(
            "payments total {} does not match sale total {}",
            cents_to_amount(paid_cents),
            cents_to_amount(total_cents)
        ));
    }

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    // Only needed for a store_credit payment's ledger entry and the
    // fiscal payload's buyer fields below, but fetched up front (inside
    // the transaction) so a customer_id pointing at nothing fails before
    // any of the sale's rows are written.
    let customer_info: Option<(i64, String, String, String)> = match &input.customer_id {
        Some(customer_id) => {
            let row: Option<(i64, String, String, String)> = sqlx::query_as(
                "SELECT default_due_days, doc_type, doc_number, name FROM customers WHERE id = ?",
            )
            .bind(customer_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            Some(row.ok_or_else(|| format!("customer {customer_id} not found"))?)
        }
        None => None,
    };
    let default_due_days = customer_info.as_ref().map(|c| c.0);
    let buyer = customer_info.as_ref().map(|c| FiscalBuyerInput {
        doc_type: c.1.clone(),
        doc_number: c.2.clone(),
        name: c.3.clone(),
    });

    let sale_id = uuid::Uuid::new_v4().to_string();
    let now_dt = chrono::Utc::now();
    let now = now_dt.to_rfc3339();

    sqlx::query(
        "INSERT INTO sales
            (id, cash_session_id, customer_id, status, subtotal, discount_total, total,
             device_id, operator_id, created_at, synced_at)
         VALUES (?, ?, ?, 'completed', ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&sale_id)
    .bind(&input.cash_session_id)
    .bind(&input.customer_id)
    .bind(cents_to_amount(subtotal_cents))
    .bind(cents_to_amount(input.discount_total_cents))
    .bind(cents_to_amount(total_cents))
    .bind(&input.device_id)
    .bind(&input.operator_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let mut item_payloads = Vec::with_capacity(input.items.len());
    let mut fiscal_items = Vec::with_capacity(input.items.len());
    for item in &input.items {
        let unit_row: (f64, String) =
            sqlx::query_as("SELECT factor_to_stock, unit_label FROM product_units WHERE id = ?")
                .bind(&item.unit_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("unit {} not found", item.unit_id))?;
        let (factor_to_stock, unit_label) = unit_row;
        let quantity_stock = item.quantity * factor_to_stock;

        let product_row: (String, String, Option<String>, Option<String>, Option<String>) =
            sqlx::query_as(
                "SELECT name, ncm, cfop_default, cst_csosn, sku FROM products WHERE id = ?",
            )
            .bind(&item.product_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("product {} not found", item.product_id))?;
        let (product_name, ncm, cfop_default, cst_csosn, sku) = product_row;
        let line_cents = line_total_cents(item.unit_price_cents, item.quantity);
        let unit_price = cents_to_amount(item.unit_price_cents);
        let line_total = cents_to_amount(line_cents);
        let item_id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO sale_items
                (id, sale_id, product_id, unit_id, quantity, quantity_stock, unit_price,
                 discount, line_total)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&item_id)
        .bind(&sale_id)
        .bind(&item.product_id)
        .bind(&item.unit_id)
        .bind(item.quantity)
        .bind(quantity_stock)
        .bind(unit_price)
        .bind(line_total)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        item_payloads.push(SaleItemEventPayload {
            id: item_id,
            product_id: item.product_id.clone(),
            unit_id: item.unit_id.clone(),
            quantity: item.quantity,
            quantity_stock,
            unit_price,
            discount: 0.0,
            line_total,
        });

        fiscal_items.push(FiscalItemInput {
            product_name,
            product_sku_or_id: sku.unwrap_or_else(|| item.product_id.clone()),
            ncm,
            // "5102" (venda de mercadoria adquirida de terceiros, intra-
            // state) is the MVP fallback for a product registered
            // without its own CFOP — matches this module's existing
            // stance on scope cuts (see the module doc comment): the
            // registry screen should require one, but nothing here blocks
            // a sale over a missing tax field; the provider's own
            // validation is what actually enforces it (see
            // fiscal_client::emit's rejected-outcome path).
            cfop: cfop_default.unwrap_or_else(|| "5102".to_string()),
            cst_csosn: cst_csosn.unwrap_or_default(),
            unit_label,
            quantity: item.quantity,
            unit_price,
            line_total,
        });

        let movement_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO stock_movements
                (id, product_id, delta_quantity, reason, reference_id, device_id, created_at,
                 synced_at)
             VALUES (?, ?, ?, 'sale', ?, ?, ?, NULL)",
        )
        .bind(&movement_id)
        .bind(&item.product_id)
        .bind(-quantity_stock)
        .bind(&sale_id)
        .bind(&input.device_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE stock_levels SET quantity = quantity - ?, updated_at = ? WHERE product_id = ?",
        )
        .bind(quantity_stock)
        .bind(&now)
        .bind(&item.product_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let movement_payload = StockMovementEventPayload {
            id: movement_id.clone(),
            product_id: item.product_id.clone(),
            delta_quantity: -quantity_stock,
            reason: "sale".to_string(),
            reference_id: Some(sale_id.clone()),
            device_id: input.device_id.clone(),
            created_at: now.clone(),
        };
        outbox::record(&mut tx, "stock_movement", &movement_id, &input.device_id, &movement_payload)
            .await?;
    }

    let mut payments = Vec::with_capacity(input.payments.len());
    let mut payment_payloads = Vec::with_capacity(input.payments.len());
    for payment in &input.payments {
        let payment_id = uuid::Uuid::new_v4().to_string();
        let amount = cents_to_amount(payment.amount_cents);
        sqlx::query(
            "INSERT INTO sale_payments (id, sale_id, method, amount, installments, gateway_ref)
             VALUES (?, ?, ?, ?, ?, NULL)",
        )
        .bind(&payment_id)
        .bind(&sale_id)
        .bind(&payment.method)
        .bind(amount)
        .bind(payment.installments)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        payment_payloads.push(SalePaymentEventPayload {
            id: payment_id.clone(),
            method: payment.method.clone(),
            amount,
            installments: payment.installments,
        });

        if payment.method == "store_credit" {
            // Validated above: store_credit requires customer_id, and
            // default_due_days was fetched for exactly that customer.
            let customer_id = input.customer_id.as_ref().expect("validated above");
            let due_days = default_due_days.expect("validated above");
            let due_date = (now_dt + chrono::Duration::days(due_days)).to_rfc3339();
            let ledger_id = uuid::Uuid::new_v4().to_string();

            sqlx::query(
                "INSERT INTO customer_ledger
                    (id, customer_id, entry_type, amount, due_date, reference_sale_id, notes,
                     device_id, created_at, synced_at)
                 VALUES (?, ?, 'debit', ?, ?, ?, NULL, ?, ?, NULL)",
            )
            .bind(&ledger_id)
            .bind(customer_id)
            .bind(amount)
            .bind(&due_date)
            .bind(&sale_id)
            .bind(&input.device_id)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

            let ledger_payload = CustomerLedgerEventPayload {
                id: ledger_id.clone(),
                customer_id: customer_id.clone(),
                entry_type: "debit".to_string(),
                amount,
                due_date: Some(due_date),
                reference_sale_id: Some(sale_id.clone()),
                notes: None,
                device_id: input.device_id.clone(),
                created_at: now.clone(),
            };
            outbox::record(&mut tx, "customer_ledger", &ledger_id, &input.device_id, &ledger_payload)
                .await?;
        }

        payments.push(SalePayment {
            id: payment_id,
            method: payment.method.clone(),
            amount_cents: payment.amount_cents,
            installments: payment.installments,
        });
    }

    let sale_payload = SaleEventPayload {
        id: sale_id.clone(),
        cash_session_id: input.cash_session_id.clone(),
        customer_id: input.customer_id.clone(),
        status: "completed".to_string(),
        subtotal: cents_to_amount(subtotal_cents),
        discount_total: cents_to_amount(input.discount_total_cents),
        total: cents_to_amount(total_cents),
        device_id: input.device_id.clone(),
        operator_id: input.operator_id.clone(),
        created_at: now.clone(),
        items: item_payloads,
        payments: payment_payloads,
    };
    outbox::record(&mut tx, "sale", &sale_id, &input.device_id, &sale_payload).await?;

    // Plan section 5.1, step 2: finalizing the sale always creates a
    // fiscal_documents row and enqueues it — the sale is already valid
    // operationally at this point (section 1.2) regardless of whether
    // emission below succeeds, is rejected, or can't reach the provider
    // at all.
    let fiscal_payments: Vec<FiscalPaymentInput> = payments
        .iter()
        .map(|p| FiscalPaymentInput {
            method: p.method.clone(),
            amount: cents_to_amount(p.amount_cents),
        })
        .collect();
    let fiscal_payload = fiscal_client::build_payload(
        crate::config::fiscal_issuer_cnpj(),
        &fiscal_items,
        &fiscal_payments,
        buyer.as_ref(),
        cents_to_amount(input.discount_total_cents),
    );
    let fiscal_document_id = uuid::Uuid::new_v4().to_string();
    let fiscal_queue_id = uuid::Uuid::new_v4().to_string();
    let fiscal_payload_json = serde_json::to_string(&fiscal_payload).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO fiscal_documents
            (id, sale_id, doc_type, status, attempts, created_at, updated_at)
         VALUES (?, ?, 'NFCE', 'pending', 0, ?, ?)",
    )
    .bind(&fiscal_document_id)
    .bind(&sale_id)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO fiscal_queue (id, fiscal_document_id, payload_json, next_retry_at, retry_count)
         VALUES (?, ?, ?, NULL, 0)",
    )
    .bind(&fiscal_queue_id)
    .bind(&fiscal_document_id)
    .bind(&fiscal_payload_json)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    // Phase 9 (plan section 5.2's back-office stuck-in-contingency alert):
    // mirrors this freshly-created 'pending' row centrally, same as every
    // later status transition (fiscal_client::mark_authorized/mark_rejected).
    fiscal_client::record_fiscal_document_event(&mut tx, &fiscal_document_id).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    // Best-effort synchronous first attempt: NFC-e emission is synchronous
    // on the provider's side (see fiscal_client's doc comment), so a
    // connected store gets an authorized DANFCE to print within this same
    // call in the common case. A connectivity failure here is exactly
    // plan section 5.2's contingency mode — the row stays queued and
    // `fiscal_client::spawn_worker`'s background retries pick it up, so a
    // failure here is only ever logged, never surfaced as an error to the
    // operator (the sale itself already succeeded).
    let fiscal_status = fiscal_client::try_emit(&pool, &fiscal_document_id)
        .await
        .unwrap_or_else(|e| {
            eprintln!("fiscal: initial emission attempt for {sale_id} failed: {e}");
            "pending".to_string()
        });

    Ok(Sale {
        id: sale_id,
        cash_session_id: input.cash_session_id,
        status: "completed".to_string(),
        subtotal_cents,
        discount_total_cents: input.discount_total_cents,
        total_cents,
        device_id: input.device_id,
        operator_id: input.operator_id,
        customer_id: input.customer_id,
        created_at: now,
        payments,
        fiscal_document_id,
        fiscal_status,
    })
}
