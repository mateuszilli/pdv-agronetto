//! Purchasing and receiving commands (plan sections 3.1 and 4.6, roadmap
//! Phase 8).
//!
//! Unlike `sale_items`, `purchase_order_items` has no `unit_id` — an
//! order's `quantity_ordered`/`quantity_received` are always expressed in
//! the product's `stock_unit` (you order bulk stock, not a selling unit),
//! so no conversion factor applies before writing a `stock_movements`
//! delta the way `sales.rs`/`returns.rs` need one.
//!
//! Status lifecycle: this pass skips the plan's `draft` state — a
//! purchase order is created already `sent` (there's no separate
//! edit-before-sending step in this MVP) and transitions to
//! `partially_received` or `received` as `receive_purchase_order_items`
//! is called, mirroring the same "trust the client's freshly computed
//! status" convention `commands::returns` uses for `sales.status`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

use crate::db;
use crate::money::{amount_to_cents, cents_to_amount};
use crate::outbox;

/// Quantities are `f64` (see `money.rs`'s note on SQLite's REAL affinity)
/// — absorbs float round-trip noise comparing a receive quantity against
/// what's still outstanding, same tolerance `commands::returns` uses.
const QUANTITY_EPSILON: f64 = 1e-6;

/// Mirrors `purchase_order_items` (plan section 3.1), nested inside a
/// `purchase_order` outbox event — same one-atomic-write convention
/// `returns.rs`'s `ReturnItemEventPayload` uses.
#[derive(Debug, Serialize)]
struct PurchaseOrderItemEventPayload {
    id: String,
    product_id: String,
    quantity_ordered: f64,
    quantity_received: f64,
    unit_cost: f64,
}

/// Mirrors `purchase_orders` plus its nested items. Re-emitted in full
/// (every item, not just the ones that changed) whenever the order's
/// status or any item's `quantity_received` changes, so the backend's
/// upsert always sees a complete, consistent snapshot.
#[derive(Debug, Serialize)]
struct PurchaseOrderEventPayload {
    id: String,
    supplier_name: String,
    status: String,
    created_at: String,
    items: Vec<PurchaseOrderItemEventPayload>,
}

/// Mirrors `stock_movements` (plan section 3.1) — same shape as
/// `sales.rs`'s/`returns.rs`'s `StockMovementEventPayload`, duplicated per
/// this codebase's convention of not sharing small payload structs across
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

#[derive(Debug, Serialize)]
pub struct PurchaseOrderSummary {
    pub id: String,
    pub supplier_name: String,
    pub status: String,
    pub created_at: String,
    pub item_count: i64,
    pub total_cost_cents: i64,
}

/// Lists every purchase order, most recent first — the purchasing
/// screen's landing list.
#[tauri::command]
pub async fn list_purchase_orders<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<PurchaseOrderSummary>, String> {
    let pool = db::pool(&app).await?;
    let rows: Vec<(String, String, String, String, i64, f64)> = sqlx::query_as(
        "SELECT po.id, po.supplier_name, po.status, po.created_at,
                COUNT(poi.id) as item_count,
                COALESCE(SUM(poi.quantity_ordered * poi.unit_cost), 0) as total_cost
         FROM purchase_orders po
         LEFT JOIN purchase_order_items poi ON poi.purchase_order_id = po.id
         GROUP BY po.id, po.supplier_name, po.status, po.created_at
         ORDER BY po.created_at DESC",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(
            |(id, supplier_name, status, created_at, item_count, total_cost)| PurchaseOrderSummary {
                id,
                supplier_name,
                status,
                created_at,
                item_count,
                total_cost_cents: amount_to_cents(total_cost),
            },
        )
        .collect())
}

#[derive(Debug, Serialize)]
pub struct PurchaseOrderItemDetail {
    pub id: String,
    pub product_id: String,
    pub product_name: String,
    pub stock_unit: String,
    pub quantity_ordered: f64,
    pub quantity_received: f64,
    pub unit_cost_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct PurchaseOrderDetail {
    pub id: String,
    pub supplier_name: String,
    pub status: String,
    pub created_at: String,
    pub items: Vec<PurchaseOrderItemDetail>,
}

/// Loads one order's header plus every line item, joined with the
/// product's current name/`stock_unit` for display — used by both the
/// order detail screen and as the return value of `create_purchase_order`
/// / `receive_purchase_order_items`.
#[tauri::command]
pub async fn get_purchase_order<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<PurchaseOrderDetail, String> {
    let pool = db::pool(&app).await?;

    let header: Option<(String, String, String)> = sqlx::query_as(
        "SELECT supplier_name, status, created_at FROM purchase_orders WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let (supplier_name, status, created_at) =
        header.ok_or_else(|| format!("purchase order {id} not found"))?;

    let item_rows: Vec<(String, String, String, String, f64, f64, f64)> = sqlx::query_as(
        "SELECT poi.id, poi.product_id, p.name, p.stock_unit, poi.quantity_ordered,
                poi.quantity_received, poi.unit_cost
         FROM purchase_order_items poi
         JOIN products p ON p.id = poi.product_id
         WHERE poi.purchase_order_id = ?
         ORDER BY p.name",
    )
    .bind(&id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let items = item_rows
        .into_iter()
        .map(
            |(item_id, product_id, product_name, stock_unit, quantity_ordered, quantity_received, unit_cost)| {
                PurchaseOrderItemDetail {
                    id: item_id,
                    product_id,
                    product_name,
                    stock_unit,
                    quantity_ordered,
                    quantity_received,
                    unit_cost_cents: amount_to_cents(unit_cost),
                }
            },
        )
        .collect();

    Ok(PurchaseOrderDetail {
        id,
        supplier_name,
        status,
        created_at,
        items,
    })
}

#[derive(Debug, Deserialize)]
pub struct PurchaseOrderItemInput {
    pub product_id: String,
    pub quantity_ordered: f64,
    pub unit_cost_cents: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreatePurchaseOrderInput {
    pub device_id: String,
    pub supplier_name: String,
    pub items: Vec<PurchaseOrderItemInput>,
}

/// Creates a purchase order with `status = 'sent'` (see module doc) plus
/// one `purchase_order_items` row per line, all `quantity_received = 0`.
#[tauri::command]
pub async fn create_purchase_order<R: Runtime>(
    app: AppHandle<R>,
    input: CreatePurchaseOrderInput,
) -> Result<PurchaseOrderDetail, String> {
    let supplier_name = input.supplier_name.trim().to_string();
    if supplier_name.is_empty() {
        return Err("supplier name is required".into());
    }
    if input.items.is_empty() {
        return Err("a purchase order needs at least one item".into());
    }
    if input
        .items
        .iter()
        .any(|i| i.quantity_ordered <= 0.0 || i.unit_cost_cents < 0)
    {
        return Err(
            "ordered quantities must be greater than zero and unit costs cannot be negative".into(),
        );
    }

    let pool = db::pool(&app).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO purchase_orders (id, supplier_name, status, created_at)
         VALUES (?, ?, 'sent', ?)",
    )
    .bind(&id)
    .bind(&supplier_name)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let mut item_payloads = Vec::with_capacity(input.items.len());
    for item in &input.items {
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM products WHERE id = ?")
            .bind(&item.product_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("product {} not found", item.product_id));
        }

        let item_id = uuid::Uuid::new_v4().to_string();
        let unit_cost = cents_to_amount(item.unit_cost_cents);
        sqlx::query(
            "INSERT INTO purchase_order_items
                (id, purchase_order_id, product_id, quantity_ordered, quantity_received, unit_cost)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&item_id)
        .bind(&id)
        .bind(&item.product_id)
        .bind(item.quantity_ordered)
        .bind(unit_cost)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        item_payloads.push(PurchaseOrderItemEventPayload {
            id: item_id,
            product_id: item.product_id.clone(),
            quantity_ordered: item.quantity_ordered,
            quantity_received: 0.0,
            unit_cost,
        });
    }

    let payload = PurchaseOrderEventPayload {
        id: id.clone(),
        supplier_name,
        status: "sent".to_string(),
        created_at: now,
        items: item_payloads,
    };
    outbox::record(&mut tx, "purchase_order", &id, &input.device_id, &payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    get_purchase_order(app, id).await
}

#[derive(Debug, Deserialize)]
pub struct ReceiveItemInput {
    pub purchase_order_item_id: String,
    pub quantity: f64,
}

#[derive(Debug, Deserialize)]
pub struct ReceivePurchaseOrderInput {
    pub device_id: String,
    pub purchase_order_id: String,
    pub items: Vec<ReceiveItemInput>,
}

/// Records goods received against an open order (plan section 4.6:
/// "on receiving goods (full or partial), generates positive
/// `stock_movements` and updates `quantity_received`"). Can be called
/// multiple times across separate deliveries, same partial-progress
/// pattern `returns::create_return` uses for `sale_items`.
#[tauri::command]
pub async fn receive_purchase_order_items<R: Runtime>(
    app: AppHandle<R>,
    input: ReceivePurchaseOrderInput,
) -> Result<PurchaseOrderDetail, String> {
    if input.items.is_empty() {
        return Err("select at least one item to receive".into());
    }
    if input.items.iter().any(|i| i.quantity <= 0.0) {
        return Err("received quantities must be greater than zero".into());
    }

    let pool = db::pool(&app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let status: Option<(String,)> =
        sqlx::query_as("SELECT status FROM purchase_orders WHERE id = ?")
            .bind(&input.purchase_order_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let (status,) = status.ok_or_else(|| format!("purchase order {} not found", input.purchase_order_id))?;
    if status == "received" {
        return Err("this purchase order has already been fully received".into());
    }

    let now = chrono::Utc::now().to_rfc3339();

    for item in &input.items {
        let row: Option<(String, String, f64, f64)> = sqlx::query_as(
            "SELECT purchase_order_id, product_id, quantity_ordered, quantity_received
             FROM purchase_order_items WHERE id = ?",
        )
        .bind(&item.purchase_order_item_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let (order_id, product_id, quantity_ordered, quantity_received) = row
            .ok_or_else(|| format!("purchase order item {} not found", item.purchase_order_item_id))?;
        if order_id != input.purchase_order_id {
            return Err(format!(
                "item {} does not belong to purchase order {}",
                item.purchase_order_item_id, input.purchase_order_id
            ));
        }

        let outstanding = quantity_ordered - quantity_received;
        if item.quantity > outstanding + QUANTITY_EPSILON {
            return Err(format!(
                "only {outstanding} remaining to receive for item {}",
                item.purchase_order_item_id
            ));
        }

        sqlx::query(
            "UPDATE purchase_order_items SET quantity_received = quantity_received + ?
             WHERE id = ?",
        )
        .bind(item.quantity)
        .bind(&item.purchase_order_item_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let movement_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO stock_movements
                (id, product_id, delta_quantity, reason, reference_id, device_id, created_at,
                 synced_at)
             VALUES (?, ?, ?, 'purchase', ?, ?, ?, NULL)",
        )
        .bind(&movement_id)
        .bind(&product_id)
        .bind(item.quantity)
        .bind(&input.purchase_order_id)
        .bind(&input.device_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE stock_levels SET quantity = quantity + ?, updated_at = ? WHERE product_id = ?",
        )
        .bind(item.quantity)
        .bind(&now)
        .bind(&product_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let movement_payload = StockMovementEventPayload {
            id: movement_id.clone(),
            product_id: product_id.clone(),
            delta_quantity: item.quantity,
            reason: "purchase".to_string(),
            reference_id: Some(input.purchase_order_id.clone()),
            device_id: input.device_id.clone(),
            created_at: now.clone(),
        };
        outbox::record(&mut tx, "stock_movement", &movement_id, &input.device_id, &movement_payload)
            .await?;
    }

    // Recompute the order's status from every item's fresh totals (inside
    // this same transaction, so it sees the updates just written above) —
    // same "recompute status from all lines" shape create_return uses for
    // sales.status.
    let totals: Vec<(f64, f64)> = sqlx::query_as(
        "SELECT quantity_ordered, quantity_received FROM purchase_order_items
         WHERE purchase_order_id = ?",
    )
    .bind(&input.purchase_order_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let fully_received = totals
        .iter()
        .all(|(ordered, received)| ordered - received <= QUANTITY_EPSILON);
    let any_received = totals.iter().any(|(_, received)| *received > QUANTITY_EPSILON);
    let new_status = if fully_received {
        "received"
    } else if any_received {
        "partially_received"
    } else {
        "sent"
    };

    sqlx::query("UPDATE purchase_orders SET status = ? WHERE id = ?")
        .bind(new_status)
        .bind(&input.purchase_order_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    let (supplier_name, created_at): (String, String) = sqlx::query_as(
        "SELECT supplier_name, created_at FROM purchase_orders WHERE id = ?",
    )
    .bind(&input.purchase_order_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let all_items: Vec<(String, String, f64, f64, f64)> = sqlx::query_as(
        "SELECT id, product_id, quantity_ordered, quantity_received, unit_cost
         FROM purchase_order_items WHERE purchase_order_id = ?",
    )
    .bind(&input.purchase_order_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let item_payloads = all_items
        .into_iter()
        .map(
            |(item_id, product_id, quantity_ordered, quantity_received, unit_cost)| {
                PurchaseOrderItemEventPayload {
                    id: item_id,
                    product_id,
                    quantity_ordered,
                    quantity_received,
                    unit_cost,
                }
            },
        )
        .collect();

    let payload = PurchaseOrderEventPayload {
        id: input.purchase_order_id.clone(),
        supplier_name,
        status: new_status.to_string(),
        created_at,
        items: item_payloads,
    };
    outbox::record(&mut tx, "purchase_order", &input.purchase_order_id, &input.device_id, &payload)
        .await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    get_purchase_order(app, input.purchase_order_id).await
}
