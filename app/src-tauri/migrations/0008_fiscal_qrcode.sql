-- Focus NFe's emission response includes a ready-made `qrcode_url` for
-- the DANFCE's QR code (see fiscal_client::emit) — plan section 3.4's
-- fiscal_documents columns (mirrored in 0004_fiscal.sql) have nowhere to
-- keep it, and printing an accurate DANFCE (plan section 6) needs it, so
-- this adds one column rather than re-deriving/guessing SEFAZ's QR
-- payload format ourselves.
ALTER TABLE fiscal_documents ADD COLUMN qrcode_url TEXT;
