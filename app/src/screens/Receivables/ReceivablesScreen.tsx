import { useState } from "react";
import { ReceivablesList } from "./ReceivablesList";
import { CustomerLedger } from "./CustomerLedger";

type View = { mode: "list" } | { mode: "detail"; id: string };

/** Accounts-receivable screen (plan section 4.3): customers with an
 * outstanding "fiado" balance, with a detail view to record payments. */
export function ReceivablesScreen() {
  const [view, setView] = useState<View>({ mode: "list" });

  if (view.mode === "list") {
    return <ReceivablesList onSelect={(id) => setView({ mode: "detail", id })} />;
  }

  return <CustomerLedger customerId={view.id} onBack={() => setView({ mode: "list" })} />;
}
