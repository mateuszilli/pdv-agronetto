import { useState } from "react";
import { useAuth } from "../../hooks/useAuth";
import { useCashSessionStore } from "../../state/cashSessionStore";
import { parseCentsInput } from "../../lib/money";

/** Plan section 4.4: "Opening: operator enters the starting cash amount." */
export function OpenCashRegister() {
  const { session: authSession } = useAuth();
  const open = useCashSessionStore((s) => s.open);
  const load = useCashSessionStore((s) => s.load);
  // Surfaced separately from the local submit error: if the initial lookup
  // for an already-open session failed (rather than genuinely finding
  // none), this screen would otherwise show with no explanation at all,
  // and submitting would then hit the "already open" guard for a session
  // the operator never knew existed.
  const loadError = useCashSessionStore((s) => s.error);
  const [amount, setAmount] = useState("0.00");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!authSession) return;
    const cents = parseCentsInput(amount);
    if (cents === null || cents < 0) {
      setError("enter a valid opening amount");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await open(authSession.deviceId, authSession.operator.id, cents);
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <main className="login-screen">
      <form className="login-card" onSubmit={handleSubmit}>
        <h1>Open cash register</h1>
        <p className="login-subtitle">Enter the starting cash amount</p>

        {loadError && (
          <div className="login-error">
            <p>Could not check for an already-open session: {loadError}</p>
            <button
              type="button"
              onClick={() => authSession && load(authSession.deviceId)}
              disabled={submitting}
            >
              Retry check
            </button>
          </div>
        )}

        <label className="login-field">
          <span>Opening amount (R$)</span>
          <input
            type="text"
            inputMode="decimal"
            autoFocus
            value={amount}
            onChange={(e) => setAmount(e.currentTarget.value)}
            disabled={submitting}
          />
        </label>

        {error && <p className="login-error">{error}</p>}

        <button type="submit" disabled={submitting}>
          {submitting ? "Opening..." : "Open register"}
        </button>
      </form>
    </main>
  );
}
