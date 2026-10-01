import { invoke } from "@tauri-apps/api/core";

export interface OperatorInfo {
  id: string;
  name: string;
}

export interface LoginResponse {
  token: string;
  expires_at: string;
  device_id: string;
  operator: OperatorInfo;
}

/** Invokes the `login` Tauri command (src-tauri/src/commands/auth.rs), which
 * calls the Go backend's `POST /auth/login`. */
export async function login(
  deviceId: string,
  operatorPin: string,
): Promise<LoginResponse> {
  return invoke<LoginResponse>("login", { deviceId, operatorPin });
}

// --- Inventory (src-tauri/src/commands/inventory.rs) ---

export interface ProductSummary {
  id: string;
  sku: string | null;
  barcode: string | null;
  name: string;
  stock_unit: string;
  active: boolean;
  unit_id: string;
  unit_label: string;
  sell_price_cents: number;
  factor_to_stock: number;
  unit_count: number;
  quantity: number;
}

export interface ProductUnitOption {
  id: string;
  product_id: string;
  unit_label: string;
  factor_to_stock: number;
  sell_price_cents: number;
  is_default_sale: boolean;
  active: boolean;
}

export interface ProductDetail extends ProductSummary {
  ncm: string;
  cfop_default: string | null;
  cst_csosn: string | null;
  cost_price_cents: number;
  min_quantity: number;
}

export interface ProductInput {
  sku: string | null;
  barcode: string | null;
  name: string;
  ncm: string;
  cfop_default: string | null;
  cst_csosn: string | null;
  stock_unit: string;
  cost_price_cents: number;
  unit_label: string;
  sell_price_cents: number;
  min_quantity: number;
  active: boolean;
}

export interface LowStockProduct {
  product_id: string;
  name: string;
  sku: string | null;
  stock_unit: string;
  quantity: number;
  min_quantity: number;
}

/** Products at or below their configured low-stock threshold (plan
 * section 4.6) — used by the product screen's alert banner and the
 * purchasing screen's reorder view. */
export async function listLowStockProducts(): Promise<LowStockProduct[]> {
  return invoke<LowStockProduct[]>("list_low_stock_products");
}

export async function listProducts(query?: string): Promise<ProductSummary[]> {
  return invoke<ProductSummary[]>("list_products", { query: query ?? null });
}

export async function getProduct(id: string): Promise<ProductDetail> {
  return invoke<ProductDetail>("get_product", { id });
}

export async function createProduct(
  deviceId: string,
  input: ProductInput,
): Promise<ProductDetail> {
  return invoke<ProductDetail>("create_product", { deviceId, input });
}

export async function updateProduct(
  deviceId: string,
  id: string,
  input: ProductInput,
): Promise<ProductDetail> {
  return invoke<ProductDetail>("update_product", { deviceId, id, input });
}

/** Active units only — used by the sale screen's fast unit selector. */
export async function listProductUnits(productId: string): Promise<ProductUnitOption[]> {
  return invoke<ProductUnitOption[]>("list_product_units", { productId });
}

/** Active and inactive units — used by the product form's unit manager. */
export async function listAllProductUnits(productId: string): Promise<ProductUnitOption[]> {
  return invoke<ProductUnitOption[]>("list_all_product_units", { productId });
}

export async function createProductUnit(
  deviceId: string,
  productId: string,
  unitLabel: string,
  factorToStock: number,
  sellPriceCents: number,
  isDefaultSale: boolean,
): Promise<ProductUnitOption> {
  return invoke<ProductUnitOption>("create_product_unit", {
    deviceId,
    productId,
    unitLabel,
    factorToStock,
    sellPriceCents,
    isDefaultSale,
  });
}

export async function updateProductUnit(
  deviceId: string,
  id: string,
  unitLabel: string,
  factorToStock: number,
  sellPriceCents: number,
  isDefaultSale: boolean,
  active: boolean,
): Promise<ProductUnitOption> {
  return invoke<ProductUnitOption>("update_product_unit", {
    deviceId,
    id,
    unitLabel,
    factorToStock,
    sellPriceCents,
    isDefaultSale,
    active,
  });
}

// --- Cash register (src-tauri/src/commands/cash.rs) ---

export interface CashSession {
  id: string;
  device_id: string;
  operator_id: string;
  opening_amount_cents: number;
  closing_amount_cents: number | null;
  expected_amount_cents: number | null;
  status: "open" | "closed";
  opened_at: string;
  closed_at: string | null;
}

export type CashMovementType = "cash_in" | "cash_out";

export interface CashMovement {
  id: string;
  cash_session_id: string;
  movement_type: CashMovementType;
  amount_cents: number;
  reason: string | null;
  created_at: string;
}

export async function getOpenCashSession(
  deviceId: string,
): Promise<CashSession | null> {
  return invoke<CashSession | null>("get_open_cash_session", { deviceId });
}

export async function openCashSession(
  deviceId: string,
  operatorId: string,
  openingAmountCents: number,
): Promise<CashSession> {
  return invoke<CashSession>("open_cash_session", {
    deviceId,
    operatorId,
    openingAmountCents,
  });
}

export async function recordCashMovement(
  cashSessionId: string,
  movementType: CashMovementType,
  amountCents: number,
  reason: string,
): Promise<CashMovement> {
  return invoke<CashMovement>("record_cash_movement", {
    cashSessionId,
    movementType,
    amountCents,
    reason,
  });
}

export async function listCashMovements(
  cashSessionId: string,
): Promise<CashMovement[]> {
  return invoke<CashMovement[]>("list_cash_movements", { cashSessionId });
}

export async function closeCashSession(
  cashSessionId: string,
  countedAmountCents: number,
): Promise<CashSession> {
  return invoke<CashSession>("close_cash_session", {
    cashSessionId,
    countedAmountCents,
  });
}

// --- Sales (src-tauri/src/commands/sales.rs) ---

/** `store_credit` (fiado) requires an identified customer — plan section
 * 4.3 — enforced both here (the sale screen only offers it once a
 * customer is identified via F3) and again in the backend command. */
export type PaymentMethod =
  | "cash"
  | "debit"
  | "credit"
  | "pix"
  | "boleto"
  | "store_credit";

export interface SaleItemInput {
  product_id: string;
  unit_id: string;
  quantity: number;
  unit_price_cents: number;
}

export interface SalePaymentInput {
  method: PaymentMethod;
  amount_cents: number;
  installments: number;
}

export interface FinalizeSaleInput {
  cash_session_id: string;
  device_id: string;
  operator_id: string;
  customer_id: string | null;
  items: SaleItemInput[];
  discount_total_cents: number;
  payments: SalePaymentInput[];
}

export interface SalePayment {
  id: string;
  method: PaymentMethod;
  amount_cents: number;
  installments: number;
}

/** `"authorized"`, `"rejected"`, or `"pending"` (queued for the
 * background worker/manual retry — either no connectivity at the moment
 * of sale, or no fiscal provider configured yet; the UI treats both the
 * same, per plan section 5.2's contingency mode). */
export type FiscalStatus = "authorized" | "rejected" | "pending";

export interface Sale {
  id: string;
  cash_session_id: string;
  status: string;
  subtotal_cents: number;
  discount_total_cents: number;
  total_cents: number;
  device_id: string;
  operator_id: string;
  customer_id: string | null;
  created_at: string;
  payments: SalePayment[];
  fiscal_document_id: string;
  fiscal_status: FiscalStatus;
}

export async function finalizeSale(input: FinalizeSaleInput): Promise<Sale> {
  return invoke<Sale>("finalize_sale", { input });
}

// --- Fiscal (src-tauri/src/commands/fiscal.rs) ---

export interface FiscalDocumentView {
  id: string;
  sale_id: string;
  status: string;
  access_key: string | null;
  xml_url: string | null;
  danfe_url: string | null;
  rejection_reason: string | null;
  /** Set when a return brought this sale to fully returned but the
   * resulting whole-document cancellation didn't succeed (past the
   * 30-minute window, rejected by SEFAZ, or a connectivity failure) —
   * `status` stays `"authorized"` in that case. `null` once cancellation
   * actually succeeds (`status` becomes `"cancelled"` instead) or if it
   * was never relevant to this document. */
  cancellation_note: string | null;
  attempts: number;
  created_at: string;
  updated_at: string;
}

export async function getFiscalStatus(saleId: string): Promise<FiscalDocumentView | null> {
  return invoke<FiscalDocumentView | null>("get_fiscal_status", { saleId });
}

/** Manual re-emit after the operator fixes whatever a rejection called
 * out (e.g. a product's NCM) — plan section 5.1. */
export async function retryFiscalEmission(
  fiscalDocumentId: string,
): Promise<FiscalDocumentView> {
  return invoke<FiscalDocumentView>("retry_fiscal_emission", { fiscalDocumentId });
}

/** Outcome of a whole-document NFC-e cancellation attempt (plan section
 * 4.5, implemented per Focus NFe's actual contract — see
 * `fiscal_client::try_cancel_for_return`'s doc comment on the Rust side).
 * `"cancelled"`/`"failed"`/`"skipped"` only ever apply to a return that
 * brought its sale to fully `"returned"`; a partial return never attempts
 * this and won't produce one of these values. */
export type CancellationStatus = "not_applicable" | "cancelled" | "skipped" | "failed";

/** Manual retry for a full return whose automatic cancellation attempt
 * (fired by `createReturn` itself) didn't succeed — as long as the
 * 30-minute window from the original authorization hasn't closed yet. */
export async function retryFiscalCancellation(saleId: string): Promise<CancellationStatus> {
  return invoke<CancellationStatus>("retry_fiscal_cancellation", { saleId });
}

/** Renders and sends the receipt to the configured printer, based on the
 * sale's current fiscal status — the same command doubles as "reprint"
 * once a queued sale gets authorized in the background (plan section
 * 5.2/9). */
export async function printReceipt(saleId: string): Promise<void> {
  return invoke<void>("print_receipt", { saleId });
}

// --- Customers and store credit (src-tauri/src/commands/customers.rs) ---

export type DocType = "CPF" | "CNPJ";

export interface CustomerSummary {
  id: string;
  doc_type: DocType;
  doc_number: string;
  name: string;
  phone: string | null;
  credit_limit_cents: number;
  balance_owed_cents: number;
  active: boolean;
}

export interface CustomerDetail extends CustomerSummary {
  address_json: string | null;
  default_due_days: number;
  created_at: string;
}

export interface CustomerInput {
  doc_type: DocType;
  doc_number: string;
  name: string;
  phone: string | null;
  address_json: string | null;
  credit_limit_cents: number;
  default_due_days: number;
  active: boolean;
}

/** Searches customers by name or doc number — used by both the registry
 * screen and the sale screen's F3 "identify customer" overlay. */
export async function listCustomers(query?: string): Promise<CustomerSummary[]> {
  return invoke<CustomerSummary[]>("list_customers", { query: query ?? null });
}

/** Customers with a positive outstanding balance (accounts-receivable
 * screen, plan section 4.3), highest balance first. */
export async function listReceivables(): Promise<CustomerSummary[]> {
  return invoke<CustomerSummary[]>("list_receivables");
}

export async function getCustomer(id: string): Promise<CustomerDetail> {
  return invoke<CustomerDetail>("get_customer", { id });
}

export async function createCustomer(
  deviceId: string,
  input: CustomerInput,
): Promise<CustomerDetail> {
  return invoke<CustomerDetail>("create_customer", { deviceId, input });
}

export async function updateCustomer(
  deviceId: string,
  id: string,
  input: CustomerInput,
): Promise<CustomerDetail> {
  return invoke<CustomerDetail>("update_customer", { deviceId, id, input });
}

export type LedgerEntryType = "debit" | "credit";

export interface LedgerEntry {
  id: string;
  entry_type: LedgerEntryType;
  amount_cents: number;
  due_date: string | null;
  reference_sale_id: string | null;
  notes: string | null;
  created_at: string;
}

export async function listLedgerEntries(customerId: string): Promise<LedgerEntry[]> {
  return invoke<LedgerEntry[]>("list_ledger_entries", { customerId });
}

/** Records a partial (or full) payment against a customer's balance — a
 * `credit` ledger entry. Printing the receipt is deferred to Phase 6. */
export async function recordPayment(
  customerId: string,
  deviceId: string,
  amountCents: number,
  notes: string | null,
): Promise<LedgerEntry> {
  return invoke<LedgerEntry>("record_payment", {
    customerId,
    deviceId,
    amountCents,
    notes,
  });
}

// --- Sync (src-tauri/src/sync/mod.rs) ---

export interface SyncSummary {
  pushed: number;
  pulled: number;
}

/** Runs one push/pull cycle immediately, instead of waiting for the
 * background worker's next interval — mainly for a status indicator and
 * for testing reconnection (plan section 9). */
export async function syncNow(): Promise<SyncSummary> {
  return invoke<SyncSummary>("sync_now");
}

// --- Returns (src-tauri/src/commands/returns.rs) ---

export interface SaleSearchInput {
  sale_id_query?: string | null;
  access_key?: string | null;
  customer_doc_number?: string | null;
  date_from?: string | null;
  date_to?: string | null;
}

export interface SaleSearchResult {
  id: string;
  status: string;
  total_cents: number;
  created_at: string;
  customer_name: string | null;
  customer_doc_number: string | null;
  access_key: string | null;
}

export async function searchSalesForReturn(input: SaleSearchInput): Promise<SaleSearchResult[]> {
  return invoke<SaleSearchResult[]>("search_sales_for_return", { input });
}

export interface ReturnableSaleItem {
  sale_item_id: string;
  product_id: string;
  product_name: string;
  unit_id: string;
  unit_label: string;
  quantity: number;
  quantity_returned: number;
  quantity_returnable: number;
  unit_price_cents: number;
  line_total_cents: number;
}

export interface SaleForReturn {
  id: string;
  status: string;
  customer_id: string | null;
  customer_name: string | null;
  total_cents: number;
  created_at: string;
  items: ReturnableSaleItem[];
}

export async function getSaleForReturn(saleId: string): Promise<SaleForReturn> {
  return invoke<SaleForReturn>("get_sale_for_return", { saleId });
}

export type RefundMethod = "cash" | "debit" | "credit" | "pix" | "store_credit";

export interface ReturnItemInput {
  sale_item_id: string;
  quantity: number;
}

export interface CreateReturnInput {
  original_sale_id: string;
  device_id: string;
  operator_id: string;
  reason: string | null;
  refund_method: RefundMethod;
  items: ReturnItemInput[];
}

export interface ReturnResult {
  id: string;
  original_sale_id: string;
  total_refunded_cents: number;
  refund_method: RefundMethod;
  sale_status: string;
  created_at: string;
  /** `null` unless this return brought the sale to fully `"returned"` —
   * only then is a whole-document NFC-e cancellation even attempted. */
  fiscal_cancellation: CancellationStatus | null;
}

export async function createReturn(input: CreateReturnInput): Promise<ReturnResult> {
  return invoke<ReturnResult>("create_return", { input });
}

// --- Purchasing (src-tauri/src/commands/purchasing.rs) ---

export type PurchaseOrderStatus = "draft" | "sent" | "partially_received" | "received";

export interface PurchaseOrderSummary {
  id: string;
  supplier_name: string;
  status: PurchaseOrderStatus;
  created_at: string;
  item_count: number;
  total_cost_cents: number;
}

export async function listPurchaseOrders(): Promise<PurchaseOrderSummary[]> {
  return invoke<PurchaseOrderSummary[]>("list_purchase_orders");
}

export interface PurchaseOrderItemDetail {
  id: string;
  product_id: string;
  product_name: string;
  stock_unit: string;
  quantity_ordered: number;
  quantity_received: number;
  unit_cost_cents: number;
}

export interface PurchaseOrderDetail {
  id: string;
  supplier_name: string;
  status: PurchaseOrderStatus;
  created_at: string;
  items: PurchaseOrderItemDetail[];
}

export async function getPurchaseOrder(id: string): Promise<PurchaseOrderDetail> {
  return invoke<PurchaseOrderDetail>("get_purchase_order", { id });
}

export interface PurchaseOrderItemInput {
  product_id: string;
  quantity_ordered: number;
  unit_cost_cents: number;
}

export interface CreatePurchaseOrderInput {
  device_id: string;
  supplier_name: string;
  items: PurchaseOrderItemInput[];
}

export async function createPurchaseOrder(
  input: CreatePurchaseOrderInput,
): Promise<PurchaseOrderDetail> {
  return invoke<PurchaseOrderDetail>("create_purchase_order", { input });
}

export interface ReceiveItemInput {
  purchase_order_item_id: string;
  quantity: number;
}

export interface ReceivePurchaseOrderInput {
  device_id: string;
  purchase_order_id: string;
  items: ReceiveItemInput[];
}

export async function receivePurchaseOrderItems(
  input: ReceivePurchaseOrderInput,
): Promise<PurchaseOrderDetail> {
  return invoke<PurchaseOrderDetail>("receive_purchase_order_items", { input });
}
