import { create } from "zustand";
import {
  type CashSession,
  getOpenCashSession,
  openCashSession,
  closeCashSession,
} from "../lib/api";

type CashSessionStatus = "idle" | "loading" | "ready";

interface CashSessionState {
  session: CashSession | null;
  status: CashSessionStatus;
  error: string | null;
  load: (deviceId: string) => Promise<void>;
  open: (
    deviceId: string,
    operatorId: string,
    openingAmountCents: number,
  ) => Promise<void>;
  close: (countedAmountCents: number) => Promise<CashSession>;
}

/** The device's open cash_sessions row — plan section 4.4: "sales cannot
 * start without an open cash_sessions on the device." */
export const useCashSessionStore = create<CashSessionState>((set, get) => ({
  session: null,
  status: "idle",
  error: null,

  load: async (deviceId: string) => {
    set({ status: "loading", error: null });
    try {
      const session = await getOpenCashSession(deviceId);
      set({ session, status: "ready" });
    } catch (err) {
      set({ status: "ready", error: String(err) });
    }
  },

  open: async (deviceId, operatorId, openingAmountCents) => {
    set({ error: null });
    const session = await openCashSession(deviceId, operatorId, openingAmountCents);
    set({ session });
  },

  close: async (countedAmountCents: number) => {
    const current = get().session;
    if (!current) throw new Error("no open cash session");
    const closed = await closeCashSession(current.id, countedAmountCents);
    set({ session: null });
    return closed;
  },
}));
