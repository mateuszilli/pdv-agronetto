-- Fiscal documents and emission queue — plan section 3.4
--
-- Note on `payload_json`: the JSON body sent to the fiscal provider's API
-- (Focus NFe or similar) must use the field names and Portuguese values
-- defined by that provider's contract — e.g. `natureza_operacao`, `cfop`,
-- `cliente`, `produtos`, `impostos`. Do not translate these keys; they are
-- part of an external API contract, not this codebase's naming.

CREATE TABLE fiscal_documents (
    id                  TEXT PRIMARY KEY,
    sale_id             TEXT NOT NULL REFERENCES sales(id),
    doc_type            TEXT NOT NULL,         -- 'NFCE','NFE' (future B2B)
    status              TEXT NOT NULL,         -- 'pending','processing','authorized','rejected','contingency','cancelled'
    provider_ref        TEXT,                  -- external id at the fiscal provider
    access_key          TEXT,                  -- 44-digit access key, once authorized
    xml_url             TEXT,
    danfe_url           TEXT,
    rejection_reason    TEXT,
    attempts            INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

-- local emission queue — anything not yet confirmed as authorized
CREATE TABLE fiscal_queue (
    id                  TEXT PRIMARY KEY,
    fiscal_document_id  TEXT NOT NULL REFERENCES fiscal_documents(id),
    payload_json        TEXT NOT NULL,        -- sent to the fiscal provider using its required Portuguese field names
    next_retry_at       TEXT,
    retry_count         INTEGER NOT NULL DEFAULT 0
);
