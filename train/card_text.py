"""Card text as token ids for the network.

Each card the model embeds (minions, tokens, spells, in the engine's order)
gets the printed text of its Battlegrounds card from
ref/hsjson-cards.json, tokenised: markup and `[x]`/`[DNT]` tags are dropped,
`+N/+M` is one `<buff>` token, every other number is `<num>`, and a printed
"(N left!)" counter is dropped (the observation carries the real count).
Keywords stay words. Golden and normal copies share the name and so the text.
The hero/power table (one starting power per hero in hero order, then the
`[[power]]` entries) and the trinket table get the same rows by the card id in
heroes.toml and trinkets.toml, on one shared vocabulary with the cards.
Dark Gifts need no rows of their own: the observation shows a gift as its
effect card's id in the main card table, so the card rows already cover them.

The output is `{"vocab": [...], "tokens": [[...], ...], "missing": [...]}`:
row 0 is the empty card and every row is padded with 0 to MAX_TOKENS. The
vocabulary is sorted, token id = index + 1, 0 is padding. `tokens` and
`vocab` go into the checkpoint's model_opts, so resume and the eval scripts
rebuild the model without the export. `model.load` reads today's export once
to widen the word table when the vocabulary grew (and falls back to the saved
list when `ref/` is absent); `missing` is only for the run's report.

`build_text` makes the card table and the power table together;
`build_power_table` adds the power rows to an existing card vocabulary and
`remap_table` moves the card rows onto it.

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
HEROES_TOML = ROOT / "heroes.toml"
TRINKETS_TOML = ROOT / "trinkets.toml"
DARKGIFTS_TOML = ROOT / "darkgifts.toml"

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
    """({name: printed text}, [names without one]) from the card export.

    A `Gift: ...` token takes its gift's exact card id from darkgifts.toml;
    the name alone cannot tell a strong gift from its base card."""
    cards = json.loads(Path(path).read_text())
    by_name = _by_name(cards)
    by_id = _by_id(cards)
    try:
        gifts = {token: card_id for card_id, token in gift_entries()}
    except OSError:
        gifts = {}
    texts, missing = {}, []
    for name in names:
        card = by_id.get(gifts.get(name))
        if card is None or not (card.get("text") or "").strip():
            card = _lookup(name, by_name)
        if card is None:
            missing.append(name)
        else:
            texts[name] = card["text"]
    return texts, missing


def _by_id(cards):
    return {card["id"]: card for card in cards if card.get("id")}


def read_id_texts(ids, path=REF):
    """({card id: printed text}, [ids without one]) from the card export.

    Hero powers, trinkets and Dark Gifts are looked up by their card id, not
    by name: a trinket's name can occur twice (lesser and greater), its card
    id does not."""
    by_id = _by_id(json.loads(Path(path).read_text()))
    texts, missing = {}, []
    for card_id in ids:
        card = by_id.get(card_id)
        if card is None or not (card.get("text") or "").strip():
            missing.append(card_id)
        else:
            texts[card_id] = card["text"]
    return texts, missing


def _rows(tokenized, vocab):
    """The fixed (len(tokenized) + 1, MAX_TOKENS) rows for `tokenized`, row 0
    the empty card, on the shared vocabulary `vocab`."""
    index = {token: i + 1 for i, token in enumerate(vocab)}
    rows = [[0] * MAX_TOKENS]
    for tokens in tokenized:
        ids = [index[token] for token in tokens[:MAX_TOKENS]]
        rows.append(ids + [0] * (MAX_TOKENS - len(ids)))
    return rows


def build_table(names, path=REF):
    """The fixed (len(names) + 1, MAX_TOKENS) token table and its vocabulary.

    Row 0 is the empty card; a card with no text is an all-zero row. Text
    longer than MAX_TOKENS is cut there."""
    texts, missing = read_texts(names, path)
    tokenized = [tokenize(texts.get(name, "")) for name in names]
    vocab = sorted({token for tokens in tokenized for token in tokens})
    return {"vocab": vocab, "tokens": _rows(tokenized, vocab), "missing": missing}


def _check_count(cards, names, what):
    if len(cards) != len(names):
        raise SystemExit(f"card_text.py: the engine has {len(names)} {what}s but the "
                         f"TOML lists {len(cards)} card ids: rebuild the table")


def build_text(card_names, power_names, trinket_names, path=REF):
    """(card table, power table) on one shared vocabulary.

    The power table holds the hero/power table's rows in `HeroDb::powers`
    order (a hero's own row is its starting power) and the trinket table's
    rows in engine order, both read from the TOMLs' card ids. Row 0 of each
    is empty and textless rows are all zero."""
    card_texts, card_missing = read_texts(card_names, path)
    power_cards, trinket_cards = power_ids(), trinket_ids()
    _check_count(power_cards, power_names, "hero power")
    _check_count(trinket_cards, trinket_names, "trinket")
    power_texts, power_missing = read_id_texts(power_cards, path)
    trinket_texts, trinket_missing = read_id_texts(trinket_cards, path)
    card_tokens = [tokenize(card_texts.get(name, "")) for name in card_names]
    power_tokens = [tokenize(power_texts.get(card_id, "")) for card_id in power_cards]
    trinket_tokens = [tokenize(trinket_texts.get(card_id, "")) for card_id in trinket_cards]
    vocab = sorted({token for tokens in card_tokens + power_tokens + trinket_tokens
                    for token in tokens})
    return (
        {"vocab": vocab, "tokens": _rows(card_tokens, vocab), "missing": card_missing},
        {"vocab": vocab, "powers": _rows(power_tokens, vocab),
         "trinkets": _rows(trinket_tokens, vocab),
         "missing": {"powers": power_missing, "trinkets": trinket_missing}},
    )


def build_power_table(power_names, trinket_names, vocab=None, path=REF):
    """The hero/power and trinket rows, their words added to `vocab`.

    For a run that adds power text to an existing card-text table: the
    returned vocabulary is `vocab` widened by the new words, so the caller
    must move the card rows onto it (see `remap_table`)."""
    power_cards, trinket_cards = power_ids(), trinket_ids()
    _check_count(power_cards, power_names, "hero power")
    _check_count(trinket_cards, trinket_names, "trinket")
    power_texts, power_missing = read_id_texts(power_cards, path)
    trinket_texts, trinket_missing = read_id_texts(trinket_cards, path)
    power_tokens = [tokenize(power_texts.get(card_id, "")) for card_id in power_cards]
    trinket_tokens = [tokenize(trinket_texts.get(card_id, "")) for card_id in trinket_cards]
    words = sorted(set(vocab or ()) | {token for tokens in power_tokens + trinket_tokens
                                       for token in tokens})
    return {"vocab": words, "powers": _rows(power_tokens, words),
            "trinkets": _rows(trinket_tokens, words),
            "missing": {"powers": power_missing, "trinkets": trinket_missing}}


def remap_table(table, vocab):
    """The same card rows on the shared vocabulary `vocab`.

    A word that is gone becomes padding, like model.load's widening."""
    if list(table.get("vocab") or []) == list(vocab):
        return table
    at = {word: i + 1 for i, word in enumerate(vocab)}
    lut = [0] + [at.get(word, 0) for word in table.get("vocab") or []]
    out = dict(table)
    out["vocab"] = list(vocab)
    out["tokens"] = [[lut[token] for token in row] for row in table["tokens"]]
    return out


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


def power_names(path=HEROES_TOML):
    """The hero/power table's names, in `HeroDb::powers` order: one starting
    power per hero in hero order, then the `[[power]]` entries."""
    data = tomllib.loads(Path(path).read_text())
    return ([hero["power"]["name"] for hero in data.get("hero", [])]
            + [power["name"] for power in data.get("power", [])])


def power_ids(path=HEROES_TOML):
    """The card id of every row `power_names` returns."""
    data = tomllib.loads(Path(path).read_text())
    return ([hero["power"]["card_id"] for hero in data.get("hero", [])]
            + [power["card_id"] for power in data.get("power", [])])


def _trinket_field(field, path=TRINKETS_TOML):
    """One field of every `[[trinket]]` block, in TrinketDb::parse order.
    tomllib rejects the file's multi-line inline tables (the Rust `toml`
    crate accepts them), so the field lines are read directly."""
    out, block = [], False
    for line in Path(path).read_text().splitlines():
        s = line.strip()
        if s == "[[trinket]]":
            block = True
        elif s.startswith("[["):
            block = False
        elif block:
            match = re.match(rf'{field}\s*=\s*"(.*)"\s*(?:#.*)?$', s)
            if match:
                out.append(match.group(1))
                block = False
    return out


def trinket_names(path=TRINKETS_TOML):
    return _trinket_field("name", path)


def trinket_ids(path=TRINKETS_TOML):
    return _trinket_field("card_id", path)


def gift_entries(path=DARKGIFTS_TOML):
    """(card id, cards.toml token name) per Dark Gift, in the TOML's order."""
    data = tomllib.loads(Path(path).read_text())
    return [(gift["id"], gift["card"]) for gift in data.get("gift", [])]


def build_gift_report(path=REF, gifts=DARKGIFTS_TOML):
    """(gifts, missing card ids) for the Dark Gifts.

    A gift reaches the network only as the card id of an effect card in the
    main card table (the gift slots on minions, and the Discover slots' gift
    fields during an offer), so it is already a card-text row. No separate
    input is built."""
    entries = gift_entries(gifts)
    _, missing = read_id_texts([card_id for card_id, _ in entries], path)
    return len(entries), missing


def main(argv):
    names = table_names()
    table = build_table(names)
    rows = table["tokens"]
    lengths = [sum(1 for token in row if token) for row in rows]
    print(f"{len(names)} cards, {len(table['vocab'])} words, "
          f"{MAX_TOKENS} tokens per card, longest text {max(lengths)} tokens")
    missing = table["missing"]
    print(f"{len(missing)} cards with no text: {', '.join(missing)}")
    cards, powers = build_text(names, power_names(), trinket_names())
    power_rows, trinket_rows = powers["powers"], powers["trinkets"]
    print(f"{len(power_rows) - 1} hero powers, "
          f"{len(powers['missing']['powers'])} without text; "
          f"{len(trinket_rows) - 1} trinkets, "
          f"{len(powers['missing']['trinkets'])} without text")
    gifts, gift_missing = build_gift_report()
    print(f"{gifts} Dark Gifts, {len(gift_missing)} without text (already card rows: "
          "a gift is embedded by its effect card's id, no separate input)")
    print(f"one shared vocabulary for cards, powers and trinkets: {len(powers['vocab'])} words")
    for name in argv:
        if name in names:
            print(f"{name}: {tokenize(read_texts([name])[0].get(name, ''))}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
