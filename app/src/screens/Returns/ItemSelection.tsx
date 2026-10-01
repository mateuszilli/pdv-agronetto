import { useEffect, useState } from "react";
import { useAuth } from "../../hooks/useAuth";
import {
  type ReturnResult,
  type SaleForReturn,
  createReturn,
  getSaleForReturn,
} from "../../lib/api";
import { centsToDisplay } from "../../lib/money";
import { formatQuantity, parseQuantityInput } from "../../lib/quantity";

interface ItemSelectionProps {
  saleId: string;
  onBack: () => void;
  onDone: (result: ReturnResult) => void;
}

const REFUND_METHODS = ["cash", "debit", "credit", "pix", "store_credit"] as const;

/** Loads one sale's returnable items, lets the operator pick a quantity
 * per line (blank/zero = not part of this return), a refund method, and
 * an optional reason, then confirms the return in one call (plan section
 * 4.5: item-by-item, partial returns supported). */
export function ItemSelection({ saleId, onBack, onDone }: ItemSelectionProps) {
  const { session } = useAuth();
  const [sale, setSale] = useState<SaleForReturn | null>(null);
  const [quantities, setQuantities] = useState<Record<string, string>>({});
  const [refundMethod, setRefundMethod] = useState<(typeof REFUND_METHODS)[number]>("cash");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setLoading(true);
    getSaleForReturn(saleId)
      .then(setSale)
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }, [saleId]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onBack();
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onBack]);

  const hasCustomer = !!sale?.customer_id;

  const refundTotalCents = sale
    ? sale.items.reduce((sum, item) => {
        const quantity = parseQuantityInput(quantities[item.sale_item_id] ?? "");
        if (!quantity) return sum;
        return sum + Math.round(item.unit_price_cents * quantity);
      }, 0)
    : 0;

  async function handleConfirm() {
    if (!sale || !session) return;
    const items = sale.items
      .map((item) => ({
        sale_item_id: item.sale_item_id,
        quantity: parseQuantityInput(quantities[item.sale_item_id] ?? ""),
      }))
      .filter((item): item is { sale_item_id: string; quantity: number } => item.quantity !== null);

    if (items.length === 0) {
      setError("select a quantity for at least one item");
      return;
    }
    for (const item of items) {
      const original = sale.items.find((i) => i.sale_item_id === item.sale_item_id);
      if (original && item.quantity > original.quantity_returnable + 1e-6) {
        setError(`quantity for ${original.product_name} exceeds what's returnable`);
        return;
      }
    }
    if (refundMethod === "store_credit" && !hasCustomer) {
      setError("store credit refund requires an identified customer");
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      const result = await createReturn({
        original_sale_id: sale.id,
        device_id: session.deviceId,
        operator_id: session.operator.id,
        reason: reason || null,
        refund_method: refundMethod,
        items,
      });
      onDone(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  if (loading) return <p>Loading...</p>;
  if (!sale) return <p className="login-error">{error}</p>;

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Return sale {sale.id}</h2>
        <button onClick={onBack} disabled={submitting}>
          Back
        </button>
      </div>
      <p>
        {sale.customer_name ?? "Unidentified consumer"} — {new Date(sale.created_at).toLocaleString()} —
        total R$ {centsToDisplay(sale.total_cents)} — status {sale.status}
      </p>

      <table className="product-table">
        <thead>
          <tr>
            <th>Product</th>
            <th>Unit</th>
            <th>Sold</th>
            <th>Returned</th>
            <th>Returnable</th>
            <th>Unit price</th>
            <th>Return qty</th>
          </tr>
        </thead>
        <tbody>
          {sale.items.map((item) => (
            <tr key={item.sale_item_id}>
              <td>{item.product_name}</td>
              <td>{item.unit_label}</td>
              <td>{formatQuantity(item.quantity)}</td>
              <td>{formatQuantity(item.quantity_returned)}</td>
              <td>{formatQuantity(item.quantity_returnable)}</td>
              <td>R$ {centsToDisplay(item.unit_price_cents)}</td>
              <td>
                <input
                  type="text"
                  inputMode="decimal"
                  placeholder="0"
                  value={quantities[item.sale_item_id] ?? ""}
                  onChange={(e) =>
                    setQuantities((q) => ({ ...q, [item.sale_item_id]: e.currentTarget.value }))
                  }
                  disabled={submitting || item.quantity_returnable <= 0}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <form
        className="cash-movement-form"
        onSubmit={(e) => {
          e.preventDefault();
          handleConfirm();
        }}
      >
        <select
          value={refundMethod}
          onChange={(e) => setRefundMethod(e.currentTarget.value as (typeof REFUND_METHODS)[number])}
          disabled={submitting}
        >
          {REFUND_METHODS.map((method) => (
            <option key={method} value={method} disabled={method === "store_credit" && !hasCustomer}>
              {method}
            </option>
          ))}
        </select>
        <input
          type="text"
          placeholder="Reason (optional)"
          value={reason}
          onChange={(e) => setReason(e.currentTarget.value)}
          disabled={submitting}
        />
        <span>Refund total: R$ {centsToDisplay(refundTotalCents)}</span>
        <button type="submit" disabled={submitting}>
          {submitting ? "Recording..." : "Confirm return"}
        </button>
      </form>

      {error && <p className="login-error">{error}</p>}
    </div>
  );
}
