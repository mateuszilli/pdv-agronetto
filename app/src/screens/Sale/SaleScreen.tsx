import { useEffect, useRef, useState } from "react";
import { useAuth } from "../../hooks/useAuth";
import { useCashSessionStore } from "../../state/cashSessionStore";
import { useCartStore, subtotalCents, lineTotalCents } from "../../state/cartStore";
import {
  type CustomerSummary,
  type PaymentMethod,
  type ProductSummary,
  type ProductUnitOption,
  type Sale,
  finalizeSale,
  getCustomer,
  listCustomers,
  listProducts,
  listProductUnits,
  printReceipt,
  retryFiscalEmission,
} from "../../lib/api";
import { centsToDisplay, parseCentsInput } from "../../lib/money";
import { formatQuantity, parseQuantityInput } from "../../lib/quantity";

const PAYMENT_METHODS: { key: PaymentMethod; label: string }[] = [
  { key: "cash", label: "Cash" },
  { key: "debit", label: "Debit" },
  { key: "credit", label: "Credit" },
  { key: "pix", label: "Pix" },
  { key: "boleto", label: "Boleto" },
  { key: "store_credit", label: "Store credit (fiado)" },
];

function methodLabel(method: PaymentMethod): string {
  return PAYMENT_METHODS.find((m) => m.key === method)?.label ?? method;
}

/** One row the operator has committed on the payment screen — plan section
 * 4.1: "the payment screen allows multiple methods on the same sale...
 * validating that the sum matches the total." Installments always default
 * to 1; there's no UI for installment count yet. */
interface PaymentEntry {
  method: PaymentMethod;
  amountCents: number;
}

/** What triggered the unit selector overlay (plan section 4.1's "fast
 * selector, numeric 1-2-3, no mouse required"): either adding a new item
 * whose product has more than one active selling unit, or F5 on an
 * existing cart line to switch its unit. */
type UnitSelector =
  | { mode: "add"; product: ProductSummary; units: ProductUnitOption[] }
  | { mode: "switch"; cartIndex: number; units: ProductUnitOption[] };

/**
 * Keyboard-first sale screen (plan section 4.1). The search field stays
 * focused; the barcode scanner behaves like fast typing + Enter into that
 * same field, no dedicated scanner mode needed. F3 identifies a customer
 * by name or CPF/CNPJ (plan section 4.3) — required for the store_credit
 * ("fiado") payment method, and attached to the sale either way.
 */
export function SaleScreen() {
  const { session: authSession } = useAuth();
  const cashSession = useCashSessionStore((s) => s.session);
  const {
    items,
    discountCents,
    selectedIndex,
    customerId,
    customerName,
    addProduct,
    removeItem,
    setQuantity,
    setUnit,
    setSelectedIndex,
    setDiscountCents,
    setCustomer,
    clear,
  } = useCartStore();

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<ProductSummary[]>([]);
  const [highlighted, setHighlighted] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const [unitSelector, setUnitSelector] = useState<UnitSelector | null>(null);
  const [unitHighlighted, setUnitHighlighted] = useState(0);

  const [qtyEdit, setQtyEdit] = useState<{ index: number; text: string } | null>(null);

  const [customerSelectorOpen, setCustomerSelectorOpen] = useState(false);
  const [customerQuery, setCustomerQuery] = useState("");
  const [customerResults, setCustomerResults] = useState<CustomerSummary[]>([]);
  const [customerHighlighted, setCustomerHighlighted] = useState(0);

  const [paymentStep, setPaymentStep] = useState(false);
  const [payments, setPayments] = useState<PaymentEntry[]>([]);
  const [currentMethod, setCurrentMethod] = useState<PaymentMethod>("cash");
  const [amountText, setAmountText] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [completedSale, setCompletedSale] = useState<Sale | null>(null);
  const [fiscalStatus, setFiscalStatus] = useState<Sale["fiscal_status"] | null>(null);
  const [fiscalRejectionReason, setFiscalRejectionReason] = useState<string | null>(null);
  const [printError, setPrintError] = useState<string | null>(null);
  const [retrying, setRetrying] = useState(false);
  /** Fetched when entering the payment step, if a customer is identified —
   * lets the store_credit option show a non-blocking credit-limit warning
   * (plan section 4.3: "not blocked by default ... just a warning"). */
  const [customerBalance, setCustomerBalance] = useState<{
    balanceOwedCents: number;
    creditLimitCents: number;
  } | null>(null);

  const searchRef = useRef<HTMLInputElement>(null);
  const discountRef = useRef<HTMLInputElement>(null);
  const paymentAmountRef = useRef<HTMLInputElement>(null);
  const customerSearchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!query || paymentStep) {
      setResults([]);
      return;
    }
    const handle = setTimeout(() => {
      listProducts(query)
        .then((r) => {
          setResults(r);
          setHighlighted(0);
        })
        .catch((err) => setError(String(err)));
    }, 120);
    return () => clearTimeout(handle);
  }, [query, paymentStep]);

  useEffect(() => {
    if (!customerSelectorOpen) {
      setCustomerResults([]);
      return;
    }
    const handle = setTimeout(() => {
      listCustomers(customerQuery || undefined)
        .then((r) => {
          setCustomerResults(r);
          setCustomerHighlighted(0);
        })
        .catch((err) => setError(String(err)));
    }, 120);
    return () => clearTimeout(handle);
  }, [customerQuery, customerSelectorOpen]);

  useEffect(() => {
    if (customerSelectorOpen) customerSearchRef.current?.focus();
  }, [customerSelectorOpen]);

  const subtotal = subtotalCents(items);
  const total = Math.max(0, subtotal - discountCents);
  const paidCents = payments.reduce((sum, p) => sum + p.amountCents, 0);
  const remainingCents = total - paidCents;

  function enterPaymentStep() {
    setPayments([]);
    setCurrentMethod("cash");
    setAmountText(centsToDisplay(total));
    setPaymentStep(true);
    if (customerId) {
      getCustomer(customerId)
        .then((c) =>
          setCustomerBalance({
            balanceOwedCents: c.balance_owed_cents,
            creditLimitCents: c.credit_limit_cents,
          }),
        )
        .catch(() => setCustomerBalance(null));
    } else {
      setCustomerBalance(null);
    }
  }

  function openCustomerSelector() {
    setCustomerQuery("");
    setCustomerSelectorOpen(true);
  }

  function selectCustomer(customer: CustomerSummary) {
    setCustomer({ id: customer.id, name: customer.name });
    setCustomerSelectorOpen(false);
    searchRef.current?.focus();
  }

  function handleCustomerSearchKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCustomerHighlighted((h) => Math.min(h + 1, customerResults.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCustomerHighlighted((h) => Math.max(h - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const customer = customerResults[customerHighlighted];
      if (customer) selectCustomer(customer);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setCustomerSelectorOpen(false);
      searchRef.current?.focus();
    }
  }

  function addPayment() {
    if (remainingCents <= 0) return;
    const parsed = parseCentsInput(amountText);
    if (parsed === null || parsed <= 0) {
      setError("enter a valid payment amount");
      return;
    }
    const amountCents = Math.min(parsed, remainingCents);
    const nextRemaining = remainingCents - amountCents;
    setPayments((prev) => [...prev, { method: currentMethod, amountCents }]);
    setAmountText(centsToDisplay(nextRemaining));
    setError(null);
    paymentAmountRef.current?.focus();
  }

  function removePayment(index: number) {
    setPayments((prev) => {
      const next = prev.filter((_, i) => i !== index);
      const nextPaid = next.reduce((sum, p) => sum + p.amountCents, 0);
      setAmountText(centsToDisplay(total - nextPaid));
      return next;
    });
  }

  function selectProduct(product: ProductSummary) {
    setQuery("");
    setResults([]);
    if (product.unit_count > 1) {
      listProductUnits(product.id)
        .then((units) => {
          setUnitHighlighted(0);
          setUnitSelector({ mode: "add", product, units });
        })
        .catch((err) => setError(String(err)));
      return;
    }
    addProduct(product);
  }

  function addHighlighted() {
    const product = results[highlighted];
    if (!product) return;
    selectProduct(product);
  }

  function handleSearchKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlighted((h) => Math.min(h + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlighted((h) => Math.max(h - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      addHighlighted();
    }
  }

  function confirmUnitSelection(index: number) {
    if (!unitSelector) return;
    const unit = unitSelector.units[index];
    if (!unit) return;
    if (unitSelector.mode === "add") {
      addProduct(unitSelector.product, unit);
    } else {
      setUnit(unitSelector.cartIndex, unit);
    }
    setUnitSelector(null);
    searchRef.current?.focus();
  }

  // Keyboard handling for the unit selector overlay: digits 1-9 jump
  // straight to that option, arrows move the highlight, Enter confirms.
  useEffect(() => {
    if (!unitSelector) return;
    function handleUnitKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        setUnitSelector(null);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setUnitHighlighted((h) => Math.min(h + 1, (unitSelector?.units.length ?? 1) - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setUnitHighlighted((h) => Math.max(h - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        confirmUnitSelection(unitHighlighted);
      } else if (/^[1-9]$/.test(e.key)) {
        e.preventDefault();
        confirmUnitSelection(parseInt(e.key, 10) - 1);
      }
    }
    window.addEventListener("keydown", handleUnitKeyDown);
    return () => window.removeEventListener("keydown", handleUnitKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [unitSelector, unitHighlighted]);

  useEffect(() => {
    function handleGlobalKeyDown(e: KeyboardEvent) {
      if (paymentStep || unitSelector || customerSelectorOpen) return;
      if (e.key === "F2") {
        e.preventDefault();
        searchRef.current?.focus();
      } else if (e.key === "F3") {
        e.preventDefault();
        openCustomerSelector();
      } else if (e.key === "F4") {
        e.preventDefault();
        discountRef.current?.focus();
      } else if (e.key === "F5") {
        e.preventDefault();
        if (selectedIndex < 0) return;
        const item = items[selectedIndex];
        listProductUnits(item.productId)
          .then((units) => {
            setUnitHighlighted(Math.max(0, units.findIndex((u) => u.id === item.unitId)));
            setUnitSelector({ mode: "switch", cartIndex: selectedIndex, units });
          })
          .catch((err) => setError(String(err)));
      } else if (e.key === "F8") {
        e.preventDefault();
        if (items.length === 0) {
          setError("add at least one item before finalizing");
          return;
        }
        setError(null);
        enterPaymentStep();
      } else if (e.key === "F9") {
        e.preventDefault();
        if (selectedIndex >= 0) removeItem(selectedIndex);
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (items.length > 0 && window.confirm("Cancel this sale?")) {
          clear();
          setQuery("");
        }
      }
    }
    window.addEventListener("keydown", handleGlobalKeyDown);
    return () => window.removeEventListener("keydown", handleGlobalKeyDown);
  }, [paymentStep, unitSelector, customerSelectorOpen, items, selectedIndex, removeItem, clear, total]);

  async function handleFinalize() {
    if (!authSession || !cashSession) return;
    if (remainingCents !== 0 || payments.length === 0) return;
    setSubmitting(true);
    setError(null);
    try {
      const sale = await finalizeSale({
        cash_session_id: cashSession.id,
        device_id: authSession.deviceId,
        operator_id: authSession.operator.id,
        customer_id: customerId,
        items: items.map((i) => ({
          product_id: i.productId,
          unit_id: i.unitId,
          quantity: i.quantity,
          unit_price_cents: i.unitPriceCents,
        })),
        discount_total_cents: discountCents,
        payments: payments.map((p) => ({
          method: p.method,
          amount_cents: p.amountCents,
          installments: 1,
        })),
      });
      setCompletedSale(sale);
      setFiscalStatus(sale.fiscal_status);
      setFiscalRejectionReason(null);
      setPrintError(null);
      clear();
      setPaymentStep(false);
      // Best-effort: the sale already succeeded regardless of whether this
      // print goes through (plan section 1.2 — a sale is never blocked by
      // anything downstream of finalizing it).
      printReceipt(sale.id).catch((err) => setPrintError(String(err)));
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  async function handleRetryFiscal() {
    if (!completedSale) return;
    setRetrying(true);
    try {
      const view = await retryFiscalEmission(completedSale.fiscal_document_id);
      setFiscalStatus(view.status as Sale["fiscal_status"]);
      setFiscalRejectionReason(view.rejection_reason);
      if (view.status === "authorized") {
        printReceipt(completedSale.id).catch((err) => setPrintError(String(err)));
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setRetrying(false);
    }
  }

  function handleReprint() {
    if (!completedSale) return;
    setPrintError(null);
    printReceipt(completedSale.id).catch((err) => setPrintError(String(err)));
  }

  // F8 finalizes the sale from the cart (enters the payment step) and, once
  // the operator has entered payments covering the full total, confirms it —
  // one key, same as the cart's "F8 finalize sale / go to payment" hint.
  useEffect(() => {
    if (!paymentStep) return;
    function handlePaymentKeyDown(e: KeyboardEvent) {
      if (e.key === "F8") {
        e.preventDefault();
        if (!submitting && remainingCents === 0 && payments.length > 0) handleFinalize();
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (!submitting) setPaymentStep(false);
      }
    }
    window.addEventListener("keydown", handlePaymentKeyDown);
    return () => window.removeEventListener("keydown", handlePaymentKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paymentStep, remainingCents, payments, submitting]);

  if (completedSale) {
    return (
      <div className="panel">
        <h2>Sale completed</h2>
        <p className="sale-total">Total: R$ {centsToDisplay(completedSale.total_cents)}</p>
        <ul className="payment-list">
          {completedSale.payments.map((p) => (
            <li key={p.id}>
              <span>{methodLabel(p.method)}</span>
              <span>R$ {centsToDisplay(p.amount_cents)}</span>
            </li>
          ))}
        </ul>

        {fiscalStatus === "authorized" && (
          <p className="fiscal-status fiscal-status-authorized">NFC-e authorized.</p>
        )}
        {fiscalStatus === "pending" && (
          <p className="fiscal-status fiscal-status-pending">
            Provisional receipt printed — NFC-e is queued for emission (contingency mode).
          </p>
        )}
        {fiscalStatus === "rejected" && (
          <p className="fiscal-status fiscal-status-rejected">
            NFC-e rejected: {fiscalRejectionReason ?? "no reason given"}. Fix the product's
            registration, then retry.
          </p>
        )}
        {printError && <p className="login-error">Printing failed: {printError}</p>}

        <div className="panel-actions">
          {fiscalStatus !== "authorized" && (
            <button onClick={handleRetryFiscal} disabled={retrying}>
              {retrying ? "Retrying..." : "Retry fiscal emission"}
            </button>
          )}
          <button onClick={handleReprint}>Reprint receipt</button>
        </div>

        <button
          autoFocus
          onClick={() => {
            setCompletedSale(null);
            setFiscalStatus(null);
            setFiscalRejectionReason(null);
            setPrintError(null);
            searchRef.current?.focus();
          }}
        >
          New sale
        </button>
      </div>
    );
  }

  if (paymentStep) {
    return (
      <div className="panel">
        <h2>Payment</h2>
        <p className="sale-total">Total: R$ {centsToDisplay(total)}</p>

        {payments.length > 0 && (
          <ul className="payment-list">
            {payments.map((p, i) => (
              <li key={i}>
                <span>{methodLabel(p.method)}</span>
                <span>R$ {centsToDisplay(p.amountCents)}</span>
                <button onClick={() => removePayment(i)} disabled={submitting}>
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}

        <p>
          Customer: {customerName ?? "none identified"}
          {customerBalance && (
            <>
              {" "}
              (balance R$ {centsToDisplay(customerBalance.balanceOwedCents)} / limit R${" "}
              {centsToDisplay(customerBalance.creditLimitCents)})
            </>
          )}
        </p>

        <p>Remaining: R$ {centsToDisplay(remainingCents)}</p>

        {remainingCents > 0 && (
          <>
            <div className="payment-methods">
              {PAYMENT_METHODS.map((m) => {
                const needsCustomer = m.key === "store_credit" && !customerId;
                return (
                  <button
                    key={m.key}
                    className={m.key === currentMethod ? "payment-method-selected" : ""}
                    onClick={() => setCurrentMethod(m.key)}
                    disabled={submitting || needsCustomer}
                    title={needsCustomer ? "Identify a customer first (F3)" : undefined}
                  >
                    {m.label}
                  </button>
                );
              })}
            </div>
            <label className="login-field login-field-inline">
              <span>Amount (R$)</span>
              <input
                ref={paymentAmountRef}
                type="text"
                inputMode="decimal"
                value={amountText}
                onChange={(e) => setAmountText(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addPayment();
                  }
                }}
                disabled={submitting}
              />
            </label>
            {currentMethod === "store_credit" &&
              customerBalance &&
              (() => {
                const enteredCents = parseCentsInput(amountText) ?? 0;
                const projectedCents = customerBalance.balanceOwedCents + enteredCents;
                if (projectedCents <= customerBalance.creditLimitCents) return null;
                return (
                  <p className="payment-warning">
                    This exceeds {customerName}&apos;s credit limit by R${" "}
                    {centsToDisplay(projectedCents - customerBalance.creditLimitCents)} — the
                    sale will still proceed (plan section 4.3: warning, not a block).
                  </p>
                );
              })()}
            <div className="panel-actions">
              <button onClick={addPayment} disabled={submitting}>
                Add payment
              </button>
            </div>
          </>
        )}

        {error && <p className="login-error">{error}</p>}
        <div className="panel-actions">
          <button onClick={() => setPaymentStep(false)} disabled={submitting}>
            Back to cart
          </button>
          <button
            onClick={handleFinalize}
            disabled={submitting || remainingCents !== 0 || payments.length === 0}
            autoFocus={remainingCents === 0}
          >
            {submitting ? "Finalizing..." : "Confirm payment (F8)"}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="sale-screen">
      <div className="sale-search">
        <input
          ref={searchRef}
          type="text"
          placeholder="Search product by name, SKU, or scan barcode..."
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={handleSearchKeyDown}
        />
        {results.length > 0 && (
          <ul className="search-results">
            {results.map((r, i) => (
              <li
                key={r.id}
                className={i === highlighted ? "search-result-highlighted" : ""}
                onClick={() => {
                  setHighlighted(i);
                  selectProduct(r);
                }}
              >
                <span>{r.name}</span>
                <span>R$ {centsToDisplay(r.sell_price_cents)}</span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="sale-customer-bar">
        {customerName ? (
          <>
            <span>
              Customer: <strong>{customerName}</strong>
            </span>
            <button type="button" onClick={() => setCustomer(null)}>
              Clear (F3)
            </button>
          </>
        ) : (
          <button type="button" onClick={openCustomerSelector}>
            Identify customer (F3)
          </button>
        )}
      </div>

      {customerSelectorOpen && (
        <div
          className="unit-selector-backdrop"
          onClick={() => setCustomerSelectorOpen(false)}
        >
          <div className="customer-selector" onClick={(e) => e.stopPropagation()}>
            <h3>Identify customer</h3>
            <input
              ref={customerSearchRef}
              type="text"
              placeholder="Search by name or CPF/CNPJ"
              value={customerQuery}
              onChange={(e) => setCustomerQuery(e.currentTarget.value)}
              onKeyDown={handleCustomerSearchKeyDown}
            />
            <ul>
              {customerResults.map((c, i) => (
                <li
                  key={c.id}
                  className={i === customerHighlighted ? "unit-selector-highlighted" : ""}
                  onClick={() => selectCustomer(c)}
                >
                  <span>
                    {c.name} ({c.doc_type} {c.doc_number})
                  </span>
                  <span>R$ {centsToDisplay(c.balance_owed_cents)}</span>
                </li>
              ))}
              {customerResults.length === 0 && (
                <li className="customer-selector-empty">No customers found.</li>
              )}
            </ul>
            <p className="unit-selector-hint">Enter to select, Esc to cancel.</p>
          </div>
        </div>
      )}

      {unitSelector && (
        <div className="unit-selector-backdrop" onClick={() => setUnitSelector(null)}>
          <div className="unit-selector" onClick={(e) => e.stopPropagation()}>
            <h3>
              {unitSelector.mode === "add" ? unitSelector.product.name : "Change selling unit"}
            </h3>
            {unitSelector.units.length === 0 ? (
              <p>This product has no active selling units.</p>
            ) : (
              <ul>
                {unitSelector.units.map((u, i) => (
                  <li
                    key={u.id}
                    className={i === unitHighlighted ? "unit-selector-highlighted" : ""}
                    onClick={() => confirmUnitSelection(i)}
                  >
                    <span>
                      <span className="unit-key">{i + 1}</span>
                      {u.unit_label}
                    </span>
                    <span>R$ {centsToDisplay(u.sell_price_cents)}</span>
                  </li>
                ))}
              </ul>
            )}
            <p className="unit-selector-hint">Press 1-9, or Esc to cancel.</p>
          </div>
        </div>
      )}

      <table className="cart-table">
        <thead>
          <tr>
            <th>Product</th>
            <th>Unit</th>
            <th>Qty</th>
            <th>Unit price</th>
            <th>Line total</th>
          </tr>
        </thead>
        <tbody>
          {items.map((item, i) => (
            <tr
              key={`${item.productId}-${item.unitId}`}
              className={i === selectedIndex ? "cart-row-selected" : ""}
              onClick={() => setSelectedIndex(i)}
            >
              <td>{item.name}</td>
              <td>{item.unitLabel}</td>
              <td>
                <input
                  type="text"
                  inputMode="decimal"
                  value={qtyEdit?.index === i ? qtyEdit.text : formatQuantity(item.quantity)}
                  onClick={(e) => e.stopPropagation()}
                  onFocus={() => setQtyEdit({ index: i, text: formatQuantity(item.quantity) })}
                  onChange={(e) => setQtyEdit({ index: i, text: e.currentTarget.value })}
                  onBlur={() => {
                    const parsed = qtyEdit && qtyEdit.index === i ? parseQuantityInput(qtyEdit.text) : null;
                    if (parsed !== null) setQuantity(i, parsed);
                    setQtyEdit(null);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      e.currentTarget.blur();
                    }
                  }}
                />
              </td>
              <td>R$ {centsToDisplay(item.unitPriceCents)}</td>
              <td>R$ {centsToDisplay(lineTotalCents(item))}</td>
            </tr>
          ))}
          {items.length === 0 && (
            <tr>
              <td colSpan={5} className="cart-empty">
                Cart is empty — search for a product above.
              </td>
            </tr>
          )}
        </tbody>
      </table>

      <div className="sale-totals">
        <label className="login-field login-field-inline">
          <span>Discount (R$)</span>
          <input
            ref={discountRef}
            type="text"
            inputMode="decimal"
            value={centsToDisplay(discountCents)}
            onChange={(e) => setDiscountCents(parseCentsInput(e.currentTarget.value) ?? 0)}
          />
        </label>
        <p>Subtotal: R$ {centsToDisplay(subtotal)}</p>
        <p className="sale-total">Total: R$ {centsToDisplay(total)}</p>
      </div>

      {error && <p className="login-error">{error}</p>}

      <footer className="sale-shortcuts">
        <span>F2 Search</span>
        <span>F3 Identify customer</span>
        <span>F4 Discount</span>
        <span>F5 Change unit</span>
        <span>F8 Finalize</span>
        <span>F9 Remove item</span>
        <span>ESC Cancel sale</span>
      </footer>
    </div>
  );
}
