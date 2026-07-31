import { create } from "zustand";
import { login as loginRequest } from "../lib/api";
import {
  type AuthSession,
  loadSession,
  saveSession,
  clearSession,
} from "./authSession";

type AuthStatus =
  | "restoring"
  | "unauthenticated"
  | "authenticating"
  | "authenticated";

interface AuthState {
  session: AuthSession | null;
  status: AuthStatus;
  error: string | null;
  restore: () => Promise<void>;
  login: (deviceId: string, operatorPin: string) => Promise<void>;
  logout: () => Promise<void>;
}

export const useAuthStore = create<AuthState>((set) => ({
  session: null,
  status: "restoring",
  error: null,

  restore: async () => {
    const session = await loadSession();
    set({ session, status: session ? "authenticated" : "unauthenticated" });
  },

  login: async (deviceId: string, operatorPin: string) => {
    set({ status: "authenticating", error: null });
    try {
      const response = await loginRequest(deviceId, operatorPin);
      const session: AuthSession = {
        token: response.token,
        expiresAt: response.expires_at,
        deviceId: response.device_id,
        operator: response.operator,
      };
      await saveSession(session);
      set({ session, status: "authenticated" });
    } catch (err) {
      set({ status: "unauthenticated", error: String(err) });
      throw err;
    }
  },

  logout: async () => {
    await clearSession();
    set({ session: null, status: "unauthenticated" });
  },
}));
