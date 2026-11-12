# losscape

Free, self-hosted, high-performance ML experiment tracking: log metrics from your training code,
and follow and compare your runs in a web interface on your own server.

[![CI](https://github.com/BoyeGuillaume/losscape/actions/workflows/ci.yml/badge.svg)](https://github.com/BoyeGuillaume/losscape/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/losscape-sdk)](https://pypi.org/project/losscape-sdk/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

- [Quick start](#quick-start)
- [Using the SDK](#using-the-sdk)
- [Connecting the SDK to a server](#connecting-the-sdk-to-a-server)
- [Running the server](#running-the-server)
- [Development](#development)

## Quick start

You need Docker with Compose v2 for the server, and Python 3.10 or newer for the SDK.

**1. Start the server**, with the [published image](https://hub.docker.com/r/madproton/losscape):

```sh
git clone https://github.com/BoyeGuillaume/losscape && cd losscape
echo 'LOSSCAPE_IMAGE=madproton/losscape:latest' > .env
docker compose up -d --wait --no-build
```

To build it from source instead, which takes a few minutes, skip the `.env` line and run
`docker compose up -d --build --wait`.

**2. Set it up** at <http://localhost:8080>:

1. Create an account. The first one becomes administrator, and needs the setup token the server
   printed when it started, so that nobody else can claim your server by getting there first:

   ```sh
   docker compose logs losscape | grep -A2 "setup token"
   ```

2. Create a project with **New project**. The SDK can't create projects.
3. Create an API key under **Settings**, and copy it: it's only shown once.

**3. Install the SDK** where your training runs, and tell it about the server:

```sh
pip install losscape-sdk
export LOSSCAPE_SERVER=http://localhost:8080
export LOSSCAPE_API_KEY=df_...
```

The package is `losscape-sdk`, imported as `lsc`. The package named plain `losscape` on PyPI is an
unrelated project.

**4. Log a run:**

```python
import lsc

with lsc.init(project="my-project", config={"lr": 1e-3, "batch_size": 64}) as run:
    for step in range(1000):
        loss = 1 / (step + 1)
        run.log({"train/loss": loss, "val": {"acc": 1 - loss}})
    run.summary["best_loss"] = loss
```

Use the name of the project you created. The run shows up in the web interface while it's running.

## Using the SDK

The API follows Weights & Biases, so it should look familiar.

```python
import lsc

run = lsc.init(
    project="my-project",
    name="baseline",                   # default: a random name like brave-falcon-42
    config={"lr": 1e-3},               # what this run tries; you can filter and compare on it
    tags=["baseline"],
    group="sweep-1",
    notes="First try",
)

run.config["optimizer"] = "adam"       # can still be added to
run.log({"loss": 0.5})                 # each call is one step
run.log({"loss": 0.4, "acc": 0.9}, step=10)  # or choose the step
lsc.log({"loss": 0.3})                 # the current run, from anywhere in your code
run.summary["best_acc"] = 0.9          # the run's headline figures

run.finish()                           # waits for the upload; `with` does this for you
```

- **Recorded for you:** console output, system metrics (CPU, memory, disk, network, NVIDIA GPUs), and
  the environment (git commit and uncommitted diff, command line, packages, hardware). Console capture
  is off on Windows and in Jupyter.
- **Secrets:** everyone with access to the project can read this, so what looks like a secret is
  replaced by `***` before it's stored. That only catches what can be recognized, so turn off what
  you don't want recorded: `lsc.init(git_diff=False, console=False)`. See
  [Secrets](sdk/python/README.md#secrets).
- **Models:** `run.describe_model(model, args=(example_input,))` records the layers, shapes, parameters
  and FLOPs of a `torch.nn.Module`, for the run's Model tab.
- **Resuming:** `lsc.init(id=run_id, resume=True)` continues a run where it stopped.
- **Speed:** `log()` never touches the network, so it doesn't slow down training.
- **Reading runs back:** `lsc.Api().runs("my-project")` lists runs with their config and
  summary, and `run.history(["loss"])` gives every logged point as a pandas DataFrame. See
  [Reading runs back](sdk/python/README.md#reading-runs-back).

More options and details are in the [SDK's documentation](sdk/python/README.md).

## Connecting the SDK to a server

The SDK needs two things: where the server is, and an API key. Each can be an environment variable
or an argument to `lsc.init`:

| Environment variable | `lsc.init` argument | Default |
|---|---|---|
| `LOSSCAPE_SERVER` | `server` | `http://localhost:8080` |
| `LOSSCAPE_API_KEY` | `api_key` | none: required, unless offline |
| `LOSSCAPE_PROJECT` | `project` | `uncategorized` |
| `LOSSCAPE_MODE` | `mode` | `online`, or `offline`, `disabled` |

Prefer the environment variables for the key, so it stays out of your code. You also need to be the
project's owner or an editor. The owner adds people under the project's **Settings**.

### Training on another machine

By default the server only listens on its own machine, because anyone who reaches a new server first
can create its administrator account. You have two ways to reach it from elsewhere.

**With an SSH tunnel**, for your own use. Nothing is exposed, and the default address keeps working.
Run this on the training machine, and leave it running:

```sh
ssh -N -L 8080:localhost:8080 you@your-server
```

**With HTTPS**, for a team. Point a domain at the server, open ports 80 and 443, and add this to `.env`
next to `compose.yaml`:

```sh
LOSSCAPE_DOMAIN=losscape.example.com
LOSSCAPE_PUBLIC_URL=https://losscape.example.com
```

```sh
docker compose --profile https up -d --wait --no-build
```

Then, on the training machine:

```sh
export LOSSCAPE_SERVER=https://losscape.example.com
```

Read [Before opening it to others](docs/DEPLOYMENT.md#before-opening-it-to-others) first, and
create your account right after starting it. Plain HTTP
across a network would send passwords and API keys in the clear.

### When the server can't be reached

Training doesn't stop, and nothing is lost. Runs are written to a local log in `./.losscape` first
(worth adding to your `.gitignore`) and uploaded from there, with retries. If a run ended before its
data got through, upload it later, from the same folder:

```sh
losscape sync
```

For a machine with no connection at all, save runs locally and sync them from somewhere that has one:

```sh
LOSSCAPE_MODE=offline python train.py
losscape sync
```

### Troubleshooting

In the cases where the server refuses or can't be reached, the data stays in the local log, so run
`losscape sync` once it's fixed.

| What you see | What it means | What to do |
|---|---|---|
| `ValueError: no API key` | The key isn't set here | Set `LOSSCAPE_API_KEY`, in the shell that runs your script |
| `server rejected runs/init: unauthorized` | The key is wrong or was revoked | Create a new key under **Settings** |
| `unknown_project: project "..." does not exist` | The project doesn't exist, or you aren't a member of it | Check the spelling, create it in the web interface, or ask its owner to add you |
| `not uploaded yet (...) Connection refused` | Nothing is listening at that address | Check `docker compose ps`, `LOSSCAPE_SERVER`, and your SSH tunnel |
| `ModuleNotFoundError: No module named 'lsc'` | The SDK isn't installed in this environment | Run `pip install losscape-sdk`. Plain `losscape` is a different project |

For problems with the server itself, see
[Troubleshooting](docs/DEPLOYMENT.md#troubleshooting) in the deployment guide.

## Running the server

```sh
docker compose up -d --wait --no-build   # start
docker compose logs -f losscape          # follow its logs
docker compose down                      # stop, and keep the data
```

To update, run `git pull` and `docker compose pull losscape`, then start it again. For real use, pin a
version such as `madproton/losscape:0.1.4` in `.env` rather than `latest`, so that you choose when it
changes. If you build from source, use `--build` in place of `--no-build`, with `LOSSCAPE_IMAGE` unset.

This runs the server and web interface, Postgres (accounts, projects, runs) and ClickHouse (metrics and
console output). A small team fits on 4 vCPU and 8 GB of memory.

[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) covers everything else: HTTPS, external sign-in, all settings,
your own name and look, published images, upgrades, and backups.

## Development

Requirements: Rust (stable), [uv](https://docs.astral.sh/uv/), Node 24 (`web/.nvmrc`), Docker. No `protoc` needed.

```sh
cp .env.example .env
make dev-db       # Postgres + ClickHouse in Docker
make dev-server   # API on http://localhost:8080
make dev-web      # UI on http://localhost:5173 (or build it: the server serves web/build)
```

Open the UI and create the first account, with the setup token `make dev-server` printed: it
becomes administrator and owns any existing projects. Create an API key under Settings and export it for the SDK, benchmarks and the crash
test (`LOSSCAPE_API_KEY=df_...`).

```sh
make test lint    # what CI runs; server integration tests need make dev-db
make deploy-test  # a fresh deployment in Docker, a restart, check nothing is lost
make bench        # ingest throughput against a running server
make crash-test   # SIGKILL the server mid-load, check nothing is lost
```

The code is laid out like this:

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

## Accounts and security

- Passwords: Argon2id (m = 19 MiB, t = 2, p = 1, random 16-byte salt), failed sign-ins
  rate-limited, same response for unknown users and wrong passwords. No rules on strength:
  forms show an estimate and advice, in the browser.
- Browser sessions: random 256-bit token in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` over
  HTTPS); stored as SHA-256. Cross-site state-changing requests are refused.
- API keys (`df_...`): per user, shown once, stored as SHA-256, revocable, and revoked when the
  password changes. They log runs and read data; managing keys, members and projects takes a
  signed-in session, so that a leaked key can't keep itself alive or delete anything.
- Projects: created in the UI (New project), never by the SDK; the creator is owner. One owner
  per project (manages members and settings; ownership moves only by transfer), editors (log
  runs), viewers (read). Non-members get 404.
- External sign-in: OpenID Connect (PKCE, verified ID token) and GitHub; see
  [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).
- Behind a reverse proxy, client addresses come from `X-Forwarded-For` only if it's listed in
  `LOSSCAPE_TRUSTED_PROXIES`.
- SQL is always parameterized; the UI escapes all output, behind a strict
  Content-Security-Policy (scripts from the server only, inline bootstrap allowed by hash).

## License

[Apache-2.0](LICENSE).
