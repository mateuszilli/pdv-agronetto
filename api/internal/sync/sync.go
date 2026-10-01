// Package sync implements the outbox push/pull protocol (plan sections
// 1.3 and 3.5): devices POST batches of outbox_events to /sync/push
// (idempotent on event_id) and GET /sync/pull?since=<cursor> for changes
// from other devices.
//
// DEVIATION FROM THE PLAN: section 3's schema names four outbox entity
// types (sale, stock_movement, customer_ledger, cash_movement), but
// central Postgres enforces foreign keys from those tables onto products,
// product_units, customers, and cash_sessions — none of which the plan
// lists as outbox entities. Without syncing them too, the very first sale
// pushed from a real device would violate a foreign key centrally (its
// product/customer/cash_session would never have arrived). This package
// therefore also applies "product", "product_unit", "customer", and
// "cash_session" events, mirrored on the client in
// app/src-tauri/src/commands/{inventory,customers,cash}.rs.
package sync

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/mzillipolli/pdv-backend/internal/auth"
)

const pullBatchSize = 500
const epoch = "1970-01-01T00:00:00Z"

// Handler holds the dependencies sync endpoints need.
type Handler struct {
	pool *pgxpool.Pool
}

func NewHandler(pool *pgxpool.Pool) *Handler {
	return &Handler{pool: pool}
}

// outboxEvent mirrors the outbox_events table (migrations/0005/000005).
// payload_json is accepted/returned as raw JSON rather than a
// double-encoded string, which is friendlier over HTTP than the TEXT
// column it's stored in.
type outboxEvent struct {
	ID          string          `json:"id" binding:"required"`
	EntityType  string          `json:"entity_type" binding:"required"`
	EntityID    string          `json:"entity_id" binding:"required"`
	PayloadJSON json.RawMessage `json:"payload_json" binding:"required"`
	DeviceID    string          `json:"device_id" binding:"required"`
	CreatedAt   string          `json:"created_at" binding:"required"`
}

type pushRequest struct {
	Events []outboxEvent `json:"events" binding:"required"`
}

type pushResponse struct {
	Accepted []string `json:"accepted"`
}

// Push handles POST /sync/push. Each event is applied idempotently: the
// outbox row itself is deduplicated on `id` (its client-generated
// event_id), and only a newly-inserted row triggers applying the event's
// effect to the relevant central table(s) (plan section 1.3's
// idempotency rule). A malformed event is logged and skipped rather than
// failing the whole batch — the client only marks the ids present in
// `accepted` as synced, so a skipped event is retried on the next push.
func (h *Handler) Push(c *gin.Context) {
	var req pushRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	ctx := c.Request.Context()
	accepted := make([]string, 0, len(req.Events))

	for _, ev := range req.Events {
		ok, err := h.applyOne(ctx, ev)
		if err != nil {
			log.Printf("sync: failed to apply %s event %s: %v", ev.EntityType, ev.ID, err)
			continue
		}
		if ok {
			accepted = append(accepted, ev.ID)
		}
	}

	c.JSON(http.StatusOK, pushResponse{Accepted: accepted})
}

// applyOne records the outbox event and, if it wasn't already known,
// applies its effect — both inside one transaction so a partially-applied
// event is never left marked as recorded.
func (h *Handler) applyOne(ctx context.Context, ev outboxEvent) (bool, error) {
	tx, err := h.pool.Begin(ctx)
	if err != nil {
		return false, err
	}
	defer tx.Rollback(ctx)

	tag, err := tx.Exec(ctx, `
		INSERT INTO outbox_events (id, entity_type, entity_id, payload_json, device_id, created_at, synced_at)
		VALUES ($1, $2, $3, $4, $5, $6, now())
		ON CONFLICT (id) DO NOTHING
	`, ev.ID, ev.EntityType, ev.EntityID, string(ev.PayloadJSON), ev.DeviceID, ev.CreatedAt)
	if err != nil {
		return false, fmt.Errorf("insert outbox_events: %w", err)
	}

	if tag.RowsAffected() == 1 {
		if err := applyEntity(ctx, tx, ev.EntityType, ev.PayloadJSON); err != nil {
			return false, fmt.Errorf("apply %s: %w", ev.EntityType, err)
		}
	}

	if err := tx.Commit(ctx); err != nil {
		return false, err
	}
	return true, nil
}

func applyEntity(ctx context.Context, tx pgx.Tx, entityType string, payload json.RawMessage) error {
	switch entityType {
	case "product":
		return applyProduct(ctx, tx, payload)
	case "product_unit":
		return applyProductUnit(ctx, tx, payload)
	case "customer":
		return applyCustomer(ctx, tx, payload)
	case "cash_session":
		return applyCashSession(ctx, tx, payload)
	case "cash_movement":
		return applyCashMovement(ctx, tx, payload)
	case "stock_movement":
		return applyStockMovement(ctx, tx, payload)
	case "customer_ledger":
		return applyCustomerLedger(ctx, tx, payload)
	case "sale":
		return applySale(ctx, tx, payload)
	case "return":
		return applyReturn(ctx, tx, payload)
	case "purchase_order":
		return applyPurchaseOrder(ctx, tx, payload)
	case "stock_level":
		return applyStockLevel(ctx, tx, payload)
	case "fiscal_document":
		return applyFiscalDocument(ctx, tx, payload)
	default:
		return fmt.Errorf("unknown entity_type %q", entityType)
	}
}

// --- Apply: product / product_unit / customer ---
//
// These three are the only entity types edited from more than one place
// (a product/customer registry screen), so they're the only ones that
// need a real last-write-wins guard (plan section 1.3) — the `WHERE`
// clause on each upsert only applies an incoming write if it's at least
// as new as what's already stored, keyed by the `updated_at` column
// migrations 0007/000008 added for exactly this purpose.

type productPayload struct {
	ID          string  `json:"id"`
	SKU         *string `json:"sku"`
	Barcode     *string `json:"barcode"`
	Name        string  `json:"name"`
	NCM         string  `json:"ncm"`
	CFOPDefault *string `json:"cfop_default"`
	CSTCSOSN    *string `json:"cst_csosn"`
	StockUnit   string  `json:"stock_unit"`
	CostPrice   float64 `json:"cost_price"`
	Active      bool    `json:"active"`
	UpdatedAt   string  `json:"updated_at"`
}

func applyProduct(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var p productPayload
	if err := json.Unmarshal(raw, &p); err != nil {
		return err
	}

	if _, err := tx.Exec(ctx, `
		INSERT INTO products
			(id, sku, barcode, name, ncm, cfop_default, cst_csosn, stock_unit, cost_price,
			 active, updated_at, deleted_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,NULL)
		ON CONFLICT (id) DO UPDATE SET
			sku = EXCLUDED.sku, barcode = EXCLUDED.barcode, name = EXCLUDED.name,
			ncm = EXCLUDED.ncm, cfop_default = EXCLUDED.cfop_default,
			cst_csosn = EXCLUDED.cst_csosn, stock_unit = EXCLUDED.stock_unit,
			cost_price = EXCLUDED.cost_price, active = EXCLUDED.active,
			updated_at = EXCLUDED.updated_at
		WHERE products.updated_at <= EXCLUDED.updated_at
	`, p.ID, p.SKU, p.Barcode, p.Name, p.NCM, p.CFOPDefault, p.CSTCSOSN, p.StockUnit, p.CostPrice,
		p.Active, p.UpdatedAt); err != nil {
		return err
	}

	// A product's first sync needs a stock_levels row to exist before any
	// stock_movement referencing it can apply its delta (see
	// applyStockMovement).
	_, err := tx.Exec(ctx, `
		INSERT INTO stock_levels (product_id, quantity, min_quantity, updated_at)
		VALUES ($1, 0, 0, $2)
		ON CONFLICT (product_id) DO NOTHING
	`, p.ID, p.UpdatedAt)
	return err
}

type productUnitPayload struct {
	ID            string  `json:"id"`
	ProductID     string  `json:"product_id"`
	UnitLabel     string  `json:"unit_label"`
	FactorToStock float64 `json:"factor_to_stock"`
	SellPrice     float64 `json:"sell_price"`
	IsDefaultSale bool    `json:"is_default_sale"`
	Active        bool    `json:"active"`
	UpdatedAt     string  `json:"updated_at"`
}

func applyProductUnit(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var u productUnitPayload
	if err := json.Unmarshal(raw, &u); err != nil {
		return err
	}

	_, err := tx.Exec(ctx, `
		INSERT INTO product_units
			(id, product_id, unit_label, factor_to_stock, sell_price, is_default_sale, active,
			 updated_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
		ON CONFLICT (id) DO UPDATE SET
			unit_label = EXCLUDED.unit_label, factor_to_stock = EXCLUDED.factor_to_stock,
			sell_price = EXCLUDED.sell_price, is_default_sale = EXCLUDED.is_default_sale,
			active = EXCLUDED.active, updated_at = EXCLUDED.updated_at
		WHERE product_units.updated_at <= EXCLUDED.updated_at
	`, u.ID, u.ProductID, u.UnitLabel, u.FactorToStock, u.SellPrice, u.IsDefaultSale, u.Active,
		u.UpdatedAt)
	return err
}

type customerPayload struct {
	ID             string  `json:"id"`
	DocType        string  `json:"doc_type"`
	DocNumber      string  `json:"doc_number"`
	Name           string  `json:"name"`
	Phone          *string `json:"phone"`
	AddressJSON    *string `json:"address_json"`
	CreditLimit    float64 `json:"credit_limit"`
	DefaultDueDays int64   `json:"default_due_days"`
	Active         bool    `json:"active"`
	CreatedAt      string  `json:"created_at"`
	UpdatedAt      string  `json:"updated_at"`
}

func applyCustomer(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var c customerPayload
	if err := json.Unmarshal(raw, &c); err != nil {
		return err
	}

	_, err := tx.Exec(ctx, `
		INSERT INTO customers
			(id, doc_type, doc_number, name, phone, address_json, credit_limit,
			 default_due_days, active, created_at, updated_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
		ON CONFLICT (id) DO UPDATE SET
			doc_type = EXCLUDED.doc_type, doc_number = EXCLUDED.doc_number,
			name = EXCLUDED.name, phone = EXCLUDED.phone, address_json = EXCLUDED.address_json,
			credit_limit = EXCLUDED.credit_limit, default_due_days = EXCLUDED.default_due_days,
			active = EXCLUDED.active, updated_at = EXCLUDED.updated_at
		WHERE customers.updated_at <= EXCLUDED.updated_at
	`, c.ID, c.DocType, c.DocNumber, c.Name, c.Phone, c.AddressJSON, c.CreditLimit,
		c.DefaultDueDays, c.Active, c.CreatedAt, c.UpdatedAt)
	return err
}

// --- Apply: cash_session ---
//
// A session only ever has one owning device (it opens and closes it),
// so there's no concurrent-edit race to guard against — a plain
// upsert-by-id is correct.

type cashSessionPayload struct {
	ID             string   `json:"id"`
	DeviceID       string   `json:"device_id"`
	OperatorID     string   `json:"operator_id"`
	OpeningAmount  float64  `json:"opening_amount"`
	ClosingAmount  *float64 `json:"closing_amount"`
	ExpectedAmount *float64 `json:"expected_amount"`
	Status         string   `json:"status"`
	OpenedAt       string   `json:"opened_at"`
	ClosedAt       *string  `json:"closed_at"`
}

func applyCashSession(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var s cashSessionPayload
	if err := json.Unmarshal(raw, &s); err != nil {
		return err
	}

	_, err := tx.Exec(ctx, `
		INSERT INTO cash_sessions
			(id, device_id, operator_id, opening_amount, closing_amount, expected_amount,
			 status, opened_at, closed_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
		ON CONFLICT (id) DO UPDATE SET
			closing_amount = EXCLUDED.closing_amount, expected_amount = EXCLUDED.expected_amount,
			status = EXCLUDED.status, closed_at = EXCLUDED.closed_at
	`, s.ID, s.DeviceID, s.OperatorID, s.OpeningAmount, s.ClosingAmount, s.ExpectedAmount, s.Status,
		s.OpenedAt, s.ClosedAt)
	return err
}

// --- Apply: cash_movement / customer_ledger ---
//
// Both are append-only (plan section 3.3/3.2 — a movement is never
// edited, and a customer's balance is always SUM(debit) - SUM(credit)
// over the ledger, never a stored value), so applying either is just an
// idempotent insert.

type cashMovementPayload struct {
	ID            string  `json:"id"`
	CashSessionID string  `json:"cash_session_id"`
	Type          string  `json:"type"`
	Amount        float64 `json:"amount"`
	Reason        *string `json:"reason"`
	CreatedAt     string  `json:"created_at"`
}

func applyCashMovement(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var m cashMovementPayload
	if err := json.Unmarshal(raw, &m); err != nil {
		return err
	}

	_, err := tx.Exec(ctx, `
		INSERT INTO cash_movements (id, cash_session_id, type, amount, reason, created_at)
		VALUES ($1,$2,$3,$4,$5,$6)
		ON CONFLICT (id) DO NOTHING
	`, m.ID, m.CashSessionID, m.Type, m.Amount, m.Reason, m.CreatedAt)
	return err
}

type customerLedgerPayload struct {
	ID              string  `json:"id"`
	CustomerID      string  `json:"customer_id"`
	EntryType       string  `json:"entry_type"`
	Amount          float64 `json:"amount"`
	DueDate         *string `json:"due_date"`
	ReferenceSaleID *string `json:"reference_sale_id"`
	Notes           *string `json:"notes"`
	DeviceID        string  `json:"device_id"`
	CreatedAt       string  `json:"created_at"`
}

func applyCustomerLedger(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var l customerLedgerPayload
	if err := json.Unmarshal(raw, &l); err != nil {
		return err
	}

	_, err := tx.Exec(ctx, `
		INSERT INTO customer_ledger
			(id, customer_id, entry_type, amount, due_date, reference_sale_id, notes, device_id,
			 created_at, synced_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9, now())
		ON CONFLICT (id) DO NOTHING
	`, l.ID, l.CustomerID, l.EntryType, l.Amount, l.DueDate, l.ReferenceSaleID, l.Notes, l.DeviceID,
		l.CreatedAt)
	return err
}

// --- Apply: stock_movement ---
//
// Applies its delta to stock_levels (plan section 1.3: stock balances
// sync as deltas, never absolute values) only the first time this
// movement is seen (RowsAffected()==1 on the insert) — re-delivering an
// already-applied movement must never double-count it.

type stockMovementPayload struct {
	ID            string  `json:"id"`
	ProductID     string  `json:"product_id"`
	DeltaQuantity float64 `json:"delta_quantity"`
	Reason        string  `json:"reason"`
	ReferenceID   *string `json:"reference_id"`
	DeviceID      string  `json:"device_id"`
	CreatedAt     string  `json:"created_at"`
}

func applyStockMovement(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var m stockMovementPayload
	if err := json.Unmarshal(raw, &m); err != nil {
		return err
	}

	tag, err := tx.Exec(ctx, `
		INSERT INTO stock_movements
			(id, product_id, delta_quantity, reason, reference_id, device_id, created_at, synced_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7, now())
		ON CONFLICT (id) DO NOTHING
	`, m.ID, m.ProductID, m.DeltaQuantity, m.Reason, m.ReferenceID, m.DeviceID, m.CreatedAt)
	if err != nil {
		return err
	}

	if tag.RowsAffected() != 1 {
		return nil
	}

	_, err = tx.Exec(ctx, `
		INSERT INTO stock_levels (product_id, quantity, min_quantity, updated_at)
		VALUES ($1, $2, 0, $3)
		ON CONFLICT (product_id) DO UPDATE SET
			quantity = stock_levels.quantity + EXCLUDED.quantity,
			updated_at = EXCLUDED.updated_at
	`, m.ProductID, m.DeltaQuantity, m.CreatedAt)
	return err
}

// --- Apply: sale ---
//
// A sale and its items/payments are immutable once created in this MVP
// (no edit path — only a status change via Phase 7's returns flow, see
// applyReturn below), so every insert here is a plain idempotent append.

type saleItemPayload struct {
	ID            string  `json:"id"`
	ProductID     string  `json:"product_id"`
	UnitID        string  `json:"unit_id"`
	Quantity      float64 `json:"quantity"`
	QuantityStock float64 `json:"quantity_stock"`
	UnitPrice     float64 `json:"unit_price"`
	Discount      float64 `json:"discount"`
	LineTotal     float64 `json:"line_total"`
}

type salePaymentPayload struct {
	ID           string  `json:"id"`
	Method       string  `json:"method"`
	Amount       float64 `json:"amount"`
	Installments int32   `json:"installments"`
}

type salePayload struct {
	ID            string               `json:"id"`
	CashSessionID string               `json:"cash_session_id"`
	CustomerID    *string              `json:"customer_id"`
	Status        string               `json:"status"`
	Subtotal      float64              `json:"subtotal"`
	DiscountTotal float64              `json:"discount_total"`
	Total         float64              `json:"total"`
	DeviceID      string               `json:"device_id"`
	OperatorID    string               `json:"operator_id"`
	CreatedAt     string               `json:"created_at"`
	Items         []saleItemPayload    `json:"items"`
	Payments      []salePaymentPayload `json:"payments"`
}

func applySale(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var s salePayload
	if err := json.Unmarshal(raw, &s); err != nil {
		return err
	}

	if _, err := tx.Exec(ctx, `
		INSERT INTO sales
			(id, cash_session_id, customer_id, status, subtotal, discount_total, total,
			 device_id, operator_id, created_at, synced_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10, now())
		ON CONFLICT (id) DO NOTHING
	`, s.ID, s.CashSessionID, s.CustomerID, s.Status, s.Subtotal, s.DiscountTotal, s.Total,
		s.DeviceID, s.OperatorID, s.CreatedAt); err != nil {
		return fmt.Errorf("insert sale: %w", err)
	}

	for _, item := range s.Items {
		if _, err := tx.Exec(ctx, `
			INSERT INTO sale_items
				(id, sale_id, product_id, unit_id, quantity, quantity_stock, unit_price,
				 discount, line_total)
			VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
			ON CONFLICT (id) DO NOTHING
		`, item.ID, s.ID, item.ProductID, item.UnitID, item.Quantity, item.QuantityStock,
			item.UnitPrice, item.Discount, item.LineTotal); err != nil {
			return fmt.Errorf("insert sale_item: %w", err)
		}
	}

	for _, payment := range s.Payments {
		if _, err := tx.Exec(ctx, `
			INSERT INTO sale_payments (id, sale_id, method, amount, installments, gateway_ref)
			VALUES ($1,$2,$3,$4,$5,NULL)
			ON CONFLICT (id) DO NOTHING
		`, payment.ID, s.ID, payment.Method, payment.Amount, payment.Installments); err != nil {
			return fmt.Errorf("insert sale_payment: %w", err)
		}
	}

	return nil
}

// --- Apply: return ---
//
// A return and its items are append-only once created — the operator can
// issue multiple partial returns against the same sale over time, each
// its own immutable `returns` row, so a plain idempotent insert is correct
// here too (same shape as applySale above). The accompanying
// sales.status transition ('returned' vs 'partially_returned') is
// computed client-side — the device already has every sale_item and
// prior return_items row loaded to build the return — and simply trusted
// here, the same way applySale trusts the client's Status field.
//
// applyStockMovement and applyCustomerLedger (above) need no changes to
// handle return-originated events: both treat reason/entry_type as opaque
// text, so a stock_movement with reason='return' or a customer_ledger
// entry_type='credit' flows through their existing paths untouched.

type returnItemPayload struct {
	ID         string  `json:"id"`
	SaleItemID string  `json:"sale_item_id"`
	Quantity   float64 `json:"quantity"`
}

type returnPayload struct {
	ID             string              `json:"id"`
	OriginalSaleID string              `json:"original_sale_id"`
	Reason         *string             `json:"reason"`
	TotalRefunded  float64             `json:"total_refunded"`
	RefundMethod   string              `json:"refund_method"`
	OperatorID     string              `json:"operator_id"`
	DeviceID       string              `json:"device_id"`
	CreatedAt      string              `json:"created_at"`
	SaleStatus     string              `json:"sale_status"`
	Items          []returnItemPayload `json:"items"`
}

func applyReturn(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var r returnPayload
	if err := json.Unmarshal(raw, &r); err != nil {
		return err
	}

	if _, err := tx.Exec(ctx, `
		INSERT INTO returns
			(id, original_sale_id, reason, total_refunded, refund_method, operator_id,
			 device_id, created_at, synced_at)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8, now())
		ON CONFLICT (id) DO NOTHING
	`, r.ID, r.OriginalSaleID, r.Reason, r.TotalRefunded, r.RefundMethod, r.OperatorID,
		r.DeviceID, r.CreatedAt); err != nil {
		return fmt.Errorf("insert return: %w", err)
	}

	for _, item := range r.Items {
		if _, err := tx.Exec(ctx, `
			INSERT INTO return_items (id, return_id, sale_item_id, quantity)
			VALUES ($1,$2,$3,$4)
			ON CONFLICT (id) DO NOTHING
		`, item.ID, r.ID, item.SaleItemID, item.Quantity); err != nil {
			return fmt.Errorf("insert return_item: %w", err)
		}
	}

	if _, err := tx.Exec(ctx, `
		UPDATE sales SET status = $1 WHERE id = $2
	`, r.SaleStatus, r.OriginalSaleID); err != nil {
		return fmt.Errorf("update sale status: %w", err)
	}

	return nil
}

// --- Apply: purchase_order ---
//
// Plan sections 3.1/4.6. Unlike sale/return, a purchase order is mutable
// after creation — receiving goods (possibly across several partial
// deliveries) updates its status and each item's quantity_received — so
// this is re-sent in full and upserted each time, the same "upsert the
// whole current snapshot" shape applyCashSession uses for its own
// mutable status/closing fields.

type purchaseOrderItemPayload struct {
	ID               string  `json:"id"`
	ProductID        string  `json:"product_id"`
	QuantityOrdered  float64 `json:"quantity_ordered"`
	QuantityReceived float64 `json:"quantity_received"`
	UnitCost         float64 `json:"unit_cost"`
}

type purchaseOrderPayload struct {
	ID           string                     `json:"id"`
	SupplierName string                     `json:"supplier_name"`
	Status       string                     `json:"status"`
	CreatedAt    string                     `json:"created_at"`
	Items        []purchaseOrderItemPayload `json:"items"`
}

func applyPurchaseOrder(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var po purchaseOrderPayload
	if err := json.Unmarshal(raw, &po); err != nil {
		return err
	}

	if _, err := tx.Exec(ctx, `
		INSERT INTO purchase_orders (id, supplier_name, status, created_at)
		VALUES ($1,$2,$3,$4)
		ON CONFLICT (id) DO UPDATE SET status = EXCLUDED.status
	`, po.ID, po.SupplierName, po.Status, po.CreatedAt); err != nil {
		return fmt.Errorf("insert purchase_order: %w", err)
	}

	for _, item := range po.Items {
		if _, err := tx.Exec(ctx, `
			INSERT INTO purchase_order_items
				(id, purchase_order_id, product_id, quantity_ordered, quantity_received, unit_cost)
			VALUES ($1,$2,$3,$4,$5,$6)
			ON CONFLICT (id) DO UPDATE SET quantity_received = EXCLUDED.quantity_received
		`, item.ID, po.ID, item.ProductID, item.QuantityOrdered, item.QuantityReceived,
			item.UnitCost); err != nil {
			return fmt.Errorf("insert purchase_order_item: %w", err)
		}
	}

	return nil
}

// --- Apply: stock_level (min_quantity threshold) ---
//
// Plan section 4.6's low-stock trigger. Deliberately separate from
// applyStockMovement: `quantity` is a delta-derived balance (plan section
// 1.3 — never synced as an absolute value), but `min_quantity` is a
// plain configured threshold, so it's fine to overwrite outright. No
// last-write-wins guard: this column is independent of the one
// applyStockMovement increments, so an overwrite here can never clobber a
// concurrent quantity delta.

type stockLevelPayload struct {
	ProductID   string  `json:"product_id"`
	MinQuantity float64 `json:"min_quantity"`
	UpdatedAt   string  `json:"updated_at"`
}

func applyStockLevel(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var s stockLevelPayload
	if err := json.Unmarshal(raw, &s); err != nil {
		return err
	}

	_, err := tx.Exec(ctx, `
		INSERT INTO stock_levels (product_id, quantity, min_quantity, updated_at)
		VALUES ($1, 0, $2, $3)
		ON CONFLICT (product_id) DO UPDATE SET min_quantity = EXCLUDED.min_quantity
	`, s.ProductID, s.MinQuantity, s.UpdatedAt)
	return err
}

// --- Apply: fiscal_document ---
//
// Phase 9 addition (plan section 5.2's back-office alert for a fiscal
// document stuck pending past the contingency window) — see
// migrations/000010_fiscal_documents.up.sql's comment for why this entity
// type exists at all: fiscal_documents was deliberately left unsynced in
// Phase 6, and this reverses that.
//
// The client emits one event per transition (created 'pending', then
// 'authorized' or 'rejected' — see app/src-tauri/src/commands/sales.rs and
// fiscal_client/mod.rs's mark_authorized/mark_rejected). A single device
// ever writes a given fiscal_document, so the last-write-wins guard here
// is cheap insurance (a delayed retry's event arriving after a later one,
// e.g. after a connectivity gap) rather than a real multi-writer concern
// the way product/customer's guard is.
//
// fiscal_queue itself is NOT synced — retry bookkeeping (next_retry_at,
// backoff state) has no back-office use and stays local-only.

type fiscalDocumentPayload struct {
	ID               string  `json:"id"`
	SaleID           string  `json:"sale_id"`
	DocType          string  `json:"doc_type"`
	Status           string  `json:"status"`
	AccessKey        *string `json:"access_key"`
	XMLURL           *string `json:"xml_url"`
	DanfeURL         *string `json:"danfe_url"`
	RejectionReason  *string `json:"rejection_reason"`
	CancellationNote *string `json:"cancellation_note"`
	Attempts         int     `json:"attempts"`
	CreatedAt        string  `json:"created_at"`
	UpdatedAt        string  `json:"updated_at"`
}

func applyFiscalDocument(ctx context.Context, tx pgx.Tx, raw json.RawMessage) error {
	var f fiscalDocumentPayload
	if err := json.Unmarshal(raw, &f); err != nil {
		return err
	}

	_, err := tx.Exec(ctx, `
		INSERT INTO fiscal_documents
			(id, sale_id, doc_type, status, provider_ref, access_key, xml_url, danfe_url,
			 rejection_reason, cancellation_note, attempts, created_at, updated_at)
		VALUES ($1,$2,$3,$4,NULL,$5,$6,$7,$8,$9,$10,$11,$12)
		ON CONFLICT (id) DO UPDATE SET
			status = EXCLUDED.status, access_key = EXCLUDED.access_key,
			xml_url = EXCLUDED.xml_url, danfe_url = EXCLUDED.danfe_url,
			rejection_reason = EXCLUDED.rejection_reason,
			cancellation_note = EXCLUDED.cancellation_note, attempts = EXCLUDED.attempts,
			updated_at = EXCLUDED.updated_at
		WHERE fiscal_documents.updated_at <= EXCLUDED.updated_at
	`, f.ID, f.SaleID, f.DocType, f.Status, f.AccessKey, f.XMLURL, f.DanfeURL,
		f.RejectionReason, f.CancellationNote, f.Attempts, f.CreatedAt, f.UpdatedAt)
	return err
}

// --- Pull ---

type pullResponse struct {
	Events []outboxEvent `json:"events"`
	Cursor string        `json:"cursor"`
}

// Pull handles GET /sync/pull?since=<cursor>: events created by any
// device other than the caller's own (identified from its JWT, not a
// query param — see auth.RequireAuth), in order, since the cursor. The
// new cursor is the last returned event's created_at, so the next call
// picks up right after it; callers with nothing new just get their same
// cursor back.
func (h *Handler) Pull(c *gin.Context) {
	since := c.Query("since")
	if since == "" {
		since = epoch
	}
	requestingDevice, _ := c.Get(auth.DeviceIDKey)
	deviceID, _ := requestingDevice.(string)

	ctx := c.Request.Context()
	rows, err := h.pool.Query(ctx, `
		SELECT id, entity_type, entity_id, payload_json, device_id, created_at
		FROM outbox_events
		WHERE created_at > $1 AND device_id != $2
		ORDER BY created_at
		LIMIT $3
	`, since, deviceID, pullBatchSize)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	events := make([]outboxEvent, 0)
	cursor := since

	for rows.Next() {
		var (
			id, entityType, entityID, eventDeviceID, payloadJSON string
			createdAt                                            time.Time
		)
		if err := rows.Scan(&id, &entityType, &entityID, &payloadJSON, &eventDeviceID, &createdAt); err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		createdAtStr := createdAt.UTC().Format(time.RFC3339Nano)
		events = append(events, outboxEvent{
			ID:          id,
			EntityType:  entityType,
			EntityID:    entityID,
			PayloadJSON: json.RawMessage(payloadJSON),
			DeviceID:    eventDeviceID,
			CreatedAt:   createdAtStr,
		})
		cursor = createdAtStr
	}
	if err := rows.Err(); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}

	c.JSON(http.StatusOK, pullResponse{Events: events, Cursor: cursor})
}
