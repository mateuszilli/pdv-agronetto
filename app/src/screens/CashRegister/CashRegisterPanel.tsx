import { useEffect, useState } from "react";
import { useCashSessionStore } from "../../state/cashSessionStore";
import {
  type CashMovement,
  type CashMovementType,
  listCashMovements,
  recordCashMovement,
} from "../../lib/api";
import { centsToDisplay, parseCentsInput } from "../../lib/money";

/** Plan section 4.4: cash-out ("sangria") / cash-in ("suprimento") entries
 * during an open session, plus closing with a reported (never blocking)
 * discrepancy between expected and counted amounts. */
export function CashRegisterPanel() {
  const session = useCashSessionStore((s) => s.session);
  const close = useCashSessionStore((s) => s.close);

  const [movements, setMovements] = useState<CashMovement[]>([]);
  const [movementType, setMovementType] = useState<CashMovementType>("cash_out");
  const [amount, setAmount] = useState("");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const [closing, setClosing] = useState(false);
  const [countedAmount, setCountedAmount] = useState("0.00");
  const [closeResult, setCloseResult] = useState<{
    expectedCents: number;
    countedCents: number;
  } | null>(null);

  async function refreshMovements(sessionId: string) {
    setMovements(await listCashMovements(sessionId));
  }

  useEffect(() => {
    if (session) refreshMovements(session.id).catch((err) => setError(String(err)));
  }, [session]);

  if (!session) return null;

  async function handleAddMovement(e: React.FormEvent) {
    e.preventDefault();
    const cents = parseCentsInput(amount);
    if (cents === null || cents <= 0) {
      setError("enter a valid amount");
      return;
    }
    if (!reason.trim()) {
      setError("a reason is required");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await recordCashMovement(session!.id, movementType, cents, reason);
      setAmount("");
      setReason("");
      await refreshMovements(session!.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  async function handleClose(e: React.FormEvent) {
    e.preventDefault();
    const cents = parseCentsInput(countedAmount);
    if (cents === null || cents < 0) {
      setError("enter a valid counted amount");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const closed = await close(cents);
      setCloseResult({
        expectedCents: closed.expected_amount_cents ?? 0,
        countedCents: closed.closing_amount_cents ?? 0,
      });
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  if (closeResult) {
    const discrepancy = closeResult.countedCents - closeResult.expectedCents;
    return (
      <div className="panel">
        <h2>Register closed</h2>
        <p>Expected: R$ {centsToDisplay(closeResult.expectedCents)}</p>
        <p>Counted: R$ {centsToDisplay(closeResult.countedCents)}</p>
        <p className={discrepancy === 0 ? "" : "login-error"}>
          Discrepancy: R$ {centsToDisplay(discrepancy)}
        </p>
      </div>
    );
  }

  if (closing) {
    return (
      <form className="panel" onSubmit={handleClose}>
        <h2>Close cash register</h2>
        <label className="login-field">
          <span>Counted amount (R$)</span>
          <input
            type="text"
            inputMode="decimal"
            autoFocus
            value={countedAmount}
            onChange={(e) => setCountedAmount(e.currentTarget.value)}
            disabled={submitting}
          />
        </label>
        {error && <p className="login-error">{error}</p>}
        <div className="panel-actions">
          <button type="button" onClick={() => setClosing(false)} disabled={submitting}>
            Back
          </button>
          <button type="submit" disabled={submitting}>
            {submitting ? "Closing..." : "Confirm close"}
          </button>
        </div>
      </form>
    );
  }

  return (
    <div className="panel">
      <h2>Cash register</h2>
      <p>
        Opened at {new Date(session.opened_at).toLocaleString()} with R${" "}
        {centsToDisplay(session.opening_amount_cents)}
      </p>

      <form className="cash-movement-form" onSubmit={handleAddMovement}>
        <select
          value={movementType}
          onChange={(e) => setMovementType(e.currentTarget.value as CashMovementType)}
          disabled={submitting}
        >
          <option value="cash_out">Cash out (sangria)</option>
          <option value="cash_in">Cash in (suprimento)</option>
        </select>
        <input
          type="text"
          inputMode="decimal"
          placeholder="Amount"
          value={amount}
          onChange={(e) => setAmount(e.currentTarget.value)}
          disabled={submitting}
        />
        <input
          type="text"
          placeholder="Reason"
          value={reason}
          onChange={(e) => setReason(e.currentTarget.value)}
          disabled={submitting}
        />
        <button type="submit" disabled={submitting}>
          Add
        </button>
      </form>

      {error && <p className="login-error">{error}</p>}

      <ul className="cash-movement-list">
        {movements.map((m) => (
          <li key={m.id}>
            <span>{m.movement_type === "cash_in" ? "+" : "-"}</span>
            <span>R$ {centsToDisplay(m.amount_cents)}</span>
            <span>{m.reason}</span>
            <span>{new Date(m.created_at).toLocaleTimeString()}</span>
          </li>
        ))}
      </ul>

      <button onClick={() => setClosing(true)}>Close register</button>
    </div>
  );
}
