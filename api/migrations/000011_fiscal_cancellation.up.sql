-- Mirrors app/src-tauri/migrations/0010_fiscal_cancellation.sql — see
-- that file for why this column exists (Phase 7's return-triggered
-- whole-document NFC-e cancellation, plan section 4.5).

ALTER TABLE fiscal_documents ADD COLUMN cancellation_note TEXT;
