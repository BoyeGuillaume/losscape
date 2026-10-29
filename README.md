# losscape

Free, self-hosted, high-performance ML experiment tracking. See [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md).

> Early development: the ingest API is done (step 1); the Python SDK comes next.

## Layout

```
proto/                  protobuf schemas: the SDK <-> server contract
crates/
  losscape-core/       shared types (generated protocol, run IDs)
  losscape-server/     Axum API server
  losscape-loadgen/    ingest load generator (benchmarks, crash test)
  losscape-py/         native core of the Python SDK (PyO3)
sdk/python/             Python SDK (maturin package)
web/                    Vite + React + TypeScript UI
deploy/                 docker compose files
```

## Development

Requirements: Rust (stable), [uv](https://docs.astral.sh/uv/), Node 20+, Docker. No `protoc` needed.

```sh
cp .env.example .env
make dev-db       # Postgres + ClickHouse in Docker
make dev-server   # API on http://localhost:8080
make dev-web      # UI on http://localhost:5173
make test lint    # what CI runs; server integration tests need make dev-db
make bench        # ingest throughput against a running server
make crash-test   # SIGKILL the server mid-load, check nothing is lost
```
