import { useEffect, useState } from "react";
import { useAuth } from "../../hooks/useAuth";
import { getOrCreateDeviceId } from "../../lib/device";

export function LoginScreen() {
  const { login, status, error } = useAuth();
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [pin, setPin] = useState("");

  useEffect(() => {
    getOrCreateDeviceId().then(setDeviceId);
  }, []);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!deviceId) return;
    try {
      await login(deviceId, pin);
    } catch {
      // error is already surfaced via the store's `error` field
    }
  }

  const submitting = status === "authenticating";

  return (
    <main className="login-screen">
      <form className="login-card" onSubmit={handleSubmit}>
        <h1>PDV</h1>
        <p className="login-subtitle">Operator sign-in</p>

        <label className="login-field">
          <span>Operator PIN</span>
          <input
            type="password"
            inputMode="numeric"
            autoFocus
            value={pin}
            onChange={(e) => setPin(e.currentTarget.value)}
            disabled={submitting}
          />
        </label>

        {error && <p className="login-error">{error}</p>}

        <button type="submit" disabled={submitting || !deviceId || !pin}>
          {submitting ? "Signing in..." : "Sign in"}
        </button>

        <p className="login-device">
          Device: {deviceId ? deviceId.slice(0, 8) : "..."}
        </p>
      </form>
    </main>
  );
}
