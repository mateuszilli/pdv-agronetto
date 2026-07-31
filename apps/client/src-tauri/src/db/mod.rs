//! Local SQLite layer. The database itself is opened and pooled by
//! `tauri-plugin-sql` (sqlx under the hood); this module only owns the
//! migration set and the connection identifier the frontend uses to open it.

use tauri_plugin_sql::{Migration, MigrationKind};

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
    ]
}
