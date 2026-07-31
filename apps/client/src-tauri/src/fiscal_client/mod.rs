//! HTTP client for the fiscal provider API (e.g. Focus NFe) — plan section 5.
//!
//! Submits `fiscal_queue` payloads for NFC-e emission and polls/receives
//! authorization results to update `fiscal_documents`. Payload field names
//! are dictated by the provider's contract (Portuguese keys) and must not
//! be translated — see the note in migrations/0004_fiscal.sql.
//!
//! Not implemented in Phase 0 (Foundation) — this is Phase 6 scope.

// TODO(phase 6): provider client (auth, emission request, status polling),
// contingency-mode handling, and retry/backoff against fiscal_queue.
