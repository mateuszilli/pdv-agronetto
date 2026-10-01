/**
 * Sale-item quantities are decimals (plan section 4.2 — a loose kg or a
 * cut meter, not just whole units), unlike money which stays in integer
 * cents. Parsing/formatting lives here so every quantity input on the
 * sale screen agrees on precision (3 decimal places is enough for a scale
 * reading in kg or a tape measurement in meters).
 */

const MAX_DECIMALS = 3;
const QUANTITY_PATTERN = /^\d+(\.\d{1,3})?$/;

export function parseQuantityInput(value: string): number | null {
  const trimmed = value.trim().replace(",", ".");
  if (trimmed === "" || !QUANTITY_PATTERN.test(trimmed)) return null;
  const parsed = parseFloat(trimmed);
  return parsed > 0 ? parsed : null;
}

export function formatQuantity(value: number): string {
  const fixed = value.toFixed(MAX_DECIMALS);
  return fixed.replace(/\.?0+$/, "") || "0";
}
