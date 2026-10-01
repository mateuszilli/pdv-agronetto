import { useEffect, useState } from "react";
import { type CustomerSummary, listCustomers } from "../../lib/api";
import { centsToDisplay } from "../../lib/money";

interface CustomerListProps {
  onSelect: (id: string) => void;
  onCreate: () => void;
}

export function CustomerList({ onSelect, onCreate }: CustomerListProps) {
  const [query, setQuery] = useState("");
  const [customers, setCustomers] = useState<CustomerSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const handle = setTimeout(() => {
      listCustomers(query || undefined)
        .then(setCustomers)
        .catch((err) => setError(String(err)));
    }, 150);
    return () => clearTimeout(handle);
  }, [query]);

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Customers</h2>
        <button onClick={onCreate}>New customer</button>
      </div>

      <input
        type="text"
        placeholder="Search by name or CPF/CNPJ"
        autoFocus
        value={query}
        onChange={(e) => setQuery(e.currentTarget.value)}
      />

      {error && <p className="login-error">{error}</p>}

      <table className="product-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Doc</th>
            <th>Phone</th>
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
              <td>{c.phone ?? "-"}</td>
              <td className={c.balance_owed_cents > c.credit_limit_cents ? "login-error" : ""}>
                R$ {centsToDisplay(c.balance_owed_cents)}
              </td>
              <td>R$ {centsToDisplay(c.credit_limit_cents)}</td>
            </tr>
          ))}
          {customers.length === 0 && (
            <tr>
              <td colSpan={5} className="cart-empty">
                No customers found.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
