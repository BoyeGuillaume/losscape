# losscape Python SDK

```python
import losscape

with losscape.init(project="mnist", config={"lr": 1e-3}) as run:
    for step in range(1000):
        run.log({"train/loss": loss}, step=step)
```

The logging API lands in step 2 of the roadmap; this package currently only ships the native core.
