import Database from "@tauri-apps/plugin-sql";

/** Must match `db::DB_URL` in src-tauri/src/db/mod.rs. */
const DB_URL = "sqlite:pdv.db";

let db: Database | null = null;

/** Opens (and, on first run, migrates) the local SQLite database. */
export async function getDb(): Promise<Database> {
  if (!db) db = await Database.load(DB_URL);
  return db;
}
