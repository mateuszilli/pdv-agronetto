import { useEffect, useState } from "react";
import {
  type CustomerDetail,
  type CustomerInput,
  type DocType,
  createCustomer,
  getCustomer,
  updateCustomer,
} from "../../lib/api";
import { useAuth } from "../../hooks/useAuth";
import { centsToDisplay, parseCentsInput } from "../../lib/money";

interface CustomerFormProps {
  customerId: string | null;
  onDone: () => void;
  onCancel: () => void;
}

const emptyForm = {
  docType: "CPF" as DocType,
  docNumber: "",
  name: "",
  phone: "",
  addressJson: "",
  creditLimit: "0.00",
  defaultDueDays: "30",
  active: true,
};

/** Create/edit form for the CPF/CNPJ customer registry (plan section 4.3). */
export function CustomerForm({ customerId, onDone, onCancel }: CustomerFormProps) {
  const { session } = useAuth();
  const [form, setForm] = useState(emptyForm);
  const [loading, setLoading] = useState(customerId !== null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [balanceOwedCents, setBalanceOwedCents] = useState<number | null>(null);

  useEffect(() => {
    if (!customerId) return;
    setLoading(true);
    getCustomer(customerId)
      .then((c: CustomerDetail) => {
        setForm({
          docType: c.doc_type,
          docNumber: c.doc_number,
          name: c.name,
          phone: c.phone ?? "",
          addressJson: c.address_json ?? "",
          creditLimit: centsToDisplay(c.credit_limit_cents),
          defaultDueDays: String(c.default_due_days),
          active: c.active,
        });
        setBalanceOwedCents(c.balance_owed_cents);
      })
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }, [customerId]);

  function set<K extends keyof typeof form>(key: K, value: (typeof form)[K]) {
    setForm((f) => ({ ...f, [key]: value }));
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const creditLimitCents = parseCentsInput(form.creditLimit);
    const defaultDueDays = parseInt(form.defaultDueDays, 10);
    if (!form.name.trim() || !form.docNumber.trim()) {
      setError("name and doc number are required");
      return;
    }
    if (creditLimitCents === null || creditLimitCents < 0) {
      setError("enter a valid credit limit");
      return;
    }
    if (!Number.isFinite(defaultDueDays) || defaultDueDays < 0) {
      setError("enter a valid number of due days");
      return;
    }

    const input: CustomerInput = {
      doc_type: form.docType,
      doc_number: form.docNumber,
      name: form.name,
      phone: form.phone || null,
      address_json: form.addressJson || null,
      credit_limit_cents: creditLimitCents,
      default_due_days: defaultDueDays,
      active: form.active,
    };

    if (!session) return;

    setSubmitting(true);
    setError(null);
    try {
      if (customerId) {
        await updateCustomer(session.deviceId, customerId, input);
      } else {
        await createCustomer(session.deviceId, input);
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
      <h2>{customerId ? "Edit customer" : "New customer"}</h2>

      <label className="login-field">
        <span>Name</span>
        <input value={form.name} onChange={(e) => set("name", e.currentTarget.value)} autoFocus />
      </label>

      <div className="form-row">
        <label className="login-field">
          <span>Doc type</span>
          <select
            value={form.docType}
            onChange={(e) => set("docType", e.currentTarget.value as DocType)}
          >
            <option value="CPF">CPF</option>
            <option value="CNPJ">CNPJ</option>
          </select>
        </label>
        <label className="login-field">
          <span>Doc number</span>
          <input
            value={form.docNumber}
            onChange={(e) => set("docNumber", e.currentTarget.value)}
          />
        </label>
        <label className="login-field">
          <span>Phone</span>
          <input value={form.phone} onChange={(e) => set("phone", e.currentTarget.value)} />
        </label>
      </div>

      <div className="form-row">
        <label className="login-field">
          <span>Credit limit (R$)</span>
          <input
            inputMode="decimal"
            value={form.creditLimit}
            onChange={(e) => set("creditLimit", e.currentTarget.value)}
          />
        </label>
        <label className="login-field">
          <span>Fiado due days</span>
          <input
            inputMode="numeric"
            value={form.defaultDueDays}
            onChange={(e) => set("defaultDueDays", e.currentTarget.value)}
          />
        </label>
      </div>

      {balanceOwedCents !== null && (
        <p className="unit-panel-hint">
          Current balance owed: R$ {centsToDisplay(balanceOwedCents)}
        </p>
      )}

      {customerId && (
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
    </form>
  );
}
