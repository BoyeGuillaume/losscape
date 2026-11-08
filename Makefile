COMPOSE := docker compose -f deploy/docker-compose.dev.yml
PY := cd sdk/python && uv run

.PHONY: help dev-db dev-db-down dev-server dev-web build test lint fmt bench crash-test up down deploy-test wheel notices server-notices

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

up: ## Build and start the whole thing in Docker, on http://localhost:8080
	docker compose up -d --build --wait

down: ## Stop it (keeps data)
	docker compose down

deploy-test: export LOSSCAPE_SETUP_TOKEN = deploy-test-token
deploy-test: ## Fresh deployment in Docker, restart, check nothing is lost (deletes its data)
	docker compose -p losscape-test up -d --build --wait
	cd sdk/python && uv run python ../../scripts/deploy_smoke.py --state ../../target/deploy-smoke.json
	docker compose -p losscape-test restart losscape
	cd sdk/python && uv run python ../../scripts/deploy_smoke.py --after-restart --state ../../target/deploy-smoke.json
	docker compose -p losscape-test down -v
	rm -f target/deploy-smoke.json

wheel: notices ## Build the SDK's wheel for this machine into target/dist
	rm -rf target/dist
	cd sdk/python && uv run maturin build --release --out ../../target/dist

server-notices: ## List the server's libraries for the About page (after building the web UI)
	python3 scripts/third_party_notices.py --package losscape-server --output web/build/THIRD-PARTY-NOTICES-SERVER.txt --json web/build/third-party-server.json

notices: ## Write the SDK's license notices, and check every crate's license is allowed
	python3 scripts/third_party_notices.py --check
	python3 scripts/third_party_notices.py --package losscape-py --output sdk/python/src/lsc/THIRD-PARTY-NOTICES.txt

crash-test: ## SIGKILL the server mid-load, restart, check no point is lost
	cargo build --release -p losscape-server -p losscape-loadgen
	python3 scripts/crash_test.py
