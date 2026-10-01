/**
 * Money is represented as integer cents everywhere in the frontend — cart
 * math, totals, discounts — so arithmetic never touches floating point.
 * Conversion to a decimal string happens only at the edges: formatting
 * for display, or parsing an operator's keyboard input.
 */

export function centsToDisplay(cents: number): string {
  const negative = cents < 0;
  const abs = Math.abs(cents);
  const value = `${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")}`;
  return negative ? `-${value}` : value;
}

/** Parses a decimal string (e.g. from a text input) into integer cents. */
export function parseCentsInput(value: string): number | null {
  const trimmed = value.trim().replace(",", ".");
  if (trimmed === "") return null;
  if (!/^-?\d+(\.\d{1,2})?$/.test(trimmed)) return null;
  const [whole, fraction = ""] = trimmed.split(".");
  const sign = whole.startsWith("-") ? -1 : 1;
  const wholeCents = Math.abs(parseInt(whole, 10)) * 100;
  const fractionCents = parseInt(fraction.padEnd(2, "0"), 10) || 0;
  return sign * (wholeCents + fractionCents);
}
