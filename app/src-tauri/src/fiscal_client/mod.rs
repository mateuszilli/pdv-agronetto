//! HTTP client for the fiscal provider API (Focus NFe) — plan section 5.
//!
//! Submits `fiscal_queue` payloads for NFC-e emission and applies the
//! result to `fiscal_documents`. NFC-e emission is synchronous end to end
//! (the provider's own docs: "todos os processos envolvendo NFC-e são
//! síncronos") — a single POST returns `autorizado`/`erro_autorizacao`
//! directly, so there is no separate poll step in the common case.
//!
//! Payload field names are dictated by the provider's contract
//! (Portuguese keys) and must not be translated — see the note in
//! migrations/0004_fiscal.sql. Contract confirmed against
//! <https://doc.focusnfe.com.br/reference/emitir_nfce>,
//! <https://doc.focusnfe.com.br/reference/ambiente> and
//! <https://doc.focusnfe.com.br/reference/autenticacao> — re-verify
//! against the live docs during homologation testing (plan section 9)
//! before going to production, since this was built without a real
//! sandbox account to test end to end against (see README).
//!
//! Contingency mode (plan section 5.2) isn't a separate code path here:
//! "no connectivity at the moment of sale" is simply whatever
//! [`EmitError::Connectivity`] looks like when `try_emit` is called
//! inline from `commands::sales::finalize_sale` — the row just stays
//! queued and `spawn_worker` retries it with backoff until the sale
//! finalize's original best-effort attempt is superseded by a later
//! success.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite, Transaction};
use tauri::{AppHandle, Runtime};

use crate::config;
use crate::db;
use crate::outbox;

const WORKER_INTERVAL: Duration = Duration::from_secs(20);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const QUEUE_BATCH_SIZE: i64 = 20;
const BASE_BACKOFF_SECS: i64 = 30;
const MAX_BACKOFF_SECS: i64 = 1800;

// --- Request payload ---
//
// Field names and codes below follow Focus NFe's documented NFC-e
// contract (see module doc). `data_emissao` is intentionally NOT part of
// what gets persisted in `fiscal_queue.payload_json` at sale time — the
// provider rejects an emission dated more than ~5 minutes in the past,
// which a queued/retried document routinely would be. `emit` stamps it
// fresh on every attempt instead (see below).

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NfceItem {
    pub numero_item: i32,
    pub codigo_produto: String,
    pub descricao: String,
    pub codigo_ncm: String,
    pub cfop: String,
    pub unidade_comercial: String,
    pub quantidade_comercial: f64,
    pub valor_unitario_comercial: f64,
    pub valor_bruto: f64,
    pub valor_desconto: f64,
    pub unidade_tributavel: String,
    pub quantidade_tributavel: f64,
    pub valor_unitario_tributavel: f64,
    /// Origin of the goods (0 = nacional). The plan's product schema
    /// (section 3.1) has no origin column, and every pilot product is
    /// assumed domestically sourced for the MVP — flagged here rather
    /// than silently guessed, since it's the kind of fiscal detail
    /// homologation testing (plan section 9) needs to confirm per
    /// product before production use.
    pub icms_origem: String,
    /// Either a CST or a CSOSN code, passed through verbatim from
    /// `products.cst_csosn` — which one it is depends on the *product's*
    /// own registration, not a single store-wide tax regime, so this
    /// layer doesn't need to branch on Simples Nacional vs. regular
    /// regime (per the product owner's direction).
    pub icms_situacao_tributaria: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NfcePayment {
    pub forma_pagamento: String,
    pub valor_pagamento: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NfcePayload {
    pub cnpj_emitente: String,
    pub data_emissao: String,
    pub presenca_comprador: u8,
    pub modalidade_frete: u8,
    pub local_destino: u8,
    pub natureza_operacao: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpf_destinatario: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cnpj_destinatario: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nome_destinatario: Option<String>,
    pub items: Vec<NfceItem>,
    pub formas_pagamento: Vec<NfcePayment>,
}

/// One sale line's fiscal-relevant fields, already resolved from
/// `products`/`product_units` by the caller (`commands::sales`) — this
/// module only shapes them into the provider's contract, it doesn't
/// query the local database for catalog data.
pub struct FiscalItemInput {
    pub product_name: String,
    pub product_sku_or_id: String,
    pub ncm: String,
    pub cfop: String,
    pub cst_csosn: String,
    pub unit_label: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub line_total: f64,
}

pub struct FiscalPaymentInput {
    pub method: String,
    pub amount: f64,
}

pub struct FiscalBuyerInput {
    pub doc_type: String,
    pub doc_number: String,
    pub name: String,
}

/// SEFAZ's standard payment-method table (used across NFC-e/NF-e).
/// `store_credit` ("fiado") has no real-time-payment code of its own —
/// SEFAZ never sees the running tab, only that the sale happened — so it
/// maps to "99 Outros", the standard fallback for non-listed methods.
fn payment_method_code(method: &str) -> &'static str {
    match method {
        "cash" => "01",
        "credit" => "03",
        "debit" => "04",
        "boleto" => "15",
        "pix" => "17",
        _ => "99",
    }
}

/// Builds the request payload from already-resolved sale data. A
/// sale-level discount (the only kind the MVP sales screen supports —
/// see `commands::sales`'s doc comment) is prorated across items by
/// gross value so `sum(valor_bruto) - sum(valor_desconto)` reconciles
/// with the sale's actual total; item-level discounts stay 0 until a
/// line-discount UI exists, matching `sale_items.discount` today.
pub fn build_payload(
    issuer_cnpj: String,
    items: &[FiscalItemInput],
    payments: &[FiscalPaymentInput],
    buyer: Option<&FiscalBuyerInput>,
    discount_total: f64,
) -> NfcePayload {
    let gross_total_cents: i64 = items
        .iter()
        .map(|i| (i.line_total * 100.0).round() as i64)
        .sum();
    let discount_total_cents = (discount_total * 100.0).round() as i64;

    let mut allocated_cents = 0i64;
    let mut nfce_items = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let line_total_cents = (item.line_total * 100.0).round() as i64;
        let is_last = idx == items.len() - 1;
        let item_discount_cents = if is_last {
            discount_total_cents - allocated_cents
        } else if gross_total_cents > 0 {
            discount_total_cents * line_total_cents / gross_total_cents
        } else {
            0
        };
        allocated_cents += item_discount_cents;

        nfce_items.push(NfceItem {
            numero_item: (idx + 1) as i32,
            codigo_produto: item.product_sku_or_id.clone(),
            descricao: item.product_name.clone(),
            codigo_ncm: item.ncm.clone(),
            cfop: item.cfop.clone(),
            unidade_comercial: item.unit_label.clone(),
            quantidade_comercial: item.quantity,
            valor_unitario_comercial: item.unit_price,
            valor_bruto: item.line_total,
            valor_desconto: item_discount_cents as f64 / 100.0,
            unidade_tributavel: item.unit_label.clone(),
            quantidade_tributavel: item.quantity,
            valor_unitario_tributavel: item.unit_price,
            icms_origem: "0".to_string(),
            icms_situacao_tributaria: item.cst_csosn.clone(),
        });
    }

    let formas_pagamento = payments
        .iter()
        .map(|p| NfcePayment {
            forma_pagamento: payment_method_code(&p.method).to_string(),
            valor_pagamento: p.amount,
        })
        .collect();

    NfcePayload {
        cnpj_emitente: issuer_cnpj,
        // Overwritten with the real send time on every attempt — see
        // `emit`. Set here too so a payload built and inspected before
        // its first attempt is still a well-formed request body.
        data_emissao: chrono::Utc::now().to_rfc3339(),
        presenca_comprador: 1,
        // MVP defaults: counter sale, no shipping, same-state buyer.
        // `local_destino` in particular assumes the pilot store and its
        // customers share a state — the plan doesn't track customer
        // address structurally enough to compute this per sale (section
        // 3.2's `address_json` is unstructured), so this needs revisiting
        // if the pilot serves out-of-state buyers.
        modalidade_frete: 9,
        local_destino: 1,
        natureza_operacao: "Venda ao consumidor".to_string(),
        cpf_destinatario: buyer
            .filter(|b| b.doc_type == "CPF")
            .map(|b| b.doc_number.clone()),
        cnpj_destinatario: buyer
            .filter(|b| b.doc_type == "CNPJ")
            .map(|b| b.doc_number.clone()),
        nome_destinatario: buyer.map(|b| b.name.clone()),
        items: nfce_items,
        formas_pagamento,
    }
}

// --- Provider response ---

#[derive(Debug, Deserialize)]
struct NfceResponse {
    status: String,
    #[serde(default)]
    mensagem_sefaz: Option<String>,
    #[serde(default)]
    chave_nfe: Option<String>,
    #[serde(default)]
    caminho_xml_nota_fiscal: Option<String>,
    #[serde(default)]
    caminho_danfe: Option<String>,
    #[serde(default)]
    qrcode_url: Option<String>,
}

pub enum EmitOutcome {
    Authorized {
        access_key: String,
        xml_url: Option<String>,
        danfe_url: Option<String>,
        qrcode_url: Option<String>,
    },
    Rejected {
        reason: String,
    },
    /// `processando_autorizacao` — the provider's docs describe NFC-e as
    /// always synchronous, so this is expected to be rare (e.g. under
    /// provider-side load); treated the same as a connectivity failure
    /// by callers (retry with backoff).
    Pending,
}

pub enum EmitError {
    /// No `FISCAL_PROVIDER_TOKEN` configured — the pilot store hasn't
    /// been onboarded with the provider yet (see
    /// `config::fiscal_provider_token`'s doc comment).
    NotConfigured,
    Connectivity(String),
}

/// Sends one emission attempt. Never mutates the local database — pure
/// HTTP in, outcome out — so it can be unit-tested independently of
/// `try_emit`'s bookkeeping.
pub async fn emit(payload: &NfcePayload, ref_id: &str) -> Result<EmitOutcome, EmitError> {
    let token = config::fiscal_provider_token().ok_or(EmitError::NotConfigured)?;

    let mut request_body = payload.clone();
    request_body.data_emissao = chrono::Utc::now().to_rfc3339();

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| EmitError::Connectivity(e.to_string()))?;
    let url = format!("{}/v2/nfce?ref={}", config::fiscal_provider_base_url(), ref_id);

    let response = client
        .post(&url)
        .basic_auth(&token, Some(""))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| EmitError::Connectivity(format!("failed to reach {url}: {e}")))?;

    let status_code = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| EmitError::Connectivity(e.to_string()))?;

    // A validation rejection (erro_autorizacao/denegado) is a normal,
    // meaningful outcome even when the provider reports it via a non-2xx
    // HTTP status, so it's always worth trying to parse the body before
    // treating a non-2xx response as a bare connectivity failure.
    let parsed: NfceResponse = match serde_json::from_str(&body) {
        Ok(parsed) => parsed,
        Err(_) if !status_code.is_success() => {
            return Err(EmitError::Connectivity(format!(
                "provider returned {status_code}: {body}"
            )));
        }
        Err(e) => return Err(EmitError::Connectivity(e.to_string())),
    };

    match parsed.status.as_str() {
        "autorizado" => Ok(EmitOutcome::Authorized {
            access_key: parsed.chave_nfe.ok_or_else(|| {
                EmitError::Connectivity("authorized response missing chave_nfe".to_string())
            })?,
            xml_url: parsed.caminho_xml_nota_fiscal,
            danfe_url: parsed.caminho_danfe,
            qrcode_url: parsed.qrcode_url,
        }),
        "erro_autorizacao" | "denegado" => Ok(EmitOutcome::Rejected {
            reason: parsed
                .mensagem_sefaz
                .unwrap_or_else(|| "rejected by SEFAZ (no message provided)".to_string()),
        }),
        "processando_autorizacao" => Ok(EmitOutcome::Pending),
        other => Err(EmitError::Connectivity(format!(
            "unexpected status '{other}' from provider"
        ))),
    }
}

// --- Outbox sync (Phase 9) ---
//
// fiscal_documents was deliberately left unsynced in Phase 6 — fiscal
// status is a local-device, provider-facing concern per the plan's own
// architecture (section 1.1); see the README's "Known deviations". Phase
// 9 reverses that for this table specifically: the plan's back-office
// alert for a document stuck pending past the contingency window
// (section 5.2) needs central visibility, so every status transition now
// also records a `fiscal_document` outbox event, applied by the
// backend's `internal/sync.applyFiscalDocument`. `fiscal_queue` (retry
// bookkeeping — next_retry_at, backoff state) stays local-only; it has
// no back-office use.

#[derive(Debug, sqlx::FromRow)]
struct FiscalDocumentRow {
    sale_id: String,
    doc_type: String,
    status: String,
    access_key: Option<String>,
    xml_url: Option<String>,
    danfe_url: Option<String>,
    rejection_reason: Option<String>,
    cancellation_note: Option<String>,
    attempts: i64,
    created_at: String,
    updated_at: String,
    device_id: String,
}

#[derive(Debug, Serialize)]
struct FiscalDocumentEventPayload {
    id: String,
    sale_id: String,
    doc_type: String,
    status: String,
    access_key: Option<String>,
    xml_url: Option<String>,
    danfe_url: Option<String>,
    rejection_reason: Option<String>,
    cancellation_note: Option<String>,
    attempts: i64,
    created_at: String,
    updated_at: String,
}

/// Records a `fiscal_document` outbox event mirroring the row's *current*
/// state. Called right after every insert/update to `fiscal_documents`,
/// inside the same transaction, so the event and the row it describes
/// always commit together (the invariant `outbox::record`'s own doc
/// comment describes). `device_id` is read off the owning sale rather
/// than threaded through every caller: the fiscal worker (`run_once`) has
/// no device identity of its own, but every `fiscal_documents` row
/// belongs to exactly one `sales` row that already has one.
pub(crate) async fn record_fiscal_document_event(
    tx: &mut Transaction<'_, Sqlite>,
    fiscal_document_id: &str,
) -> Result<(), String> {
    let row: FiscalDocumentRow = sqlx::query_as(
        "SELECT s.id as sale_id, fd.doc_type, fd.status, fd.access_key, fd.xml_url,
                fd.danfe_url, fd.rejection_reason, fd.cancellation_note, fd.attempts,
                fd.created_at, fd.updated_at, s.device_id
           FROM fiscal_documents fd
           JOIN sales s ON s.id = fd.sale_id
          WHERE fd.id = ?",
    )
    .bind(fiscal_document_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    let payload = FiscalDocumentEventPayload {
        id: fiscal_document_id.to_string(),
        sale_id: row.sale_id,
        doc_type: row.doc_type,
        status: row.status,
        access_key: row.access_key,
        xml_url: row.xml_url,
        danfe_url: row.danfe_url,
        rejection_reason: row.rejection_reason,
        cancellation_note: row.cancellation_note,
        attempts: row.attempts,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };
    outbox::record(tx, "fiscal_document", fiscal_document_id, &row.device_id, &payload).await
}

// --- Queue processing ---

fn backoff_secs(retry_count: i64) -> i64 {
    let secs = BASE_BACKOFF_SECS.saturating_mul(1i64 << retry_count.clamp(0, 6));
    secs.min(MAX_BACKOFF_SECS)
}

async fn mark_authorized(
    pool: &Pool<Sqlite>,
    doc_id: &str,
    queue_id: &str,
    access_key: &str,
    xml_url: Option<String>,
    danfe_url: Option<String>,
    qrcode_url: Option<String>,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE fiscal_documents
            SET status = 'authorized', access_key = ?, xml_url = ?, danfe_url = ?,
                qrcode_url = ?, attempts = attempts + 1, updated_at = ?
          WHERE id = ?",
    )
    .bind(access_key)
    .bind(&xml_url)
    .bind(&danfe_url)
    .bind(&qrcode_url)
    .bind(&now)
    .bind(doc_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM fiscal_queue WHERE id = ?")
        .bind(queue_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    record_fiscal_document_event(&mut tx, doc_id).await?;

    tx.commit().await.map_err(|e| e.to_string())
}

async fn mark_rejected(
    pool: &Pool<Sqlite>,
    doc_id: &str,
    queue_id: &str,
    reason: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE fiscal_documents
            SET status = 'rejected', rejection_reason = ?, attempts = attempts + 1, updated_at = ?
          WHERE id = ?",
    )
    .bind(reason)
    .bind(&now)
    .bind(doc_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM fiscal_queue WHERE id = ?")
        .bind(queue_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    record_fiscal_document_event(&mut tx, doc_id).await?;

    tx.commit().await.map_err(|e| e.to_string())
}

async fn reschedule(
    pool: &Pool<Sqlite>,
    doc_id: &str,
    queue_id: &str,
    retry_count: i64,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    let next_retry_at = (now + chrono::Duration::seconds(backoff_secs(retry_count))).to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query("UPDATE fiscal_documents SET attempts = attempts + 1, updated_at = ? WHERE id = ?")
        .bind(now.to_rfc3339())
        .bind(doc_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("UPDATE fiscal_queue SET retry_count = retry_count + 1, next_retry_at = ? WHERE id = ?")
        .bind(&next_retry_at)
        .bind(queue_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())
}

/// Attempts emission for one already-queued fiscal document and applies
/// the outcome to `fiscal_documents`/`fiscal_queue`. Used both by the
/// background worker (`run_once`, for due rows) and for on-demand
/// attempts (`commands::sales::finalize_sale`'s best-effort first try,
/// `commands::fiscal::retry_fiscal_emission`'s manual re-emit) — those
/// two only differ in *when* they call this, not in what it does.
///
/// Returns a short status string (`"authorized"`, `"rejected"`,
/// `"pending"`) rather than `EmitOutcome` directly, since callers past
/// the Tauri command boundary need a `Serialize`-friendly result anyway.
pub async fn try_emit(pool: &Pool<Sqlite>, fiscal_document_id: &str) -> Result<String, String> {
    let queue_row: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT id, payload_json, retry_count FROM fiscal_queue WHERE fiscal_document_id = ?",
    )
    .bind(fiscal_document_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let Some((queue_id, payload_json, retry_count)) = queue_row else {
        return Err("fiscal document has no pending queue entry".to_string());
    };
    let payload: NfcePayload = serde_json::from_str(&payload_json).map_err(|e| e.to_string())?;

    match emit(&payload, fiscal_document_id).await {
        Ok(EmitOutcome::Authorized {
            access_key,
            xml_url,
            danfe_url,
            qrcode_url,
        }) => {
            mark_authorized(
                pool,
                fiscal_document_id,
                &queue_id,
                &access_key,
                xml_url,
                danfe_url,
                qrcode_url,
            )
            .await?;
            Ok("authorized".to_string())
        }
        Ok(EmitOutcome::Rejected { reason }) => {
            mark_rejected(pool, fiscal_document_id, &queue_id, &reason).await?;
            Ok("rejected".to_string())
        }
        Ok(EmitOutcome::Pending) => {
            reschedule(pool, fiscal_document_id, &queue_id, retry_count).await?;
            Ok("pending".to_string())
        }
        Err(EmitError::NotConfigured) => Ok("pending".to_string()),
        Err(EmitError::Connectivity(msg)) => {
            eprintln!("fiscal: emission attempt for {fiscal_document_id} failed: {msg}");
            reschedule(pool, fiscal_document_id, &queue_id, retry_count).await?;
            Ok("pending".to_string())
        }
    }
}

async fn run_once<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let pool = db::pool(app).await?;

    if config::fiscal_provider_token().is_none() {
        let pending: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM fiscal_queue")
            .fetch_one(&pool)
            .await
            .map_err(|e| e.to_string())?;
        if pending.0 > 0 {
            println!(
                "fiscal: no provider token configured, skipping emission \
                 ({} document(s) waiting in the queue)",
                pending.0
            );
        }
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let due: Vec<(String,)> = sqlx::query_as(
        "SELECT fiscal_document_id FROM fiscal_queue
          WHERE next_retry_at IS NULL OR next_retry_at <= ?
          ORDER BY next_retry_at
          LIMIT ?",
    )
    .bind(&now)
    .bind(QUEUE_BATCH_SIZE)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    for (doc_id,) in due {
        if let Err(e) = try_emit(&pool, &doc_id).await {
            eprintln!("fiscal: failed to process {doc_id}: {e}");
        }
    }

    Ok(())
}

/// Spawns the background emission-retry loop. Call once from `lib.rs`'s
/// `.setup()`, alongside `sync::spawn_worker`.
pub fn spawn_worker<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = run_once(&app).await {
                eprintln!("fiscal: {e}");
            }
            tokio::time::sleep(WORKER_INTERVAL).await;
        }
    });
}

// --- Cancellation (Phase 7 — plan section 4.5) ---
//
// Focus NFe's NFC-e contract has no separate "reversal note" the way a
// formal NF-e devolução would (see `commands::returns`'s module doc for
// the full reasoning) — the only fiscal remedy it offers at all is
// cancelling the *entire* original document: `DELETE /v2/nfce/{ref}`
// (same `ref` used at emission — Focus's own docs confirm the reference
// is reused, not a separate provider id), with a required
// `justificativa` (15-255 chars), and only within 30 minutes of
// authorization (confirmed against
// <https://doc.focusnfe.com.br/reference/cancelar_nfce> — tighter than
// this module's emission-side doc comment originally assumed "same-day";
// re-verify during homologation testing like the rest of this client, per
// the README).
//
// `try_cancel_for_return` is therefore scoped to exactly one case: a
// return that brings its sale to fully `returned` (plan section 4.5's
// per-item returns can get there either in one call or cumulatively
// across several `create_return` calls — either way, whichever call
// tips the balance is the one that triggers this). A partial return never
// calls this — there's nothing in Focus NFe's contract that could
// represent "half cancelled" (an explicit product decision, not an
// oversight; partial returns stay bookkeeping-only, exactly as before).
//
// This is a single best-effort attempt at return time, mirroring
// `try_emit`'s inline call from `finalize_sale` — not a retried
// background queue like emission's `fiscal_queue`: a queued retry has
// little use once most of the 30-minute window has already elapsed, and
// `commands::fiscal::retry_fiscal_cancellation` covers an operator
// wanting to retry sooner after a transient failure. Per an explicit
// product decision, a return is never blocked on the outcome: on any
// non-`Cancelled` result, `fiscal_documents.status` simply stays
// `authorized` with `cancellation_note` recording why, surfaced on the
// Returns screen and synced to the back office through the same path
// Phase 9 already wired up for this table.

/// How long after authorization Focus NFe still accepts a cancellation.
const CANCELLATION_WINDOW_MINUTES: i64 = 30;
/// Stop attempting this many minutes early. A request that lands right at
/// the real boundary due to ordinary network/processing latency would
/// just come back rejected anyway, so there's no point spending the round
/// trip — and it keeps the "why we skipped" message honest.
const CANCELLATION_SAFETY_MARGIN_MINUTES: i64 = 5;

#[derive(Debug, Deserialize)]
struct NfceCancelResponse {
    status: String,
    #[serde(default)]
    mensagem_sefaz: Option<String>,
}

pub enum CancelOutcome {
    Cancelled,
    Rejected { reason: String },
}

pub enum CancelError {
    NotConfigured,
    Connectivity(String),
}

/// Sends one cancellation attempt. Mirrors `emit`'s shape: pure HTTP in,
/// outcome out, no database access, so it can be reasoned about (and
/// eventually tested) independently of `try_cancel_for_return`'s
/// bookkeeping.
pub async fn cancel(ref_id: &str, justificativa: &str) -> Result<CancelOutcome, CancelError> {
    let token = config::fiscal_provider_token().ok_or(CancelError::NotConfigured)?;

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| CancelError::Connectivity(e.to_string()))?;
    let url = format!("{}/v2/nfce/{}", config::fiscal_provider_base_url(), ref_id);

    let response = client
        .delete(&url)
        .basic_auth(&token, Some(""))
        .json(&serde_json::json!({ "justificativa": justificativa }))
        .send()
        .await
        .map_err(|e| CancelError::Connectivity(format!("failed to reach {url}: {e}")))?;

    let status_code = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| CancelError::Connectivity(e.to_string()))?;

    // Same reasoning as `emit`: a validation rejection is a normal,
    // meaningful outcome even when reported via a non-2xx HTTP status, so
    // it's always worth trying to parse the body first.
    let parsed: NfceCancelResponse = match serde_json::from_str(&body) {
        Ok(parsed) => parsed,
        Err(_) if !status_code.is_success() => {
            return Err(CancelError::Connectivity(format!(
                "provider returned {status_code}: {body}"
            )));
        }
        Err(e) => return Err(CancelError::Connectivity(e.to_string())),
    };

    match parsed.status.as_str() {
        "cancelado" => Ok(CancelOutcome::Cancelled),
        "erro_cancelamento" => Ok(CancelOutcome::Rejected {
            reason: parsed
                .mensagem_sefaz
                .unwrap_or_else(|| "cancellation rejected by SEFAZ (no message provided)".to_string()),
        }),
        other => Err(CancelError::Connectivity(format!(
            "unexpected status '{other}' from provider"
        ))),
    }
}

async fn set_cancellation_note(pool: &Pool<Sqlite>, doc_id: &str, note: &str) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query("UPDATE fiscal_documents SET cancellation_note = ?, updated_at = ? WHERE id = ?")
        .bind(note)
        .bind(&now)
        .bind(doc_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    record_fiscal_document_event(&mut tx, doc_id).await?;
    tx.commit().await.map_err(|e| e.to_string())
}

async fn mark_cancelled(pool: &Pool<Sqlite>, doc_id: &str) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE fiscal_documents
            SET status = 'cancelled', cancellation_note = NULL, updated_at = ?
          WHERE id = ?",
    )
    .bind(&now)
    .bind(doc_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    record_fiscal_document_event(&mut tx, doc_id).await?;
    tx.commit().await.map_err(|e| e.to_string())
}

/// Outcome surfaced to callers past this module's boundary
/// (`commands::returns::create_return`,
/// `commands::fiscal::retry_fiscal_cancellation`) — coarser than
/// `CancelOutcome`/`CancelError` since callers only need to display a
/// short message, not branch on the distinction between a SEFAZ rejection
/// and a connectivity failure (both leave the same trail in
/// `fiscal_documents.cancellation_note` for anyone who does need it).
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CancellationStatus {
    /// No `authorized` NFC-e exists for this sale — nothing to cancel
    /// (still pending/rejected/contingency, or already cancelled).
    NotApplicable,
    Cancelled,
    /// Not even attempted (past the window, or no provider configured).
    /// `fiscal_documents.status` stays `authorized`.
    Skipped,
    /// Attempted and failed (rejected by SEFAZ, or a connectivity error).
    /// `fiscal_documents.status` stays `authorized`.
    Failed,
}

/// Focus NFe requires 15-255 characters. The base message alone already
/// clears the lower bound (a UUID is 36 characters), so only the upper
/// bound needs enforcing here.
fn justificativa_for(sale_id: &str, return_reason: Option<&str>) -> String {
    let base = format!("Devolucao integral da venda {sale_id}");
    let full = match return_reason {
        Some(r) if !r.trim().is_empty() => format!("{base}: {}", r.trim()),
        _ => base,
    };
    full.chars().take(255).collect()
}

/// Attempts to cancel the sale's most recent `authorized` NFC-e. Called
/// once a return brings that sale to fully `returned` — see the module
/// doc above for why this is the only case that calls it, and why a
/// non-`Cancelled` result never fails the return itself. `Err` is
/// reserved for a genuine database failure, the same distinction
/// `try_emit` draws between its own bookkeeping errors and a normal
/// "provider said no" outcome.
pub async fn try_cancel_for_return(
    pool: &Pool<Sqlite>,
    sale_id: &str,
    return_reason: Option<&str>,
) -> Result<CancellationStatus, String> {
    let doc: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, status, updated_at FROM fiscal_documents
          WHERE sale_id = ? AND doc_type = 'NFCE'
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(sale_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let Some((doc_id, status, updated_at)) = doc else {
        return Ok(CancellationStatus::NotApplicable);
    };
    if status != "authorized" {
        return Ok(CancellationStatus::NotApplicable);
    }

    let authorized_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let elapsed = chrono::Utc::now() - authorized_at;
    let allowed_minutes = CANCELLATION_WINDOW_MINUTES - CANCELLATION_SAFETY_MARGIN_MINUTES;
    if elapsed > chrono::Duration::minutes(allowed_minutes) {
        let note = format!(
            "Cancelamento nao tentado: NFC-e autorizada ha mais de {allowed_minutes} minutos \
             (Focus NFe permite cancelamento apenas ate {CANCELLATION_WINDOW_MINUTES} minutos \
             apos a emissao)."
        );
        set_cancellation_note(pool, &doc_id, &note).await?;
        return Ok(CancellationStatus::Skipped);
    }

    let justificativa = justificativa_for(sale_id, return_reason);
    match cancel(&doc_id, &justificativa).await {
        Ok(CancelOutcome::Cancelled) => {
            mark_cancelled(pool, &doc_id).await?;
            Ok(CancellationStatus::Cancelled)
        }
        Ok(CancelOutcome::Rejected { reason }) => {
            set_cancellation_note(pool, &doc_id, &format!("Cancelamento rejeitado pela SEFAZ: {reason}"))
                .await?;
            Ok(CancellationStatus::Failed)
        }
        Err(CancelError::NotConfigured) => {
            set_cancellation_note(
                pool,
                &doc_id,
                "Cancelamento nao tentado: nenhum provedor fiscal configurado.",
            )
            .await?;
            Ok(CancellationStatus::Skipped)
        }
        Err(CancelError::Connectivity(msg)) => {
            eprintln!("fiscal: cancellation attempt for {doc_id} failed: {msg}");
            set_cancellation_note(
                pool,
                &doc_id,
                &format!("Falha ao tentar cancelar (sem conexao com o provedor): {msg}"),
            )
            .await?;
            Ok(CancellationStatus::Failed)
        }
    }
}
