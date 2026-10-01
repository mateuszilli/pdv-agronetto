//! Product catalog commands (plan section 3.1, roadmap Phases 1-2 and 8).
//!
//! Every product still gets exactly one default `product_units` row
//! (`is_default_sale = 1`, created with `factor_to_stock = 1`) managed via
//! the plain product fields on `ProductInput`/`create_product`/
//! `update_product`. Additional selling units with their own conversion
//! factor (the "50kg bag" vs "loose kg" case from plan section 4.2) are
//! managed separately through `list_product_units`/`create_product_unit`/
//! `update_product_unit`. Purchasing/receiving (plan section 4.6) lives in
//! `commands::purchasing`.
//!
//! `min_quantity` (the low-stock trigger, plan section 3.1's
//! `stock_levels.min_quantity`) is edited here as a plain product field —
//! it's conceptually part of a product's catalog registration even though
//! it's stored on `stock_levels` alongside the delta-derived `quantity`.
//! Unlike `quantity` itself, it's a directly-set threshold, not a balance,
//! so it syncs as a plain last-write-wins field (a `stock_level` outbox
//! event) rather than a delta — see `applyStockLevel` on the backend.

const AT_LEAST_ONE_ACTIVE_UNIT: &str = "a product needs at least one active selling unit";

use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction};
use tauri::{AppHandle, Runtime};

use crate::db;
use crate::money::{amount_to_cents, cents_to_amount};
use crate::outbox;

/// Mirrors the `products` row shape the backend's apply logic upserts into
/// central Postgres (plan section 3.1), keyed by `updated_at` for
/// last-write-wins (plan section 1.3).
#[derive(Debug, Serialize)]
struct ProductEventPayload<'a> {
    id: &'a str,
    sku: &'a Option<String>,
    barcode: &'a Option<String>,
    name: &'a str,
    ncm: &'a str,
    cfop_default: &'a Option<String>,
    cst_csosn: &'a Option<String>,
    stock_unit: &'a str,
    cost_price: f64,
    active: bool,
    updated_at: &'a str,
}

/// Mirrors the `min_quantity` column of `stock_levels` (plan section 3.1)
/// — a plain last-write-wins threshold, not a balance, so it's synced on
/// its own rather than folded into `stock_movement`'s delta-based events.
#[derive(Debug, Serialize)]
struct StockLevelEventPayload<'a> {
    product_id: &'a str,
    min_quantity: f64,
    updated_at: &'a str,
}

/// Mirrors `product_units` (plan section 3.1). Emitted for every row of a
/// product's units whenever any one of them changes, since
/// `update_product_unit` can flip `is_default_sale` on a sibling row too.
#[derive(Debug, Serialize)]
struct ProductUnitEventPayload {
    id: String,
    product_id: String,
    unit_label: String,
    factor_to_stock: f64,
    sell_price: f64,
    is_default_sale: bool,
    active: bool,
    updated_at: String,
}

/// Emits a `product_unit` outbox event for every unit row of `product_id`
/// — covers the case where updating one unit's `is_default_sale` flips it
/// off on another (see `update_product_unit`).
async fn emit_all_units_for_product(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    device_id: &str,
) -> Result<(), String> {
    let rows = sqlx::query(
        "SELECT id, product_id, unit_label, factor_to_stock, sell_price, is_default_sale,
                active, updated_at
         FROM product_units WHERE product_id = ?",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    for row in &rows {
        let sell_price: f64 = row.try_get("sell_price").map_err(|e| e.to_string())?;
        let payload = ProductUnitEventPayload {
            id: row.try_get("id").map_err(|e| e.to_string())?,
            product_id: product_id.to_string(),
            unit_label: row.try_get("unit_label").map_err(|e| e.to_string())?,
            factor_to_stock: row.try_get("factor_to_stock").map_err(|e| e.to_string())?,
            sell_price,
            is_default_sale: row.try_get::<i64, _>("is_default_sale").map_err(|e| e.to_string())? != 0,
            active: row.try_get::<i64, _>("active").map_err(|e| e.to_string())? != 0,
            updated_at: row.try_get("updated_at").map_err(|e| e.to_string())?,
        };
        let entity_id = payload.id.clone();
        outbox::record(tx, "product_unit", &entity_id, device_id, &payload).await?;
    }
    Ok(())
}

/// Row shape used by product search (sale screen) and the catalog list.
/// `unit_id`/`unit_label`/`sell_price_cents`/`factor_to_stock` describe the
/// product's default selling unit; `unit_count` tells the sale screen
/// whether to add it straight away (1 unit) or open the fast numeric unit
/// selector first (plan section 4.1).
#[derive(Debug, Serialize)]
pub struct ProductSummary {
    pub id: String,
    pub sku: Option<String>,
    pub barcode: Option<String>,
    pub name: String,
    pub stock_unit: String,
    pub active: bool,
    pub unit_id: String,
    pub unit_label: String,
    pub sell_price_cents: i64,
    pub factor_to_stock: f64,
    pub unit_count: i64,
    pub quantity: f64,
}

/// Full record used by the product create/edit form.
#[derive(Debug, Serialize)]
pub struct ProductDetail {
    pub id: String,
    pub sku: Option<String>,
    pub barcode: Option<String>,
    pub name: String,
    pub ncm: String,
    pub cfop_default: Option<String>,
    pub cst_csosn: Option<String>,
    pub stock_unit: String,
    pub cost_price_cents: i64,
    pub active: bool,
    pub unit_id: String,
    pub unit_label: String,
    pub sell_price_cents: i64,
    pub quantity: f64,
    pub min_quantity: f64,
}

/// One row of `product_units` (plan section 3.1) — an alternative selling
/// unit with its conversion factor back to `stock_unit`.
#[derive(Debug, Serialize)]
pub struct ProductUnitOption {
    pub id: String,
    pub product_id: String,
    pub unit_label: String,
    pub factor_to_stock: f64,
    pub sell_price_cents: i64,
    pub is_default_sale: bool,
    pub active: bool,
}

fn row_to_unit_option(row: &sqlx::sqlite::SqliteRow) -> Result<ProductUnitOption, sqlx::Error> {
    let sell_price: f64 = row.try_get("sell_price")?;
    Ok(ProductUnitOption {
        id: row.try_get("id")?,
        product_id: row.try_get("product_id")?,
        unit_label: row.try_get("unit_label")?,
        factor_to_stock: row.try_get("factor_to_stock")?,
        sell_price_cents: amount_to_cents(sell_price),
        is_default_sale: row.try_get::<i64, _>("is_default_sale")? != 0,
        active: row.try_get::<i64, _>("active")? != 0,
    })
}

#[derive(Debug, Deserialize)]
pub struct ProductInput {
    pub sku: Option<String>,
    pub barcode: Option<String>,
    pub name: String,
    pub ncm: String,
    pub cfop_default: Option<String>,
    pub cst_csosn: Option<String>,
    pub stock_unit: String,
    pub cost_price_cents: i64,
    pub unit_label: String,
    pub sell_price_cents: i64,
    #[serde(default)]
    pub min_quantity: f64,
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

const SUMMARY_SELECT: &str = "
    SELECT p.id, p.sku, p.barcode, p.name, p.stock_unit, p.active,
           u.id as unit_id, u.unit_label, u.sell_price, u.factor_to_stock,
           (SELECT COUNT(*) FROM product_units u2
             WHERE u2.product_id = p.id AND u2.active = 1) as unit_count,
           COALESCE(sl.quantity, 0) as quantity
    FROM products p
    JOIN product_units u ON u.product_id = p.id AND u.is_default_sale = 1
    LEFT JOIN stock_levels sl ON sl.product_id = p.id";

fn row_to_summary(row: &sqlx::sqlite::SqliteRow) -> Result<ProductSummary, sqlx::Error> {
    let sell_price: f64 = row.try_get("sell_price")?;
    Ok(ProductSummary {
        id: row.try_get("id")?,
        sku: row.try_get("sku")?,
        barcode: row.try_get("barcode")?,
        name: row.try_get("name")?,
        stock_unit: row.try_get("stock_unit")?,
        active: row.try_get::<i64, _>("active")? != 0,
        unit_id: row.try_get("unit_id")?,
        unit_label: row.try_get("unit_label")?,
        sell_price_cents: amount_to_cents(sell_price),
        factor_to_stock: row.try_get("factor_to_stock")?,
        unit_count: row.try_get("unit_count")?,
        quantity: row.try_get("quantity")?,
    })
}

/// Searches active products by name, SKU, or barcode (including secondary
/// barcodes in `product_barcodes`). `query: None` (or empty) returns the
/// first 50 active products alphabetically — enough for the sale screen's
/// incremental search and the catalog list.
#[tauri::command]
pub async fn list_products<R: Runtime>(
    app: AppHandle<R>,
    query: Option<String>,
) -> Result<Vec<ProductSummary>, String> {
    let pool = db::pool(&app).await?;
    let query = non_empty(query);
    let like = query.as_ref().map(|q| format!("%{q}%")).unwrap_or_default();
    let has_query = query.is_some();

    let sql = format!(
        "{SUMMARY_SELECT}
         WHERE p.active = 1
           AND (? = 0 OR p.name LIKE ? OR p.sku LIKE ? OR p.barcode LIKE ?
                OR EXISTS (
                    SELECT 1 FROM product_barcodes pb
                    WHERE pb.product_id = p.id AND pb.barcode LIKE ?
                ))
         ORDER BY p.name
         LIMIT 50"
    );

    let rows = sqlx::query(&sql)
        .bind(!has_query)
        .bind(&like)
        .bind(&like)
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

#[tauri::command]
pub async fn get_product<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<ProductDetail, String> {
    let pool = db::pool(&app).await?;

    let row = sqlx::query(
        "SELECT p.id, p.sku, p.barcode, p.name, p.ncm, p.cfop_default, p.cst_csosn,
                p.stock_unit, p.cost_price, p.active,
                u.id as unit_id, u.unit_label, u.sell_price,
                COALESCE(sl.quantity, 0) as quantity,
                COALESCE(sl.min_quantity, 0) as min_quantity
         FROM products p
         JOIN product_units u ON u.product_id = p.id AND u.is_default_sale = 1
         LEFT JOIN stock_levels sl ON sl.product_id = p.id
         WHERE p.id = ?",
    )
    .bind(&id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("product {id} not found"))?;

    let cost_price: f64 = row.try_get("cost_price").map_err(|e| e.to_string())?;
    let sell_price: f64 = row.try_get("sell_price").map_err(|e| e.to_string())?;

    Ok(ProductDetail {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        sku: row.try_get("sku").map_err(|e| e.to_string())?,
        barcode: row.try_get("barcode").map_err(|e| e.to_string())?,
        name: row.try_get("name").map_err(|e| e.to_string())?,
        ncm: row.try_get("ncm").map_err(|e| e.to_string())?,
        cfop_default: row.try_get("cfop_default").map_err(|e| e.to_string())?,
        cst_csosn: row.try_get("cst_csosn").map_err(|e| e.to_string())?,
        stock_unit: row.try_get("stock_unit").map_err(|e| e.to_string())?,
        cost_price_cents: amount_to_cents(cost_price),
        active: row.try_get::<i64, _>("active").map_err(|e| e.to_string())? != 0,
        unit_id: row.try_get("unit_id").map_err(|e| e.to_string())?,
        unit_label: row.try_get("unit_label").map_err(|e| e.to_string())?,
        sell_price_cents: amount_to_cents(sell_price),
        quantity: row.try_get("quantity").map_err(|e| e.to_string())?,
        min_quantity: row.try_get("min_quantity").map_err(|e| e.to_string())?,
    })
}

#[tauri::command]
pub async fn create_product<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
    input: ProductInput,
) -> Result<ProductDetail, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let ncm = input.ncm.trim().to_string();
    if ncm.is_empty() {
        return Err("ncm is required".into());
    }
    let stock_unit = input.stock_unit.trim().to_string();
    if stock_unit.is_empty() {
        return Err("stock_unit is required".into());
    }
    let unit_label = input.unit_label.trim().to_string();
    if unit_label.is_empty() {
        return Err("unit_label is required".into());
    }
    if input.min_quantity < 0.0 {
        return Err("min_quantity cannot be negative".into());
    }

    let pool = db::pool(&app).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let unit_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let sku = non_empty(input.sku);
    let barcode = non_empty(input.barcode);
    let cfop_default = non_empty(input.cfop_default);
    let cst_csosn = non_empty(input.cst_csosn);
    let cost_price = cents_to_amount(input.cost_price_cents);
    let sell_price = cents_to_amount(input.sell_price_cents);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO products
            (id, sku, barcode, name, ncm, cfop_default, cst_csosn, stock_unit,
             cost_price, active, updated_at, deleted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, NULL)",
    )
    .bind(&id)
    .bind(&sku)
    .bind(&barcode)
    .bind(&name)
    .bind(&ncm)
    .bind(&cfop_default)
    .bind(&cst_csosn)
    .bind(&stock_unit)
    .bind(cost_price)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("failed to create product: {e}"))?;

    sqlx::query(
        "INSERT INTO product_units
            (id, product_id, unit_label, factor_to_stock, sell_price, is_default_sale, active,
             updated_at)
         VALUES (?, ?, ?, 1, ?, 1, 1, ?)",
    )
    .bind(&unit_id)
    .bind(&id)
    .bind(&unit_label)
    .bind(sell_price)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO stock_levels (product_id, quantity, min_quantity, updated_at)
         VALUES (?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(input.min_quantity)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(barcode) = &barcode {
        sqlx::query(
            "INSERT INTO product_barcodes (id, product_id, barcode, unit_id) VALUES (?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&id)
        .bind(barcode)
        .bind(&unit_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("barcode already in use: {e}"))?;
    }

    let product_payload = ProductEventPayload {
        id: &id,
        sku: &sku,
        barcode: &barcode,
        name: &name,
        ncm: &ncm,
        cfop_default: &cfop_default,
        cst_csosn: &cst_csosn,
        stock_unit: &stock_unit,
        cost_price,
        active: true,
        updated_at: &now,
    };
    outbox::record(&mut tx, "product", &id, &device_id, &product_payload).await?;
    emit_all_units_for_product(&mut tx, &id, &device_id).await?;

    let stock_level_payload = StockLevelEventPayload {
        product_id: &id,
        min_quantity: input.min_quantity,
        updated_at: &now,
    };
    outbox::record(&mut tx, "stock_level", &id, &device_id, &stock_level_payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    get_product(app, id).await
}

#[tauri::command]
pub async fn update_product<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
    id: String,
    input: ProductInput,
) -> Result<ProductDetail, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("name is required".into());
    }
    if input.min_quantity < 0.0 {
        return Err("min_quantity cannot be negative".into());
    }

    let pool = db::pool(&app).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let sku = non_empty(input.sku);
    let barcode = non_empty(input.barcode);
    let cfop_default = non_empty(input.cfop_default);
    let cst_csosn = non_empty(input.cst_csosn);
    let ncm = input.ncm.trim().to_string();
    let stock_unit = input.stock_unit.trim().to_string();
    let unit_label = input.unit_label.trim().to_string();
    let cost_price = cents_to_amount(input.cost_price_cents);
    let sell_price = cents_to_amount(input.sell_price_cents);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let result = sqlx::query(
        "UPDATE products
         SET sku = ?, barcode = ?, name = ?, ncm = ?, cfop_default = ?, cst_csosn = ?,
             stock_unit = ?, cost_price = ?, active = ?, updated_at = ?
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&sku)
    .bind(&barcode)
    .bind(&name)
    .bind(&ncm)
    .bind(&cfop_default)
    .bind(&cst_csosn)
    .bind(&stock_unit)
    .bind(cost_price)
    .bind(input.active)
    .bind(&now)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if result.rows_affected() == 0 {
        return Err(format!("product {id} not found"));
    }

    sqlx::query(
        "UPDATE product_units SET unit_label = ?, sell_price = ?, updated_at = ?
         WHERE product_id = ? AND is_default_sale = 1",
    )
    .bind(&unit_label)
    .bind(sell_price)
    .bind(&now)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("UPDATE stock_levels SET min_quantity = ?, updated_at = ? WHERE product_id = ?")
        .bind(input.min_quantity)
        .bind(&now)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    let product_payload = ProductEventPayload {
        id: &id,
        sku: &sku,
        barcode: &barcode,
        name: &name,
        ncm: &ncm,
        cfop_default: &cfop_default,
        cst_csosn: &cst_csosn,
        stock_unit: &stock_unit,
        cost_price,
        active: input.active,
        updated_at: &now,
    };
    outbox::record(&mut tx, "product", &id, &device_id, &product_payload).await?;
    emit_all_units_for_product(&mut tx, &id, &device_id).await?;

    let stock_level_payload = StockLevelEventPayload {
        product_id: &id,
        min_quantity: input.min_quantity,
        updated_at: &now,
    };
    outbox::record(&mut tx, "stock_level", &id, &device_id, &stock_level_payload).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    get_product(app, id).await
}

/// Lists a product's active selling units (plan section 4.2), default
/// unit first. Used by the sale screen's fast unit selector and by the
/// product form's unit-management section.
#[tauri::command]
pub async fn list_product_units<R: Runtime>(
    app: AppHandle<R>,
    product_id: String,
) -> Result<Vec<ProductUnitOption>, String> {
    let pool = db::pool(&app).await?;
    let rows = sqlx::query(
        "SELECT id, product_id, unit_label, factor_to_stock, sell_price, is_default_sale, active
         FROM product_units
         WHERE product_id = ? AND active = 1
         ORDER BY is_default_sale DESC, unit_label",
    )
    .bind(&product_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter()
        .map(row_to_unit_option)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Same as `list_product_units`, but includes inactive units — used by the
/// product form so an operator can reactivate a retired unit.
#[tauri::command]
pub async fn list_all_product_units<R: Runtime>(
    app: AppHandle<R>,
    product_id: String,
) -> Result<Vec<ProductUnitOption>, String> {
    let pool = db::pool(&app).await?;
    let rows = sqlx::query(
        "SELECT id, product_id, unit_label, factor_to_stock, sell_price, is_default_sale, active
         FROM product_units
         WHERE product_id = ?
         ORDER BY is_default_sale DESC, unit_label",
    )
    .bind(&product_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter()
        .map(row_to_unit_option)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Adds an alternative selling unit to an existing product (e.g. the
/// "50kg bag" on top of a "Loose kg" default — plan section 4.2).
#[tauri::command]
pub async fn create_product_unit<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
    product_id: String,
    unit_label: String,
    factor_to_stock: f64,
    sell_price_cents: i64,
    is_default_sale: bool,
) -> Result<ProductUnitOption, String> {
    let unit_label = unit_label.trim().to_string();
    if unit_label.is_empty() {
        return Err("unit label is required".into());
    }
    if factor_to_stock <= 0.0 {
        return Err("conversion factor must be greater than zero".into());
    }

    let pool = db::pool(&app).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM products WHERE id = ?")
        .bind(&product_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err(format!("product {product_id} not found"));
    }

    if is_default_sale {
        sqlx::query(
            "UPDATE product_units SET is_default_sale = 0, updated_at = ? WHERE product_id = ?",
        )
        .bind(&now)
        .bind(&product_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO product_units
            (id, product_id, unit_label, factor_to_stock, sell_price, is_default_sale, active,
             updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 1, ?)",
    )
    .bind(&id)
    .bind(&product_id)
    .bind(&unit_label)
    .bind(factor_to_stock)
    .bind(cents_to_amount(sell_price_cents))
    .bind(is_default_sale)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    emit_all_units_for_product(&mut tx, &product_id, &device_id).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(ProductUnitOption {
        id,
        product_id,
        unit_label,
        factor_to_stock,
        sell_price_cents,
        is_default_sale,
        active: true,
    })
}

/// Edits or (de)activates an existing selling unit. Guards two invariants
/// the sale screen and stock conversion both rely on: a product always has
/// at least one active unit, and at most one active default unit (setting
/// `is_default_sale` clears it on the product's other units).
#[tauri::command]
pub async fn update_product_unit<R: Runtime>(
    app: AppHandle<R>,
    device_id: String,
    id: String,
    unit_label: String,
    factor_to_stock: f64,
    sell_price_cents: i64,
    is_default_sale: bool,
    active: bool,
) -> Result<ProductUnitOption, String> {
    let unit_label = unit_label.trim().to_string();
    if unit_label.is_empty() {
        return Err("unit label is required".into());
    }
    if factor_to_stock <= 0.0 {
        return Err("conversion factor must be greater than zero".into());
    }

    let pool = db::pool(&app).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT product_id, is_default_sale FROM product_units WHERE id = ?")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let (product_id, was_default) = row.ok_or_else(|| format!("unit {id} not found"))?;
    let was_default = was_default != 0;

    if !active {
        if is_default_sale {
            return Err("a selling unit can't be both inactive and the default".into());
        }
        let (active_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM product_units WHERE product_id = ? AND active = 1 AND id != ?",
        )
        .bind(&product_id)
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        if active_count == 0 {
            return Err(AT_LEAST_ONE_ACTIVE_UNIT.into());
        }
        if was_default {
            return Err(
                "cannot deactivate the default selling unit — set another unit as default first"
                    .into(),
            );
        }
    }

    if is_default_sale {
        sqlx::query(
            "UPDATE product_units SET is_default_sale = 0, updated_at = ?
             WHERE product_id = ? AND id != ?",
        )
        .bind(&now)
        .bind(&product_id)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    } else if was_default {
        return Err(
            "cannot unset the default selling unit directly — set another unit as default instead"
                .into(),
        );
    }

    sqlx::query(
        "UPDATE product_units
         SET unit_label = ?, factor_to_stock = ?, sell_price = ?, is_default_sale = ?, active = ?,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(&unit_label)
    .bind(factor_to_stock)
    .bind(cents_to_amount(sell_price_cents))
    .bind(is_default_sale)
    .bind(active)
    .bind(&now)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    emit_all_units_for_product(&mut tx, &product_id, &device_id).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(ProductUnitOption {
        id,
        product_id,
        unit_label,
        factor_to_stock,
        sell_price_cents,
        is_default_sale,
        active,
    })
}

/// One row of the low-stock alert (plan section 4.6: "the dashboard and
/// product screen flag items where `stock_levels.quantity <= min_quantity`").
#[derive(Debug, Serialize)]
pub struct LowStockProduct {
    pub product_id: String,
    pub name: String,
    pub sku: Option<String>,
    pub stock_unit: String,
    pub quantity: f64,
    pub min_quantity: f64,
}

/// Active products at or below their configured `min_quantity` threshold,
/// most-depleted first (largest shortfall = `min_quantity - quantity`) —
/// used by the product screen's alert banner and the purchasing screen's
/// "what needs reordering" view.
#[tauri::command]
pub async fn list_low_stock_products<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<LowStockProduct>, String> {
    let pool = db::pool(&app).await?;
    let rows: Vec<(String, String, Option<String>, String, f64, f64)> = sqlx::query_as(
        "SELECT p.id, p.name, p.sku, p.stock_unit, sl.quantity, sl.min_quantity
         FROM products p
         JOIN stock_levels sl ON sl.product_id = p.id
         WHERE p.active = 1 AND sl.min_quantity > 0 AND sl.quantity <= sl.min_quantity
         ORDER BY (sl.min_quantity - sl.quantity) DESC",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(product_id, name, sku, stock_unit, quantity, min_quantity)| LowStockProduct {
            product_id,
            name,
            sku,
            stock_unit,
            quantity,
            min_quantity,
        })
        .collect())
}
