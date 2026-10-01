# PDV Client (Tauri + React + TypeScript)

Desktop point-of-sale client. See the [repo root README](../../README.md)
for the full local setup (Postgres, backend, env vars) and the
[implementation plan](../../docs/PDV-MVP-Implementation-Plan.md) for
architecture and roadmap.

## Running in dev mode

From the repo root:

```bash
make dev-client
```

Equivalent to, from this directory:

```bash
npm install   # first run only
npm run tauri dev
```

This starts the Vite dev server, compiles the Rust/Tauri shell, and opens
the app window. On first launch it opens (and migrates) a local SQLite
database under the OS app-data directory — see `src-tauri/migrations/` for
the schema and `src-tauri/src/db/mod.rs` for how migrations are registered.

The app boots to an operator login screen; signing in calls the Go
backend's `POST /auth/login` (see `src-tauri/src/commands/auth.rs`), so the
backend must be running too (`make dev-backend` from the repo root). The
backend URL defaults to `http://localhost:8080` and is overridable via the
`PDV_BACKEND_URL` env var.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
