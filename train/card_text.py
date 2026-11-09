"""Card text as token ids for the network.

Each card the model embeds (minions, tokens, spells, in the engine's order)
gets the printed text of its Battlegrounds card from
ref/hsjson-cards.json, tokenised: markup and `[x]`/`[DNT]` tags are dropped,
`+N/+M` is one `<buff>` token, every other number is `<num>`, and a printed
"(N left!)" counter is dropped (the observation carries the real count).
Keywords stay words. Golden and normal copies share the name and so the text.

The output is `{"vocab": [...], "tokens": [[...], ...], "missing": [...]}`:
row 0 is the empty card and every row is padded with 0 to MAX_TOKENS. The
vocabulary is sorted, token id = index + 1, 0 is padding. `tokens` and
`vocab` go into the checkpoint's model_opts, so resume and eval never read
ref/ again; `missing` is only for the run's report.

    python3 -I train/card_text.py
"""

from __future__ import annotations

import json
import math
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REF = ROOT / "ref" / "hsjson-cards.json"
CARDS_TOML = ROOT / "cards.toml"

# The longest printed text in the 2026-11-09 export is 28 tokens; leave room
# and keep the table a fixed shape.
MAX_TOKENS = 32

_MARKUP = re.compile(r"<[^>]*>|\[[^\]]*\]")
_COUNTER = re.compile(r"\(\s*\d+[^)]*?\bleft!\s*\)", re.IGNORECASE)
_BUFF = re.compile(r"\+\s*\d+\s*/\s*\+?\s*\d+")
_NUMBER = re.compile(r"\d+")
_WORD = re.compile(r"[\x01\x02]|[a-z]+(?:['\u2019][a-z]+)?")
_BUFF_CHAR, _NUM_CHAR = "\x01", "\x02"

_GIFT = "Gift: "
_GIFT_STRONG = " strong"
_DARK_PARADOX = re.compile(r"^(.*) t(\d+)$")


def tokenize(text):
    """The words of one card's printed text, as the model's token names.

    `+2/+3` is `<buff>`, other numbers are `<num>`, a `(3 left!)` style
    counter is dropped, and markup never reaches the words."""
    if not text:
        return []
    text = _MARKUP.sub(" ", text)
    text = _COUNTER.sub(" ", text)
    text = _BUFF.sub(_BUFF_CHAR, text)
    text = _NUMBER.sub(_NUM_CHAR, text)
    text = text.lower().replace("\u2019", "'").replace("\xa0", " ")
    words = []
    for word in _WORD.findall(text):
        if word == _BUFF_CHAR:
            words.append("<buff>")
        elif word == _NUM_CHAR:
            words.append("<num>")
        else:
            words.append(word)
    return words


def _by_name(cards):
    out = {}
    for card in cards:
        name = card.get("name")
        if name:
            out.setdefault(name, []).append(card)
    return out


def _pick(candidates):
    """One card's text by name: a Battlegrounds minion or spell, normal
    before golden. None when the name has no printed Battlegrounds text."""
    cards = [c for c in candidates if c.get("set") == "BATTLEGROUNDS"
             and (c.get("text") or "").strip()]
    if not cards:
        return None
    return min(cards, key=lambda c: (
        c.get("type") not in ("MINION", "SPELL"),
        bool(c.get("battlegroundsNormalDbfId")),
        c.get("dbfId") or 0,
    ))


def _lookup(name, by_name):
    key = name
    if key.startswith(_GIFT):
        key = key[len(_GIFT):]
        if key.endswith(_GIFT_STRONG):
            key = key[:-len(_GIFT_STRONG)]
    match = _DARK_PARADOX.match(key)
    if match:
        base, suffix = match.groups()
        versions = [c for c in by_name.get(base, [])
                    if (c.get("id") or "").endswith("t" + suffix)
                    and not c.get("battlegroundsNormalDbfId")]
        card = _pick(versions)
        if card is not None:
            return card
    return _pick(by_name.get(key, []))


def read_texts(names, path=REF):
    """({name: printed text}, [names without one]) from the card export."""
    cards = json.loads(Path(path).read_text())
    by_name = _by_name(cards)
    texts, missing = {}, []
    for name in names:
        card = _lookup(name, by_name)
        if card is None:
            missing.append(name)
        else:
            texts[name] = card["text"]
    return texts, missing


def build_table(names, path=REF):
    """The fixed (len(names) + 1, MAX_TOKENS) token table and its vocabulary.

    Row 0 is the empty card; a card with no text is an all-zero row. Text
    longer than MAX_TOKENS is cut there."""
    texts, missing = read_texts(names, path)
    tokenized = [tokenize(texts.get(name, "")) for name in names]
    vocab = sorted({token for tokens in tokenized for token in tokens})
    index = {token: i + 1 for i, token in enumerate(vocab)}
    rows = [[0] * MAX_TOKENS]
    for tokens in tokenized:
        ids = [index[token] for token in tokens[:MAX_TOKENS]]
        rows.append(ids + [0] * (MAX_TOKENS - len(ids)))
    return {"vocab": vocab, "tokens": rows, "missing": missing}


FORMS = ROOT / "data" / "card_forms.json"
FORM_FIELDS = ("trigger", "effect", "target", "tribes", "scaling", "scales_with", "wants")


def build_forms(names, path=FORMS):
    """One numeric row per card from data/card_forms.json, row 0 the empty
    card: a multi-hot of every field value, then log1p of the two buff
    numbers. A card with no form is all zero. The vocabulary comes from the
    file, sorted, and is saved with the rows in the checkpoint."""
    forms = json.loads(Path(path).read_text())
    vocab = sorted({f"{f}={v}" for form in forms.values() for f in FORM_FIELDS
                    for v in (form[f] if isinstance(form[f], list) else [form[f]])})
    col = {k: i for i, k in enumerate(vocab)}
    n = len(vocab) + 2
    rows = [[0.0] * n]
    missing = []
    for name in names:
        row = [0.0] * n
        form = forms.get(name)
        if form is None:
            missing.append(name)
        else:
            for f in FORM_FIELDS:
                for v in form[f] if isinstance(form[f], list) else [form[f]]:
                    row[col[f"{f}={v}"]] = 1.0
            row[-2] = math.log1p(form["buff_attack"]) / 3.0
            row[-1] = math.log1p(form["buff_health"]) / 3.0
        rows.append(row)
    return {"form_vocab": vocab, "forms": rows, "form_missing": missing}


def table_names(path=CARDS_TOML):
    """Card names by id from a cards.toml, in the engine's order (minions,
    then tokens, then spells). For the standalone report, not the model."""
    data = tomllib.loads(Path(path).read_text())
    return [card["name"] for part in ("minion", "token", "spell")
            for card in data.get(part, [])]


def main(argv):
    names = table_names()
    table = build_table(names)
    rows = table["tokens"]
    lengths = [sum(1 for token in row if token) for row in rows]
    print(f"{len(names)} cards, {len(table['vocab'])} words, "
          f"{MAX_TOKENS} tokens per card, longest text {max(lengths)} tokens")
    missing = table["missing"]
    print(f"{len(missing)} cards with no text: {', '.join(missing)}")
    for name in argv:
        if name in names:
            print(f"{name}: {tokenize(read_texts([name])[0].get(name, ''))}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
