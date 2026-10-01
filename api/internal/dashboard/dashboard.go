// Package dashboard serves the owner-facing back-office page (plan
// section 4.7: "a simple web dashboard ... can be a lightweight web page
// served by the Go backend itself, with no need for a separate frontend
// in the MVP").
//
// The page itself is a static HTML+vanilla-JS shell (go:embed'd, no build
// step, no new frontend dependency) served unauthenticated at GET
// /dashboard — same as any other static HTML page. It logs in through
// the existing POST /auth/login (the same device+operator-PIN flow every
// Tauri terminal uses — plan section 2 has no separate "owner" role) and
// then calls internal/reports's JSON endpoints client-side, bearer token
// in hand, exactly like any other authenticated API caller. No session
// cookie or server-side template state: the Go side stays a plain static
// file server for this route.
package dashboard

import (
	_ "embed"
	"net/http"

	"github.com/gin-gonic/gin"
)

//go:embed static/dashboard.html
var pageHTML []byte

// Page handles GET /dashboard.
func Page(c *gin.Context) {
	c.Data(http.StatusOK, "text/html; charset=utf-8", pageHTML)
}
