// Command api runs the PDV sync backend: device/operator authentication
// and the sync push/pull endpoints (plan sections 2 and 3.5). See
// internal/{auth,sync} for the implementations and the other internal/*
// packages for their planned future responsibilities.
package main

import (
	"context"
	"log"
	"net/http"
	"os"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/joho/godotenv"

	"github.com/mzillipolli/pdv-backend/internal/auth"
	"github.com/mzillipolli/pdv-backend/internal/dashboard"
	"github.com/mzillipolli/pdv-backend/internal/reports"
	"github.com/mzillipolli/pdv-backend/internal/sync"
)

func main() {
	// Best-effort: in prod, env vars are set by the environment itself.
	_ = godotenv.Load()

	databaseURL := requireEnv("DATABASE_URL")
	jwtSecret := requireEnv("JWT_SECRET")
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	pool, err := pgxpool.New(ctx, databaseURL)
	if err != nil {
		log.Fatalf("failed to create postgres pool: %v", err)
	}
	defer pool.Close()

	if err := pool.Ping(ctx); err != nil {
		log.Fatalf("failed to connect to postgres at startup: %v", err)
	}
	log.Println("connected to postgres")

	if err := auth.EnsureSeedOperator(ctx, pool); err != nil {
		log.Fatalf("failed to seed demo operator: %v", err)
	}

	authHandler := auth.NewHandler(pool, []byte(jwtSecret))
	syncHandler := sync.NewHandler(pool)
	reportsHandler := reports.NewHandler(pool)

	router := gin.Default()

	router.GET("/healthz", func(c *gin.Context) {
		if err := pool.Ping(c.Request.Context()); err != nil {
			c.JSON(http.StatusServiceUnavailable, gin.H{"status": "db unreachable"})
			return
		}
		c.JSON(http.StatusOK, gin.H{"status": "ok"})
	})

	router.POST("/auth/login", authHandler.Login)

	syncRoutes := router.Group("/sync", authHandler.RequireAuth())
	syncRoutes.POST("/push", syncHandler.Push)
	syncRoutes.GET("/pull", syncHandler.Pull)

	// Phase 9 (plan section 4.7): behind the same operator JWT as /sync/*
	// — see internal/reports's package doc for why.
	reportsRoutes := router.Group("/reports", authHandler.RequireAuth())
	reportsRoutes.GET("/margin", reportsHandler.Margin)
	reportsRoutes.GET("/dead-stock", reportsHandler.DeadStock)
	reportsRoutes.GET("/avg-ticket", reportsHandler.AvgTicket)
	reportsRoutes.GET("/abc-curve", reportsHandler.ABCCurve)
	reportsRoutes.GET("/fiscal-alerts", reportsHandler.FiscalAlerts)

	// The dashboard page itself is a static shell (unauthenticated, like
	// any HTML page) that logs in and calls /reports/* client-side with
	// the returned bearer token — see internal/dashboard's package doc.
	router.GET("/dashboard", dashboard.Page)

	log.Printf("listening on :%s", port)
	if err := router.Run(":" + port); err != nil {
		log.Fatalf("server stopped: %v", err)
	}
}

func requireEnv(key string) string {
	value := os.Getenv(key)
	if value == "" {
		log.Fatalf("missing required env var %s", key)
	}
	return value
}
