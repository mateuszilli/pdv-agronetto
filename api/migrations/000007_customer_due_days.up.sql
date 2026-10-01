-- Mirrors app/src-tauri/migrations/0006_customer_due_days.sql — see that
-- file for why this column exists beyond plan section 3.2.

ALTER TABLE customers ADD COLUMN default_due_days INTEGER NOT NULL DEFAULT 30;
