COMPOSE := docker compose -f deploy/docker-compose.dev.yml
PY := cd sdk/python && uv run

.PHONY: help dev-db dev-db-down dev-server dev-web build test lint fmt bench crash-test

help: ## Show targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-14s %s\n", $$1, $$2}'

dev-db: ## Start Postgres + ClickHouse
	$(COMPOSE) up -d --wait

dev-db-down: ## Stop dev databases (keeps data)
	$(COMPOSE) down

dev-server: ## Run the API server on :8080
	cargo run -p losscape-server

dev-web: ## Run the web app on :5173
	cd web && npm run dev

build: ## Build everything
	cargo build --workspace
	cd sdk/python && uv sync
	cd web && npm ci && npm run build

test: ## Run all tests
	cargo test --workspace
	$(PY) pytest
	cd web && npm test

lint: ## Lint and type-check everything
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	$(PY) ruff check . ../../scripts
	$(PY) ruff format --check . ../../scripts
	cd web && npm run lint && npm run check

fmt: ## Format everything
	cd web && npm run format
	cargo fmt --all
	$(PY) ruff format . ../../scripts
	$(PY) ruff check --fix . ../../scripts

bench: ## Ingest benchmark against a running server (make dev-server)
	cargo run --release -p losscape-loadgen -- --runs 32 --metrics 32 --steps 20000

crash-test: ## SIGKILL the server mid-load, restart, check no point is lost
	cargo build --release -p losscape-server -p losscape-loadgen
	python3 scripts/crash_test.py
