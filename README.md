# PDV Monorepo

Offline-first point-of-sale system for agricultural supplies and construction
materials stores in Brazil. Desktop client (Tauri) + cloud sync backend
(Go + Postgres). See [docs/PDV-MVP-Implementation-Plan.md](docs/PDV-MVP-Implementation-Plan.md)
for the full architecture, data model and roadmap.

**Status:** Phase 0 (Foundation) — monorepo scaffold, local schemas, and a
device/operator login round-trip between the client and backend. No sale
screen, catalog UI, printing, or fiscal integration yet; those land in
later phases per the roadmap.

## Layout

```
apps/
├── client/   # Tauri v2 + React + TypeScript desktop app (local SQLite)
└── backend/  # Go (Gin) sync backend (Postgres)
docs/
└── PDV-MVP-Implementation-Plan.md
docker-compose.yml   # local Postgres for dev
Makefile             # common dev commands
```

## Prerequisites

- [Node.js](https://nodejs.org/) 18+ and npm
- [Rust](https://www.rust-lang.org/tools/install) (stable) — required by Tauri
- [Go](https://go.dev/dl/) 1.23+
- [Docker](https://www.docker.com/) (Postgres runs via `docker compose`)
- Tauri's platform dependencies for your OS — see the
  [Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/)
  (on macOS this is just Xcode Command Line Tools)

## First-time setup

```bash
cp .env.example .env   # adjust values if needed; defaults work for local dev
cd apps/client && npm install && cd ../..
```

## Running everything locally

In separate terminals, from the repo root:

```bash
make db-up            # starts Postgres via docker-compose
make migrate-backend   # applies Postgres migrations
make dev-backend        # starts the Go API on :8080
make dev-client          # starts the Tauri app (applies SQLite migrations on boot)
```

The client boots to a login screen. It ships with one seeded demo operator
(logged to the backend's console on first boot — PIN `1234`, printed as
`auth: seeded demo operator "Demo Operator" with PIN "1234" (dev only)`).
Signing in calls the backend's `POST /auth/login`, and the returned JWT is
stored locally via `tauri-plugin-store`.

### Client dev notes

- Local data lives in a SQLite database managed by `tauri-plugin-sql`;
  migrations under `apps/client/src-tauri/migrations/` run automatically
  the first time the app opens the database — there's no separate CLI step
  (`make migrate-client` just prints this).
- The backend URL the client calls defaults to `http://localhost:8080`,
  overridable via the `PDV_BACKEND_URL` env var (see `.env.example`).
- Rust module layout under `src-tauri/src/`: `db/` (SQLite + migrations),
  `commands/` (Tauri command handlers invoked from the frontend),
  `printing/`, `sync/`, `fiscal_client/` — the latter three are stubs with
  `TODO`s for later phases (1, 5, 6).

### Backend dev notes

- Config is via env vars (`DATABASE_URL`, `JWT_SECRET`, `PORT`); `make`
  loads `.env` automatically for both `docker compose` and `go run`.
- `internal/auth` and `internal/sync` have real Phase 0 logic (JWT
  issuance backed by a seeded `operators` table; `/sync/push` and
  `/sync/pull` stubs with the right request/response shapes). The other
  `internal/*` packages (`sales`, `customers`, `inventory`, `reports`,
  `fiscal`) are empty packages with doc comments describing their future
  responsibility per the implementation plan.
- `GET /healthz` reports Postgres reachability.

## Known deviations from the implementation plan

Section 3 of the plan doesn't include `operators` or `devices` tables —
every other table just carries a bare `device_id`/`operator_id` string.
Phase 0 needs somewhere to actually check a PIN against, so
`apps/backend/migrations/000006_auth.up.sql` adds minimal `operators` and
`devices` tables for this purpose. Devices auto-register on first
successful login; there's no separate device-provisioning flow yet.
