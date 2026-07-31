import { useEffect } from "react";
import { useAuth } from "./hooks/useAuth";
import { LoginScreen } from "./screens/Login/LoginScreen";
import { getDb } from "./lib/db";
import "./App.css";

function App() {
  const { status, session, restore, logout } = useAuth();

  useEffect(() => {
    // Opens the local SQLite db, applying migrations on first run.
    getDb().catch((err) => console.error("failed to open local database", err));
    restore();
  }, [restore]);

  if (status === "restoring") {
    return (
      <main className="container">
        <p>Loading...</p>
      </main>
    );
  }

  if (status !== "authenticated" || !session) {
    return <LoginScreen />;
  }

  return (
    <main className="container">
      <header className="shell-header">
        <span>
          Signed in as <strong>{session.operator.name}</strong>
        </span>
        <button onClick={() => logout()}>Sign out</button>
      </header>
      <p>Authenticated shell — sale/cash/inventory screens arrive in later phases.</p>
    </main>
  );
}

export default App;
