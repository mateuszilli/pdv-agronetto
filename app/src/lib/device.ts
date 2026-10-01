import { Store } from "@tauri-apps/plugin-store";

const STORE_FILE = "device.json";
const DEVICE_ID_KEY = "device_id";

let store: Store | null = null;
async function getStore(): Promise<Store> {
  if (!store) store = await Store.load(STORE_FILE);
  return store;
}

/**
 * Each terminal has its own persistent identity (plan section 2: "JWT per
 * device + operator"). Generated once on first boot and reused afterwards.
 */
export async function getOrCreateDeviceId(): Promise<string> {
  const s = await getStore();
  const existing = await s.get<string>(DEVICE_ID_KEY);
  if (existing) return existing;

  const id = crypto.randomUUID();
  await s.set(DEVICE_ID_KEY, id);
  await s.save();
  return id;
}
