-- Adds the "configurable per customer" due-date term plan section 4.3
-- describes for customer_ledger debit entries ("default: 30 days,
-- configurable per customer"), which section 3.2's customers table has no
-- column for. Same kind of gap-fill as migrations 000006_auth in the
-- backend (see README's "Known deviations" section).

ALTER TABLE customers ADD COLUMN default_due_days INTEGER NOT NULL DEFAULT 30;
