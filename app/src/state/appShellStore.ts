import { create } from "zustand";

/**
 * Which top-level screen the authenticated shell shows. Hand-rolled on
 * purpose — a single-window kiosk app with a handful of screens doesn't
 * need a routing library or URL concept, so this just mirrors the same
 * conditional-render pattern App.tsx already uses for auth status.
 */
export type Screen =
  | "sale"
  | "inventory"
  | "cash"
  | "customers"
  | "receivables"
  | "returns"
  | "purchasing";

interface AppShellState {
  screen: Screen;
  setScreen: (screen: Screen) => void;
}

export const useAppShell = create<AppShellState>((set) => ({
  screen: "sale",
  setScreen: (screen) => set({ screen }),
}));
