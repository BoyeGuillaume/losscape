# Running losscape

## Try it

You need Docker with Compose v2 and a few minutes for the first build.

```sh
git clone https://github.com/BoyeGuillaume/losscape && cd losscape
docker compose up -d --build --wait
```

Open <http://localhost:8080> and create an account: **the first one becomes administrator**.
Then create a project, and an API key under Settings, and log a run:

```sh
pip install losscape-sdk   # see "The Python SDK" below
export LOSSCAPE_API_KEY=df_...
```

```python
import lsc

with lsc.init(project="my-project") as run:
    for step in range(100):
        run.log({"loss": 1 / (step + 1)})
```

This runs three containers: `losscape` (server and web UI, stateless), `postgres` (accounts,
projects, runs) and `clickhouse` (metrics and console output). The databases aren't reachable
from outside them. A small team fits on 4 vCPU and 8 GB of memory.

## Before opening it to others

By default it listens on this machine only, because **anyone who reaches a new server first
can create its administrator account**. Before changing that:

1. **Create your account first.**
2. **Set database passwords, before the very first start.** They're fixed when the databases
   are created. Copy `deploy/env.example` to `.env`, then `openssl rand -hex 32` for each of
   `LOSSCAPE_POSTGRES_PASSWORD` and `LOSSCAPE_CLICKHOUSE_PASSWORD`. The defaults are only
   protected by the databases being unreachable from outside.
3. **Use HTTPS**, or passwords and API keys cross the network in the clear.
4. **Decide who can sign up.** With `LOSSCAPE_SIGNUP=closed`, nobody can create an account
   after the first, except through a linked external sign-in. By default anyone who can reach
   the server can.
5. **Know the password policy.** Any non-empty password is accepted; people see a strength
   estimate and advice. If you need more than that, use external sign-in.

## HTTPS

### With the included Caddy

Point a domain at the machine, open ports 80 and 443, and add to `.env`:

```sh
LOSSCAPE_DOMAIN=losscape.example.com
LOSSCAPE_PUBLIC_URL=https://losscape.example.com
```

```sh
docker compose --profile https up -d --build --wait
```

Caddy gets and renews the certificate. Port 8080 stays on this machine only.

### With your own reverse proxy

Set `LOSSCAPE_PUBLIC_URL`, and tell the server who to believe about client addresses. Failed
sign-ins are limited per address: without this, all your users look like the proxy and share
one limit. With a proxy on the same machine as the containers, that's their network's gateway:

```sh
LOSSCAPE_PUBLIC_URL=https://losscape.example.com
LOSSCAPE_TRUSTED_PROXIES=172.30.42.1
```

Only list your proxies: anyone else on the list could claim to be any address. The proxy must
set `X-Forwarded-For`, keep the `Host` header, and not buffer responses, since live updates are
a long-lived stream. For nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_buffering off;
    proxy_read_timeout 1h;
    client_max_body_size 32m;  # LOSSCAPE_MAX_BODY_MB
}
```

## Your own name and look

In `.env`, with any images in a `branding/` folder next to `compose.yaml`:

```sh
LOSSCAPE_APP_NAME=Acme Experiments
LOSSCAPE_FAVICON=/branding/favicon.svg           # browser tabs, and next to the name
LOSSCAPE_AUTH_BACKGROUND=/branding/background.jpg  # behind the sign-in and sign-up forms
```

Images can be png, jpg, webp, avif, gif, svg or ico, up to 16 MB. They're read at startup, and a
wrong path stops the server with an error saying which setting is wrong. To apply a change:
`docker compose up -d --force-recreate losscape`.

The name is the interface's. The Python package, the `losscape` command and the SDK's messages
keep theirs, and the About page says what it runs on.

## External sign-in

Register an application with the provider, with this redirect URI, where `<id>` is the name you
give it:

```
<LOSSCAPE_PUBLIC_URL>/api/v0/auth/oauth/<id>/callback
```

Then in `.env`:

```sh
LOSSCAPE_PUBLIC_URL=https://losscape.example.com
LOSSCAPE_OAUTH=google,github

# Any OpenID Connect provider: Google, Microsoft, GitLab, Keycloak, Authentik, Okta...
LOSSCAPE_OAUTH_GOOGLE_ISSUER=https://accounts.google.com
LOSSCAPE_OAUTH_GOOGLE_CLIENT_ID=...
LOSSCAPE_OAUTH_GOOGLE_CLIENT_SECRET=...
LOSSCAPE_OAUTH_GOOGLE_NAME=Google        # on the button; optional

# GitHub; for GitHub Enterprise add _URL and _API_URL
LOSSCAPE_OAUTH_GITHUB_CLIENT_ID=...
LOSSCAPE_OAUTH_GITHUB_CLIENT_SECRET=...
```

A provider is `oidc` unless its id is `github`, or `LOSSCAPE_OAUTH_<ID>_TYPE` says otherwise.
Leave `LOSSCAPE_OAUTH` unset and the sign-in page shows no such buttons.

## All settings

From the environment or, without containers, a TOML file: `losscape-server --config <file>`,
or `LOSSCAPE_CONFIG=<file>`. See `deploy/losscape.example.toml`. The environment wins over the
file, so secrets can stay out of it. An unknown key in the file stops the server from starting,
so a typo can't quietly leave a setting at its default.

| Environment variable | In the file | Default | |
|---|---|---|---|
| `DATABASE_URL` | `database.url` | required | Postgres |
| `CLICKHOUSE_URL` | `clickhouse.url` | required | ClickHouse, HTTP interface |
| `CLICKHOUSE_USER` | `clickhouse.user` | `default` | |
| `CLICKHOUSE_PASSWORD` | `clickhouse.password` | empty | |
| `CLICKHOUSE_DATABASE` | `clickhouse.database` | `losscape` | |
| `LOSSCAPE_LISTEN` | `listen` | `0.0.0.0:8080` | |
| `LOSSCAPE_PUBLIC_URL` | `public_url` | none | Required for OAuth; `https://` makes cookies `Secure` |
| `LOSSCAPE_SIGNUP` | `signup` | `open` | `open` or `closed` |
| `LOSSCAPE_TRUSTED_PROXIES` | `trusted_proxies` | none | Addresses or networks, comma-separated |
| `LOSSCAPE_OAUTH` | one `[oauth.<id>]` section each | none | Provider ids, comma-separated |
| `LOSSCAPE_OAUTH_<ID>_*` | `oauth.<id>.*` | | `TYPE`, `ISSUER`, `URL`, `API_URL`, `CLIENT_ID`, `CLIENT_SECRET`, `NAME` |
| `LOSSCAPE_APP_NAME` | `app_name` | `losscape` | Name shown in the interface |
| `LOSSCAPE_FAVICON` | `favicon` | built-in | Image file: browser tabs, and next to the name |
| `LOSSCAPE_AUTH_BACKGROUND` | `auth_background` | plain | Image file: sign-in and sign-up pages |
| `LOSSCAPE_WEB_DIR` | `web_dir` | `web/build` | The built web UI; API only if absent |
| `LOSSCAPE_MAX_BODY_MB` | `max_body_mb` | `32` | Largest request |
| `LOSSCAPE_FLUSH_INTERVAL_MS` | `ingest.flush_interval_ms` | `250` | Longest a batch waits to be written |
| `LOSSCAPE_FLUSH_MAX_ROWS` | `ingest.flush_max_rows` | `200000` | Write as soon as this many points wait |
| `LOSSCAPE_INGEST_QUEUE` | `ingest.queue` | `4096` | Batches queued before requests wait |
| `RUST_LOG` | | `info` | Log level |

Only for `compose.yaml`, in `.env`: `LOSSCAPE_BIND` (`127.0.0.1`), `LOSSCAPE_PORT` (`8080`),
`LOSSCAPE_POSTGRES_PASSWORD`, `LOSSCAPE_CLICKHOUSE_PASSWORD`, `LOSSCAPE_DOMAIN`, and
`LOSSCAPE_IMAGE`, to run an image you built or pulled elsewhere with `--no-build`.

## Operating it

| | |
|---|---|
| `GET /healthz` | The process is up |
| `GET /readyz` | Both databases answer; what the container health check uses |
| `docker compose logs -f losscape` | Logs |
| `docker compose down` | Stops everything, keeps the data |
| `docker compose down -v` | **Deletes all data** |

Stopping is safe at any time. The server writes what's queued, then exits. SDKs keep everything
in a local log until the server confirms it's stored, and retry for as long as it takes, so runs
in progress lose nothing while it's down, whether it's restarting or being upgraded.

### Upgrading

Back up first: there's no way to downgrade a database once migrated.

```sh
git pull
docker compose up -d --build --wait
```

Database migrations run by themselves at startup. Older SDKs keep working with newer servers.

### Backing up

Both databases are needed, and belong together. Stop the server first, so they match; SDKs just
wait, and catch up afterwards.

```sh
mkdir -p backup
docker compose stop losscape

docker compose exec -T postgres pg_dump -U losscape -Fc losscape > backup/postgres.dump

docker compose stop clickhouse
docker run --rm -v losscape_clickhouse:/data:ro -v "$PWD/backup":/backup alpine \
    tar czf /backup/clickhouse.tar.gz -C /data .

docker compose up -d --wait
```

Keep your `.env` as well. To restore, into an empty deployment with the same passwords:

```sh
docker compose down -v
docker compose up -d --wait postgres
docker compose exec -T postgres pg_restore -U losscape -d losscape < backup/postgres.dump

docker run --rm -v losscape_clickhouse:/data -v "$PWD/backup":/backup alpine \
    sh -c 'rm -rf /data/* && tar xzf /backup/clickhouse.tar.gz -C /data && chown -R 101:101 /data'

docker compose up -d --wait
```

Try a restore once, somewhere safe, before you depend on it.

## The Python SDK

Install it wherever your training runs. The package is `losscape-sdk`, and it's imported as
`lsc`. The package named plain `losscape` on PyPI is an unrelated project.

```sh
pip install losscape-sdk
```

Wheels are published for Linux (x86-64 and ARM, glibc 2.28 or newer), macOS on Apple silicon and
Windows on x86-64. For anything else, or to run unreleased code, build one, which needs Rust and
[uv](https://docs.astral.sh/uv/):

```sh
make wheel
pip install target/dist/losscape_sdk-*.whl
```

A wheel only works on systems like the one that built it: same operating system and processor,
and at least the same glibc version on Linux. To use it on a cluster, build on the oldest system
you need to support, or install from source there: `pip install ./sdk/python`.

The SDK needs `LOSSCAPE_SERVER` (default `http://localhost:8080`) and `LOSSCAPE_API_KEY`.

### Publishing a release

`.github/workflows/publish-sdk.yml` builds the wheels and publishes them when a tag `sdk-v<version>`
is pushed. It refuses a tag that doesn't match `version` in `Cargo.toml`, and PyPI never accepts the
same version twice, so bump the version first. Run by hand from the Actions tab, it builds and checks
the wheels without publishing, which is a good first test.

It stores no password: PyPI trusts this workflow directly. That needs setting up once, on
[PyPI's publishing page](https://pypi.org/manage/account/publishing/), as a pending publisher:

| Field | Value |
| --- | --- |
| PyPI project name | `losscape-sdk` |
| Owner | `BoyeGuillaume` |
| Repository name | `losscape` |
| Workflow name | `publish-sdk.yml` |
| Environment name | `pypi` |

Also create the `pypi` environment under the repository's Settings → Environments. Requiring a
reviewer there means every release waits for your approval.

```sh
git tag sdk-v0.1.1
git push origin sdk-v0.1.1
```

## Troubleshooting

- **`address already in use`:** set another `LOSSCAPE_PORT` in `.env`.
- **`Pool overlaps with other one on this docker network`:** another network uses
  `172.30.42.0/24`. Change the subnet and the two addresses in `compose.yaml`, and
  `LOSSCAPE_TRUSTED_PROXIES` if you set it.
- **`password authentication failed` after changing a database password:** it was fixed when the
  database was created. Change it in the database too, or start again with `docker compose down -v`,
  which deletes everything.
- **Everyone gets "too many attempts" behind a proxy:** see `LOSSCAPE_TRUSTED_PROXIES` above.
- **Charts stop updating live behind a proxy:** it's buffering. See the nginx example above.
- **Sign-in with a provider says "redirect URI mismatch":** it must match
  `<LOSSCAPE_PUBLIC_URL>/api/v0/auth/oauth/<id>/callback` exactly.

## Licenses of what's inside

The About page (`/about`, for anyone who can reach the server, signed in or not) lists the
versions and licenses of the server's and web interface's libraries, with the full texts at
`/THIRD-PARTY-NOTICES-SERVER.txt` and `/THIRD-PARTY-NOTICES.txt`. Running from source rather than
the image, `make server-notices` adds the server's after building the web interface. The wheel has
the SDK's inside its package. Builds
fail on a dependency whose license isn't allowed: see `scripts/third_party_notices.py` and
`web/third-party-notices.ts`.
