// Package sync will own the outbox push/pull protocol (plan sections 1.3
// and 3.5): devices POST batches of outbox_events to /sync/push
// (idempotent on event_id) and GET /sync/pull?since=<cursor> for changes
// from other terminals at the same store.
//
// Phase 0 only defines the request/response shapes and wires the routes;
// actual persistence, idempotency handling and delta-based conflict
// resolution for stock/store-credit balances are Phase 5 scope.
package sync

import (
	"encoding/json"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/jackc/pgx/v5/pgxpool"
)

// Handler holds the dependencies sync endpoints will need once push/pull
// are backed by real persistence (Phase 5). Unused for now beyond carrying
// the pool through, on purpose — kept so route wiring in main.go doesn't
// need to change shape later.
type Handler struct {
	pool *pgxpool.Pool
}

func NewHandler(pool *pgxpool.Pool) *Handler {
	return &Handler{pool: pool}
}

// outboxEvent mirrors the outbox_events table (migrations/0005/000005).
// payload_json is accepted/returned as raw JSON rather than a
// double-encoded string, which is friendlier over HTTP than the TEXT
// column it will eventually be stored in.
type outboxEvent struct {
	ID          string          `json:"id" binding:"required"`
	EntityType  string          `json:"entity_type" binding:"required"`
	EntityID    string          `json:"entity_id" binding:"required"`
	PayloadJSON json.RawMessage `json:"payload_json" binding:"required"`
	DeviceID    string          `json:"device_id" binding:"required"`
	CreatedAt   string          `json:"created_at" binding:"required"`
	SyncedAt    *string         `json:"synced_at,omitempty"`
}

type pushRequest struct {
	Events []outboxEvent `json:"events" binding:"required"`
}

type pushResponse struct {
	Accepted []string `json:"accepted"`
}

// Push handles POST /sync/push. TODO(phase 5): persist events keyed by
// their UUID event_id for idempotent dedup, and apply them (sales, stock
// deltas, ledger deltas, etc.) to central Postgres.
func (h *Handler) Push(c *gin.Context) {
	var req pushRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	accepted := make([]string, 0, len(req.Events))
	for _, ev := range req.Events {
		accepted = append(accepted, ev.ID)
	}

	c.JSON(http.StatusOK, pushResponse{Accepted: accepted})
}

type pullResponse struct {
	Events []outboxEvent `json:"events"`
	Cursor string        `json:"cursor"`
}

// Pull handles GET /sync/pull?since=<cursor>. TODO(phase 5): return other
// devices'/back office's changes for this store since the given cursor.
func (h *Handler) Pull(c *gin.Context) {
	since := c.Query("since")
	_ = since // unused until real pull logic exists

	c.JSON(http.StatusOK, pullResponse{
		Events: []outboxEvent{},
		Cursor: time.Now().UTC().Format(time.RFC3339),
	})
}
