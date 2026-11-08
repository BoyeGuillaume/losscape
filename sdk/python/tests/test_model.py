"""Describing models: the logic with stand-ins, then with PyTorch when it's installed."""

from __future__ import annotations

import pytest

from losscape import _model


class FakeTensor:
    def __init__(self, *shape: int, requires_grad: bool = True) -> None:
        self.shape = shape
        self.requires_grad = requires_grad
        self.dtype = "float32"
        self.device = "cpu"

    def numel(self) -> int:
        total = 1
        for n in self.shape:
            total *= n
        return total

    def element_size(self) -> int:
        return 4


class FakeModule:
    def __init__(self, params=None, buffers=None, children=None, extra: str = "") -> None:
        self.params = params or {}
        self.buffers_ = buffers or {}
        self.children_ = children or {}
        self.extra = extra

    def extra_repr(self) -> str:
        return self.extra

    def named_children(self):
        return iter(self.children_.items())

    def named_parameters(self, recurse: bool = True):
        seen = set()
        for name, p in self.params.items():
            if id(p) not in seen:
                seen.add(id(p))
                yield name, p
        if recurse:
            for child_name, child in self.children_.items():
                for name, p in child.named_parameters():
                    if id(p) not in seen:
                        seen.add(id(p))
                        yield f"{child_name}.{name}", p

    def named_buffers(self, recurse: bool = True):
        yield from self.buffers_.items()
        if recurse:
            for child_name, child in self.children_.items():
                for name, b in child.named_buffers():
                    yield f"{child_name}.{name}", b

    def modules(self):
        yield self
        for child in self.children_.values():
            yield from child.modules()


class Linear(FakeModule):
    def __init__(self, n: int, requires_grad: bool = True) -> None:
        super().__init__(
            {"weight": FakeTensor(n, 2, requires_grad=requires_grad)}, extra=f"features={n}"
        )


def child(node, name):
    return next(c for c in node["children"] if c["name"] == name)


def test_recognizes_modules_only():
    assert _model.is_module(FakeModule())
    for other in (3, "a model", object(), {"weight": 1}):
        assert not _model.is_module(other)


def test_counts_and_sizes():
    model = FakeModule(
        children={"a": Linear(3), "b": Linear(5, requires_grad=False)},
        buffers={"mean": FakeTensor(2)},
    )
    d = _model.describe(model)
    assert d["class"] == "FakeModule"
    assert d["params"] == 16
    assert d["trainable_params"] == 6
    assert d["buffers"] == 2
    assert d["size_bytes"] == (16 + 2) * 4
    assert d["dtypes"] == ["float32"]
    assert "input" not in d
    b = child(d["tree"], "b")
    assert (b["params"], b["trainable"], b["extra"]) == (10, 0, "features=5")
    assert b["tensors"] == [{"name": "weight", "shape": [5, 2], "trainable": False}]


def test_shared_weights_count_once():
    shared = FakeTensor(10)
    model = FakeModule(
        children={
            "embed": FakeModule({"weight": shared}),
            "head": FakeModule({"weight": shared, "bias": FakeTensor(1)}),
        }
    )
    d = _model.describe(model)
    assert d["params"] == 11
    head = child(d["tree"], "head")
    assert head["tensors"][0]["shared_with"] == "embed.weight"
    assert "shared_with" not in head["tensors"][1]


def test_shared_modules_show_once():
    block = Linear(3)
    d = _model.describe(FakeModule(children={"a": block, "b": block}))
    assert d["params"] == 6
    assert child(d["tree"], "b")["shared_with"] == "a"


def test_numbered_identical_neighbours_collapse():
    blocks = FakeModule(
        children={
            "0": Linear(3),
            "1": Linear(3),
            "2": Linear(3),
            "3": Linear(7),
            "4": Linear(3),
        }
    )
    d = _model.describe(FakeModule(children={"blocks": blocks, "fc1": Linear(2), "fc2": Linear(2)}))
    assert d["params"] == 6 * 4 + 14 + 4 + 4
    rows = child(d["tree"], "blocks")["children"]
    assert [(r["name"], r.get("repeat"), r.get("last")) for r in rows] == [
        ("0", 3, "2"),
        ("3", None, None),
        ("4", None, None),
    ]
    assert rows[0]["names"] == ["0", "1", "2"]
    assert "names" not in rows[1]
    # Named modules keep their names, even when identical.
    assert {c["name"] for c in d["tree"]["children"]} == {"blocks", "fc1", "fc2"}


def test_a_huge_model_is_cut_with_correct_totals(monkeypatch):
    monkeypatch.setattr(_model, "MAX_MODULES", 3)
    model = FakeModule(children={f"m{i}": Linear(1) for i in range(5)})
    d = _model.describe(model)
    assert d["truncated"] is True
    assert d["tree"]["hidden_children"] == 5
    assert d["params"] == 10


def test_unwraps_wrappers():
    inner = Linear(3)

    class DistributedDataParallel(FakeModule):
        module = inner

    class OptimizedModule(FakeModule):
        _orig_mod = DistributedDataParallel()

    d = _model.describe(OptimizedModule())
    assert d["class"] == "Linear"
    assert d["wrappers"] == ["OptimizedModule", "DistributedDataParallel"]


def test_lazy_parameters_are_reported_not_counted():
    class UninitializedParameter(FakeTensor):
        requires_grad = True

        def numel(self):
            raise RuntimeError("not initialized")

        @property
        def shape(self):
            raise RuntimeError("not initialized")

        @shape.setter
        def shape(self, _value):
            pass

    d = _model.describe(FakeModule({"weight": UninitializedParameter()}))
    assert (d["params"], d["uninitialized"]) == (0, 1)
    assert d["tree"]["tensors"][0]["shape"] is None


def test_metrics_by_name():
    d = {"params": 5, "trainable_params": 4, "size_bytes": 20, "tree": {}}
    assert _model.metrics(d, "model") == {
        "model/params": 5,
        "model/trainable_params": 4,
        "model/size_bytes": 20,
    }
    assert _model.metrics({**d, "flops": 9}, "encoder")["model/encoder/flops"] == 9


def test_values_read_as_shapes():
    assert _model._describe_value(FakeTensor(8, 3)) == "[8, 3]"
    assert _model._describe_value((FakeTensor(1), None)) == "([1], None)"
    assert _model._describe_value({"logits": FakeTensor(2)}) == "{logits: [2]}"
    assert _model._describe_value(3) == "int"
    long = _model._describe_value(list(range(100)))
    assert long.endswith("…)")
    assert _model._describe_call((FakeTensor(4),), {"mask": FakeTensor(2)}) == "[4], mask=[2]"


def test_a_long_description_is_cut():
    d = _model.describe(FakeModule(children={"a": Linear(1)}, extra="x" * 500), path="p" * 500)
    assert len(d["tree"]["extra"]) == _model.MAX_TEXT
    assert len(d["path"]) == _model.MAX_TEXT


# ---- With PyTorch ----


@pytest.fixture
def torch():
    return pytest.importorskip("torch")


def make_net(torch):
    class Block(torch.nn.Module):
        def __init__(self) -> None:
            super().__init__()
            self.fc = torch.nn.Linear(8, 8)
            self.norm = torch.nn.BatchNorm1d(8)

        def forward(self, x, scale=1.0):
            return self.norm(self.fc(x)) * scale

    class Net(torch.nn.Module):
        def __init__(self) -> None:
            super().__init__()
            self.embed = torch.nn.Linear(4, 8)
            self.blocks = torch.nn.ModuleList(Block() for _ in range(3))
            self.drop = torch.nn.Dropout(0.5)
            self.head = torch.nn.Linear(8, 2)

        def forward(self, x, scale=1.0, return_dict=False):
            x = self.embed(x)
            for block in self.blocks:
                x = block(x, scale=scale)
            out = self.head(self.drop(x))
            return {"logits": out, "features": x} if return_dict else out

    return Net()


def test_real_counts(torch):
    net = make_net(torch)
    for p in net.embed.parameters():
        p.requires_grad = False
    d = _model.describe(net)
    total = sum(p.numel() for p in net.parameters())
    assert d["params"] == total
    assert d["trainable_params"] == sum(p.numel() for p in net.parameters() if p.requires_grad)
    assert d["class"] == "Net"
    assert "input" not in d
    blocks = child(d["tree"], "blocks")["children"]
    assert [(b["name"], b["repeat"], b["last"]) for b in blocks] == [("0", 3, "2")]


def test_real_tied_weights_count_once(torch):
    net = torch.nn.ModuleDict({"a": torch.nn.Linear(4, 4), "b": torch.nn.Linear(4, 4)})
    net["b"].weight = net["a"].weight
    assert _model.describe(net)["params"] == 16 + 4 + 4


def test_args_and_kwargs(torch):
    net = make_net(torch)
    x = torch.zeros(2, 4)
    d = _model.describe(net, args=x)
    assert (d["input"], d["output"]) == ("[2, 4]", "[2, 2]")
    assert "forward_error" not in d
    embed = child(d["tree"], "embed")
    assert (embed["input"], embed["output"]) == ("[2, 4]", "[2, 8]")
    assert d["flops"] > 0

    both = _model.describe(net, args=(x,), kwargs={"scale": torch.ones(1), "return_dict": True})
    assert both["input"] == "[2, 4], scale=[1], return_dict=bool"
    assert both["output"] == "{logits: [2, 2], features: [2, 8]}"

    only_kwargs = _model.describe(net, kwargs={"x": x})
    assert only_kwargs["input"] == "x=[2, 4]"
    assert only_kwargs["output"] == "[2, 2]"


def test_a_forward_pass_leaves_the_model_as_it_was(torch):
    net = make_net(torch)
    net.train()
    net.blocks[0].norm.eval()  # frozen statistics, a common setup
    stats = net.blocks[1].norm.running_mean.clone()
    tracked = int(net.blocks[1].norm.num_batches_tracked)
    weights = net.head.weight.clone()
    torch.manual_seed(7)
    expected = torch.rand(1)
    torch.manual_seed(7)

    _model.describe(net, args=torch.ones(4, 4))

    assert torch.rand(1).item() == expected.item(), "random state restored"
    assert net.training and net.blocks[1].norm.training
    assert not net.blocks[0].norm.training, "each module's own mode kept"
    assert torch.equal(net.blocks[1].norm.running_mean, stats)
    assert int(net.blocks[1].norm.num_batches_tracked) == tracked
    assert torch.equal(net.head.weight, weights)
    assert all(not m._forward_hooks and not m._forward_pre_hooks for m in net.modules())
    assert net.head.weight.grad is None


def test_a_failing_forward_keeps_the_rest(torch):
    net = make_net(torch)
    net.train()
    d = _model.describe(net, args=torch.zeros(2, 99))
    assert "RuntimeError" in d["forward_error"]
    assert d["params"] == sum(p.numel() for p in net.parameters())
    assert net.training
    assert all(not m._forward_hooks and not m._forward_pre_hooks for m in net.modules())


def test_lazy_modules_get_counted_after_a_forward_pass(torch):
    net = torch.nn.Sequential(torch.nn.LazyLinear(3))
    before = _model.describe(net)
    assert (before["params"], before["uninitialized"]) == (0, 2)
    after = _model.describe(net, args=torch.zeros(1, 5))
    assert after["params"] == 5 * 3 + 3
    assert "uninitialized" not in after


def test_compile_and_data_parallel_are_unwrapped(torch):
    net = make_net(torch)
    d = _model.describe(torch.nn.DataParallel(net))
    assert (d["class"], d["wrappers"]) == ("Net", ["DataParallel"])
    assert not any(c["name"] == "module" for c in d["tree"]["children"])


def test_module_called_twice(torch):
    class Twice(torch.nn.Module):
        def __init__(self) -> None:
            super().__init__()
            self.fc = torch.nn.Linear(2, 2)

        def forward(self, x):
            return self.fc(self.fc(x))

    d = _model.describe(Twice(), args=torch.zeros(1, 2))
    assert child(d["tree"], "fc")["calls"] == 2
