import { useEffect, useState } from "react";
import { useAuth } from "../../hooks/useAuth";
import {
  type PurchaseOrderDetail as PurchaseOrderDetailType,
  getPurchaseOrder,
  receivePurchaseOrderItems,
} from "../../lib/api";
import { centsToDisplay } from "../../lib/money";
import { formatQuantity, parseQuantityInput } from "../../lib/quantity";

interface PurchaseOrderDetailProps {
  id: string;
  onBack: () => void;
}

function statusLabel(status: string): string {
  switch (status) {
    case "sent":
      return "Awaiting receipt";
    case "partially_received":
      return "Partially received";
    case "received":
      return "Received";
    default:
      return status;
  }
}

/** Purchase order detail + receiving (plan section 4.6: "on receiving
 * goods (full or partial), generates positive stock_movements and
 * updates quantity_received"). Every outstanding line gets its own
 * "receive now" input; submitting sends every non-empty one in a single
 * `receive_purchase_order_items` call, so a delivery covering several
 * lines lands as one transaction. Can be called again later for the next
 * partial delivery. */
export function PurchaseOrderDetail({ id, onBack }: PurchaseOrderDetailProps) {
  const { session } = useAuth();
  const [order, setOrder] = useState<PurchaseOrderDetailType | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [receiveText, setReceiveText] = useState<Record<string, string>>({});
  const [submitting, setSubmitting] = useState(false);

  function reload() {
    setLoading(true);
    return getPurchaseOrder(id)
      .then((o) => {
        setOrder(o);
        setReceiveText({});
      })
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  async function handleReceive() {
    if (!session || !order) return;

    const items = Object.entries(receiveText)
      .map(([itemId, text]) => ({ itemId, quantity: parseQuantityInput(text) }))
      .filter((e): e is { itemId: string; quantity: number } => e.quantity !== null);

    if (items.length === 0) {
      setError("enter a quantity to receive for at least one item");
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      await receivePurchaseOrderItems({
        device_id: session.deviceId,
        purchase_order_id: order.id,
        items: items.map((i) => ({ purchase_order_item_id: i.itemId, quantity: i.quantity })),
      });
      await reload();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  if (loading) return <p>Loading...</p>;
  if (!order) return <p className="login-error">{error}</p>;

  const fullyReceived = order.status === "received";

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>{order.supplier_name}</h2>
        <button onClick={onBack}>Back</button>
      </div>
      <p>
        <span className={`po-status po-status-${order.status}`}>{statusLabel(order.status)}</span>{" "}
        · {new Date(order.created_at).toLocaleString()}
      </p>

      <table className="product-table">
        <thead>
          <tr>
            <th>Product</th>
            <th>Ordered</th>
            <th>Received</th>
            <th>Outstanding</th>
            <th>Unit cost</th>
            {!fullyReceived && <th>Receive now</th>}
          </tr>
        </thead>
        <tbody>
          {order.items.map((item) => {
            const outstanding = item.quantity_ordered - item.quantity_received;
            return (
              <tr key={item.id}>
                <td>{item.product_name}</td>
                <td>
                  {formatQuantity(item.quantity_ordered)} {item.stock_unit}
                </td>
                <td>
                  {formatQuantity(item.quantity_received)} {item.stock_unit}
                </td>
                <td>
                  {formatQuantity(Math.max(0, outstanding))} {item.stock_unit}
                </td>
                <td>R$ {centsToDisplay(item.unit_cost_cents)}</td>
                {!fullyReceived && (
                  <td>
                    {outstanding > 0 && (
                      <input
                        inputMode="decimal"
                        placeholder="0"
                        value={receiveText[item.id] ?? ""}
                        onChange={(e) =>
                          setReceiveText((prev) => ({ ...prev, [item.id]: e.currentTarget.value }))
                        }
                        disabled={submitting}
                      />
                    )}
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>

      {error && <p className="login-error">{error}</p>}

      {!fullyReceived && (
        <div className="panel-actions">
          <button onClick={handleReceive} disabled={submitting}>
            {submitting ? "Receiving..." : "Record receipt"}
          </button>
        </div>
      )}
    </div>
  );
}
