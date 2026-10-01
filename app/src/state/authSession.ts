import { Store } from "@tauri-apps/plugin-store";
import type { OperatorInfo } from "../lib/api";

const STORE_FILE = "auth-session.json";
const SESSION_KEY = "session";

export interface AuthSession {
  token: string;
  expiresAt: string;
  deviceId: string;
  operator: OperatorInfo;
}

let store: Store | null = null;
async function getStore(): Promise<Store> {
  if (!store) store = await Store.load(STORE_FILE);
  return store;
}

export async function saveSession(session: AuthSession): Promise<void> {
  const s = await getStore();
  await s.set(SESSION_KEY, session);
  await s.save();
}

export async function loadSession(): Promise<AuthSession | null> {
  const s = await getStore();
  const session = await s.get<AuthSession>(SESSION_KEY);
  return session ?? null;
}

export async function clearSession(): Promise<void> {
  const s = await getStore();
  await s.delete(SESSION_KEY);
  await s.save();
}
