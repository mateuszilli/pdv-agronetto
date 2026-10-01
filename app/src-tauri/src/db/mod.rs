//! Local SQLite layer. The database itself is opened and pooled by
//! `tauri-plugin-sql` (sqlx under the hood); this module only owns the
//! migration set and the connection identifier the frontend uses to open it.

use sqlx::{Pool, Sqlite};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_sql::{DbInstances, DbPool, Migration, MigrationKind};

/// Identifier passed to `Database.load(...)` on the JS side and to
/// `add_migrations(...)` on the Rust side. Must match on both sides.
pub const DB_URL: &str = "sqlite:pdv.db";

/// Versioned migrations applied in order on first connection.
/// Source files live in `src-tauri/migrations/` and mirror section 3 of
/// docs/PDV-MVP-Implementation-Plan.md.
pub fn migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "catalog_and_stock",
            sql: include_str!("../../migrations/0001_catalog_and_stock.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "customers_and_credit",
            sql: include_str!("../../migrations/0002_customers_and_credit.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "sales_payments_cash",
            sql: include_str!("../../migrations/0003_sales_payments_cash.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 4,
            description: "fiscal",
            sql: include_str!("../../migrations/0004_fiscal.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 5,
            description: "sync_outbox",
            sql: include_str!("../../migrations/0005_sync_outbox.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 6,
            description: "customer_due_days",
            sql: include_str!("../../migrations/0006_customer_due_days.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 7,
            description: "sync_lww_columns",
            sql: include_str!("../../migrations/0007_sync_lww_columns.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 8,
            description: "fiscal_qrcode",
            sql: include_str!("../../migrations/0008_fiscal_qrcode.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 9,
            description: "returns_sync_columns",
            sql: include_str!("../../migrations/0009_returns_sync_columns.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 10,
            description: "fiscal_cancellation",
            sql: include_str!("../../migrations/0010_fiscal_cancellation.sql"),
            kind: MigrationKind::Up,
        },
    ]
}

/// Fetches the pooled SQLite connection the frontend opened via
/// `Database.load(DB_URL)` (see `src/lib/db.ts`). Commands that run their
/// own queries reuse this pool instead of opening a second connection to
/// the same file — `tauri-plugin-sql` keeps it in the `DbInstances` state
/// it manages, keyed by `DB_URL`.
///
/// Returns an error if the frontend hasn't called `Database.load` yet;
/// `App.tsx` awaits that before rendering any screen that invokes a
/// command needing this pool, so in practice it's always present.
pub async fn pool<R: Runtime>(app: &AppHandle<R>) -> Result<Pool<Sqlite>, String> {
    let instances = app.state::<DbInstances>();
    let lock = instances.0.read().await;
    match lock.get(DB_URL) {
        Some(DbPool::Sqlite(pool)) => Ok(pool.clone()),
        None => Err("local database not loaded yet".into()),
    }
}
