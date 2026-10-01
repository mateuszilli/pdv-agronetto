import { useEffect, useState } from "react";
import { type PurchaseOrderSummary, listPurchaseOrders } from "../../lib/api";
import { centsToDisplay } from "../../lib/money";

interface PurchaseOrderListProps {
  onSelect: (id: string) => void;
  onCreate: () => void;
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

export function PurchaseOrderList({ onSelect, onCreate }: PurchaseOrderListProps) {
  const [orders, setOrders] = useState<PurchaseOrderSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    listPurchaseOrders()
      .then(setOrders)
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }, []);

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Purchase orders</h2>
        <button onClick={onCreate}>New purchase order</button>
      </div>

      {error && <p className="login-error">{error}</p>}
      {loading ? (
        <p>Loading...</p>
      ) : (
        <table className="product-table">
          <thead>
            <tr>
              <th>Supplier</th>
              <th>Status</th>
              <th>Items</th>
              <th>Total cost</th>
              <th>Created</th>
            </tr>
          </thead>
          <tbody>
            {orders.map((o) => (
              <tr key={o.id} onClick={() => onSelect(o.id)}>
                <td>{o.supplier_name}</td>
                <td>
                  <span className={`po-status po-status-${o.status}`}>{statusLabel(o.status)}</span>
                </td>
                <td>{o.item_count}</td>
                <td>R$ {centsToDisplay(o.total_cost_cents)}</td>
                <td>{new Date(o.created_at).toLocaleDateString()}</td>
              </tr>
            ))}
            {orders.length === 0 && (
              <tr>
                <td colSpan={5} className="cart-empty">
                  No purchase orders yet.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      )}
    </div>
  );
}
