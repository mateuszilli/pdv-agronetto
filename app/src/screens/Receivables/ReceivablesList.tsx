import { useEffect, useState } from "react";
import { type CustomerSummary, listReceivables } from "../../lib/api";
import { centsToDisplay } from "../../lib/money";

interface ReceivablesListProps {
  onSelect: (id: string) => void;
}

export function ReceivablesList({ onSelect }: ReceivablesListProps) {
  const [customers, setCustomers] = useState<CustomerSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    listReceivables()
      .then(setCustomers)
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }, []);

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Accounts receivable</h2>
      </div>

      {error && <p className="login-error">{error}</p>}
      {loading ? (
        <p>Loading...</p>
      ) : (
        <table className="product-table">
          <thead>
            <tr>
              <th>Customer</th>
              <th>Doc</th>
              <th>Balance owed</th>
              <th>Credit limit</th>
            </tr>
          </thead>
          <tbody>
            {customers.map((c) => (
              <tr key={c.id} onClick={() => onSelect(c.id)}>
                <td>{c.name}</td>
                <td>
                  {c.doc_type} {c.doc_number}
                </td>
                <td className={c.balance_owed_cents > c.credit_limit_cents ? "login-error" : ""}>
                  R$ {centsToDisplay(c.balance_owed_cents)}
                </td>
                <td>R$ {centsToDisplay(c.credit_limit_cents)}</td>
              </tr>
            ))}
            {customers.length === 0 && (
              <tr>
                <td colSpan={4} className="cart-empty">
                  No outstanding balances.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      )}
    </div>
  );
}
