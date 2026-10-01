-- Phase 9 (plan section 5.2's back-office stuck-in-contingency alert,
-- exposed via GET /reports/fiscal-alerts).
--
-- DEVIATION FROM PHASE 6: fiscal_documents was defined centrally back in
-- 000004_fiscal.up.sql (schema parity with the plan's section 3.4) but,
-- per the client README's "Known deviations", was deliberately left
-- unsynced — fiscal status was treated as a local-device, provider-facing
-- concern (plan section 1.1). Phase 9 reverses that for fiscal_documents
-- specifically (not fiscal_queue, which stays local-only: retry
-- bookkeeping has no back-office use) because the plan explicitly wants a
-- central alert when a document stays pending past the contingency window,
-- and that requires this table to actually be populated centrally. See
-- internal/sync's new applyFiscalDocument and the client-side outbox
-- emission it depends on (app/src-tauri/src/commands/sales.rs,
-- app/src-tauri/src/fiscal_client/mod.rs).
--
-- This migration only adds the index the alert query needs — the table
-- and its columns already match what Phase 6's client code populates.

CREATE INDEX fiscal_documents_status_created_at_idx ON fiscal_documents (status, created_at);
