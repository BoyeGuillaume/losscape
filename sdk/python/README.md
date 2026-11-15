# losscape Python SDK

Install it with `pip install losscape-sdk`, and import it as `lsc`. The package named plain
`losscape` is an unrelated project.

```python
import lsc

with lsc.init(project="mnist", config={"lr": 1e-3, "batch_size": 64}) as run:
    for step in range(10_000):
        loss = train_step()
        run.log({"train/loss": loss, "lr": scheduler.get_last_lr()[0]})
        if step % 1000 == 0:
            run.log({"val": {"loss": val_loss, "acc": val_acc}}, step=step)  # -> val/loss, val/acc
    run.summary["best_acc"] = best_acc
```

The API follows Weights & Biases: `init`, `log` (with `step` / `commit`), `config`,
`summary`, `finish`, context manager, `lsc.log(...)` on the current run.

The project must exist: create it in the web UI (New project) and be its owner or an editor.
A run logged to an unknown project is refused by the server; its data stays in the local log,
so `losscape sync` uploads it once the project is created.

## Checkpoints

Save a checkpoint however you like, then log the folder (or a single file): it uploads in the
background, beside the metrics, which never wait for it.

```python
torch.save(model.state_dict(), "ckpt/model.pt")
torch.save(optimizer.state_dict(), "ckpt/optim.pt")
run.log_checkpoint("ckpt/")  # at the step last logged; or step=...

# Resuming, after a preempted job:
run = lsc.init(project="mnist", id=run_id, resume=True)
if (ckpt := run.latest_checkpoint()) is not None:  # a local folder, or None
    model.load_state_dict(torch.load(ckpt / "model.pt"))
```

- `log_checkpoint` copies the folder into the run's local log before returning, so the next
  save can overwrite it at once (instant on btrfs and XFS). `move=True` hands it over instead.
- Files upload in parts of 16 MiB, compressed with zstd when that saves at least 5% (configs,
  mostly-zero tensors), each part checked on arrival. An interrupted upload resumes where it
  stopped, with the run or with `losscape sync`; the local copy is deleted once uploaded.
- `finish()` waits for checkpoints as long as they make progress, printing it.
- One checkpoint per step: the server refuses another at the same step (it stays in the
  local log, with why). Delete checkpoints with `checkpoint.delete()`.
- `latest_checkpoint()` and `checkpoint(step)` use a copy still on this machine, or
  download it, into `checkpoint-cache/<run id>/<step>` under `LOSSCAPE_DIR`, kept until you
  delete it.
- The server must have checkpoint storage (`LOSSCAPE_STORAGE`, see docs/DEPLOYMENT.md).

## Reading runs back

`lsc.Api()` reads what the server stored, for analysis in scripts and notebooks. It connects
like `lsc.init()` does: `LOSSCAPE_SERVER` and `LOSSCAPE_API_KEY`, or `lsc.Api(server, api_key)`.

```python
import lsc

api = lsc.Api()
runs = api.runs("mnist")  # newest first; api.runs(..., archived=True)
best = max(runs, key=lambda r: r.summary.get("val/acc", 0))
print(best.name, best.config["lr"], best.state)

table = runs.to_pandas()  # one row per run: config.* and summary.* columns
df = best.history(["train/loss", "val/acc"])  # step column, one column per metric
for point in best.scan_history("train/loss"):  # streamed: metric, step, time, value
    ...
```

- `history()` and `to_pandas()` need pandas: `pip install 'losscape-sdk[pandas]'`. The rest
  needs nothing beyond Python.
- History has every logged point, not the downsampled ones the charts draw. Steps where a
  metric wasn't logged are NaN; `history(..., wide=False)` gives one row per point instead.
- Without `keys`, history has every metric of the run except system metrics
  (`system=True` adds them). `min_step` and `max_step` limit the steps.
- `api.run(run_id)` gets one run; ids are the ones `lsc.init()` gives (`run.id`).
- `run.checkpoints` lists its checkpoints by step (`run.checkpoint(step)` for one), with
  `step`, `size`, `files`; `checkpoint.download(to, files=...)` downloads all of its files or
  some (checked against their SHA-256), and `checkpoint.delete()` deletes it.
- Metrics as logged: renames and computed metrics of the web UI aren't applied.

## How it works

- `log()` appends to an in-memory buffer (~1 µs for 20 metrics) and never touches the network.
- A Rust background thread writes the buffer every 100 ms as one batch to a local log
  (`.losscape/runs/<run id>/`) and uploads from there, retrying until the server confirms.
- Killed process, server down, offline mode: the data stays in the local log.
  `losscape sync` uploads it. At most ~100 ms of logging is lost on `kill -9`.

Also recorded with every run: console output (what the process and its subprocesses
print, progress bars kept as one line), system metrics, and the environment (git commit
and uncommitted diff, command line, hostname, selected environment variables, untracked file
names, packages, OS, CPU, GPUs and driver, Slurm job, container).

## Secrets

Everyone with access to a project can read what its runs record, so before anything is written
to the local log or uploaded, what looks like a secret is replaced by `***`:

- strings under names like `api_key`, `hf_token` or `password` in the config, summary and
  environment variables, and everything inside an object with such a name;
- anywhere, including console output and the git diff: well-known kinds of credentials (GitHub,
  OpenAI, Hugging Face, AWS, Slack, GitLab, PyPI and losscape keys, JSON web tokens), passwords in
  URLs, `Bearer` tokens, and assignments like `password=...` or `"api_key": "..."`.

This only catches what can be recognized: **a password printed on its own looks like any other
word**, and a secret in a file you changed is uploaded unless it has one of these shapes. If that
matters for a run, turn off what you don't want recorded: `git_diff=False`, `console=False`. Metric
names aren't checked.

The local log (`.losscape/`) is readable only by you on Linux and macOS, for what's created from
this version on. If a folder above it was already there, it keeps its permissions.

## Settings

| Argument | Environment | Default |
|---|---|---|
| `project` | `LOSSCAPE_PROJECT` | `uncategorized` |
| `mode` | `LOSSCAPE_MODE` | `online` (`offline`, `disabled`) |
| `dir` | `LOSSCAPE_DIR` | `./.losscape` |
| `server` | `LOSSCAPE_SERVER` | `http://localhost:8080` |
| `api_key` | `LOSSCAPE_API_KEY` | required online: create one in the web UI, Settings > API keys |
| `console` | `LOSSCAPE_CONSOLE` | on: stdout and stderr are recorded (off on Windows and in Jupyter) |
| `git_diff` | `LOSSCAPE_GIT_DIFF` | on: uncommitted changes are recorded (whether there are any always is) |
| `system_metrics` | `LOSSCAPE_SYSTEM_METRICS` | on: CPU, memory, disk, network, NVIDIA GPUs |
| | `LOSSCAPE_SYSTEM_INTERVAL` | 5 s |
| `finish(timeout=)` | `LOSSCAPE_FINISH_TIMEOUT` | 60 s |
| | `LOSSCAPE_SILENT=1` | print nothing |

## Other things

- `lsc.utils.random_name()` -> `brave-falcon-42` (not affected by `random.seed`).
- `lsc.init(id=..., resume=True)` continues a run where it stopped.
- Forked children (e.g. DataLoader workers) can't log to the parent's run; their calls are
  ignored with a warning.

## Development

```sh
uv sync                                    # builds the Rust core in place
uv run pytest                              # unit tests
uv run python benchmarks/bench_log.py      # log() latency
```
