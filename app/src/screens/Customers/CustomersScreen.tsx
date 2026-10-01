import { useState } from "react";
import { CustomerList } from "./CustomerList";
import { CustomerForm } from "./CustomerForm";

type View = { mode: "list" } | { mode: "create" } | { mode: "edit"; id: string };

/** CPF/CNPJ customer registry (plan section 4.3). Mirrors the Inventory
 * screen's list/form split. */
export function CustomersScreen() {
  const [view, setView] = useState<View>({ mode: "list" });

  if (view.mode === "list") {
    return (
      <CustomerList
        onSelect={(id) => setView({ mode: "edit", id })}
        onCreate={() => setView({ mode: "create" })}
      />
    );
  }

  return (
    <CustomerForm
      customerId={view.mode === "edit" ? view.id : null}
      onDone={() => setView({ mode: "list" })}
      onCancel={() => setView({ mode: "list" })}
    />
  );
}
