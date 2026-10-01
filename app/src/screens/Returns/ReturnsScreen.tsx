import { useState } from "react";
import { SaleSearch } from "./SaleSearch";
import { ItemSelection } from "./ItemSelection";
import { type CancellationStatus, type ReturnResult, retryFiscalCancellation } from "../../lib/api";
import { centsToDisplay } from "../../lib/money";

type View =
  | { mode: "search" }
  | { mode: "select-items"; saleId: string }
  | { mode: "done"; result: ReturnResult };

/** Short operator-facing message for each `CancellationStatus` the
 * automatic attempt (or a manual retry) can land on — mirrors the
 * Portuguese notes `fiscal_client::try_cancel_for_return` writes to
 * `fiscal_documents.cancellation_note` on the Rust side, but in English
 * for this screen's own copy. */
const CANCELLATION_MESSAGES: Record<CancellationStatus, string> = {
  cancelled: "NFC-e cancelled.",
  skipped: "NFC-e not cancelled (past the 30-minute window, or no fiscal provider configured).",
  failed: "NFC-e cancellation failed (rejected by SEFAZ, or a connectivity error).",
  not_applicable: "No authorized NFC-e to cancel for this sale.",
};

/** Returns screen (plan section 4.5): search for the original sale,
 * select which items (and how much of each) are being returned, and
 * confirm — one screen per step, mirroring the Receivables screen's
 * local-state router. */
export function ReturnsScreen() {
  const [view, setView] = useState<View>({ mode: "search" });

  if (view.mode === "search") {
    return <SaleSearch onSelect={(saleId) => setView({ mode: "select-items", saleId })} />;
  }

  if (view.mode === "select-items") {
    return (
      <ItemSelection
        saleId={view.saleId}
        onBack={() => setView({ mode: "search" })}
        onDone={(result) => setView({ mode: "done", result })}
      />
    );
  }

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Return recorded</h2>
      </div>
      <p>
        Refunded R$ {centsToDisplay(view.result.total_refunded_cents)} via{" "}
        {view.result.refund_method}.
      </p>
      <p>Sale status: {view.result.sale_status}</p>
      {view.result.fiscal_cancellation && (
        <FiscalCancellationNote
          saleId={view.result.original_sale_id}
          status={view.result.fiscal_cancellation}
        />
      )}
      <button onClick={() => setView({ mode: "search" })}>New return</button>
    </div>
  );
}

/** Shows the whole-document NFC-e cancellation outcome for a full return,
 * with a manual retry for `"skipped"`/`"failed"` — a transient
 * connectivity blip, or a rejection the operator wants to try again for,
 * can still succeed as long as the 30-minute window from the original
 * authorization hasn't fully closed (plan section 4.5; see
 * `fiscal_client::try_cancel_for_return`'s doc comment on the Rust side
 * for why there's no automatic background retry). */
function FiscalCancellationNote({ saleId, status }: { saleId: string; status: CancellationStatus }) {
  const [current, setCurrent] = useState(status);
  const [retrying, setRetrying] = useState(false);

  async function handleRetry() {
    setRetrying(true);
    try {
      setCurrent(await retryFiscalCancellation(saleId));
    } finally {
      setRetrying(false);
    }
  }

  return (
    <p>
      {CANCELLATION_MESSAGES[current]}{" "}
      {(current === "skipped" || current === "failed") && (
        <button onClick={handleRetry} disabled={retrying}>
          {retrying ? "Retrying..." : "Retry cancellation"}
        </button>
      )}
    </p>
  );
}
