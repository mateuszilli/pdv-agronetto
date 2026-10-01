import { useState } from "react";
import { type SaleSearchResult, searchSalesForReturn } from "../../lib/api";
import { centsToDisplay } from "../../lib/money";

interface SaleSearchProps {
  onSelect: (saleId: string) => void;
}

type Mode = "sale_id" | "access_key" | "customer_doc";

/** Search for the sale a return applies to — by sale id, NFC-e access
 * key, or customer document number (plan section 4.5 lists all three as
 * valid lookups). Only one mode is active at a time. */
export function SaleSearch({ onSelect }: SaleSearchProps) {
  const [mode, setMode] = useState<Mode>("sale_id");
  const [query, setQuery] = useState("");
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [results, setResults] = useState<SaleSearchResult[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!query.trim()) {
      setError("enter a search value");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const rows = await searchSalesForReturn({
        sale_id_query: mode === "sale_id" ? query : null,
        access_key: mode === "access_key" ? query : null,
        customer_doc_number: mode === "customer_doc" ? query : null,
        date_from: mode === "customer_doc" && dateFrom ? new Date(dateFrom).toISOString() : null,
        date_to: mode === "customer_doc" && dateTo ? new Date(dateTo).toISOString() : null,
      });
      setResults(rows);
      setSearched(true);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Returns</h2>
      </div>

      <form className="cash-movement-form" onSubmit={handleSubmit}>
        <select
          value={mode}
          onChange={(e) => {
            setMode(e.currentTarget.value as Mode);
            setQuery("");
          }}
        >
          <option value="sale_id">Sale id</option>
          <option value="access_key">NFC-e access key</option>
          <option value="customer_doc">Customer CPF/CNPJ</option>
        </select>
        <input
          type="text"
          placeholder={
            mode === "sale_id"
              ? "Sale id (or part of it)"
              : mode === "access_key"
                ? "44-digit access key"
                : "Customer document number"
          }
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          disabled={loading}
        />
        {mode === "customer_doc" && (
          <>
            <input
              type="date"
              value={dateFrom}
              onChange={(e) => setDateFrom(e.currentTarget.value)}
              disabled={loading}
            />
            <input
              type="date"
              value={dateTo}
              onChange={(e) => setDateTo(e.currentTarget.value)}
              disabled={loading}
            />
          </>
        )}
        <button type="submit" disabled={loading}>
          {loading ? "Searching..." : "Search"}
        </button>
      </form>

      {error && <p className="login-error">{error}</p>}

      {searched && (
        <table className="product-table">
          <thead>
            <tr>
              <th>Sale</th>
              <th>Date</th>
              <th>Customer</th>
              <th>Total</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            {results.map((r) => (
              <tr key={r.id} onClick={() => onSelect(r.id)}>
                <td>{r.id}</td>
                <td>{new Date(r.created_at).toLocaleString()}</td>
                <td>{r.customer_name ?? "—"}</td>
                <td>R$ {centsToDisplay(r.total_cents)}</td>
                <td>{r.status}</td>
              </tr>
            ))}
            {results.length === 0 && (
              <tr>
                <td colSpan={5} className="cart-empty">
                  No sales found.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      )}
    </div>
  );
}
