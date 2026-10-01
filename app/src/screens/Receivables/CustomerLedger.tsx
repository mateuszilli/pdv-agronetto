import { useEffect, useState } from "react";
import { useAuth } from "../../hooks/useAuth";
import {
  type CustomerDetail,
  type LedgerEntry,
  getCustomer,
  listLedgerEntries,
  recordPayment,
} from "../../lib/api";
import { centsToDisplay, parseCentsInput } from "../../lib/money";

interface CustomerLedgerProps {
  customerId: string;
  onBack: () => void;
}

/** A customer's store-credit ledger, plus the "record a partial payment"
 * form (plan section 4.3). Printing the payment receipt is Phase 6 scope. */
export function CustomerLedger({ customerId, onBack }: CustomerLedgerProps) {
  const { session } = useAuth();
  const [customer, setCustomer] = useState<CustomerDetail | null>(null);
  const [entries, setEntries] = useState<LedgerEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const [amount, setAmount] = useState("");
  const [notes, setNotes] = useState("");
  const [submitting, setSubmitting] = useState(false);

  function reload() {
    setLoading(true);
    return Promise.all([getCustomer(customerId), listLedgerEntries(customerId)])
      .then(([c, e]) => {
        setCustomer(c);
        setEntries(e);
      })
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [customerId]);

  async function handleRecordPayment(e: React.FormEvent) {
    e.preventDefault();
    if (!session) return;
    const cents = parseCentsInput(amount);
    if (cents === null || cents <= 0) {
      setError("enter a valid payment amount");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await recordPayment(customerId, session.deviceId, cents, notes || null);
      setAmount("");
      setNotes("");
      await reload();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  if (loading) return <p>Loading...</p>;
  if (!customer) return <p className="login-error">{error}</p>;

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>{customer.name}</h2>
        <button onClick={onBack}>Back</button>
      </div>
      <p>
        {customer.doc_type} {customer.doc_number}
      </p>
      <p>
        Balance owed:{" "}
        <strong className={customer.balance_owed_cents > customer.credit_limit_cents ? "login-error" : ""}>
          R$ {centsToDisplay(customer.balance_owed_cents)}
        </strong>{" "}
        / limit R$ {centsToDisplay(customer.credit_limit_cents)}
      </p>

      <form className="cash-movement-form" onSubmit={handleRecordPayment}>
        <input
          type="text"
          inputMode="decimal"
          placeholder="Payment amount"
          value={amount}
          onChange={(e) => setAmount(e.currentTarget.value)}
          disabled={submitting}
        />
        <input
          type="text"
          placeholder="Notes (optional)"
          value={notes}
          onChange={(e) => setNotes(e.currentTarget.value)}
          disabled={submitting}
        />
        <button type="submit" disabled={submitting}>
          {submitting ? "Recording..." : "Record payment"}
        </button>
      </form>

      {error && <p className="login-error">{error}</p>}

      <ul className="cash-movement-list">
        {entries.map((entry) => (
          <li key={entry.id}>
            <span>{entry.entry_type === "debit" ? "Fiado sale" : "Payment"}</span>
            <span>
              {entry.entry_type === "debit" ? "+" : "-"}R$ {centsToDisplay(entry.amount_cents)}
            </span>
            <span>{entry.due_date ? `due ${new Date(entry.due_date).toLocaleDateString()}` : ""}</span>
            <span>{entry.notes ?? ""}</span>
            <span>{new Date(entry.created_at).toLocaleString()}</span>
          </li>
        ))}
        {entries.length === 0 && <li>No ledger entries yet.</li>}
      </ul>
    </div>
  );
}
