-- Auth: devices and operators.
--
-- DEVIATION FROM THE PLAN: section 3 of the implementation plan has no
-- `operators` or `devices` table — every other table just carries a bare
-- `device_id`/`operator_id` string with no table backing it, because JWT
-- issuance is a backend concern the plan describes ("Auth | JWT per device
-- + operator (cashier PIN)") without spelling out where credentials live.
-- Phase 0 needs somewhere to check a PIN against, so this migration adds
-- the minimal tables required for that. Flagged here per the session
-- instructions rather than silently added.

CREATE TABLE operators (
    id          UUID PRIMARY KEY,
    name        TEXT NOT NULL,
    pin_hash    TEXT NOT NULL,
    active      BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- device_id is TEXT (not UUID) to match the type used for `device_id`
-- columns everywhere else in this schema (sales, stock_movements, etc).
-- Devices are auto-registered on first successful login in Phase 0 — there
-- is no separate device-provisioning flow yet.
CREATE TABLE devices (
    id              TEXT PRIMARY KEY,
    label           TEXT,
    active          BOOLEAN NOT NULL DEFAULT true,
    first_seen_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
