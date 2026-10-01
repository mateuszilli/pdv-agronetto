import { useEffect, useState } from "react";
import { useAuth } from "../../hooks/useAuth";
import {
  type ProductSummary,
  type PurchaseOrderItemInput,
  createPurchaseOrder,
  listProducts,
} from "../../lib/api";
import { centsToDisplay, parseCentsInput } from "../../lib/money";
import { formatQuantity, parseQuantityInput } from "../../lib/quantity";

interface CreatePurchaseOrderProps {
  onCreated: (id: string) => void;
  onCancel: () => void;
}

/** One line the operator has added to the order being built — kept
 * separate from `PurchaseOrderItemInput` so the form can also carry the
 * product's display name (not part of the command's input shape). */
interface DraftItem {
  productId: string;
  productName: string;
  stockUnit: string;
  quantityOrdered: number;
  unitCostCents: number;
}

/** Create-purchase-order form (plan sections 3.1 and 4.6): pick a
 * supplier, then build up a list of products to order with a quantity
 * (always in the product's stock_unit — see purchasing.rs's module doc)
 * and a per-unit cost, which can differ from the catalog's reference
 * cost_price since it reflects this specific supplier's invoice. */
export function CreatePurchaseOrder({ onCreated, onCancel }: CreatePurchaseOrderProps) {
  const { session } = useAuth();
  const [supplierName, setSupplierName] = useState("");
  const [items, setItems] = useState<DraftItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<ProductSummary[]>([]);
  const [picked, setPicked] = useState<ProductSummary | null>(null);
  const [quantityText, setQuantityText] = useState("1");
  const [unitCostText, setUnitCostText] = useState("0.00");

  useEffect(() => {
    if (!query) {
      setResults([]);
      return;
    }
    const handle = setTimeout(() => {
      listProducts(query)
        .then(setResults)
        .catch((err) => setError(String(err)));
    }, 150);
    return () => clearTimeout(handle);
  }, [query]);

  function pickProduct(product: ProductSummary) {
    setPicked(product);
    setQuery(product.name);
    setResults([]);
  }

  function addItem() {
    if (!picked) {
      setError("search for and select a product first");
      return;
    }
    const quantity = parseQuantityInput(quantityText);
    const unitCostCents = parseCentsInput(unitCostText);
    if (quantity === null) {
      setError("enter a quantity greater than zero");
      return;
    }
    if (unitCostCents === null || unitCostCents < 0) {
      setError("enter a valid unit cost");
      return;
    }
    setItems((prev) => [
      ...prev,
      {
        productId: picked.id,
        productName: picked.name,
        stockUnit: picked.stock_unit,
        quantityOrdered: quantity,
        unitCostCents,
      },
    ]);
    setPicked(null);
    setQuery("");
    setQuantityText("1");
    setUnitCostText("0.00");
    setError(null);
  }

  function removeItem(index: number) {
    setItems((prev) => prev.filter((_, i) => i !== index));
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!session) return;
    if (!supplierName.trim()) {
      setError("supplier name is required");
      return;
    }
    if (items.length === 0) {
      setError("add at least one item");
      return;
    }

    const input: PurchaseOrderItemInput[] = items.map((i) => ({
      product_id: i.productId,
      quantity_ordered: i.quantityOrdered,
      unit_cost_cents: i.unitCostCents,
    }));

    setSubmitting(true);
    setError(null);
    try {
      const order = await createPurchaseOrder({
        device_id: session.deviceId,
        supplier_name: supplierName.trim(),
        items: input,
      });
      onCreated(order.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form className="panel" onSubmit={handleSubmit}>
      <div className="panel-header">
        <h2>New purchase order</h2>
      </div>

      <label className="login-field">
        <span>Supplier</span>
        <input
          value={supplierName}
          onChange={(e) => setSupplierName(e.currentTarget.value)}
          autoFocus
        />
      </label>

      <div className="unit-new-row">
        <label className="login-field">
          <span>Product</span>
          <input
            value={query}
            onChange={(e) => {
              setQuery(e.currentTarget.value);
              setPicked(null);
            }}
            placeholder="Search by name, SKU, or barcode"
          />
          {results.length > 0 && (
            <ul className="search-results">
              {results.map((r) => (
                <li key={r.id} onClick={() => pickProduct(r)}>
                  <span>{r.name}</span>
                  <span>{r.stock_unit}</span>
                </li>
              ))}
            </ul>
          )}
        </label>
        <label className="login-field">
          <span>Quantity ({picked?.stock_unit ?? "stock unit"})</span>
          <input
            inputMode="decimal"
            value={quantityText}
            onChange={(e) => setQuantityText(e.currentTarget.value)}
          />
        </label>
        <label className="login-field">
          <span>Unit cost (R$)</span>
          <input
            inputMode="decimal"
            value={unitCostText}
            onChange={(e) => setUnitCostText(e.currentTarget.value)}
          />
        </label>
        <button type="button" onClick={addItem}>
          Add item
        </button>
      </div>

      <table className="product-table">
        <thead>
          <tr>
            <th>Product</th>
            <th>Quantity</th>
            <th>Unit cost</th>
            <th>Line total</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {items.map((item, i) => (
            <tr key={i}>
              <td>{item.productName}</td>
              <td>
                {formatQuantity(item.quantityOrdered)} {item.stockUnit}
              </td>
              <td>R$ {centsToDisplay(item.unitCostCents)}</td>
              <td>R$ {centsToDisplay(Math.round(item.unitCostCents * item.quantityOrdered))}</td>
              <td>
                <button type="button" onClick={() => removeItem(i)}>
                  ×
                </button>
              </td>
            </tr>
          ))}
          {items.length === 0 && (
            <tr>
              <td colSpan={5} className="cart-empty">
                No items added yet.
              </td>
            </tr>
          )}
        </tbody>
      </table>

      {error && <p className="login-error">{error}</p>}

      <div className="panel-actions">
        <button type="button" onClick={onCancel} disabled={submitting}>
          Cancel
        </button>
        <button type="submit" disabled={submitting}>
          {submitting ? "Creating..." : "Create purchase order"}
        </button>
      </div>
    </form>
  );
}
