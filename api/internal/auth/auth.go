// Package auth issues JWTs per device + operator (plan section 2), backed
// by a seeded `operators` table and an auto-registering `devices` table.
//
// DEVIATION FROM THE PLAN: neither `operators` nor `devices` appears in
// section 3's schema — see migrations/000006_auth.up.sql for why Phase 0
// adds them.
package auth

import (
	"context"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/golang-jwt/jwt/v5"
	"github.com/google/uuid"
	"github.com/jackc/pgx/v5/pgxpool"
	"golang.org/x/crypto/bcrypt"
)

// Context keys RequireAuth sets for downstream handlers (plan section
// 3.5's sync endpoints use these to scope pull/push to the caller's own
// device without trusting a client-supplied query param).
const (
	DeviceIDKey   = "device_id"
	OperatorIDKey = "operator_id"
)

const tokenTTL = 12 * time.Hour

// Dev-only seed credentials — see EnsureSeedOperator.
const (
	seedOperatorName = "Demo Operator"
	seedOperatorPIN  = "1234"
)

// Handler holds the dependencies needed to serve auth endpoints.
type Handler struct {
	pool      *pgxpool.Pool
	jwtSecret []byte
}

func NewHandler(pool *pgxpool.Pool, jwtSecret []byte) *Handler {
	return &Handler{pool: pool, jwtSecret: jwtSecret}
}

type loginRequest struct {
	DeviceID    string `json:"device_id" binding:"required"`
	OperatorPIN string `json:"operator_pin" binding:"required"`
}

type operatorInfo struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type loginResponse struct {
	Token     string       `json:"token"`
	ExpiresAt string       `json:"expires_at"`
	DeviceID  string       `json:"device_id"`
	Operator  operatorInfo `json:"operator"`
}

type claims struct {
	DeviceID string `json:"device_id"`
	jwt.RegisteredClaims
}

// Login handles POST /auth/login: verifies the operator PIN against the
// seeded operators table, auto-registers the device on first sight, and
// issues a JWT scoped to this device + operator pair.
func (h *Handler) Login(c *gin.Context) {
	var req loginRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	ctx := c.Request.Context()

	operatorID, operatorName, err := h.matchOperatorPIN(ctx, req.OperatorPIN)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to verify operator"})
		return
	}
	if operatorID == "" {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "invalid operator PIN"})
		return
	}

	if err := h.upsertDevice(ctx, req.DeviceID); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to register device"})
		return
	}

	expiresAt := time.Now().Add(tokenTTL)
	token := jwt.NewWithClaims(jwt.SigningMethodHS256, claims{
		DeviceID: req.DeviceID,
		RegisteredClaims: jwt.RegisteredClaims{
			Subject:   operatorID,
			ExpiresAt: jwt.NewNumericDate(expiresAt),
			IssuedAt:  jwt.NewNumericDate(time.Now()),
		},
	})

	signed, err := token.SignedString(h.jwtSecret)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to sign token"})
		return
	}

	c.JSON(http.StatusOK, loginResponse{
		Token:     signed,
		ExpiresAt: expiresAt.UTC().Format(time.RFC3339),
		DeviceID:  req.DeviceID,
		Operator:  operatorInfo{ID: operatorID, Name: operatorName},
	})
}

// matchOperatorPIN scans active operators and bcrypt-compares the given PIN
// against each stored hash, since PINs aren't indexed/unique by design.
// Fine for a handful of seeded operators; a real lookup scheme is a
// later-phase concern. Returns ("", "", nil) when nothing matches.
func (h *Handler) matchOperatorPIN(ctx context.Context, pin string) (id string, name string, err error) {
	rows, err := h.pool.Query(ctx, `SELECT id, name, pin_hash FROM operators WHERE active = true`)
	if err != nil {
		return "", "", err
	}
	defer rows.Close()

	for rows.Next() {
		var (
			rowID, rowName, rowHash string
		)
		if err := rows.Scan(&rowID, &rowName, &rowHash); err != nil {
			return "", "", err
		}
		if bcrypt.CompareHashAndPassword([]byte(rowHash), []byte(pin)) == nil {
			return rowID, rowName, nil
		}
	}
	return "", "", rows.Err()
}

func (h *Handler) upsertDevice(ctx context.Context, deviceID string) error {
	_, err := h.pool.Exec(ctx, `
		INSERT INTO devices (id, first_seen_at, last_seen_at)
		VALUES ($1, now(), now())
		ON CONFLICT (id) DO UPDATE SET last_seen_at = now()
	`, deviceID)
	return err
}

// EnsureSeedOperator inserts a single dev/demo operator if none exist yet,
// so a fresh `make db-up && make migrate-backend && make dev-backend` has
// something to log in with. Not something a production deployment would do.
func EnsureSeedOperator(ctx context.Context, pool *pgxpool.Pool) error {
	var count int
	if err := pool.QueryRow(ctx, `SELECT count(*) FROM operators`).Scan(&count); err != nil {
		return err
	}
	if count > 0 {
		return nil
	}

	hash, err := bcrypt.GenerateFromPassword([]byte(seedOperatorPIN), bcrypt.DefaultCost)
	if err != nil {
		return err
	}

	_, err = pool.Exec(ctx, `
		INSERT INTO operators (id, name, pin_hash, active)
		VALUES ($1, $2, $3, true)
	`, uuid.New().String(), seedOperatorName, string(hash))
	if err != nil {
		return err
	}

	log.Printf("auth: seeded demo operator %q with PIN %q (dev only)", seedOperatorName, seedOperatorPIN)
	return nil
}

// RequireAuth validates the bearer JWT issued by Login and sets
// DeviceIDKey/OperatorIDKey in the request context for downstream
// handlers — used to gate /sync/push and /sync/pull (plan sections 1.3
// and 2: each device/operator pair sync's under its own identity).
func (h *Handler) RequireAuth() gin.HandlerFunc {
	return func(c *gin.Context) {
		header := c.GetHeader("Authorization")
		token, ok := strings.CutPrefix(header, "Bearer ")
		if !ok || token == "" {
			c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{"error": "missing bearer token"})
			return
		}

		parsed, err := jwt.ParseWithClaims(token, &claims{}, func(t *jwt.Token) (interface{}, error) {
			return h.jwtSecret, nil
		})
		if err != nil || !parsed.Valid {
			c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{"error": "invalid or expired token"})
			return
		}

		parsedClaims, ok := parsed.Claims.(*claims)
		if !ok {
			c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{"error": "invalid token claims"})
			return
		}

		c.Set(DeviceIDKey, parsedClaims.DeviceID)
		c.Set(OperatorIDKey, parsedClaims.Subject)
		c.Next()
	}
}
