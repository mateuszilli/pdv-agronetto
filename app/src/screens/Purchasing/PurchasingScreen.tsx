import { useState } from "react";
import { PurchaseOrderList } from "./PurchaseOrderList";
import { CreatePurchaseOrder } from "./CreatePurchaseOrder";
import { PurchaseOrderDetail } from "./PurchaseOrderDetail";

type View = { mode: "list" } | { mode: "create" } | { mode: "detail"; id: string };

/** Purchasing/receiving screen (plan sections 3.1 and 4.6, roadmap Phase
 * 8): simple purchase orders against a supplier, with a detail view to
 * record goods received (full or partial) and drive the resulting stock
 * movements. */
export function PurchasingScreen() {
  const [view, setView] = useState<View>({ mode: "list" });

  if (view.mode === "create") {
    return (
      <CreatePurchaseOrder
        onCreated={(id) => setView({ mode: "detail", id })}
        onCancel={() => setView({ mode: "list" })}
      />
    );
  }

  if (view.mode === "detail") {
    return (
      <PurchaseOrderDetail id={view.id} onBack={() => setView({ mode: "list" })} />
    );
  }

  return (
    <PurchaseOrderList
      onSelect={(id) => setView({ mode: "detail", id })}
      onCreate={() => setView({ mode: "create" })}
    />
  );
}
