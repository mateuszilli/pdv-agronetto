// Package reports exposes the owner-facing dashboard queries (plan
// section 4.7): margin per product, dead stock, average ticket, ABC
// curve — all computed directly from sales/sale_items/products in central
// Postgres, no BI engine, "just parameterized views/queries exposed
// through simple endpoints" (plan's own words). Phase 9 also adds
// GET /reports/fiscal-alerts for plan section 5.2's back-office alert on
// fiscal documents stuck pending past the contingency window — see
// migrations/000010_fiscal_documents.up.sql for why that table is synced
// centrally at all.
//
// Every endpoint here sits behind the same operator JWT
// (auth.RequireAuth) as /sync/*, per an explicit decision: the plan's
// auth model (section 2) has no separate "owner" role, so this reuses the
// one login every device already has rather than inventing one.
package reports

import (
	"net/http"
	"strconv"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/jackc/pgx/v5/pgxpool"
)

// Handler holds the dependencies report endpoints need.
type Handler struct {
	pool *pgxpool.Pool
}

func NewHandler(pool *pgxpool.Pool) *Handler {
	return &Handler{pool: pool}
}

const dateLayout = "2006-01-02"

// defaultPeriod resolves the `from`/`to` query params shared by the
// margin, avg-ticket, and abc-curve endpoints. Bare dates (`2026-08-01`)
// are accepted, not just RFC3339, since a human is the most likely caller
// of a back-office report. Absent params default to the trailing 30 days
// — the plan says "aggregated by period" without naming a default window,
// and 30 days is a reasonable one for a small store's dashboard.
func defaultPeriod(c *gin.Context) (from, to time.Time, err error) {
	now := time.Now().UTC()
	to = now
	from = now.AddDate(0, 0, -30)

	if v := c.Query("from"); v != "" {
		from, err = time.Parse(dateLayout, v)
		if err != nil {
			return from, to, err
		}
	}
	if v := c.Query("to"); v != "" {
		to, err = time.Parse(dateLayout, v)
		if err != nil {
			return from, to, err
		}
		// A bare date for `to` means "through the end of that day", not
		// midnight at its start — otherwise `to=2026-08-04` would exclude
		// every sale made that day.
		to = to.AddDate(0, 0, 1)
	}
	return from, to, nil
}

// positiveIntParam parses a `?name=N` query param as a positive int,
// returning `def` when absent.
func positiveIntParam(c *gin.Context, name string, def int) (int, error) {
	v := c.Query(name)
	if v == "" {
		return def, nil
	}
	n, err := strconv.Atoi(v)
	if err != nil || n <= 0 {
		if err == nil {
			err = errNotPositive
		}
		return 0, err
	}
	return n, nil
}

var errNotPositive = &paramError{"must be a positive integer"}

type paramError struct{ msg string }

func (e *paramError) Error() string { return e.msg }

func badRequest(c *gin.Context, err error) {
	c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
}

func internalError(c *gin.Context, err error) {
	c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
}

// --- Margin per product (plan section 4.7) ---
//
// The plan's formula, (sell_price - cost_price) / sell_price, is
// conceptual — sell_price actually lives on product_units (a product can
// have several, plan section 3.1), not products, and doesn't account for
// per-sale discounts or returns. This computes the same thing from
// realized transactions instead: revenue and cost per product, netted
// against any returned quantity on each line, aggregated over the period.
// Cancelled sales are excluded entirely (never valid revenue); a fully
// `returned` sale nets to ~0 through the same mechanism rather than
// needing its own filter.

// netSaleItemsCTE is shared by Margin and ABCCurve — both need the same
// "revenue/cost per product, netted for returns" base rows, just
// aggregated and presented differently.
const netSaleItemsCTE = `
	WITH period_sales AS (
		SELECT id FROM sales
		 WHERE status <> 'cancelled' AND created_at >= $1 AND created_at < $2
	),
	item_returns AS (
		SELECT sale_item_id, SUM(quantity) AS returned_qty
		FROM return_items GROUP BY sale_item_id
	),
	net_items AS (
		SELECT
			si.product_id,
			-- Fraction of the line still standing after returns, clamped to
			-- [0,1] so a return recorded against more than what's left (which
			-- shouldn't happen, but this is a read-only report, not the
			-- source of truth for that invariant) can't flip revenue negative.
			GREATEST(0, 1 - COALESCE(ir.returned_qty, 0) / NULLIF(si.quantity, 0)) AS net_fraction,
			si.line_total,
			si.quantity_stock,
			si.quantity
		FROM sale_items si
		JOIN period_sales ps ON ps.id = si.sale_id
		LEFT JOIN item_returns ir ON ir.sale_item_id = si.id
	)
`

type marginRow struct {
	ProductID    string  `json:"product_id"`
	Name         string  `json:"name"`
	SKU          *string `json:"sku"`
	QuantitySold float64 `json:"quantity_sold"`
	Revenue      float64 `json:"revenue"`
	Cost         float64 `json:"cost"`
	MarginAmount float64 `json:"margin_amount"`
	MarginPct    float64 `json:"margin_pct"`
}

// Margin handles GET /reports/margin?from=&to=.
func (h *Handler) Margin(c *gin.Context) {
	from, to, err := defaultPeriod(c)
	if err != nil {
		badRequest(c, err)
		return
	}

	rows, err := h.pool.Query(c.Request.Context(), netSaleItemsCTE+`
		SELECT p.id, p.name, p.sku,
		       SUM(ni.net_fraction * ni.quantity) AS quantity_sold,
		       SUM(ni.net_fraction * ni.line_total) AS revenue,
		       SUM(ni.net_fraction * ni.quantity_stock * p.cost_price) AS cost
		  FROM net_items ni
		  JOIN products p ON p.id = ni.product_id
		 GROUP BY p.id, p.name, p.sku
		 ORDER BY revenue DESC
	`, from, to)
	if err != nil {
		internalError(c, err)
		return
	}
	defer rows.Close()

	result := make([]marginRow, 0)
	for rows.Next() {
		var m marginRow
		if err := rows.Scan(&m.ProductID, &m.Name, &m.SKU, &m.QuantitySold, &m.Revenue, &m.Cost); err != nil {
			internalError(c, err)
			return
		}
		m.MarginAmount = m.Revenue - m.Cost
		if m.Revenue != 0 {
			m.MarginPct = m.MarginAmount / m.Revenue
		}
		result = append(result, m)
	}
	if err := rows.Err(); err != nil {
		internalError(c, err)
		return
	}

	c.JSON(http.StatusOK, gin.H{"from": from, "to": to, "products": result})
}

// --- Dead stock (plan section 4.7) ---
//
// "Products with no stock_movements of type 'sale' in the last N days
// (configurable, default 60)."

type deadStockRow struct {
	ProductID  string     `json:"product_id"`
	Name       string     `json:"name"`
	SKU        *string    `json:"sku"`
	StockUnit  string     `json:"stock_unit"`
	Quantity   float64    `json:"quantity"`
	LastSaleAt *time.Time `json:"last_sale_at"`
}

// DeadStock handles GET /reports/dead-stock?days=60.
func (h *Handler) DeadStock(c *gin.Context) {
	days, err := positiveIntParam(c, "days", 60)
	if err != nil {
		badRequest(c, err)
		return
	}

	rows, err := h.pool.Query(c.Request.Context(), `
		SELECT p.id, p.name, p.sku, p.stock_unit, sl.quantity,
		       MAX(sm.created_at) FILTER (WHERE sm.reason = 'sale') AS last_sale_at
		  FROM products p
		  LEFT JOIN stock_levels sl ON sl.product_id = p.id
		  LEFT JOIN stock_movements sm ON sm.product_id = p.id
		 WHERE p.active = true
		 GROUP BY p.id, p.name, p.sku, p.stock_unit, sl.quantity
		HAVING MAX(sm.created_at) FILTER (WHERE sm.reason = 'sale') IS NULL
		    OR MAX(sm.created_at) FILTER (WHERE sm.reason = 'sale') < now() - make_interval(days => $1)
		 ORDER BY last_sale_at NULLS FIRST
	`, days)
	if err != nil {
		internalError(c, err)
		return
	}
	defer rows.Close()

	result := make([]deadStockRow, 0)
	for rows.Next() {
		var d deadStockRow
		if err := rows.Scan(&d.ProductID, &d.Name, &d.SKU, &d.StockUnit, &d.Quantity, &d.LastSaleAt); err != nil {
			internalError(c, err)
			return
		}
		result = append(result, d)
	}
	if err := rows.Err(); err != nil {
		internalError(c, err)
		return
	}

	c.JSON(http.StatusOK, gin.H{"days": days, "products": result})
}

// --- Average ticket (plan section 4.7) ---
//
// "SUM(sales.total) / COUNT(sales.id) by period." Applied per-day across
// the requested window (a single number is a degenerate one-row case of
// the same query) so the dashboard can plot a trend, plus one overall
// summary for the headline figure. Uses sales.total as-is — no return
// netting here, unlike Margin/ABCCurve — because the plan's formula names
// `sales.total` directly rather than a line-level figure; only `returned`
// (fully reversed, no revenue) and `cancelled` sales are excluded.

type avgTicketDay struct {
	Day       time.Time `json:"day"`
	SaleCount int64     `json:"sale_count"`
	Revenue   float64   `json:"revenue"`
	AvgTicket float64   `json:"avg_ticket"`
}

const excludedSaleStatuses = `status NOT IN ('cancelled', 'returned')`

// AvgTicket handles GET /reports/avg-ticket?from=&to=.
func (h *Handler) AvgTicket(c *gin.Context) {
	from, to, err := defaultPeriod(c)
	if err != nil {
		badRequest(c, err)
		return
	}

	rows, err := h.pool.Query(c.Request.Context(), `
		SELECT date_trunc('day', created_at) AS day, COUNT(*), SUM(total)
		  FROM sales
		 WHERE `+excludedSaleStatuses+` AND created_at >= $1 AND created_at < $2
		 GROUP BY 1
		 ORDER BY 1
	`, from, to)
	if err != nil {
		internalError(c, err)
		return
	}
	defer rows.Close()

	days := make([]avgTicketDay, 0)
	for rows.Next() {
		var d avgTicketDay
		if err := rows.Scan(&d.Day, &d.SaleCount, &d.Revenue); err != nil {
			internalError(c, err)
			return
		}
		if d.SaleCount > 0 {
			d.AvgTicket = d.Revenue / float64(d.SaleCount)
		}
		days = append(days, d)
	}
	if err := rows.Err(); err != nil {
		internalError(c, err)
		return
	}

	var overall struct {
		SaleCount int64
		Revenue   float64
	}
	if err := h.pool.QueryRow(c.Request.Context(), `
		SELECT COUNT(*), COALESCE(SUM(total), 0) FROM sales
		 WHERE `+excludedSaleStatuses+` AND created_at >= $1 AND created_at < $2
	`, from, to).Scan(&overall.SaleCount, &overall.Revenue); err != nil {
		internalError(c, err)
		return
	}
	var overallAvg float64
	if overall.SaleCount > 0 {
		overallAvg = overall.Revenue / float64(overall.SaleCount)
	}

	c.JSON(http.StatusOK, gin.H{
		"from": from, "to": to,
		"sale_count": overall.SaleCount, "revenue": overall.Revenue, "avg_ticket": overallAvg,
		"by_day": days,
	})
}

// --- ABC curve (plan section 4.7) ---
//
// "Product ranking by cumulative revenue — class A = top products making
// up 80% of revenue, B = next 15%, C = last 5%." Reuses the same netted
// revenue as Margin for consistency between the two reports.
//
// Classification is keyed off the cumulative percentage BEFORE each
// product (prev_cum_pct = cum_revenue excluding this row / total), not
// including it — otherwise the single product that actually crosses the
// 80% line gets pushed into B or C instead of being the top of A, and a
// store with one product (100% of revenue by definition) would always
// land in C. The displayed `cumulative_pct` is still the intuitive
// "through and including this row" figure; only the class boundary uses
// the shifted one.

type abcRow struct {
	ProductID string  `json:"product_id"`
	Name      string  `json:"name"`
	SKU       *string `json:"sku"`
	Revenue   float64 `json:"revenue"`
	CumPct    float64 `json:"cumulative_pct"`
	Class     string  `json:"class"`
}

// ABCCurve handles GET /reports/abc-curve?from=&to=.
func (h *Handler) ABCCurve(c *gin.Context) {
	from, to, err := defaultPeriod(c)
	if err != nil {
		badRequest(c, err)
		return
	}

	rows, err := h.pool.Query(c.Request.Context(), netSaleItemsCTE+`,
	product_revenue AS (
		SELECT p.id, p.name, p.sku, SUM(ni.net_fraction * ni.line_total) AS revenue
		  FROM net_items ni
		  JOIN products p ON p.id = ni.product_id
		 GROUP BY p.id, p.name, p.sku
		HAVING SUM(ni.net_fraction * ni.line_total) > 0
	),
	ranked AS (
		SELECT *,
		       SUM(revenue) OVER (ORDER BY revenue DESC ROWS UNBOUNDED PRECEDING) AS cum_revenue,
		       SUM(revenue) OVER () AS total_revenue
		  FROM product_revenue
	)
	SELECT id, name, sku, revenue,
	       cum_revenue / total_revenue AS cum_pct,
	       (cum_revenue - revenue) / total_revenue AS prev_cum_pct
	  FROM ranked
	 ORDER BY revenue DESC
	`, from, to)
	if err != nil {
		internalError(c, err)
		return
	}
	defer rows.Close()

	result := make([]abcRow, 0)
	for rows.Next() {
		var a abcRow
		var prevCumPct float64
		if err := rows.Scan(&a.ProductID, &a.Name, &a.SKU, &a.Revenue, &a.CumPct, &prevCumPct); err != nil {
			internalError(c, err)
			return
		}
		switch {
		case prevCumPct < 0.80:
			a.Class = "A"
		case prevCumPct < 0.95:
			a.Class = "B"
		default:
			a.Class = "C"
		}
		result = append(result, a)
	}
	if err := rows.Err(); err != nil {
		internalError(c, err)
		return
	}

	c.JSON(http.StatusOK, gin.H{"from": from, "to": to, "products": result})
}

// --- Fiscal alerts (Phase 9 addition, plan section 5.2) ---
//
// "Confirm with your chosen provider the maximum time allowed for
// contingency emission (typically up to 24h) — this should become an
// alert on the back-office dashboard if fiscal documents remain pending
// past that window." Needs fiscal_documents synced centrally — see
// migrations/000010_fiscal_documents.up.sql and internal/sync's
// applyFiscalDocument for why that's now the case.

type fiscalAlertRow struct {
	FiscalDocumentID string    `json:"fiscal_document_id"`
	SaleID           string    `json:"sale_id"`
	Status           string    `json:"status"`
	Attempts         int       `json:"attempts"`
	CreatedAt        time.Time `json:"created_at"`
	HoursPending     float64   `json:"hours_pending"`
}

// FiscalAlerts handles GET /reports/fiscal-alerts?hours=24.
func (h *Handler) FiscalAlerts(c *gin.Context) {
	hours, err := positiveIntParam(c, "hours", 24)
	if err != nil {
		badRequest(c, err)
		return
	}

	rows, err := h.pool.Query(c.Request.Context(), `
		SELECT id, sale_id, status, attempts, created_at,
		       EXTRACT(EPOCH FROM (now() - created_at)) / 3600 AS hours_pending
		  FROM fiscal_documents
		 WHERE status = 'pending' AND created_at < now() - make_interval(hours => $1)
		 ORDER BY created_at
	`, hours)
	if err != nil {
		internalError(c, err)
		return
	}
	defer rows.Close()

	result := make([]fiscalAlertRow, 0)
	for rows.Next() {
		var f fiscalAlertRow
		if err := rows.Scan(&f.FiscalDocumentID, &f.SaleID, &f.Status, &f.Attempts, &f.CreatedAt, &f.HoursPending); err != nil {
			internalError(c, err)
			return
		}
		result = append(result, f)
	}
	if err := rows.Err(); err != nil {
		internalError(c, err)
		return
	}

	c.JSON(http.StatusOK, gin.H{"threshold_hours": hours, "documents": result})
}
