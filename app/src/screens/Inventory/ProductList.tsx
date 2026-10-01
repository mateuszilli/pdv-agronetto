import { useEffect, useState } from "react";
import { type LowStockProduct, type ProductSummary, listLowStockProducts, listProducts } from "../../lib/api";
import { centsToDisplay } from "../../lib/money";

interface ProductListProps {
  onSelect: (id: string) => void;
  onCreate: () => void;
}

/** Low-stock alert banner (plan section 4.6: "the dashboard and product
 * screen flag items where quantity <= min_quantity"). Loaded once on
 * mount rather than on every keystroke of the search box below — it's
 * independent of the search query. */
function LowStockBanner({ onSelect }: { onSelect: (id: string) => void }) {
  const [items, setItems] = useState<LowStockProduct[]>([]);

  useEffect(() => {
    listLowStockProducts()
      .then(setItems)
      .catch(() => setItems([]));
  }, []);

  if (items.length === 0) return null;

  return (
    <div className="low-stock-banner">
      <strong>Low stock:</strong>
      {items.map((p) => (
        <button key={p.product_id} onClick={() => onSelect(p.product_id)}>
          {p.name} ({p.quantity} / {p.min_quantity} {p.stock_unit})
        </button>
      ))}
    </div>
  );
}

export function ProductList({ onSelect, onCreate }: ProductListProps) {
  const [query, setQuery] = useState("");
  const [products, setProducts] = useState<ProductSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const handle = setTimeout(() => {
      listProducts(query || undefined)
        .then(setProducts)
        .catch((err) => setError(String(err)));
    }, 150);
    return () => clearTimeout(handle);
  }, [query]);

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Products</h2>
        <button onClick={onCreate}>New product</button>
      </div>

      <LowStockBanner onSelect={onSelect} />

      <input
        type="text"
        placeholder="Search by name, SKU, or barcode"
        autoFocus
        value={query}
        onChange={(e) => setQuery(e.currentTarget.value)}
      />

      {error && <p className="login-error">{error}</p>}

      <table className="product-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>SKU</th>
            <th>Unit</th>
            <th>Price</th>
            <th>Stock</th>
          </tr>
        </thead>
        <tbody>
          {products.map((p) => (
            <tr key={p.id} onClick={() => onSelect(p.id)}>
              <td>{p.name}</td>
              <td>{p.sku ?? "-"}</td>
              <td>{p.unit_label}</td>
              <td>R$ {centsToDisplay(p.sell_price_cents)}</td>
              <td>
                {p.quantity} {p.stock_unit}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
