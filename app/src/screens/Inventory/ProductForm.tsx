import { useEffect, useState } from "react";
import {
  type ProductDetail,
  type ProductInput,
  createProduct,
  getProduct,
  updateProduct,
} from "../../lib/api";
import { useAuth } from "../../hooks/useAuth";
import { centsToDisplay, parseCentsInput } from "../../lib/money";
import { formatQuantity } from "../../lib/quantity";
import { ProductUnitsPanel } from "./ProductUnitsPanel";

interface ProductFormProps {
  productId: string | null;
  onDone: () => void;
  onCancel: () => void;
}

const emptyForm = {
  sku: "",
  barcode: "",
  name: "",
  ncm: "",
  cfopDefault: "",
  cstCsosn: "",
  stockUnit: "UN",
  costPrice: "0.00",
  unitLabel: "Unit",
  sellPrice: "0.00",
  minQuantity: "0",
  active: true,
};

/** Create/edit form for the catalog (plan section 3.1): the fields here
 * cover the product itself plus its one default selling unit. Additional
 * selling units with their own stock-conversion factor (the "50kg bag" vs
 * "loose kg" case from plan section 4.2) are managed in `ProductUnitsPanel`
 * below, once the product has been saved and has an id. */
export function ProductForm({ productId, onDone, onCancel }: ProductFormProps) {
  const { session } = useAuth();
  const [form, setForm] = useState(emptyForm);
  const [loading, setLoading] = useState(productId !== null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!productId) return;
    setLoading(true);
    getProduct(productId)
      .then((p: ProductDetail) =>
        setForm({
          sku: p.sku ?? "",
          barcode: p.barcode ?? "",
          name: p.name,
          ncm: p.ncm,
          cfopDefault: p.cfop_default ?? "",
          cstCsosn: p.cst_csosn ?? "",
          stockUnit: p.stock_unit,
          costPrice: centsToDisplay(p.cost_price_cents),
          unitLabel: p.unit_label,
          sellPrice: centsToDisplay(p.sell_price_cents),
          minQuantity: formatQuantity(p.min_quantity),
          active: p.active,
        }),
      )
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }, [productId]);

  function set<K extends keyof typeof form>(key: K, value: (typeof form)[K]) {
    setForm((f) => ({ ...f, [key]: value }));
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const costPriceCents = parseCentsInput(form.costPrice);
    const sellPriceCents = parseCentsInput(form.sellPrice);
    if (!form.name.trim() || !form.ncm.trim() || !form.stockUnit.trim() || !form.unitLabel.trim()) {
      setError("name, NCM, stock unit, and unit label are required");
      return;
    }
    if (costPriceCents === null || sellPriceCents === null) {
      setError("enter valid prices");
      return;
    }
    // Zero is a valid threshold (no low-stock alert configured), unlike a
    // cart quantity, so this parses directly rather than reusing
    // parseQuantityInput (which treats zero as invalid).
    const minQuantity = parseFloat(form.minQuantity.trim().replace(",", "."));
    if (!Number.isFinite(minQuantity) || minQuantity < 0) {
      setError("enter a valid minimum stock quantity (0 or more)");
      return;
    }

    const input: ProductInput = {
      sku: form.sku || null,
      barcode: form.barcode || null,
      name: form.name,
      ncm: form.ncm,
      cfop_default: form.cfopDefault || null,
      cst_csosn: form.cstCsosn || null,
      stock_unit: form.stockUnit,
      cost_price_cents: costPriceCents,
      unit_label: form.unitLabel,
      sell_price_cents: sellPriceCents,
      min_quantity: minQuantity,
      active: form.active,
    };

    if (!session) return;

    setSubmitting(true);
    setError(null);
    try {
      if (productId) {
        await updateProduct(session.deviceId, productId, input);
      } else {
        await createProduct(session.deviceId, input);
      }
      onDone();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  if (loading) return <p>Loading...</p>;

  return (
    <form className="panel product-form" onSubmit={handleSubmit}>
      <h2>{productId ? "Edit product" : "New product"}</h2>

      <label className="login-field">
        <span>Name</span>
        <input value={form.name} onChange={(e) => set("name", e.currentTarget.value)} autoFocus />
      </label>

      <div className="form-row">
        <label className="login-field">
          <span>SKU</span>
          <input value={form.sku} onChange={(e) => set("sku", e.currentTarget.value)} />
        </label>
        <label className="login-field">
          <span>Barcode</span>
          <input value={form.barcode} onChange={(e) => set("barcode", e.currentTarget.value)} />
        </label>
      </div>

      <div className="form-row">
        <label className="login-field">
          <span>NCM</span>
          <input value={form.ncm} onChange={(e) => set("ncm", e.currentTarget.value)} />
        </label>
        <label className="login-field">
          <span>CFOP (default)</span>
          <input
            value={form.cfopDefault}
            onChange={(e) => set("cfopDefault", e.currentTarget.value)}
          />
        </label>
        <label className="login-field">
          <span>CST/CSOSN</span>
          <input
            value={form.cstCsosn}
            onChange={(e) => set("cstCsosn", e.currentTarget.value)}
          />
        </label>
      </div>

      <div className="form-row">
        <label className="login-field">
          <span>Stock unit (KG, UN, M...)</span>
          <input
            value={form.stockUnit}
            onChange={(e) => set("stockUnit", e.currentTarget.value)}
          />
        </label>
        <label className="login-field">
          <span>Cost price (R$)</span>
          <input
            inputMode="decimal"
            value={form.costPrice}
            onChange={(e) => set("costPrice", e.currentTarget.value)}
          />
        </label>
      </div>

      <div className="form-row">
        <label className="login-field">
          <span>Selling unit label</span>
          <input
            value={form.unitLabel}
            onChange={(e) => set("unitLabel", e.currentTarget.value)}
          />
        </label>
        <label className="login-field">
          <span>Sell price (R$)</span>
          <input
            inputMode="decimal"
            value={form.sellPrice}
            onChange={(e) => set("sellPrice", e.currentTarget.value)}
          />
        </label>
        <label className="login-field">
          <span>Low-stock threshold</span>
          <input
            inputMode="decimal"
            value={form.minQuantity}
            onChange={(e) => set("minQuantity", e.currentTarget.value)}
          />
        </label>
      </div>

      {productId && (
        <label className="login-field login-field-inline">
          <input
            type="checkbox"
            checked={form.active}
            onChange={(e) => set("active", e.currentTarget.checked)}
          />
          <span>Active</span>
        </label>
      )}

      {error && <p className="login-error">{error}</p>}

      <div className="panel-actions">
        <button type="button" onClick={onCancel} disabled={submitting}>
          Cancel
        </button>
        <button type="submit" disabled={submitting}>
          {submitting ? "Saving..." : "Save"}
        </button>
      </div>

      {productId && <ProductUnitsPanel productId={productId} />}
    </form>
  );
}
