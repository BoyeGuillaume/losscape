#!/usr/bin/env python3
"""Generates web/src/lib/password-data.ts: what the password strength estimate knows.

From the frequency lists of zxcvbn (MIT: real passwords, English, names), run with:

    npm pack zxcvbn@4.4.2 && tar xzf zxcvbn-4.4.2.tgz
    python3 scripts/gen_password_data.py package/lib/frequency_lists.js
    (cd web && npx prettier --write src/lib/password-data.ts)

Letter statistics weigh each word by its rank, so they follow what people actually type.
The file is read as text, not run.
"""

import math
import re
import sys
from collections import defaultdict
from pathlib import Path

OUTPUT = Path(__file__).parent.parent / "web/src/lib/password-data.ts"
LETTERS = "abcdefghijklmnopqrstuvwxyz"

# Lists to learn letters from, and how much each counts.
TRAINING = {
    "passwords": 1.0,
    "english_wikipedia": 1.0,
    "us_tv_and_film": 1.0,
    "female_names": 0.5,
    "male_names": 0.5,
    "surnames": 0.25,
}
# (words kept, how far back they rank against the others)
DICTIONARY = {
    "passwords": (1500, 1),
    "english_wikipedia": (2500, 2),
    "us_tv_and_film": (1000, 2),
    "female_names": (300, 3),
    "male_names": (300, 3),
    "surnames": (300, 4),
}
# What people on this kind of server might reach for.
EXTRA_WORDS = ["losscape", "pytorch", "tensorflow", "changeme", "machinelearning"]
EXTRA_RANK = 50
MIN_WORD, MAX_WORD = 3, 16

# How many occurrences a context needs before it speaks for itself. Without this a pair of
# letters seen once would look perfectly predictable.
BACKOFF = 20
# Costs are kept in half bits, as one digit each; 9 reads as "no better than random".
MAX_DIGIT = 9

NOTICE = """/*! @license
 * Derived from the frequency lists of zxcvbn.
 *
 * Copyright (c) 2012-2016 Dan Wheeler and Dropbox, Inc.
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of this
 * software and associated documentation files (the "Software"), to deal in the Software
 * without restriction, including without limitation the rights to use, copy, modify, merge,
 * publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
 * to whom the Software is furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all copies or
 * substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
 * INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
 * PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
 * FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
 * OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
 * DEALINGS IN THE SOFTWARE.
 */
"""


def read_lists(path: str) -> dict[str, list[str]]:
    text = Path(path).read_text(encoding="utf-8")
    found = re.findall(r'(\w+):\s*"([^"]*)"\.split\(","\)', text)
    return {name: words.split(",") for name, words in found}


class Context:
    """What follows some letters: weighted counts, and how many times it was seen."""

    def __init__(self) -> None:
        self.weights: dict[str, float] = defaultdict(float)
        self.seen = 0

    def add(self, letter: str, weight: float) -> None:
        self.weights[letter] += weight
        self.seen += 1

    def probabilities(self, shorter: dict[str, float]) -> dict[str, float]:
        """Leaning on the shorter context until there's enough to go on."""
        total = sum(self.weights.values())
        unit = total / self.seen if self.seen else 1.0
        return {
            c: (self.weights[c] / unit + BACKOFF * shorter[c]) / (self.seen + BACKOFF)
            for c in LETTERS
        }


def digits(probabilities: dict[str, float]) -> str:
    """Half bits of surprise per letter, as one digit each."""
    return "".join(
        str(min(MAX_DIGIT, round(-math.log2(max(probabilities[c], 1e-12)) * 2))) for c in LETTERS
    )


def letter_tables(lists: dict[str, list[str]]) -> tuple[str, str, str]:
    one = Context()
    two: dict[str, Context] = defaultdict(Context)
    three: dict[str, Context] = defaultdict(Context)
    for name, list_weight in TRAINING.items():
        for rank, word in enumerate(lists[name], 1):
            weight = list_weight / (rank + 10)
            for run in re.findall("[a-z]+", word.lower()):
                for i, c in enumerate(run):
                    one.add(c, weight)
                    if i >= 1:
                        two[run[i - 1]].add(c, weight)
                    if i >= 2:
                        three[run[i - 2 : i]].add(c, weight)
    uniform = {c: 1 / 26 for c in LETTERS}
    p1 = one.probabilities(uniform)
    p2 = {a: two[a].probabilities(p1) for a in LETTERS}
    p3 = {a + b: three[a + b].probabilities(p2[b]) for a in LETTERS for b in LETTERS}
    return (
        digits(p1),
        "".join(digits(p2[a]) for a in LETTERS),
        "".join(digits(p3[a + b]) for a in LETTERS for b in LETTERS),
    )


def dictionary(lists: dict[str, list[str]]) -> list[str]:
    best: dict[str, int] = {word: EXTRA_RANK for word in EXTRA_WORDS}
    for name, (size, factor) in DICTIONARY.items():
        kept = (w for w in lists[name] if re.fullmatch("[a-z0-9]+", w))
        kept = [w for w in kept if MIN_WORD <= len(w) <= MAX_WORD][:size]
        for rank, word in enumerate(kept, 1):
            best[word] = min(best.get(word, rank * factor), rank * factor)
    return sorted(best, key=lambda w: (best[w], w))


def main() -> None:
    lists = read_lists(sys.argv[1])
    uni, bi, tri = letter_tables(lists)
    words = dictionary(lists)
    OUTPUT.write_text(
        NOTICE
        + "\n// Generated by scripts/gen_password_data.py: don't edit by hand.\n\n"
        + "/** Half bits of surprise of each letter, a to z; 9: no better than random. */\n"
        + f"export const UNIGRAMS = '{uni}';\n"
        + "/** The same after each letter: 26 x 26. */\n"
        + f"export const BIGRAMS = '{bi}';\n"
        + "/** The same after each two letters: 26 x 26 x 26. */\n"
        + f"export const TRIGRAMS = '{tri}';\n"
        + "/** Common passwords, words and names, the most likely first. */\n"
        + f"export const WORDS = '{','.join(words)}';\n",
        encoding="utf-8",
    )
    print(f"{len(words)} words, {OUTPUT.stat().st_size / 1024:.0f} KB")
    print("The build strips comments: zxcvbn's notice ships from web/third-party-notices-data.txt")


if __name__ == "__main__":
    main()
