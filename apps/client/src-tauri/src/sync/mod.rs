//! Outbox sync worker (plan sections 1.3 and 3.5).
//!
//! Reads pending rows from the local `outbox_events` table and pushes them
//! to the Go backend's `POST /sync/push`, then pulls remote changes via
//! `GET /sync/pull?since=<cursor>`. Idempotent via each event's UUID
//! `event_id`. Conflict resolution: last-write-wins per field, except
//! stock and store-credit balances, which sync as deltas.
//!
//! Not implemented in Phase 0 (Foundation) — this is Phase 5 scope.

// TODO(phase 5): background worker that batches outbox_events and calls
// the backend's /sync/push and /sync/pull endpoints on an interval and on
// reconnect.
