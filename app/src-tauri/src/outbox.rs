//! Outbox event recording (plan sections 1.3 and 3.5).
//!
//! Every command that mutates a row the backend needs to know about calls
//! `record` inside the same transaction as the mutation itself, so the
//! outbox entry and the row it describes always commit (or roll back)
//! together. The sync worker (`sync::run_once`) is the only thing that
//! reads this table.

use serde::Serialize;
use sqlx::{Sqlite, Transaction};

/// Inserts one `outbox_events` row. `payload` is serialized as-is — each
/// call site passes a struct shaped like the row(s) it describes, since
/// that's exactly what the backend's apply logic expects to deserialize.
pub async fn record<T: Serialize>(
    tx: &mut Transaction<'_, Sqlite>,
    entity_type: &str,
    entity_id: &str,
    device_id: &str,
    payload: &T,
) -> Result<(), String> {
    let payload_json = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO outbox_events
            (id, entity_type, entity_id, payload_json, device_id, created_at, synced_at)
         VALUES (?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(payload_json)
    .bind(device_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
