# losscape Python SDK

```python
import losscape

with losscape.init(project="mnist", config={"lr": 1e-3, "batch_size": 64}) as run:
    for step in range(10_000):
        loss = train_step()
        run.log({"train/loss": loss, "lr": scheduler.get_last_lr()[0]})
        if step % 1000 == 0:
            run.log({"val": {"loss": val_loss, "acc": val_acc}}, step=step)  # -> val/loss, val/acc
    run.summary["best_acc"] = best_acc
```

The API follows Weights & Biases: `init`, `log` (with `step` / `commit`), `config`,
`summary`, `finish`, context manager, `losscape.log(...)` on the current run.

## How it works

- `log()` appends to an in-memory buffer (~1 µs for 20 metrics) and never touches the network.
- A Rust background thread writes the buffer every 100 ms as one batch to a local log
  (`.losscape/runs/<run id>/`) and uploads from there, retrying until the server confirms.
- Killed process, server down, offline mode: the data stays in the local log.
  `losscape sync` uploads it. At most ~100 ms of logging is lost on `kill -9`.

## Settings

| Argument | Environment | Default |
|---|---|---|
| `project` | `LOSSCAPE_PROJECT` | `uncategorized` |
| `mode` | `LOSSCAPE_MODE` | `online` (`offline`, `disabled`) |
| `dir` | `LOSSCAPE_DIR` | `./.losscape` |
| `server` | `LOSSCAPE_SERVER` | `http://localhost:8080` |
| `api_key` | `LOSSCAPE_API_KEY` | required online |
| `finish(timeout=)` | `LOSSCAPE_FINISH_TIMEOUT` | 60 s |
| | `LOSSCAPE_SILENT=1` | print nothing |

## Other things

- `losscape.utils.random_name()` -> `brave-falcon-42` (not affected by `random.seed`).
- `losscape.init(id=..., resume=True)` continues a run where it stopped.
- Forked children (e.g. DataLoader workers) can't log to the parent's run; their calls are
  ignored with a warning.

## Development

```sh
uv sync                                    # builds the Rust core in place
uv run pytest                              # unit tests
uv run python benchmarks/bench_log.py      # log() latency
```
