.PHONY: dev-backend dev-client db-up db-down migrate-backend migrate-client

# Load .env if present so DATABASE_URL/JWT_SECRET/etc. reach both
# docker-compose and `go run`. Falls back to the compose/Go defaults
# (see .env.example) when no .env exists yet.
-include .env
export

MIGRATE_VERSION := v4.19.1

dev-backend:
	cd apps/backend && go run ./cmd/api

dev-client:
	cd apps/client && npm run tauri dev

db-up:
	docker compose up -d postgres

db-down:
	docker compose down

migrate-backend:
	cd apps/backend && go run -tags 'postgres' github.com/golang-migrate/migrate/v4/cmd/migrate@$(MIGRATE_VERSION) \
		-path=migrations -database "$$DATABASE_URL" up

migrate-client:
	@echo "SQLite migrations run automatically when the Tauri app boots"
	@echo "(tauri-plugin-sql applies apps/client/src-tauri/migrations/*.sql on first connection)."
	@echo "There is no separate CLI step — just run 'make dev-client'."
