import { create } from "zustand";
import type { ProductSummary, ProductUnitOption } from "../lib/api";

export interface CartItem {
  productId: string;
  unitId: string;
  name: string;
  unitLabel: string;
  unitPriceCents: number;
  factorToStock: number;
  quantity: number;
}

interface CartState {
  items: CartItem[];
  discountCents: number;
  selectedIndex: number;
  /** Set via F3 "identify customer" (plan section 4.1/4.3) — required for
   * the `store_credit` payment method, and attached to the sale either way. */
  customerId: string | null;
  customerName: string | null;
  /** `unit` picks a non-default selling unit (plan section 4.1's fast
   * numeric selector) — omitted, this behaves like Phase 1: add one of the
   * product's default unit. */
  addProduct: (product: ProductSummary, unit?: ProductUnitOption) => void;
  removeItem: (index: number) => void;
  setQuantity: (index: number, quantity: number) => void;
  /** F5 — switches the selected line to a different selling unit,
   * converting the quantity so the physical amount ordered is preserved
   * (e.g. 2 "50kg bag" becomes 100 "Loose kg") — plan section 4.1/4.2. */
  setUnit: (index: number, unit: ProductUnitOption) => void;
  setSelectedIndex: (index: number) => void;
  setDiscountCents: (cents: number) => void;
  setCustomer: (customer: { id: string; name: string } | null) => void;
  clear: () => void;
}

/** Cart math is done in integer cents throughout — see lib/money.ts. Only
 * a sale-level discount is supported in Phase 1 (per-line discounts and
 * split payments are later-phase scope). */
export const useCartStore = create<CartState>((set) => ({
  items: [],
  discountCents: 0,
  selectedIndex: -1,
  customerId: null,
  customerName: null,

  addProduct: (product, unit) =>
    set((state) => {
      const unitId = unit?.id ?? product.unit_id;
      const existing = state.items.findIndex(
        (i) => i.productId === product.id && i.unitId === unitId,
      );
      if (existing >= 0) {
        const items = [...state.items];
        items[existing] = { ...items[existing], quantity: items[existing].quantity + 1 };
        return { items, selectedIndex: existing };
      }
      const items = [
        ...state.items,
        {
          productId: product.id,
          unitId,
          name: product.name,
          unitLabel: unit?.unit_label ?? product.unit_label,
          unitPriceCents: unit?.sell_price_cents ?? product.sell_price_cents,
          factorToStock: unit?.factor_to_stock ?? product.factor_to_stock,
          quantity: 1,
        },
      ];
      return { items, selectedIndex: items.length - 1 };
    }),

  removeItem: (index) =>
    set((state) => {
      const items = state.items.filter((_, i) => i !== index);
      return { items, selectedIndex: Math.min(state.selectedIndex, items.length - 1) };
    }),

  setQuantity: (index, quantity) =>
    set((state) => {
      if (quantity <= 0) {
        const items = state.items.filter((_, i) => i !== index);
        return { items, selectedIndex: Math.min(state.selectedIndex, items.length - 1) };
      }
      const items = [...state.items];
      items[index] = { ...items[index], quantity };
      return { items };
    }),

  setUnit: (index, unit) =>
    set((state) => {
      const item = state.items[index];
      if (!item) return state;
      const quantityStock = item.quantity * item.factorToStock;
      const quantity = quantityStock / unit.factor_to_stock;
      const items = [...state.items];
      items[index] = {
        ...item,
        unitId: unit.id,
        unitLabel: unit.unit_label,
        unitPriceCents: unit.sell_price_cents,
        factorToStock: unit.factor_to_stock,
        quantity,
      };
      return { items };
    }),

  setSelectedIndex: (index) => set({ selectedIndex: index }),
  setDiscountCents: (discountCents) => set({ discountCents: Math.max(0, discountCents) }),
  setCustomer: (customer) =>
    set({ customerId: customer?.id ?? null, customerName: customer?.name ?? null }),
  clear: () =>
    set({
      items: [],
      discountCents: 0,
      selectedIndex: -1,
      customerId: null,
      customerName: null,
    }),
}));

/** Rounds each line's cents independently, matching the backend's
 * `line_total_cents` (src-tauri/src/commands/sales.rs) so the displayed
 * subtotal never drifts from what gets persisted. */
export function lineTotalCents(item: CartItem): number {
  return Math.round(item.unitPriceCents * item.quantity);
}

export function subtotalCents(items: CartItem[]): number {
  return items.reduce((sum, i) => sum + lineTotalCents(i), 0);
}
