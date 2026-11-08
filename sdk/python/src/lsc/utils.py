"""Small helpers that are useful on their own."""

from __future__ import annotations

import random

# fmt: off
_ADJECTIVES = (
    "able", "agile", "amber", "ancient", "arctic", "autumn", "bold", "brave", "breezy",
    "bright", "brisk", "calm", "candid", "cheerful", "clever", "cosmic", "crimson", "crisp",
    "curious", "daring", "dawn", "deep", "eager", "early", "electric", "elegant", "emerald",
    "epic", "fair", "fancy", "fearless", "fierce", "fluent", "frosty", "gentle", "giant",
    "gilded", "glad", "golden", "graceful", "grand", "happy", "hardy", "hidden", "honest",
    "humble", "icy", "jolly", "keen", "kind", "lively", "lucky", "lunar", "magic", "mellow",
    "merry", "mighty", "misty", "modest", "noble", "nimble", "northern", "patient", "peaceful",
    "polar", "proud", "quick", "quiet", "radiant", "rapid", "rare", "restless", "rising",
    "royal", "rustic", "scarlet", "serene", "sharp", "shiny", "silent", "silver", "sleek",
    "smooth", "snowy", "solar", "sparkling", "spring", "stellar", "steady", "stormy", "sturdy",
    "sunny", "swift", "tidy", "tranquil", "vast", "velvet", "vivid", "wandering", "warm",
    "wild", "wise", "witty", "young", "zesty",
)

_NOUNS = (
    "aurora", "badger", "beacon", "bison", "blossom", "breeze", "brook", "canyon", "cedar",
    "comet", "condor", "coral", "cosmos", "crane", "creek", "dawn", "delta", "dolphin", "dune",
    "eagle", "ember", "falcon", "fern", "fjord", "flame", "forest", "fox", "galaxy", "garden",
    "gazelle", "glacier", "grove", "harbor", "hawk", "heron", "horizon", "island", "jaguar",
    "lagoon", "lake", "lark", "leaf", "leopard", "lion", "lotus", "lynx", "maple", "meadow",
    "meteor", "moon", "moose", "mountain", "nebula", "oak", "ocean", "orbit", "orchid",
    "otter", "owl", "panda", "panther", "peak", "pebble", "pine", "planet", "pond", "prairie",
    "pulsar", "quasar", "rain", "raven", "reef", "river", "robin", "sail", "sequoia", "shadow",
    "sky", "snow", "sparrow", "spruce", "star", "stone", "storm", "summit", "sun", "swan",
    "thunder", "tide", "tiger", "trail", "tundra", "valley", "violet", "voyage", "wave",
    "willow", "wind", "wolf", "wren",
)
# fmt: on


def random_name(seed: int | str | None = None) -> str:
    """A readable name like ``brave-falcon-42``.

    Uses its own random generator, so ``random.seed(...)`` in a training script doesn't
    make every run get the same name. Pass ``seed`` for a reproducible name.
    """
    rng = random.Random(seed)
    return f"{rng.choice(_ADJECTIVES)}-{rng.choice(_NOUNS)}-{rng.randint(1, 99)}"
