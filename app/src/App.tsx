import { useEffect, useState } from "react";
import { useAuth } from "./hooks/useAuth";
import { LoginScreen } from "./screens/Login/LoginScreen";
import { OpenCashRegister } from "./screens/CashRegister/OpenCashRegister";
import { CashRegisterPanel } from "./screens/CashRegister/CashRegisterPanel";
import { InventoryScreen } from "./screens/Inventory/InventoryScreen";
import { SaleScreen } from "./screens/Sale/SaleScreen";
import { CustomersScreen } from "./screens/Customers/CustomersScreen";
import { ReceivablesScreen } from "./screens/Receivables/ReceivablesScreen";
import { ReturnsScreen } from "./screens/Returns/ReturnsScreen";
import { PurchasingScreen } from "./screens/Purchasing/PurchasingScreen";
import { useCashSessionStore } from "./state/cashSessionStore";
import { useAppShell } from "./state/appShellStore";
import { getDb } from "./lib/db";
import { syncNow } from "./lib/api";
import "./App.css";

/** Manual "sync now" trigger (plan section 9's reconnection testing is a
 * lot easier with a button than waiting for the background worker's next
 * interval — see src-tauri/src/sync/mod.rs). */
function SyncStatus() {
  const [state, setState] = useState<
    { kind: "idle" } | { kind: "syncing" } | { kind: "done"; label: string } | { kind: "error"; message: string }
  >({ kind: "idle" });

  async function handleClick() {
    setState({ kind: "syncing" });
    try {
      const summary = await syncNow();
      setState({ kind: "done", label: `pushed ${summary.pushed}, pulled ${summary.pulled}` });
    } catch (err) {
      setState({ kind: "error", message: String(err) });
    }
  }

  return (
    <span className="sync-status">
      <button onClick={handleClick} disabled={state.kind === "syncing"}>
        {state.kind === "syncing" ? "Syncing..." : "Sync now"}
      </button>
      {state.kind === "done" && <span>{state.label}</span>}
      {state.kind === "error" && <span className="login-error">{state.message}</span>}
    </span>
  );
}

function App() {
  const { status, session, restore, logout } = useAuth();
  const [dbReady, setDbReady] = useState(false);

  const cashStatus = useCashSessionStore((s) => s.status);
  const cashSession = useCashSessionStore((s) => s.session);
  const loadCashSession = useCashSessionStore((s) => s.load);

  const { screen, setScreen } = useAppShell();

  useEffect(() => {
    // Migrations run on first connection; screens that invoke commands
    // reading this same pool (cash/inventory/sales) must wait for it.
    getDb()
      .catch((err) => console.error("failed to open local database", err))
      .finally(() => setDbReady(true));
    restore();
  }, [restore]);

  useEffect(() => {
    if (dbReady && session) loadCashSession(session.deviceId);
  }, [dbReady, session, loadCashSession]);

  if (status === "restoring" || !dbReady) {
    return (
      <main className="container">
        <p>Loading...</p>
      </main>
    );
  }

  if (status !== "authenticated" || !session) {
    return <LoginScreen />;
  }

  if (cashStatus === "loading" || cashStatus === "idle") {
    return (
      <main className="container">
        <p>Loading cash register...</p>
      </main>
    );
  }

  if (!cashSession) {
    return <OpenCashRegister />;
  }

  return (
    <main className="app-shell">
      <header className="shell-header">
        <nav className="shell-nav">
          <button
            className={screen === "sale" ? "shell-nav-active" : ""}
            onClick={() => setScreen("sale")}
          >
            Sale
          </button>
          <button
            className={screen === "inventory" ? "shell-nav-active" : ""}
            onClick={() => setScreen("inventory")}
          >
            Products
          </button>
          <button
            className={screen === "cash" ? "shell-nav-active" : ""}
            onClick={() => setScreen("cash")}
          >
            Cash register
          </button>
          <button
            className={screen === "customers" ? "shell-nav-active" : ""}
            onClick={() => setScreen("customers")}
          >
            Customers
          </button>
          <button
            className={screen === "receivables" ? "shell-nav-active" : ""}
            onClick={() => setScreen("receivables")}
          >
            Receivables
          </button>
          <button
            className={screen === "returns" ? "shell-nav-active" : ""}
            onClick={() => setScreen("returns")}
          >
            Returns
          </button>
          <button
            className={screen === "purchasing" ? "shell-nav-active" : ""}
            onClick={() => setScreen("purchasing")}
          >
            Purchasing
          </button>
        </nav>
        <SyncStatus />
        <span>
          Signed in as <strong>{session.operator.name}</strong>
        </span>
        <button onClick={() => logout()}>Sign out</button>
      </header>

      <section className="shell-body">
        {screen === "sale" && <SaleScreen />}
        {screen === "inventory" && <InventoryScreen />}
        {screen === "cash" && <CashRegisterPanel />}
        {screen === "customers" && <CustomersScreen />}
        {screen === "receivables" && <ReceivablesScreen />}
        {screen === "returns" && <ReturnsScreen />}
        {screen === "purchasing" && <PurchasingScreen />}
      </section>
    </main>
  );
}

export default App;
