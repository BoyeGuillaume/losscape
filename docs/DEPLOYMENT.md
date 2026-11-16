# Running losscape

## Try it

You need Docker with Compose v2 and a few minutes for the first build.

```sh
git clone https://github.com/BoyeGuillaume/losscape && cd losscape
docker compose up -d --build --wait
```

Open <http://localhost:8080> and create an account: **the first one becomes administrator**. It
needs the setup token the server printed when it started:

```sh
docker compose logs losscape | grep -A2 "setup token"
```

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

By default it listens on this machine only. Before changing that:

1. **Create your account first.** The setup token keeps others from doing it before you, but
   there's no reason to leave it open. To choose the token instead of reading it from the log, for
   an automated deployment, set `LOSSCAPE_SETUP_TOKEN`.
2. **Set database passwords, before the very first start.** They're fixed when the databases
   are created. Copy `deploy/env.example` to `.env`, then `openssl rand -hex 32` for each of
   `LOSSCAPE_POSTGRES_PASSWORD` and `LOSSCAPE_CLICKHOUSE_PASSWORD`. The defaults are only
   protected by the databases being unreachable from outside.
3. **Use HTTPS** ([below](#https)), or passwords and API keys cross the network in the clear.
4. **Decide who can sign up.** With `LOSSCAPE_SIGNUP=closed`, nobody can create an account
   after the first, except through a linked external sign-in. By default anyone who can reach
   the server can.
5. **Know the password policy.** Any non-empty password is accepted; people see a strength
   estimate and advice. If you need more than that, use external sign-in.
6. **Decide on an administrator.** It owns every project. With `LOSSCAPE_ADMIN=forbidden`
   there's none: the first account is like the others, and an administrator account from before
   can't sign in, nor use its sessions or API keys. Allowed again with no administrator, the
   server prints a setup token at startup (or takes `LOSSCAPE_SETUP_TOKEN`), and an account
   becomes administrator with it (Settings, Account): nobody does without it.
7. **Decide who creates projects.** Anyone signed in, by default. With
   `LOSSCAPE_PROJECTS=approval`, people ask for one, and the administrator accepts or rejects it
   (Settings, Administration); accepted, it's owned by who asked. With `LOSSCAPE_ADMIN=forbidden`
   too, projects are locked: nobody can create one. That's a way to lock a server down: create
   the administrator and the projects, then forbid admin; to add a project later, restart with
   `LOSSCAPE_ADMIN=allowed`, add it, and lock again.

## HTTPS

With `deploy/compose.https.yaml`, Caddy serves losscape on port 443, and nothing else is open:
plain HTTP isn't served at all (not even to redirect), the server's own port is closed, and the
databases were never reachable. Add to `.env`:

```sh
COMPOSE_FILE=compose.yaml:deploy/compose.https.yaml
LOSSCAPE_DOMAIN=losscape.example.com
LOSSCAPE_PUBLIC_URL=https://losscape.example.com
```

then choose where the certificate comes from, and start it as usual:

```sh
docker compose up -d --build --wait
```

It needs Docker Compose 2.24.4 or newer. With `COMPOSE_FILE` in `.env`, every `docker compose`
command (`logs`, `down`...) includes Caddy by itself. The `https` URL also makes browsers refuse
plain HTTP for that name from then on (HSTS), and session cookies `Secure`.

### A certificate from Let's Encrypt

The default (`LOSSCAPE_CERTIFICATE=acme`), for a public name. Point the domain at the machine and
open port 443 to the internet: Let's Encrypt checks it there (TLS-ALPN), so port 80 stays closed.
Caddy gets the certificate at the first start, and renews it.

### Your own certificate, for internal DNS

For a name only your network knows (`losscape.corp.internal`), Let's Encrypt can't help: use a
certificate from your organization's CA. Put it next to `compose.yaml`:

```
certs/cert.pem   # the certificate, then its chain (intermediates)
certs/key.pem    # its private key, unencrypted
```

```sh
LOSSCAPE_CERTIFICATE=files
LOSSCAPE_CERTS_DIR=./certs   # the default; anywhere else, absolute or from compose.yaml
```

The certificate must name `LOSSCAPE_DOMAIN` (subject alternative name), and every machine using
it, browsers and training machines alike, must trust your CA. That's usually already the case on
managed machines; for the SDK elsewhere, see [The Python SDK](#the-python-sdk). After renewing
the files:

```sh
docker compose exec caddy caddy reload --config /etc/caddy/Caddyfile --force
```

No CA yet? [A private CA](#a-private-ca) makes one with `openssl`.

To listen on one interface only, rather than all of them: `LOSSCAPE_HTTPS_BIND=10.0.0.4`.

### Why Caddy

It gets and renews certificates by itself, over port 443 alone, in a configuration of a dozen
lines (`deploy/Caddyfile`), and serves HTTP/3. nginx does the job just as well behind; it needs
certbot (or another ACME client) and port 80 or DNS access for its certificates. Any proxy works:
see below.

### With your own reverse proxy

Leave `COMPOSE_FILE` out: the server then listens on `127.0.0.1:8080` of this machine, in plain
HTTP, for your proxy. Set `LOSSCAPE_PUBLIC_URL`, and tell the server who to believe about client
addresses. Failed sign-ins are limited per address: without this, all your users look like the
proxy and share one limit. With a proxy on the same machine as the containers, that's their
network's gateway:

```sh
LOSSCAPE_PUBLIC_URL=https://losscape.example.com
LOSSCAPE_TRUSTED_PROXIES=172.30.42.1
```

Only list your proxies: anyone else on the list could claim to be any address. The proxy must
set `X-Forwarded-For`, keep the `Host` header, and not buffer responses, since live updates are
a long-lived stream. For nginx, HTTPS only, with your own certificate:

```nginx
server {
    listen 443 ssl;
    listen [::]:443 ssl;
    http2 on;
    server_name losscape.example.com;
    ssl_certificate     /etc/nginx/tls/cert.pem;   # with its chain
    ssl_certificate_key /etc/nginx/tls/key.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_buffering off;
        proxy_request_buffering off;
        proxy_read_timeout 1h;
        client_max_body_size 32m;  # LOSSCAPE_MAX_BODY_MB
    }
}
# No `listen 80` anywhere: plain HTTP isn't served.
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
<LOSSCAPE_PUBLIC_URL>/api/v1/auth/oauth/<id>/callback
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
| `DATABASE_URL` | `database.url` | required | Postgres; TLS with `?sslmode=verify-full&sslrootcert=…` (and `sslcert`, `sslkey`) |
| `CLICKHOUSE_URL` | `clickhouse.url` | required | ClickHouse, HTTP interface |
| `CLICKHOUSE_USER` | `clickhouse.user` | `default` | |
| `CLICKHOUSE_PASSWORD` | `clickhouse.password` | empty | |
| `CLICKHOUSE_DATABASE` | `clickhouse.database` | `losscape` | |
| `CLICKHOUSE_CA_CERT` | `clickhouse.ca_cert` | public CAs | For `https://`: the CA (PEM) its certificate must be from |
| `CLICKHOUSE_CLIENT_CERT`, `CLICKHOUSE_CLIENT_KEY` | `clickhouse.client_cert`, `clickhouse.client_key` | none | A client certificate (PEM), for mutual TLS |
| `LOSSCAPE_LISTEN` | `listen` | `0.0.0.0:8080` | |
| `LOSSCAPE_TLS_CERT`, `LOSSCAPE_TLS_KEY` | `tls.cert`, `tls.key` | none | Serve HTTPS only, with this certificate (and chain) and key, PEM; read again when they change |
| `LOSSCAPE_PUBLIC_URL` | `public_url` | none | Required for OAuth; `https://` makes cookies `Secure` |
| `LOSSCAPE_SIGNUP` | `signup` | `open` | `open` or `closed` |
| `LOSSCAPE_SETUP_TOKEN` | `setup_token` | printed at startup | What becoming administrator takes, while there's none |
| `LOSSCAPE_ADMIN` | `admin` | `allowed` | `forbidden`: no administrator account, ever; existing ones can't be used |
| `LOSSCAPE_PROJECTS` | `projects` | `open` | `approval`: new projects take an administrator's approval (with admin forbidden: none can be made) |
| `LOSSCAPE_TRUSTED_PROXIES` | `trusted_proxies` | none | Addresses or networks, comma-separated |
| `LOSSCAPE_OAUTH` | one `[oauth.<id>]` section each | none | Provider ids, comma-separated |
| `LOSSCAPE_OAUTH_<ID>_*` | `oauth.<id>.*` | | `TYPE`, `ISSUER`, `URL`, `API_URL`, `CLIENT_ID`, `CLIENT_SECRET`, `NAME` |
| `LOSSCAPE_APP_NAME` | `app_name` | `losscape` | Name shown in the interface |
| `LOSSCAPE_FAVICON` | `favicon` | built-in | Image file: browser tabs, and next to the name |
| `LOSSCAPE_AUTH_BACKGROUND` | `auth_background` | plain | Image file: sign-in and sign-up pages |
| `LOSSCAPE_WEB_DIR` | `web_dir` | `web/build` | The built web UI; API only if absent |
| `LOSSCAPE_MAX_BODY_MB` | `max_body_mb` | `32` | Largest request |
| `LOSSCAPE_STORAGE` | `storage` | none (`/data/checkpoints` in `compose.yaml`) | Where checkpoints go: an absolute path, or `s3://bucket/prefix`; none: not kept |
| `LOSSCAPE_MAX_CHECKPOINT_GB` | `max_checkpoint_gb` | `100` | Largest checkpoint, all its files together |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`, `AWS_ENDPOINT` | | | For `s3://` storage; `AWS_ENDPOINT` for MinIO, R2 and others |
| `LOSSCAPE_FLUSH_INTERVAL_MS` | `ingest.flush_interval_ms` | `250` | Longest a batch waits to be written |
| `LOSSCAPE_FLUSH_MAX_ROWS` | `ingest.flush_max_rows` | `200000` | Write as soon as this many points wait |
| `LOSSCAPE_INGEST_QUEUE` | `ingest.queue` | `4096` | Batches queued before requests wait |
| `RUST_LOG` | | `info` | Log level |

Only for `compose.yaml`, in `.env`: `LOSSCAPE_BIND` (`127.0.0.1`), `LOSSCAPE_PORT` (`8080`),
`LOSSCAPE_POSTGRES_PASSWORD`, `LOSSCAPE_CLICKHOUSE_PASSWORD`, and `LOSSCAPE_IMAGE`, to run an
image you built or pulled elsewhere with `--no-build`. With `deploy/compose.https.yaml`
(`COMPOSE_FILE`): `LOSSCAPE_DOMAIN`, `LOSSCAPE_CERTIFICATE` (`acme` or `files`),
`LOSSCAPE_CERTS_DIR` (`./certs`) and `LOSSCAPE_HTTPS_BIND` (`0.0.0.0`).

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

### Using a published image

Instead of building, you can run the [image from Docker Hub](https://hub.docker.com/r/madproton/losscape). In `.env`, with the version you want:

```sh
LOSSCAPE_IMAGE=madproton/losscape:0.2.2
```

```sh
docker compose up -d --wait --no-build
```

Images exist for Intel and ARM Linux, as `<version>`, `<major>.<minor>` and `latest`. To upgrade,
change the version and run the same command. The databases are started the same way, so everything
else on this page applies.

### Publishing the server image

`.github/workflows/publish-image.yml` builds the image for both processors and publishes it when a
tag `server-v<version>` is pushed. It refuses a tag that doesn't match `version` in `Cargo.toml`, and
tags nothing unless both images run and report that version. Run by hand from the Actions tab, it
does all of that and publishes nothing.
Set a new version with `make bump VERSION=x.y.z` (see [Publishing a release](#publishing-a-release)).

It needs setting up once:

1. On Docker Hub, create the public repository, and a personal access token with read and write
   access.
2. In the GitHub repository's Settings → Secrets and variables → Actions, add the secrets
   `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`, and the variable `DOCKERHUB_REPOSITORY`, such as
   `madproton/losscape`.

```sh
git tag server-v0.2.2
git push origin server-v0.2.2
```

### Upgrading

Back up first: there's no way to downgrade a database once migrated.

```sh
git pull
docker compose up -d --build --wait
```

Database migrations run by themselves at startup. Older SDKs keep working with newer servers.

### Checkpoints

What runs upload with `run.log_checkpoint()` goes to `LOSSCAPE_STORAGE`: the `checkpoints`
volume of `compose.yaml` by default, or an S3-compatible store (`s3://bucket/prefix`, with the
`AWS_*` settings in `.env`). Either way, everything goes through the server, which checks who asks:
clients never reach the storage or its credentials. Plan the disk or the bucket for them:
checkpoints are large and kept until deleted, on the run page or from the SDK. With Adam, a
checkpoint of weights and optimizer state is about three times the weights (12 bytes per
parameter in fp32: 1.2 GB for 100M parameters), stored a little smaller (zstd saves about 7% on
fp32, more on bf16); the runs table's Checkpoints column shows what each run keeps. Project owners can have old
checkpoints deleted by themselves (project settings, Checkpoints): after so many days, except the
important ones and each run's last and best. Checkpoint links (`/ckpt/…`) are served by the
server like the API: a proxy in front passes them through, with large bodies and no buffering. Their files are
deleted from storage shortly after the checkpoint, run or project is.

### Backing up

Both databases are needed, and belong together, with the checkpoints volume if you keep them
there. Stop the server first, so they match; SDKs just wait, and catch up afterwards.

```sh
mkdir -p backup
docker compose stop losscape

docker compose exec -T postgres pg_dump -U losscape -Fc losscape > backup/postgres.dump

docker compose stop clickhouse
docker run --rm -v losscape_clickhouse:/data:ro -v "$PWD/backup":/backup alpine \
    tar czf /backup/clickhouse.tar.gz -C /data .
docker run --rm -v losscape_checkpoints:/data:ro -v "$PWD/backup":/backup alpine \
    tar cf /backup/checkpoints.tar -C /data .

docker compose up -d --wait
```

Keep your `.env` as well. To restore, into an empty deployment with the same passwords:

```sh
docker compose down -v
docker compose up -d --wait postgres
docker compose exec -T postgres pg_restore -U losscape -d losscape < backup/postgres.dump

docker run --rm -v losscape_clickhouse:/data -v "$PWD/backup":/backup alpine \
    sh -c 'rm -rf /data/* && tar xzf /backup/clickhouse.tar.gz -C /data && chown -R 101:101 /data'
docker run --rm -v losscape_checkpoints:/data -v "$PWD/backup":/backup alpine \
    sh -c 'rm -rf /data/* && tar xf /backup/checkpoints.tar -C /data && chown -R 65532:65532 /data'

docker compose up -d --wait
```

Try a restore once, somewhere safe, before you depend on it.

## Setting it up by hand

Without Docker, or with the databases on machines of their own. The parts, and what each must
reach:

```
browsers, SDKs ──443──▶ reverse proxy ──▶ losscape-server ──▶ Postgres    (5432)
                                                          └──▶ ClickHouse  (8123, or 8443 with TLS)
```

Only the proxy's port 443 needs to be reachable by users. The server needs to be reached by the
proxy only, and each database by the server only. Everything that crosses a network is
encrypted, and checked: each end verifies the other's certificate.

### The server

From the published image (a Linux with glibc 2.36 or newer: Debian 12, Ubuntu 24.04...):

```sh
id=$(docker create madproton/losscape:0.2.2)
sudo docker cp $id:/usr/local/bin/losscape-server /usr/local/bin/
sudo mkdir -p /opt/losscape && sudo docker cp $id:/app/web /opt/losscape/web
docker rm $id
```

Or built from source, with Rust and Node 24: `cargo build --release -p losscape-server`, and
`npm ci && npm run build` in `web/` (the result is `web/build`).

Settings go in a file (`deploy/losscape.example.toml` describes them all), secrets in the
environment. A user of its own, and a systemd unit:

```sh
sudo useradd --system --home /var/lib/losscape --shell /usr/sbin/nologin losscape
sudo mkdir -p /etc/losscape/tls
sudo cp deploy/losscape.example.toml /etc/losscape/losscape.toml   # then edit it
sudo install -m 600 /dev/null /etc/losscape/secrets.env            # DATABASE_URL, CLICKHOUSE_PASSWORD...
```

```ini
# /etc/systemd/system/losscape.service
[Unit]
Description=losscape server
Wants=network-online.target
After=network-online.target postgresql.service clickhouse-server.service

[Service]
User=losscape
ExecStart=/usr/local/bin/losscape-server --config /etc/losscape/losscape.toml
EnvironmentFile=/etc/losscape/secrets.env
Restart=on-failure
TimeoutStopSec=30
# Checkpoints, if kept on disk: storage = "/var/lib/losscape/checkpoints"
StateDirectory=losscape
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
CapabilityBoundingSet=
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload && sudo systemctl enable --now losscape
journalctl -u losscape -f
```

With the proxy on the same machine, have the server listen there only, `listen =
"127.0.0.1:8080"`, with `trusted_proxies = ["127.0.0.1"]` and `web_dir = "/opt/losscape/web"`.
The proxy's configuration is the one in [With your own reverse proxy](#with-your-own-reverse-proxy).

#### With the proxy on another machine

Then the hop between them crosses the network: the server serves HTTPS itself, and accepts only
the proxy.

```toml
listen = "10.0.0.4:8443"
trusted_proxies = ["10.0.0.3"]           # the proxy

[tls]
cert = "/etc/losscape/tls/app.pem"     # for the name the proxy uses: app.internal
key = "/etc/losscape/tls/app.key"
```

The files are read again when they change, so a renewed certificate needs no restart. The server
answers plain HTTP on that port with an error. The proxy checks the server's certificate against
your CA. With Caddy:

```caddy
losscape.example.com {
	reverse_proxy https://app.internal:8443 {
		flush_interval -1
		transport http {
			tls_trust_pool file /etc/caddy/tls/ca.pem
		}
	}
}
```

With nginx, in the `location` above:

```nginx
proxy_pass https://app.internal:8443;
proxy_ssl_verify on;
proxy_ssl_trusted_certificate /etc/nginx/tls/ca.pem;
proxy_ssl_name app.internal;
proxy_ssl_server_name on;
```

And at the firewall, port 8443 of the server open to the proxy only (below).

### A private CA

Certificates for names on internal DNS, for the server, the databases, and losscape as a client
of them, all from an authority of your own. With `openssl`, in an empty folder (`bash`):

```sh
# The authority: it signs the others. Keep ca.key offline once they're made.
openssl req -x509 -new -noenc -newkey ec -pkeyopt ec_paramgen_curve:P-256 -days 3650 \
  -subj "/CN=losscape private CA" -keyout ca.key -out ca.pem \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"

# cert FILE USAGE [NAME]: FILE.pem and FILE.key, for a server (by its DNS name) or a client.
cert() {
  openssl req -new -noenc -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
    -subj "/CN=${3:-$1}" -keyout "$1.key" -out "$1.csr"
  openssl x509 -req -in "$1.csr" -CA ca.pem -CAkey ca.key -days 825 -out "$1.pem" -extfile <(
    echo "basicConstraints=critical,CA:FALSE"
    echo "keyUsage=critical,digitalSignature"
    echo "extendedKeyUsage=$2"
    echo "subjectKeyIdentifier=hash"
    echo "authorityKeyIdentifier=keyid"
    if [ -n "$3" ]; then echo "subjectAltName=DNS:$3"; fi
  )
  rm "$1.csr"
}

cert app serverAuth app.internal       # the server, for its proxy
cert db serverAuth db.internal         # the database machine
cert losscape clientAuth               # losscape, as a client of the databases
```

Copy each `.pem` and its `.key` where it's used, with `ca.pem`, readable only by the service
that uses it (`chmod 600` the keys). Renew them before 825 days, the same way.

### Databases on the same machine

Nothing to encrypt: nothing leaves the machine. Only make sure they aren't reachable from
elsewhere.

**Postgres**, over its Unix socket, where it knows who connects without a password (peer
authentication): the `losscape` system user is the `losscape` role, and nobody else is.

```sh
sudo -u postgres createuser losscape
sudo -u postgres createdb -O losscape losscape
```

```
# pg_hba.conf, before the other lines
local   losscape   losscape   peer
```

```sh
DATABASE_URL=postgres:///losscape?host=/run/postgresql&user=losscape
```

`listen_addresses = 'localhost'` in `postgresql.conf`, the default, or `''` to listen on the
socket only.

**ClickHouse** listens on this machine only by default; to be sure, in
`/etc/clickhouse-server/config.d/listen.xml`:

```xml
<clickhouse>
    <listen_host>127.0.0.1</listen_host>
</clickhouse>
```

And a user of its own, that can only connect from here, in
`/etc/clickhouse-server/users.d/losscape.xml` (`openssl rand -hex 32` for the password, and
`echo -n "$password" | sha256sum` for its hash):

```xml
<clickhouse>
    <users>
        <losscape>
            <password_sha256_hex>…</password_sha256_hex>
            <networks><ip>127.0.0.1</ip><ip>::1</ip></networks>
            <profile>default</profile>
            <quota>default</quota>
        </losscape>
    </users>
</clickhouse>
```

```sh
CLICKHOUSE_URL=http://127.0.0.1:8123
CLICKHOUSE_USER=losscape
CLICKHOUSE_PASSWORD=…
```

### Databases on another machine

What crosses the network is encrypted with TLS, and the two ends check each other with
certificates from [your CA](#a-private-ca) (mutual TLS):

- The server checks the database's certificate, so it can't be talking to an impostor that
  would collect its password and data.
- The database checks the server's (`losscape.pem`), so a connection without it is refused
  before any sign-in: a stolen or guessed password isn't enough, and nobody else gets to try.
- The password stays, as a second factor.
- The firewall lets only the server's address in.

That's stronger than a password alone (a pre-shared secret): anyone who can reach the port
could try it, and without a checked certificate, an impostor would just be handed it. A
WireGuard tunnel between the two machines (keys, plus its optional pre-shared key) is as good,
if you'd rather run one tunnel than manage certificates; the database then listens on the
tunnel's address only.

Below, the server is `10.0.0.4`, the database machine `db.internal` (`10.0.0.5`), with `db.pem`
and `db.key`, and `ca.pem`, in each service's `tls` folder.

**Postgres** speaks TLS itself. In `postgresql.conf`:

```
listen_addresses = '10.0.0.5'
ssl = on
ssl_cert_file = '/etc/postgresql/tls/db.pem'
ssl_key_file = '/etc/postgresql/tls/db.key'
ssl_ca_file = '/etc/postgresql/tls/ca.pem'
ssl_min_protocol_version = 'TLSv1.3'
```

In `pg_hba.conf`, the only line for this database: over TLS, from the server, with a password
and a client certificate whose name is the role's (`CN=losscape`, as made above):

```
hostssl   losscape   losscape   10.0.0.4/32   scram-sha-256   clientcert=verify-full
```

And on the losscape server:

```sh
DATABASE_URL=postgres://losscape:PASSWORD@db.internal:5432/losscape?sslmode=verify-full&sslrootcert=/etc/losscape/tls/ca.pem&sslcert=/etc/losscape/tls/losscape.pem&sslkey=/etc/losscape/tls/losscape.key
```

`verify-full` checks that the certificate is from your CA and for `db.internal`.

**ClickHouse** behind a reverse proxy on its own machine: ClickHouse listens on `127.0.0.1`
only (as on the same machine, above), and the proxy takes TLS connections, from clients with a
certificate of your CA only. With Caddy:

```caddy
{
	auto_https disable_redirects
}

https://db.internal:8443 {
	tls /etc/caddy/tls/db.pem /etc/caddy/tls/db.key {
		protocols tls1.3
		client_auth {
			mode require_and_verify
			trust_pool file /etc/caddy/tls/ca.pem
		}
	}
	reverse_proxy 127.0.0.1:8123 {
		flush_interval -1
	}
}
```

Or with nginx:

```nginx
server {
    listen 10.0.0.5:8443 ssl;
    server_name db.internal;
    ssl_certificate        /etc/nginx/tls/db.pem;
    ssl_certificate_key    /etc/nginx/tls/db.key;
    ssl_protocols          TLSv1.3;
    ssl_client_certificate /etc/nginx/tls/ca.pem;
    ssl_verify_client      on;

    location / {
        proxy_pass http://127.0.0.1:8123;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_request_buffering off;
        proxy_read_timeout 300s;
        client_max_body_size 0;  # inserts can be large; only losscape gets this far
    }
}
```

ClickHouse can also serve HTTPS itself (`https_port`, with its `openSSL` settings) if you'd
rather not run a proxy. Either way, on the losscape server:

```sh
CLICKHOUSE_URL=https://db.internal:8443
CLICKHOUSE_USER=losscape
CLICKHOUSE_PASSWORD=…
CLICKHOUSE_CA_CERT=/etc/losscape/tls/ca.pem
CLICKHOUSE_CLIENT_CERT=/etc/losscape/tls/losscape.pem
CLICKHOUSE_CLIENT_KEY=/etc/losscape/tls/losscape.key
```

With `CLICKHOUSE_CA_CERT`, only certificates from that CA are trusted (not the public ones);
without it, the public ones are.

**The firewall** of the database machine, with `ufw` for instance (or the same in `nftables`, or
your cloud's security groups):

```sh
sudo ufw default deny incoming
sudo ufw allow from 10.0.0.4 to any port 5432,8443 proto tcp
sudo ufw allow from <your admin network> to any port 22 proto tcp
sudo ufw enable
```

### Check what's open

On each machine, what listens, and on which address (`0.0.0.0` and `*` are every address):

```sh
sudo ss -ltnup
```

From another machine: `nmap -Pn -p- losscape.example.com` should show 443 only. With Docker,
published ports bypass `ufw`: what's open is what `docker compose ps` lists under PORTS, which
with `deploy/compose.https.yaml` is 443 only.

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

It trusts the certificates the system trusts. With a server certificate from your own CA, add the
CA to the system's store (`sudo cp ca.pem /usr/local/share/ca-certificates/my-ca.crt && sudo
update-ca-certificates` on Debian and Ubuntu), or, without root, point `SSL_CERT_FILE` at a file
with the certificates to trust, which replaces the system's:

```sh
cat /etc/ssl/certs/ca-certificates.crt ca.pem > ~/.config/trusted.pem
export SSL_CERT_FILE=~/.config/trusted.pem
```

### Publishing a release

`.github/workflows/publish-sdk.yml` builds the wheels and publishes them when a tag `sdk-v<version>`
is pushed. It refuses a tag that doesn't match `version` in `Cargo.toml`, and PyPI never accepts the
same version twice, so bump the version first: `make bump VERSION=x.y.z` sets it everywhere it's
written (`scripts/bump_version.py`, which can also commit it with `--commit`). Run by hand from the
Actions tab, the workflow builds and checks the wheels without publishing, which is a good first test.

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
git tag sdk-v0.2.2
git push origin sdk-v0.2.2
```

## Troubleshooting

- **`address already in use`:** set another `LOSSCAPE_PORT` in `.env`.
- **`Pool overlaps with other one on this docker network`:** another network uses
  `172.30.42.0/24`. Change the subnet and the server's address in `compose.yaml`, Caddy's in
  `deploy/compose.https.yaml` (twice), and `LOSSCAPE_TRUSTED_PROXIES` if you set it.
- **`password authentication failed` after changing a database password:** it was fixed when the
  database was created. Change it in the database too, or start again with `docker compose down -v`,
  which deletes everything.
- **The setup token isn't accepted, or got lost:** a new one is printed at each start, while there
  is no administrator. Restart, and use the latest.
- **Everyone gets "too many attempts" behind a proxy:** see `LOSSCAPE_TRUSTED_PROXIES` above.
- **Charts stop updating live behind a proxy:** it's buffering. See the nginx example above.
- **`ports: !reset` isn't understood, or Caddy doesn't start:** `deploy/compose.https.yaml` needs
  Docker Compose 2.24.4 or newer (`docker compose version`).
- **The SDK says `invalid peer certificate: UnknownIssuer`:** the server's certificate is from a
  CA this machine doesn't trust. See [The Python SDK](#the-python-sdk).
- **`HTTP client: builder error` from the SDK:** `SSL_CERT_FILE` is set, but empty or to a file
  that can't be read.
- **Sign-in with a provider says "redirect URI mismatch":** it must match
  `<LOSSCAPE_PUBLIC_URL>/api/v1/auth/oauth/<id>/callback` exactly.

## Licenses of what's inside

The About page (`/about`, for anyone who can reach the server, signed in or not) lists the
versions and licenses of the server's and web interface's libraries, with the full texts at
`/THIRD-PARTY-NOTICES-SERVER.txt` and `/THIRD-PARTY-NOTICES.txt`. Running from source rather than
the image, `make server-notices` adds the server's after building the web interface. The wheel has
the SDK's inside its package. Builds
fail on a dependency whose license isn't allowed: see `scripts/third_party_notices.py` and
`web/third-party-notices.ts`.
