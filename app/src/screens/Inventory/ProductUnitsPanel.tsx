import { useEffect, useState } from "react";
import {
  type ProductUnitOption,
  createProductUnit,
  listAllProductUnits,
  updateProductUnit,
} from "../../lib/api";
import { useAuth } from "../../hooks/useAuth";
import { centsToDisplay, parseCentsInput } from "../../lib/money";

interface ProductUnitsPanelProps {
  productId: string;
}

/** Manages a product's alternative selling units (plan section 4.2) — e.g.
 * a "50kg bag" and a "Loose kg" on top of the default unit set on the main
 * product form. Each unit's `factor_to_stock` is how many `stock_unit`s
 * one of it is worth; the sale screen uses it to convert quantities and
 * derive the stock movement. */
export function ProductUnitsPanel({ productId }: ProductUnitsPanelProps) {
  const [units, setUnits] = useState<ProductUnitOption[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  function reload() {
    setLoading(true);
    listAllProductUnits(productId)
      .then(setUnits)
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }

  useEffect(reload, [productId]);

  return (
    <div className="unit-panel">
      <h3>Selling units</h3>
      <p className="unit-panel-hint">
        Alternative units for fractional/bulk sales (plan section 4.2) — e.g. a &quot;50kg
        bag&quot; on top of a &quot;Loose kg&quot; default. The conversion factor is how many
        stock units one of this unit is worth.
      </p>

      {error && <p className="login-error">{error}</p>}
      {loading ? (
        <p>Loading...</p>
      ) : (
        <table className="unit-table">
          <thead>
            <tr>
              <th>Label</th>
              <th>Factor to stock</th>
              <th>Sell price (R$)</th>
              <th>Default</th>
              <th>Active</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {units.map((u) => (
              <UnitRow key={u.id} unit={u} onSaved={reload} />
            ))}
          </tbody>
        </table>
      )}

      <NewUnitRow productId={productId} onCreated={reload} />
    </div>
  );
}

function UnitRow({ unit, onSaved }: { unit: ProductUnitOption; onSaved: () => void }) {
  const { session } = useAuth();
  const [unitLabel, setUnitLabel] = useState(unit.unit_label);
  const [factorToStock, setFactorToStock] = useState(String(unit.factor_to_stock));
  const [sellPrice, setSellPrice] = useState(centsToDisplay(unit.sell_price_cents));
  const [isDefaultSale, setIsDefaultSale] = useState(unit.is_default_sale);
  const [active, setActive] = useState(unit.active);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const factor = parseFloat(factorToStock.replace(",", "."));
    const sellPriceCents = parseCentsInput(sellPrice);
    if (!unitLabel.trim() || !Number.isFinite(factor) || factor <= 0 || sellPriceCents === null) {
      setError("enter a label, a factor greater than zero, and a valid price");
      return;
    }
    if (!session) return;

    setSaving(true);
    setError(null);
    try {
      await updateProductUnit(
        session.deviceId,
        unit.id,
        unitLabel.trim(),
        factor,
        sellPriceCents,
        isDefaultSale,
        active,
      );
      onSaved();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <tr>
      <td>
        <input value={unitLabel} onChange={(e) => setUnitLabel(e.currentTarget.value)} />
      </td>
      <td>
        <input
          inputMode="decimal"
          value={factorToStock}
          onChange={(e) => setFactorToStock(e.currentTarget.value)}
        />
      </td>
      <td>
        <input inputMode="decimal" value={sellPrice} onChange={(e) => setSellPrice(e.currentTarget.value)} />
      </td>
      <td>
        <input
          type="checkbox"
          checked={isDefaultSale}
          onChange={(e) => setIsDefaultSale(e.currentTarget.checked)}
        />
      </td>
      <td>
        <input type="checkbox" checked={active} onChange={(e) => setActive(e.currentTarget.checked)} />
      </td>
      <td>
        <button type="button" onClick={save} disabled={saving}>
          {saving ? "Saving..." : "Save"}
        </button>
        {error && <p className="login-error">{error}</p>}
      </td>
    </tr>
  );
}

function NewUnitRow({ productId, onCreated }: { productId: string; onCreated: () => void }) {
  const { session } = useAuth();
  const [unitLabel, setUnitLabel] = useState("");
  const [factorToStock, setFactorToStock] = useState("1");
  const [sellPrice, setSellPrice] = useState("0.00");
  const [isDefaultSale, setIsDefaultSale] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function add() {
    const factor = parseFloat(factorToStock.replace(",", "."));
    const sellPriceCents = parseCentsInput(sellPrice);
    if (!unitLabel.trim() || !Number.isFinite(factor) || factor <= 0 || sellPriceCents === null) {
      setError("enter a label, a factor greater than zero, and a valid price");
      return;
    }
    if (!session) return;

    setSaving(true);
    setError(null);
    try {
      await createProductUnit(
        session.deviceId,
        productId,
        unitLabel.trim(),
        factor,
        sellPriceCents,
        isDefaultSale,
      );
      setUnitLabel("");
      setFactorToStock("1");
      setSellPrice("0.00");
      setIsDefaultSale(false);
      onCreated();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="unit-new-row">
      <label className="login-field">
        <span>New unit label</span>
        <input value={unitLabel} onChange={(e) => setUnitLabel(e.currentTarget.value)} />
      </label>
      <label className="login-field">
        <span>Factor to stock</span>
        <input inputMode="decimal" value={factorToStock} onChange={(e) => setFactorToStock(e.currentTarget.value)} />
      </label>
      <label className="login-field">
        <span>Sell price (R$)</span>
        <input inputMode="decimal" value={sellPrice} onChange={(e) => setSellPrice(e.currentTarget.value)} />
      </label>
      <label className="login-field login-field-inline">
        <input
          type="checkbox"
          checked={isDefaultSale}
          onChange={(e) => setIsDefaultSale(e.currentTarget.checked)}
        />
        <span>Default</span>
      </label>
      <button type="button" onClick={add} disabled={saving}>
        {saving ? "Adding..." : "Add unit"}
      </button>
      {error && <p className="login-error">{error}</p>}
    </div>
  );
}
