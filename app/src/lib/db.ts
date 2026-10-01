import Database from "@tauri-apps/plugin-sql";

/** Must match `db::DB_URL` in src-tauri/src/db/mod.rs. */
const DB_URL = "sqlite:pdv.db";

// Cache the in-flight *promise*, not just the resolved value: React
// StrictMode double-invokes App's mount effect, so two calls can land
// before the first `Database.load` resolves. Caching only the resolved
// value let both races call `Database.load` concurrently, and since that
// runs first-run migrations, the loser could fail (table already exists /
// database locked) — silently, since the caller's `.catch` just logs it
// and marks the app "ready" anyway, leaving the Rust-side pool never
// registered for that launch.
let dbPromise: Promise<Database> | null = null;

/** Opens (and, on first run, migrates) the local SQLite database. */
export function getDb(): Promise<Database> {
  if (!dbPromise) dbPromise = Database.load(DB_URL);
  return dbPromise;
}
