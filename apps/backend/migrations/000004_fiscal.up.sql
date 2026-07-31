-- Fiscal documents and emission queue — plan section 3.4, adapted to Postgres types.
--
-- Note on `payload_json`: kept as TEXT (not JSONB) to mirror the plan's
-- schema exactly — it's an opaque blob sent verbatim to the fiscal
-- provider using that provider's own (Portuguese) field names.

CREATE TABLE fiscal_documents (
    id                  UUID PRIMARY KEY,
    sale_id             UUID NOT NULL REFERENCES sales(id),
    doc_type            TEXT NOT NULL,
    status              TEXT NOT NULL,
    provider_ref        TEXT,
    access_key          TEXT,
    xml_url             TEXT,
    danfe_url           TEXT,
    rejection_reason    TEXT,
    attempts            INTEGER NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL
);

CREATE TABLE fiscal_queue (
    id                  UUID PRIMARY KEY,
    fiscal_document_id  UUID NOT NULL REFERENCES fiscal_documents(id),
    payload_json        TEXT NOT NULL,
    next_retry_at       TIMESTAMPTZ,
    retry_count         INTEGER NOT NULL DEFAULT 0
);
