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

## How it works

- `log()` appends to an in-memory buffer (~1 µs for 20 metrics) and never touches the network.
- A Rust background thread writes the buffer every 100 ms as one batch to a local log
  (`.losscape/runs/<run id>/`) and uploads from there, retrying until the server confirms.
- Killed process, server down, offline mode: the data stays in the local log.
  `losscape sync` uploads it. At most ~100 ms of logging is lost on `kill -9`.

Also recorded with every run: console output (what the process and its subprocesses
print, progress bars kept as one line), system metrics, and the environment (git commit
and uncommitted diff, packages, OS, CPU, GPUs and driver, Slurm job, container). Values
that look like secrets are hidden: config strings under names like `api_key` or
`hf_token`, and strings shaped like known tokens.

## Settings

| Argument | Environment | Default |
|---|---|---|
| `project` | `LOSSCAPE_PROJECT` | `uncategorized` |
| `mode` | `LOSSCAPE_MODE` | `online` (`offline`, `disabled`) |
| `dir` | `LOSSCAPE_DIR` | `./.losscape` |
| `server` | `LOSSCAPE_SERVER` | `http://localhost:8080` |
| `api_key` | `LOSSCAPE_API_KEY` | required online: create one in the web UI, Settings > API keys |
| `console` | `LOSSCAPE_CONSOLE` | on: stdout and stderr are recorded (off on Windows and in Jupyter) |
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
