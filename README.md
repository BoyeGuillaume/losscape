# losscape

Free, self-hosted, high-performance ML experiment tracking. See [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md).

## Run it

```sh
docker compose up -d --build --wait
```

Then open <http://localhost:8080>: the first account created becomes administrator. HTTPS,
sign-in providers, settings, backups and upgrades are in [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Layout

```
proto/                  protobuf schemas: the SDK <-> server contract
crates/
  losscape-core/       shared types (generated protocol, run IDs)
  losscape-server/     Axum API server
  losscape-loadgen/    ingest load generator (benchmarks, crash test)
  losscape-client/     SDK core: buffer, local log, upload, sync
  losscape-py/         native core of the Python SDK (PyO3)
sdk/python/             Python SDK (maturin package)
web/                    SvelteKit UI (static SPA served by the server)
deploy/                 the image, development databases, example settings
compose.yaml            the whole thing, in Docker
```

## Development

Requirements: Rust (stable), [uv](https://docs.astral.sh/uv/), Node 24 (`web/.nvmrc`), Docker. No `protoc` needed.

```sh
cp .env.example .env
make dev-db       # Postgres + ClickHouse in Docker
make dev-server   # API on http://localhost:8080
make dev-web      # UI on http://localhost:5173 (or build it: the server serves web/build)
```

Open the UI and create the first account: it becomes administrator and owns any existing
projects. Create an API key under Settings and export it for the SDK, benchmarks and the crash
test (`LOSSCAPE_API_KEY=df_...`).

## Accounts and security

- Passwords: Argon2id (m = 19 MiB, t = 2, p = 1, random 16-byte salt), failed sign-ins
  rate-limited, same response for unknown users and wrong passwords. No rules on strength:
  forms show an estimate and advice, in the browser.
- Browser sessions: random 256-bit token in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` over
  HTTPS); stored as SHA-256. Cross-site state-changing requests are refused.
- API keys (`df_...`): per user, shown once, stored as SHA-256, revocable.
- Projects: created in the UI (New project), never by the SDK; the creator is owner. One owner
  per project (manages members and settings; ownership moves only by transfer), editors (log
  runs), viewers (read). Non-members get 404.
- External sign-in: OpenID Connect (PKCE, verified ID token) and GitHub; see
  [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).
- Behind a reverse proxy, client addresses come from `X-Forwarded-For` only if it's listed in
  `LOSSCAPE_TRUSTED_PROXIES`.
- SQL is always parameterized; the UI escapes all output, behind a strict
  Content-Security-Policy (scripts from the server only, inline bootstrap allowed by hash).

```sh
make test lint    # what CI runs; server integration tests need make dev-db
make deploy-test  # a fresh deployment in Docker, a restart, check nothing is lost
make bench        # ingest throughput against a running server
make crash-test   # SIGKILL the server mid-load, check nothing is lost
```
