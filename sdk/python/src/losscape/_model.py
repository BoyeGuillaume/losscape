"""What a model is made of, for the Model tab of its run.

Works on anything shaped like a ``torch.nn.Module``, without importing PyTorch until a
forward pass needs it. Only names, shapes, counts and short descriptions leave this file:
never weights or inputs.
"""

from __future__ import annotations

import dataclasses
from collections.abc import Mapping
from typing import Any

MAX_MODULES = 5000
MAX_TEXT = 200
MAX_ITEMS = 8
MAX_DEPTH = 4
MAX_TENSORS_PER_MODULE = 32
_WRAPPERS = {"DistributedDataParallel", "DataParallel"}


def is_module(model: Any) -> bool:
    return all(
        callable(getattr(model, attr, None))
        for attr in ("named_children", "named_parameters", "named_buffers", "modules")
    )


def _unwrap(model: Any) -> tuple[Any, list[str]]:
    """The model inside DDP, DataParallel and ``torch.compile``, and what wrapped it."""
    wrappers: list[str] = []
    while True:
        cls = type(model).__name__
        if cls in _WRAPPERS and is_module(getattr(model, "module", None)):
            wrappers.append(cls)
            model = model.module
        elif is_module(getattr(model, "_orig_mod", None)):
            wrappers.append(cls)
            model = model._orig_mod
        else:
            return model, wrappers


def _text(value: str) -> str:
    return value if len(value) <= MAX_TEXT else value[: MAX_TEXT - 1] + "…"


def _is_lazy(tensor: Any) -> bool:
    """Not created yet: ``nn.LazyLinear`` before the first forward pass."""
    return type(tensor).__name__.startswith("Uninitialized")


def _size(tensor: Any) -> int:
    return 0 if _is_lazy(tensor) else int(tensor.numel())


def _dims(tensor: Any) -> list[int] | None:
    return None if _is_lazy(tensor) else [int(n) for n in tensor.shape]


def _bytes(tensor: Any) -> int:
    return 0 if _is_lazy(tensor) else _size(tensor) * int(tensor.element_size())


def _describe_value(value: Any, depth: int = 0) -> str:
    """A tensor as its shape, and containers of them, in a line."""
    shape = getattr(value, "shape", None)
    if shape is not None and not callable(shape):
        try:
            return str([int(n) for n in shape])
        except (TypeError, ValueError):
            return type(value).__name__
    if value is None:
        return "None"
    if depth >= MAX_DEPTH:
        return "…"

    def items(pairs: list[str], total: int) -> str:
        return ", ".join(pairs + (["…"] if total > len(pairs) else []))

    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        value = {f.name: getattr(value, f.name) for f in dataclasses.fields(value)}
        value = {k: v for k, v in value.items() if v is not None}
    if isinstance(value, Mapping):
        pairs = [
            f"{k}: {_describe_value(v, depth + 1)}" for k, v in list(value.items())[:MAX_ITEMS]
        ]
        return "{" + items(pairs, len(value)) + "}"
    if isinstance(value, list | tuple):
        return (
            "("
            + items([_describe_value(v, depth + 1) for v in value[:MAX_ITEMS]], len(value))
            + ")"
        )
    return type(value).__name__


def _describe_call(args: tuple[Any, ...], kwargs: Mapping[str, Any]) -> str:
    parts = [_describe_value(a) for a in args]
    parts += [f"{k}={_describe_value(v)}" for k, v in kwargs.items()]
    return _text(", ".join(parts))


class _Calls:
    """Input and output shapes of each module during a forward pass."""

    def __init__(self) -> None:
        self.inputs: dict[int, str] = {}
        self.outputs: dict[int, str] = {}
        self.counts: dict[int, int] = {}

    def attach(self, model: Any) -> list[Any]:
        handles = []

        def before(module: Any, args: tuple[Any, ...], kwargs: dict[str, Any]) -> None:
            key = id(module)
            self.counts[key] = self.counts.get(key, 0) + 1
            self.inputs.setdefault(key, _describe_call(args, kwargs))

        def after(module: Any, _args: Any, output: Any) -> None:
            self.outputs.setdefault(id(module), _text(_describe_value(output)))

        for module in model.modules():
            handles.append(module.register_forward_pre_hook(before, with_kwargs=True))
            handles.append(module.register_forward_hook(after))
        return handles


def _forward(
    model: Any, args: tuple[Any, ...], kwargs: Mapping[str, Any]
) -> tuple[_Calls, int | None, str | None]:
    """Runs the model once, leaving it as it was. Returns shapes, FLOPs and any error."""
    calls = _Calls()
    try:
        import torch
    except ImportError:
        return calls, None, "running a model takes PyTorch"

    modules = list(model.modules())
    modes = [getattr(m, "training", False) for m in modules]
    devices = sorted(
        {p.device.index for p in model.parameters() if p.device.type == "cuda" and not _is_lazy(p)}
    )
    handles: list[Any] = []
    flops = None
    error = None
    try:
        model.eval()
        with torch.no_grad(), torch.random.fork_rng(devices=devices):
            handles = calls.attach(model)
            model(*args, **kwargs)
            for handle in handles:
                handle.remove()
            handles = []
            flops = _count_flops(model, args, kwargs)
    except Exception as err:  # noqa: BLE001 - whatever the model raises is reported
        error = _text(f"{type(err).__name__}: {err}")
    finally:
        for handle in handles:
            handle.remove()
        for module, mode in zip(modules, modes, strict=True):
            module.training = mode
    return calls, flops, error


def _count_flops(model: Any, args: tuple[Any, ...], kwargs: Mapping[str, Any]) -> int | None:
    """An estimate from PyTorch's own counter, when it can follow the model."""
    try:
        from torch.utils.flop_counter import FlopCounterMode

        with FlopCounterMode(display=False) as counter:
            model(*args, **kwargs)
        return int(counter.get_total_flops())
    except Exception:  # noqa: BLE001 - shapes are still worth having without this
        return None


def _tree(model: Any, calls: _Calls) -> tuple[dict[str, Any], dict[int, tuple[int, bool]]]:
    """The module tree, with parameter counts that count shared weights once."""
    module_paths: dict[int, str] = {}
    param_paths: dict[int, str] = {}

    def visit(
        name: str, module: Any, path: str
    ) -> tuple[dict[str, Any], dict[int, tuple[int, bool]], str]:
        key = id(module)
        cls = type(module).__name__
        if key in module_paths:
            node = {"name": name, "class": cls, "params": 0, "trainable": 0}
            node["shared_with"] = module_paths[key]
            return node, {}, f"{cls}:shared"
        module_paths[key] = path

        node = {"name": name, "class": cls}
        extra_repr = getattr(module, "extra_repr", None)
        extra = extra_repr() if callable(extra_repr) else ""
        if extra:
            node["extra"] = _text(str(extra).replace("\n", " "))
        if key in calls.inputs:
            node["input"] = calls.inputs[key]
        if key in calls.outputs:
            node["output"] = calls.outputs[key]
        if calls.counts.get(key, 0) > 1:
            node["calls"] = calls.counts[key]

        counted: dict[int, tuple[int, bool]] = {}
        tensors = []
        signature = [cls, extra, node.get("input", ""), node.get("output", "")]
        for tensor_name, param in module.named_parameters(recurse=False):
            shared = param_paths.setdefault(id(param), f"{path}.{tensor_name}".lstrip("."))
            trainable = bool(param.requires_grad)
            entry: dict[str, Any] = {
                "name": tensor_name,
                "shape": _dims(param),
                "trainable": trainable,
            }
            if shared != f"{path}.{tensor_name}".lstrip("."):
                entry["shared_with"] = shared
            tensors.append(entry)
            counted[id(param)] = (_size(param), trainable)
            signature.append(f"{tensor_name}:{entry['shape']}:{trainable}")
        for tensor_name, buffer in module.named_buffers(recurse=False):
            tensors.append({"name": tensor_name, "shape": _dims(buffer), "buffer": True})
            signature.append(f"{tensor_name}:{_dims(buffer)}")
        if tensors:
            node["tensors"] = tensors[:MAX_TENSORS_PER_MODULE]

        children = []
        for child_name, child in module.named_children():
            child_node, child_counted, child_signature = visit(
                child_name, child, f"{path}.{child_name}".lstrip(".")
            )
            counted.update(child_counted)
            shown_name = "" if child_name.isdigit() else child_name
            signature.append(f"({shown_name}={child_signature})")
            # Identical numbered neighbours, such as the blocks of a transformer, show as one.
            if children and child_name.isdigit() and children[-1][1] == child_signature:
                first = children[-1][0]
                first["repeat"] = first.get("repeat", 1) + 1
                first["last"] = child_name
                first.setdefault("names", [first["name"]]).append(child_name)
            else:
                children.append((child_node, child_signature))
        if children:
            node["children"] = [c[0] for c in children]

        node["params"] = sum(n for n, _ in counted.values())
        node["trainable"] = sum(n for n, t in counted.values() if t)
        return node, counted, "|".join(signature)

    root, counted, _ = visit("", model, "")
    return root, counted


def _limit(root: dict[str, Any]) -> bool:
    """Keeps the modules nearest the top, within ``MAX_MODULES``. True if some had to go."""
    budget = MAX_MODULES
    queue = [root]
    cut = False
    for node in queue:
        children = node.get("children", [])
        if len(children) > budget:
            node["hidden_children"] = len(children)
            del node["children"]
            cut = True
            continue
        budget -= len(children)
        queue.extend(children)
    return cut


def describe(
    model: Any,
    *,
    args: Any = None,
    kwargs: Mapping[str, Any] | None = None,
    path: str | None = None,
) -> dict[str, Any]:
    inner, wrappers = _unwrap(model)
    call_args = () if args is None else args if isinstance(args, tuple) else (args,)
    call_kwargs = dict(kwargs or {})
    ran = args is not None or bool(call_kwargs)

    calls = _Calls()
    flops = error = None
    if ran:
        calls, flops, error = _forward(inner, call_args, call_kwargs)

    # After the forward pass: that's when lazy modules get their parameters.
    tree, counted = _tree(inner, calls)
    params = list(inner.named_parameters())
    buffers = list(inner.named_buffers())
    out: dict[str, Any] = {
        "class": type(inner).__name__,
        "params": sum(n for n, _ in counted.values()),
        "trainable_params": sum(n for n, t in counted.values() if t),
        "buffers": sum(_size(b) for _, b in buffers),
        "size_bytes": sum(_bytes(p) for _, p in params + buffers),
        "dtypes": sorted({str(p.dtype) for _, p in params if not _is_lazy(p)}),
        "devices": sorted({str(p.device) for _, p in params if not _is_lazy(p)}),
        "tree": tree,
    }
    lazy = sum(1 for _, p in params if _is_lazy(p))
    if lazy:
        out["uninitialized"] = lazy
    if wrappers:
        out["wrappers"] = wrappers
    if path is not None:
        out["path"] = _text(str(path))
    if ran:
        out["input"] = _describe_call(call_args, call_kwargs)
        if id(inner) in calls.outputs:
            out["output"] = calls.outputs[id(inner)]
    if flops is not None:
        out["flops"] = flops
    if error:
        out["forward_error"] = error
    if _limit(tree):
        out["truncated"] = True
    return out


def metrics(description: Mapping[str, Any], name: str) -> dict[str, float]:
    prefix = "model/" if name == "model" else f"model/{name}/"
    keys = ["params", "trainable_params", "size_bytes", "flops"]
    return {f"{prefix}{key}": description[key] for key in keys if key in description}
