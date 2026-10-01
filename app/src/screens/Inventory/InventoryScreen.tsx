import { useState } from "react";
import { ProductList } from "./ProductList";
import { ProductForm } from "./ProductForm";

type View = { mode: "list" } | { mode: "create" } | { mode: "edit"; id: string };

export function InventoryScreen() {
  const [view, setView] = useState<View>({ mode: "list" });

  if (view.mode === "list") {
    return (
      <ProductList
        onSelect={(id) => setView({ mode: "edit", id })}
        onCreate={() => setView({ mode: "create" })}
      />
    );
  }

  return (
    <ProductForm
      productId={view.mode === "edit" ? view.id : null}
      onDone={() => setView({ mode: "list" })}
      onCancel={() => setView({ mode: "list" })}
    />
  );
}
