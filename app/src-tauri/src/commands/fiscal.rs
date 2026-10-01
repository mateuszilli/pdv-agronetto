//! Fiscal document commands: emission status, manual re-emit, manual
//! cancellation retry, receipt printing (plan section 5, plus the
//! printing pipeline from section 6).
//!
//! `print_receipt` is also what plan section 5.2 calls "reprint" — since
//! it always renders from the fiscal document's *current* state, calling
//! it again after a queued sale gets authorized in the background simply
//! prints the real DANFCE instead of the provisional slip, satisfying the
//! "DANFCE reprint tested" checklist item (plan section 9) with the same
//! command used for the original print. It's also what shows a cancelled
//! NFC-e (plan section 4.5's whole-document cancellation, triggered from
//! `commands::returns::create_return` — see `fiscal_client`'s doc
//! comment) as such on reprint.

use serde::Serialize;
use sqlx::{sqlite::SqliteRow, Row};
use tauri::{AppHandle, Runtime};

use crate::db;
use crate::fiscal_client;
use crate::money::amount_to_cents;
use crate::printing::{self, ReceiptBuilder};

#[derive(Debug, Serialize)]
pub struct FiscalDocumentView {
    pub id: String,
    pub sale_id: String,
    pub status: String,
    pub access_key: Option<String>,
    pub xml_url: Option<String>,
    pub danfe_url: Option<String>,
    pub rejection_reason: Option<String>,
    /// Set when a return brought this sale to fully `returned` but the
    /// resulting cancellation attempt didn't succeed (past the 30-minute
    /// window, rejected by SEFAZ, or a connectivity failure) — see
    /// `fiscal_client::try_cancel_for_return`. `status` stays `authorized`
    /// in that case; this is purely an informational trail. `None` once
    /// cancellation actually succeeds (`status` becomes `cancelled`
    /// instead) or if cancellation was never relevant to this document.
    pub cancellation_note: Option<String>,
    pub attempts: i64,
    pub created_at: String,
    pub updated_at: String,
}

const FISCAL_DOCUMENT_SELECT: &str = "SELECT id, sale_id, status, access_key, xml_url, danfe_url,
    rejection_reason, cancellation_note, attempts, created_at, updated_at FROM fiscal_documents";

fn row_to_view(row: &SqliteRow) -> Result<FiscalDocumentView, sqlx::Error> {
    Ok(FiscalDocumentView {
        id: row.try_get("id")?,
        sale_id: row.try_get("sale_id")?,
        status: row.try_get("status")?,
        access_key: row.try_get("access_key")?,
        xml_url: row.try_get("xml_url")?,
        danfe_url: row.try_get("danfe_url")?,
        rejection_reason: row.try_get("rejection_reason")?,
        cancellation_note: row.try_get("cancellation_note")?,
        attempts: row.try_get("attempts")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// `None` means this sale predates fiscal integration or its document row
/// is otherwise missing — callers treat that the same as "not fiscal yet".
#[tauri::command]
pub async fn get_fiscal_status<R: Runtime>(
    app: AppHandle<R>,
    sale_id: String,
) -> Result<Option<FiscalDocumentView>, String> {
    let pool = db::pool(&app).await?;
    let row = sqlx::query(&format!(
        "{FISCAL_DOCUMENT_SELECT} WHERE sale_id = ? ORDER BY created_at DESC LIMIT 1"
    ))
    .bind(&sale_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;

    row.as_ref()
        .map(row_to_view)
        .transpose()
        .map_err(|e| e.to_string())
}

/// Manual retry for a sale whose return already brought it to fully
/// `returned` but whose automatic cancellation attempt
/// (`commands::returns::create_return`) didn't succeed — see
/// `fiscal_client::try_cancel_for_return`'s doc comment on why there's no
/// background retry queue for this the way emission has one. As long as
/// the 30-minute window from the original authorization hasn't closed,
/// the operator can retry sooner than waiting it out would otherwise
/// require (e.g. right after a transient connectivity blip). A no-op
/// (`NotApplicable`) if there's nothing left to cancel — already
/// cancelled, or the document was never authorized in the first place.
#[tauri::command]
pub async fn retry_fiscal_cancellation<R: Runtime>(
    app: AppHandle<R>,
    sale_id: String,
) -> Result<fiscal_client::CancellationStatus, String> {
    let pool = db::pool(&app).await?;
    fiscal_client::try_cancel_for_return(&pool, &sale_id, None).await
}

/// Manual re-emit for a `rejected` document (plan section 5.1: "a visible
/// alert lets the operator fix the product registration and re-emit
/// manually") — the operator fixes whatever the rejection called out
/// (e.g. an invalid NCM), then retries without waiting for the
/// background worker's next tick. Works just as well for a `pending`
/// document if the operator wants to force an immediate retry instead of
/// waiting out the backoff.
#[tauri::command]
pub async fn retry_fiscal_emission<R: Runtime>(
    app: AppHandle<R>,
    fiscal_document_id: String,
) -> Result<FiscalDocumentView, String> {
    let pool = db::pool(&app).await?;
    fiscal_client::try_emit(&pool, &fiscal_document_id).await?;

    let row = sqlx::query(&format!("{FISCAL_DOCUMENT_SELECT} WHERE id = ?"))
        .bind(&fiscal_document_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "fiscal document not found".to_string())?;
    row_to_view(&row).map_err(|e| e.to_string())
}

struct SaleItemRow {
    product_name: String,
    unit_label: String,
    quantity: f64,
    unit_price_cents: i64,
    line_total_cents: i64,
}

struct SalePaymentRow {
    method: String,
    amount_cents: i64,
}

/// Renders and sends the receipt for a sale to the configured printer
/// (`PDV_PRINTER` — see `printing::build_printer`), choosing content
/// based on the sale's *current* fiscal status: the real DANFCE once
/// authorized, a clearly-labeled provisional slip while still queued
/// (plan section 5.2's contingency mode), or a rejected-but-valid notice
/// if the provider turned the document down. The cash drawer only kicks
/// when the sale included a cash payment (plan section 1's "kicked via
/// printer" — there's nothing to make change for on a card/pix/fiado
/// sale).
#[tauri::command]
pub async fn print_receipt<R: Runtime>(app: AppHandle<R>, sale_id: String) -> Result<(), String> {
    let pool = db::pool(&app).await?;

    let sale_row: Option<(f64, f64, f64, String)> = sqlx::query_as(
        "SELECT subtotal, discount_total, total, created_at FROM sales WHERE id = ?",
    )
    .bind(&sale_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let (subtotal, discount_total, total, created_at) =
        sale_row.ok_or_else(|| "sale not found".to_string())?;

    let item_rows = sqlx::query(
        "SELECT p.name AS product_name, pu.unit_label, si.quantity, si.unit_price, si.line_total
         FROM sale_items si
         JOIN products p ON p.id = si.product_id
         JOIN product_units pu ON pu.id = si.unit_id
         WHERE si.sale_id = ?",
    )
    .bind(&sale_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let items: Vec<SaleItemRow> = item_rows
        .iter()
        .map(|row| {
            let unit_price: f64 = row.try_get("unit_price")?;
            let line_total: f64 = row.try_get("line_total")?;
            Ok(SaleItemRow {
                product_name: row.try_get("product_name")?,
                unit_label: row.try_get("unit_label")?,
                quantity: row.try_get("quantity")?,
                unit_price_cents: amount_to_cents(unit_price),
                line_total_cents: amount_to_cents(line_total),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|e| e.to_string())?;

    let payment_rows = sqlx::query("SELECT method, amount FROM sale_payments WHERE sale_id = ?")
        .bind(&sale_id)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;
    let payments: Vec<SalePaymentRow> = payment_rows
        .iter()
        .map(|row| {
            let amount: f64 = row.try_get("amount")?;
            Ok(SalePaymentRow {
                method: row.try_get("method")?,
                amount_cents: amount_to_cents(amount),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|e| e.to_string())?;
    let open_drawer = payments.iter().any(|p| p.method == "cash");

    let fiscal_row = sqlx::query(
        "SELECT status, access_key, danfe_url, qrcode_url, rejection_reason
         FROM fiscal_documents WHERE sale_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&sale_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut builder = ReceiptBuilder::new();

    builder = match &fiscal_row {
        Some(row) => {
            let status: String = row.try_get("status").map_err(|e| e.to_string())?;
            match status.as_str() {
                "authorized" => {
                    let access_key: Option<String> =
                        row.try_get("access_key").map_err(|e| e.to_string())?;
                    let qrcode_url: Option<String> =
                        row.try_get("qrcode_url").map_err(|e| e.to_string())?;
                    let danfe_url: Option<String> =
                        row.try_get("danfe_url").map_err(|e| e.to_string())?;
                    let mut b = builder
                        .header("NFC-e")
                        .centered("Documento Auxiliar da Nota Fiscal")
                        .centered("de Consumidor Eletronica")
                        .feed()
                        .line(format!(
                            "Chave de acesso: {}",
                            access_key.as_deref().unwrap_or("-")
                        ));
                    if let Some(url) = qrcode_url.or(danfe_url) {
                        b = b.qrcode(url);
                    }
                    b.feed()
                }
                "rejected" => {
                    let reason: Option<String> =
                        row.try_get("rejection_reason").map_err(|e| e.to_string())?;
                    builder
                        .header("VENDA REGISTRADA")
                        .bold_line("EMISSAO FISCAL REJEITADA")
                        .line(reason.unwrap_or_else(|| "motivo nao informado".to_string()))
                        .line("Corrija o cadastro do produto e reemita.")
                        .feed()
                }
                "cancelled" => {
                    let access_key: Option<String> =
                        row.try_get("access_key").map_err(|e| e.to_string())?;
                    builder
                        .header("NFC-e CANCELADA")
                        .bold_line("Esta nota fiscal foi cancelada.")
                        .line(format!(
                            "Chave de acesso: {}",
                            access_key.as_deref().unwrap_or("-")
                        ))
                        .centered("Nao e documento fiscal valido.")
                        .feed()
                }
                _ => provisional_header(builder),
            }
        }
        None => provisional_header(builder),
    };

    builder = builder.line(format!("Data: {created_at}")).feed();

    for item in &items {
        builder = builder
            .line(item.product_name.clone())
            .line(format!(
                "  {} {} x {:.2}  = {:.2}",
                format_quantity(item.quantity),
                item.unit_label,
                item.unit_price_cents as f64 / 100.0,
                item.line_total_cents as f64 / 100.0
            ));
    }

    builder = builder
        .feed()
        .line(format!("Subtotal: {:.2}", subtotal))
        .line(format!("Desconto: {:.2}", discount_total))
        .bold_line(format!("Total: {:.2}", total))
        .feed();

    for payment in &payments {
        builder = builder.line(format!(
            "{}: {:.2}",
            payment_method_label(&payment.method),
            payment.amount_cents as f64 / 100.0
        ));
    }

    builder = builder.feed().centered("Obrigado pela preferencia!").feed();

    let receipt = builder.build(open_drawer);
    let mut printer = printing::build_printer()?;
    printer.print(&receipt)
}

fn provisional_header(builder: ReceiptBuilder) -> ReceiptBuilder {
    builder
        .header("COMPROVANTE PROVISORIO")
        .bold_line("AGUARDANDO EMISSAO FISCAL")
        .centered("Nao e documento fiscal")
        .feed()
}

fn format_quantity(quantity: f64) -> String {
    if quantity.fract() == 0.0 {
        format!("{quantity:.0}")
    } else {
        format!("{quantity:.3}")
    }
}

fn payment_method_label(method: &str) -> &'static str {
    match method {
        "cash" => "Dinheiro",
        "debit" => "Cartao de debito",
        "credit" => "Cartao de credito",
        "pix" => "Pix",
        "boleto" => "Boleto",
        "store_credit" => "Fiado",
        _ => "Outro",
    }
}
