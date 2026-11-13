"""Can a small model learn which Tavern card a strong player buys?

    .venv/bin/python train/evaltools/buy_prior.py
    .venv/bin/python train/evaltools/buy_prior.py --bot

The default rebuilds report.md.  `--bot` trains the all-data prior, saves it,
and measures the dumped bot views against it in report-bot.md.

This is a feasibility check over the two JSON logs only.  It reads no engine,
no bgpy and no card database.  It writes train/runs/buy-prior/report.md.

Streamer logs: boards/twitch/games/*.json.  A turn has shop snapshots, events
in order, and the board at the end of the turn.  Era logs:
boards/frozen/actions-2026-11-10/*.json.  A turn has start (board, tavern,
trinkets, hero powers) and actions.

Cards are matched by NAME everywhere.  One example is one Tavern view: the
Tavern at the start of a turn or right after a refresh/roll.  Its label is the
set of offered card names bought before the next refresh/roll or the end of the
turn.  The model is a small CPU PyTorch model with one embedding per card name,
context = mean(board embeddings) + turn embedding, and an MLP score for every
offered card.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import math
from pathlib import Path

import numpy as np
import torch
from torch import nn
from torch.nn import functional as F

ROOT = Path(__file__).resolve().parents[2]
STREAM_DIR = ROOT / "boards" / "twitch" / "games"
ERA_DIR = ROOT / "boards" / "frozen" / "actions-2026-11-10"
OUT_DIR = ROOT / "train" / "runs" / "buy-prior"

PAD = 0
UNK = 1
MAX_TURN = 40
EMB = 32
HIDDEN = 64
EPOCHS = 10
BATCH = 128
LR = 0.01
REFRESH_LOSS = 0.2
HOLDOUT_PERCENT = 15

BANDS = [
    ("1-6", lambda t: t <= 6),
    ("7-9", lambda t: 7 <= t <= 9),
    ("10+", lambda t: t >= 10),
]


def unique(xs):
    out = []
    seen = set()
    for x in xs:
        if x not in seen:
            seen.add(x)
            out.append(x)
    return out


def card_name(x):
    if isinstance(x, dict):
        return x.get("name")
    return x


def is_missing(name):
    return name is None or str(name).startswith("#")


def sha_bucket(name):
    h = int(hashlib.sha1(name.encode("utf-8")).hexdigest()[:8], 16)
    return h % 100


def is_holdout(name):
    return sha_bucket(name) < HOLDOUT_PERCENT


def make_view(source, channel, turn, board, meta, offers_raw, offer_spells, context_missing=False):
    """One Tavern view.  `board` is a list of (name, golden), meta a name list."""
    offers = unique(offers_raw)
    spells = {}
    for name in offers:
        spells[name] = bool(offer_spells.get(name, False))
    return {
        "source": source,
        "channel": channel,
        "turn": int(turn),
        "board": [] if board is None else list(board),
        "meta": list(meta),
        "raw_offers": list(offers_raw),
        "offers": offers,
        "offer_spells": spells,
        "buys": [],
        "ended_refresh": False,
        "context_missing": context_missing,
    }


def streamer_views(path):
    d = json.loads(path.read_text())
    channel = d.get("channel") or path.name.rsplit("-", 1)[0]
    turns = sorted(d.get("turns", []), key=lambda t: t.get("turn", 0))
    raw_views = []
    prev_board = None
    prev_turn = None
    for t in turns:
        turn = int(t.get("turn", 0))
        if prev_turn is None:
            board = [] if turn == 1 else None
            missing = turn != 1
        else:
            board = prev_board
            missing = False
        meta = []
        for x in t.get("trinkets", []):
            name = card_name(x)
            if not is_missing(name):
                meta.append(name)
        hp = t.get("hero_power")
        if not is_missing(hp):
            meta.append(hp)

        shops = t.get("shops") or []
        offers = [card_name(x) for x in shops[0]] if shops else []
        cur = make_view(path.name, channel, turn, board, meta, offers, {}, missing)
        for e in t.get("events", []):
            kind = e.get("kind")
            if kind == "buy":
                name = card_name(e.get("card"))
                if name:
                    cur["buys"].append((name, bool(e.get("spell"))))
            elif kind == "refresh":
                cur["ended_refresh"] = True
                raw_views.append(cur)
                new_shop = e.get("new_shop") or []
                offers = [card_name(x) for x in new_shop]
                cur = make_view(path.name, channel, turn, board, meta, offers, {}, missing)
        raw_views.append(cur)
        prev_board = [(card_name(x), bool(x.get("golden")))
                      for x in t.get("board", []) if not is_missing(card_name(x))]
        prev_turn = turn
    return d, raw_views


def era_views(path):
    d = json.loads(path.read_text())
    raw_views = []
    for t in d.get("turns", []):
        turn = int(t.get("turn", 0))
        start = t.get("start", {})
        board = [(card_name(x), bool(x.get("golden")))
                 for x in start.get("board", []) if not is_missing(card_name(x))]
        meta = []
        for x in start.get("trinkets", []):
            name = card_name(x)
            if not is_missing(name):
                meta.append(name)
        for x in start.get("hero_powers", []):
            name = card_name(x)
            if not is_missing(name):
                meta.append(name)

        def offers_and_spells(cards):
            names = []
            spells = {}
            for x in cards:
                name = card_name(x)
                if is_missing(name):
                    continue
                names.append(name)
                spells[name] = spells.get(name, False) or (x.get("kind") == "spell")
            return names, spells

        offers, spells = offers_and_spells(start.get("tavern", []))
        cur = make_view(path.name, path.name.rsplit("-", 1)[0], turn, board, meta,
                        offers, spells)
        for a in t.get("actions", []):
            step = a.get("step")
            if step == "buy":
                name = card_name(a.get("card"))
                if name:
                    cur["buys"].append((name, bool(a.get("spell"))))
            elif step == "roll":
                cur["ended_refresh"] = True
                raw_views.append(cur)
                offers, spells = offers_and_spells(a.get("tavern", []))
                cur = make_view(path.name, path.name.rsplit("-", 1)[0], turn, board,
                                meta, offers, spells)
        raw_views.append(cur)
    return d, raw_views


def finalize(raw_views):
    """Drop views whose context or labels cannot be trusted."""
    kept = []
    drops = collections.Counter()
    dropped_with_buy = collections.Counter()
    duplicate_views = 0
    for v in raw_views:
        if v["context_missing"]:
            reason = "no previous logged board"
        elif not v["offers"]:
            reason = "no cards on offer"
        else:
            offered = collections.Counter(v["raw_offers"])
            bought = collections.Counter(name for name, _ in v["buys"])
            bad = any(n not in offered or c > offered[n] for n, c in bought.items())
            reason = "buy whose card is not in the view" if bad else None
        if reason is not None:
            drops[reason] += 1
            if v["buys"]:
                dropped_with_buy[reason] += 1
            continue
        bought = collections.Counter(name for name, _ in v["buys"])
        v["bought"] = set(bought)
        if len(v["offers"]) != len(v["raw_offers"]):
            duplicate_views += 1
        kept.append(v)
    return kept, drops, dropped_with_buy, duplicate_views


def load_dataset(kind):
    if kind == "streamer":
        paths = sorted(STREAM_DIR.glob("*.json"))
        parser = streamer_views
    else:
        paths = sorted(ERA_DIR.glob("*.json"))
        parser = era_views
    raw = []
    for p in paths:
        if p.name == "MANIFEST.txt":
            continue
        _, views = parser(p)
        raw.extend(views)
    kept, drops, dropped_with_buy, duplicate_views = finalize(raw)
    return paths, raw, kept, drops, dropped_with_buy, duplicate_views


def band_name(turn):
    for name, fn in BANDS:
        if fn(turn):
            return name
    return "10+"


def build_vocab(train, use_meta):
    card_names = set()
    meta_names = set()
    for v in train:
        card_names.update(v["offers"])
        card_names.update(n for n, _ in v["board"])
        if use_meta:
            meta_names.update(v["meta"])
    card_names = sorted(card_names)
    meta_names = sorted(meta_names) if use_meta else []
    card_idx = {n: i + 2 for i, n in enumerate(card_names)}
    meta_idx = {n: i + 2 for i, n in enumerate(meta_names)}
    return card_names, meta_names, card_idx, meta_idx


def encode(views, card_idx, meta_idx):
    out = []
    for v in views:
        out.append({
            "offers": v["offers"],
            "bought": v["bought"],
            "turn": v["turn"],
            "ended_refresh": v["ended_refresh"],
            "offer_idx": [card_idx.get(n, UNK) for n in v["offers"]],
            "labels": [1.0 if n in v["bought"] else 0.0 for n in v["offers"]],
            "board_idx": [card_idx.get(n, UNK) for n, _ in v["board"]],
            "meta_idx": [meta_idx.get(n, UNK) for n in v["meta"]],
        })
    return out


def collate(batch):
    b = len(batch)
    k = max(1, max(len(v["offer_idx"]) for v in batch))
    bmax = max(1, max(len(v["board_idx"]) for v in batch))
    mmax = max(1, max(len(v["meta_idx"]) for v in batch))
    offers = np.zeros((b, k), dtype=np.int64)
    offer_mask = np.zeros((b, k), dtype=np.float32)
    labels = np.zeros((b, k), dtype=np.float32)
    board = np.zeros((b, bmax), dtype=np.int64)
    board_mask = np.zeros((b, bmax), dtype=np.float32)
    meta = np.zeros((b, mmax), dtype=np.int64)
    meta_mask = np.zeros((b, mmax), dtype=np.float32)
    turns = np.zeros(b, dtype=np.int64)
    refresh = np.zeros(b, dtype=np.float32)
    for i, v in enumerate(batch):
        n = len(v["offer_idx"])
        offers[i, :n] = v["offer_idx"]
        offer_mask[i, :n] = 1.0
        labels[i, :n] = v["labels"]
        n = len(v["board_idx"])
        board[i, :n] = v["board_idx"]
        board_mask[i, :n] = 1.0
        n = len(v["meta_idx"])
        meta[i, :n] = v["meta_idx"]
        meta_mask[i, :n] = 1.0
        turns[i] = min(v["turn"], MAX_TURN)
        refresh[i] = 1.0 if (v["ended_refresh"] and not v["bought"]) else 0.0
    return {
        "offers": torch.from_numpy(offers),
        "offer_mask": torch.from_numpy(offer_mask),
        "labels": torch.from_numpy(labels),
        "board": torch.from_numpy(board),
        "board_mask": torch.from_numpy(board_mask),
        "meta": torch.from_numpy(meta),
        "meta_mask": torch.from_numpy(meta_mask),
        "turns": torch.from_numpy(turns),
        "refresh": torch.from_numpy(refresh),
    }


class BuyPrior(nn.Module):
    def __init__(self, n_cards, n_meta, use_meta):
        super().__init__()
        self.use_meta = use_meta
        self.card = nn.Embedding(n_cards, EMB, padding_idx=PAD)
        self.meta = nn.Embedding(max(1, n_meta), EMB, padding_idx=PAD)
        self.turn = nn.Embedding(MAX_TURN + 1, EMB, padding_idx=PAD)
        self.score = nn.Sequential(
            nn.Linear(2 * EMB, HIDDEN),
            nn.ReLU(),
            nn.Linear(HIDDEN, HIDDEN),
            nn.ReLU(),
            nn.Linear(HIDDEN, 1),
        )
        self.refresh = nn.Sequential(
            nn.Linear(EMB, HIDDEN),
            nn.ReLU(),
            nn.Linear(HIDDEN, 1),
        )

    def forward(self, offers, board, board_mask, meta, meta_mask, turns):
        ce = self.card(offers)
        be = self.card(board)
        ctx = (be * board_mask.unsqueeze(-1)).sum(1) / board_mask.sum(1, keepdim=True).clamp(min=1.0)
        if self.use_meta:
            me = self.meta(meta)
            ctx = ctx + (me * meta_mask.unsqueeze(-1)).sum(1) / meta_mask.sum(1, keepdim=True).clamp(min=1.0)
        ctx = ctx + self.turn(turns)
        pair = torch.cat([ce, ctx.unsqueeze(1).expand(-1, ce.shape[1], -1)], dim=-1)
        logits = self.score(pair).squeeze(-1)
        refresh_logit = self.refresh(ctx).squeeze(-1)
        return logits, refresh_logit


def load_prior(path, dev):
    """Load a saved BuyPrior and its checkpoint (card ids and turn cap)."""
    ck = torch.load(path, map_location="cpu", weights_only=False)
    sd = ck["state_dict"]
    model = BuyPrior(sd["card.weight"].shape[0],
                     sd["meta.weight"].shape[0],
                     bool(ck.get("use_meta", False)))
    model.load_state_dict(sd)
    return model.to(dev).eval(), ck


def make_pidx(card_names, ck, dev):
    """Map engine card ids (0 empty) to the prior's (0 empty, 1 unknown)."""
    idx = ck["card_idx"]
    return torch.tensor([0] + [idx.get(name, 1) for name in card_names],
                        dtype=torch.long, device=dev)


def prior_logits_for(obs_np, pidx, prior, ck, dev,
                     shop_cols, board_cols, turn_col,
                     slot_present, slot_card_id):
    """The prior's logits for the 7 Tavern minions and the Tavern spell.

    `shop_cols` and `board_cols` are the first observation column of each
    slot; `turn_col` is the turn scalar. `pidx` is make_pidx's mapping for
    the engine that wrote the observations. Returns one row of 8 logits per
    observation row, in the same slot order as shop_cols."""
    o = torch.from_numpy(obs_np).to(dev, dtype=torch.float32)
    sc = torch.tensor(shop_cols, dtype=torch.long, device=dev)
    bc = torch.tensor(board_cols, dtype=torch.long, device=dev)
    sid = (o[:, sc + slot_card_id]
           * (o[:, sc + slot_present] > 0)).long()
    bid = (o[:, bc + slot_card_id]
           * (o[:, bc + slot_present] > 0)).long()
    offers = pidx[sid]
    board = pidx[bid]
    bmask = (bid > 0).float()
    turns = o[:, turn_col].long().clamp(0, int(ck.get("max_turn", MAX_TURN)))
    meta = torch.zeros((len(o), 1), dtype=torch.long, device=dev)
    meta_mask = torch.zeros((len(o), 1), dtype=torch.float32, device=dev)
    with torch.no_grad():
        logits, _ = prior(offers, board, bmask, meta, meta_mask, turns)
    return logits


def train_model(train_encoded, card_names, meta_names, use_meta, seed):
    torch.manual_seed(seed)
    model = BuyPrior(len(card_names) + 2, len(meta_names) + 2, use_meta)
    opt = torch.optim.Adam(model.parameters(), lr=LR, weight_decay=1e-4)
    n = len(train_encoded)
    for _epoch in range(EPOCHS):
        perm = torch.randperm(n).tolist()
        for start in range(0, n, BATCH):
            batch = [train_encoded[i] for i in perm[start:start + BATCH]]
            t = collate(batch)
            logits, refresh_logit = model(
                t["offers"], t["board"], t["board_mask"], t["meta"],
                t["meta_mask"], t["turns"])
            card_loss = F.binary_cross_entropy_with_logits(
                logits, t["labels"], reduction="none")
            card_loss = (card_loss * t["offer_mask"]).sum() / t["offer_mask"].sum().clamp(min=1.0)
            refresh_loss = F.binary_cross_entropy_with_logits(refresh_logit, t["refresh"])
            loss = card_loss + REFRESH_LOSS * refresh_loss
            opt.zero_grad()
            loss.backward()
            opt.step()
    model.eval()
    return model


@torch.no_grad()
def predict(model, encoded, batch_size=256):
    all_logits = []
    all_refresh = []
    for start in range(0, len(encoded), batch_size):
        batch = encoded[start:start + batch_size]
        t = collate(batch)
        logits, refresh_logit = model(
            t["offers"], t["board"], t["board_mask"], t["meta"],
            t["meta_mask"], t["turns"])
        for i, v in enumerate(batch):
            all_logits.append(logits[i, :len(v["offers"])].cpu().numpy())
        all_refresh.extend(refresh_logit.cpu().numpy().tolist())
    return all_logits, all_refresh


def auc_score(y, s):
    y = np.asarray(y, dtype=bool)
    s = np.asarray(s, dtype=float)
    npos = int(y.sum())
    nneg = int((~y).sum())
    if npos == 0 or nneg == 0:
        return float("nan")
    order = np.argsort(s, kind="mergesort")
    sorted_s = s[order]
    ranks = np.empty(len(s), dtype=float)
    i = 0
    while i < len(s):
        j = i + 1
        while j < len(s) and sorted_s[j] == sorted_s[i]:
            j += 1
        ranks[order[i:j]] = 0.5 * (i + 1 + j)
        i = j
    return float((ranks[y].sum() - npos * (npos + 1) / 2.0) / (npos * nneg))


def top1_value(offers, scores, bought):
    mx = max(scores)
    top = [i for i, s in enumerate(scores) if s == mx]
    return sum(1 for i in top if offers[i] in bought) / len(top)


def popularity_scores(train, eval_views):
    offers = collections.Counter()
    buys = collections.Counter()
    for v in train:
        for name in v["offers"]:
            offers[name] += 1
            if name in v["bought"]:
                buys[name] += 1
    return {name: buys[name] / offers[name] for name in offers}


def view_metrics(views, model_logits, model_refresh, pop):
    """Per-view rows for views with at least one buy, plus refresh AUC."""
    rows = []
    refresh_y = []
    refresh_s = []
    for v, logits, rlogit in zip(views, model_logits, model_refresh):
        refresh_y.append(1 if (v["ended_refresh"] and not v["bought"]) else 0)
        refresh_s.append(float(rlogit))
        if not v["bought"]:
            continue
        offered = v["offers"]
        bought = v["bought"]
        pop_s = [pop.get(name, 0.0) for name in offered]
        rows.append({
            "turn": v["turn"],
            "random": 1.0 / len(offered),
            "top1_pop": top1_value(offered, pop_s, bought),
            "top1_model": top1_value(offered, list(map(float, logits)), bought),
            "y": [1 if name in bought else 0 for name in offered],
            "pop": pop_s,
            "model": list(map(float, logits)),
        })
    refresh_auc = auc_score(refresh_y, refresh_s) if refresh_y else float("nan")
    return rows, refresh_auc


def aggregate(rows):
    out = {}
    groups = [("overall", rows)] + [
        (name, [r for r in rows if band_name(r["turn"]) == name]) for name, _ in BANDS
    ]
    for name, rs in groups:
        if not rs:
            out[name] = {
                "n": 0, "random": float("nan"), "top1_pop": float("nan"),
                "top1_model": float("nan"), "auc_pop": float("nan"),
                "auc_model": float("nan"),
            }
            continue
        y = [a for r in rs for a in r["y"]]
        ps = [a for r in rs for a in r["pop"]]
        ms = [a for r in rs for a in r["model"]]
        out[name] = {
            "n": len(rs),
            "random": float(np.mean([r["random"] for r in rs])),
            "top1_pop": float(np.mean([r["top1_pop"] for r in rs])),
            "top1_model": float(np.mean([r["top1_model"] for r in rs])),
            "auc_pop": auc_score(y, ps),
            "auc_model": auc_score(y, ms),
        }
    return out


def fmt(x, digits=3):
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return "n/a"
    return f"{x:.{digits}f}"


def run_scenario(name, train_views, eval_views, seed, with_meta=True):
    """Train base and base+meta on the same split, evaluate both."""
    out = {"name": name, "train_views": len(train_views), "eval_views": len(eval_views)}
    pop = popularity_scores(train_views, eval_views)
    for variant in (["base", "meta"] if with_meta else ["base"]):
        use_meta = variant == "meta"
        card_names, meta_names, card_idx, meta_idx = build_vocab(train_views, use_meta)
        train_enc = encode(train_views, card_idx, meta_idx)
        eval_enc = encode(eval_views, card_idx, meta_idx)
        model = train_model(train_enc, card_names, meta_names, use_meta,
                            seed + (1000 if use_meta else 0))
        logits, refresh = predict(model, eval_enc)
        rows, refresh_auc = view_metrics(eval_views, logits, refresh, pop)
        out[variant] = {
            "model": model,
            "vocab": card_names,
            "card_idx": card_idx,
            "metrics": aggregate(rows),
            "refresh_auc": refresh_auc,
            "eval_with_buy": len(rows),
        }
    return out


def no_context_top(model, card_names, card_idx):
    pseudo = []
    for name in card_names:
        pseudo.append({
            "offers": [name], "bought": set(), "turn": 9, "ended_refresh": False,
            "offer_idx": [card_idx[name]], "labels": [0.0], "board_idx": [],
            "meta_idx": [],
        })
    logits, _ = predict(model, pseudo, batch_size=256)
    scores = [(float(x[0]), name) for x, name in zip(logits, card_names)]
    scores.sort(reverse=True)
    return scores


def board_contexts(views):
    seen = set()
    out = []
    for v in views:
        if v["turn"] != 9 or not v["board"]:
            continue
        names = tuple(n for n, _ in v["board"])
        if names in seen:
            continue
        seen.add(names)
        out.append(v)
    return out


@torch.no_grad()
def board_cases(model, card_names, card_idx, contexts, want=10):
    """Find cards whose score moves most between two real turn-9 boards."""
    if len(contexts) < 2:
        return []
    all_ids = torch.arange(2, len(card_names) + 2, dtype=torch.long)
    ce = model.card(all_ids)
    names = card_names
    n = len(names)
    score_rows = []
    rank_rows = []
    for v in contexts:
        board_idx = [card_idx.get(name, UNK) for name, _ in v["board"]]
        board = torch.tensor([board_idx], dtype=torch.long)
        mask = torch.ones_like(board, dtype=torch.float32)
        meta = torch.zeros((1, 1), dtype=torch.long)
        meta_mask = torch.zeros((1, 1), dtype=torch.float32)
        turn = torch.tensor([9], dtype=torch.long)
        be = model.card(board)
        ctx = (be * mask.unsqueeze(-1)).sum(1) / mask.sum(1, keepdim=True).clamp(min=1.0)
        if model.use_meta:
            me = model.meta(meta)
            ctx = ctx + (me * meta_mask.unsqueeze(-1)).sum(1) / meta_mask.sum(1, keepdim=True).clamp(min=1.0)
        ctx = ctx + model.turn(turn)
        pair = torch.cat([ce, ctx.expand(n, -1)], dim=-1)
        scores = model.score(pair).squeeze(-1).numpy()
        order = np.argsort(-scores)
        ranks = np.empty(n, dtype=int)
        ranks[order] = np.arange(1, n + 1)
        score_rows.append(scores)
        rank_rows.append(ranks)
    score_rows = np.stack(score_rows)
    rank_rows = np.stack(rank_rows)
    board_sets = [set(n for n, _ in v["board"]) for v in contexts]

    candidates = []
    for ci, name in enumerate(names):
        allowed = [j for j, b in enumerate(board_sets) if name not in b]
        if len(allowed) < 2:
            continue
        vals = [(score_rows[j, ci], j) for j in allowed]
        hi_s, hi_j = max(vals)
        lo_s, lo_j = min(vals)
        gap = hi_s - lo_s
        candidates.append((gap, ci, hi_j, lo_j, hi_s, lo_s))
    candidates.sort(reverse=True)

    picked = []
    used_cards = set()
    used_boards = set()
    board_use = collections.Counter()
    for gap, ci, hi_j, lo_j, hi_s, lo_s in candidates:
        name = names[ci]
        if name in used_cards:
            continue
        pair = (hi_j, lo_j)
        if pair in used_boards and len(picked) < want:
            continue
        if board_use[hi_j] >= 2 or board_use[lo_j] >= 2:
            continue
        rank_hi = int(rank_rows[hi_j, ci])
        rank_lo = int(rank_rows[lo_j, ci])
        if rank_hi > max(20, n // 6) or rank_lo < n // 2:
            continue
        picked.append({
            "card": name, "gap": gap, "hi": hi_j, "lo": lo_j,
            "hi_score": hi_s, "lo_score": lo_s,
            "hi_rank": rank_hi, "lo_rank": rank_lo, "n": n,
        })
        used_cards.add(name)
        used_boards.add(pair)
        board_use[hi_j] += 1
        board_use[lo_j] += 1
        if len(picked) >= want:
            break
    if len(picked) < want:
        # Relax the strong rank separation but keep distinct cards.
        for gap, ci, hi_j, lo_j, hi_s, lo_s in candidates:
            name = names[ci]
            if name in used_cards:
                continue
            picked.append({
                "card": name, "gap": gap, "hi": hi_j, "lo": lo_j,
                "hi_score": hi_s, "lo_score": lo_s,
                "hi_rank": int(rank_rows[hi_j, ci]),
                "lo_rank": int(rank_rows[lo_j, ci]), "n": n,
            })
            used_cards.add(name)
            if len(picked) >= want:
                break
    return [(p, contexts[p["hi"]], contexts[p["lo"]]) for p in picked]


def board_text(v):
    parts = []
    for name, golden in v["board"]:
        parts.append(name + ("*" if golden else ""))
    return ", ".join(parts) if parts else "(empty)"


def data_row(kind, paths, raw, kept, drops, dropped_with_buy, duplicate_views):
    return {
        "kind": kind,
        "games": len(paths),
        "raw": len(raw),
        "kept": len(kept),
        "with_buy": sum(1 for v in kept if v["bought"]),
        "drops": drops,
        "dropped_with_buy": dropped_with_buy,
        "duplicates": duplicate_views,
    }


def metric_table(results):
    lines = []
    lines.append("| band | views | top1 random | top1 popularity | top1 model | top1 model+meta | AUC popularity | AUC model | AUC model+meta |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for band in ["1-6", "7-9", "10+", "overall"]:
        b = results["base"]["metrics"][band]
        m = results["meta"]["metrics"][band]
        lines.append(
            f"| {band} | {b['n']} | {fmt(b['random'])} | {fmt(b['top1_pop'])} | "
            f"{fmt(b['top1_model'])} | {fmt(m['top1_model'])} | {fmt(b['auc_pop'])} | "
            f"{fmt(b['auc_model'])} | {fmt(m['auc_model'])} |")
    return "\n".join(lines)



BOT_FILES = [
    ("lagmix-lag0", "bot-views-lagmix-lag0.json"),
    ("lagmix-lag2", "bot-views-lagmix-lag2.json"),
    ("oneturn-lag0", "bot-views-oneturn-lag0.json"),
]


def load_bot_views(path, label):
    """Read one dumped bot view file.  Drop views with no usable context."""
    raw = json.loads(path.read_text())
    kept = []
    drops = collections.Counter()
    name_stats = {
        "names": set(),
        "offers": collections.Counter(),
        "board": collections.Counter(),
        "buys": collections.Counter(),
    }
    for v in raw:
        turn = int(v.get("turn", 0))
        board_raw = v.get("board")
        offered_raw = list(v.get("offered") or [])
        spell = v.get("spell")
        if spell:
            offered_raw.append(spell)
        bought_list = list(v.get("bought") or [])
        if turn >= 1:
            for name in offered_raw:
                if name:
                    name_stats["names"].add(name)
                    name_stats["offers"][name] += 1
            for x in board_raw or []:
                name = x.get("name")
                if name:
                    name_stats["names"].add(name)
                    name_stats["board"][name] += 1
            for name in bought_list:
                if name:
                    name_stats["names"].add(name)
                    name_stats["buys"][name] += 1

        if turn < 1:
            drops["turn below 1"] += 1
            continue
        if board_raw is None:
            drops["no board"] += 1
            continue
        offers = unique(offered_raw)
        if not offers:
            drops["no cards on offer"] += 1
            continue
        offered_count = collections.Counter(offered_raw)
        bought_count = collections.Counter(bought_list)
        if any(n not in offered_count or c > offered_count[n]
               for n, c in bought_count.items()):
            drops["buy whose card is not in the view"] += 1
            continue
        board = []
        for x in board_raw:
            name = x.get("name")
            if name:
                board.append((name, bool(x.get("golden"))))
        ended_roll = v.get("ended") == "roll"
        kept.append({
            "source": f"{label}:{v.get('game')}",
            "game": v.get("game"),
            "channel": label,
            "turn": turn,
            "board": board,
            "meta": [],
            "raw_offers": offered_raw,
            "offers": offers,
            "offer_spells": {},
            "bought": set(bought_list),
            "ended_refresh": ended_roll,
            "ended_roll": ended_roll,
            "context_missing": False,
        })
    return raw, kept, drops, name_stats


def train_predict(train_views, eval_views, seed):
    card_names, meta_names, card_idx, meta_idx = build_vocab(train_views, False)
    train_enc = encode(train_views, card_idx, meta_idx)
    eval_enc = encode(eval_views, card_idx, meta_idx)
    model = train_model(train_enc, card_names, meta_names, False, seed)
    logits, _refresh = predict(model, eval_enc)
    return model, card_names, card_idx, meta_idx, logits


def max_scores(logits):
    return [float(max(x)) for x in logits]


def top1_vals(views, logits):
    return [top1_value(v["offers"], [float(x) for x in l], v["bought"])
            for v, l in zip(views, logits)]


def calc_cells(views, logits, threshold):
    """Metrics for one view set, split by band and overall."""
    out = {}
    for band in ["1-6", "7-9", "10+", "overall"]:
        ids = [i for i, v in enumerate(views)
               if band == "overall" or band_name(v["turn"]) == band]
        if not ids:
            out[band] = {
                "n_all": 0, "n_buy": 0, "top1_buy": float("nan"),
                "top1_all": float("nan"), "n_high": 0, "top1_high": float("nan"),
                "roll": float("nan"), "views_per_turn": float("nan"),
                "n_turns": 0,
            }
            continue
        scores = [float(max(logits[i])) for i in ids]
        vals = [top1_value(views[i]["offers"], [float(x) for x in logits[i]],
                           views[i]["bought"]) for i in ids]
        buy_ids = [i for i in ids if views[i]["bought"]]
        high_ids = [i for i in ids if float(max(logits[i])) > threshold]
        high_pos = {i: pos for pos, i in enumerate(ids)}
        n_turns = len({(views[i]["source"], views[i]["turn"]) for i in ids})
        out[band] = {
            "n_all": len(ids),
            "n_buy": len(buy_ids),
            "top1_buy": float(np.mean([top1_value(
                views[i]["offers"], [float(x) for x in logits[i]],
                views[i]["bought"]) for i in buy_ids])) if buy_ids else float("nan"),
            "top1_all": float(np.mean(vals)),
            "n_high": len(high_ids),
            "top1_high": float(np.mean([vals[high_pos[i]] for i in high_ids]))
            if high_ids else float("nan"),
            "roll": float(np.mean([bool(views[i].get("ended_roll",
                                                    views[i].get("ended_refresh")))
                                   for i in ids])),
            "views_per_turn": len(ids) / n_turns if n_turns else float("nan"),
            "n_turns": n_turns,
        }
    return out


def rate_cell(value, n):
    return f"{fmt(value)} ({n})"


def vpt_cell(value, n_views, n_turns):
    return f"{fmt(value, 2)} ({n_views}/{n_turns})"


def card_buy_rates(views):
    offers = collections.Counter()
    buys = collections.Counter()
    for v in views:
        if v["turn"] < 7:
            continue
        for name in v["offers"]:
            offers[name] += 1
            if name in v["bought"]:
                buys[name] += 1
    return offers, buys


def bought_score_stats(views, logits):
    vals = []
    for v, l in zip(views, logits):
        by_name = {name: float(score) for name, score in zip(v["offers"], l)}
        for name in sorted(v["bought"]):
            if name in by_name:
                vals.append(by_name[name])
    if not vals:
        return float("nan"), 0
    return float(np.mean(vals)), len(vals)


def run_bot_report(args, skept, ekept):
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    streamer_train = [v for v in skept if not is_holdout(v["source"])]
    streamer_test = [v for v in skept if is_holdout(v["source"])]

    all_views = skept + ekept
    all_card_names, all_meta_names, all_card_idx, all_meta_idx = build_vocab(all_views, False)
    all_train = encode(all_views, all_card_idx, all_meta_idx)
    all_model = train_model(all_train, all_card_names, all_meta_names, False, args.seed + 30)
    model_path = out / "buy_prior_all.pt"
    torch.save({
        "state_dict": all_model.state_dict(),
        "card_names": all_card_names,
        "card_idx": all_card_idx,
        "meta_names": [],
        "use_meta": False,
        "emb": EMB,
        "hidden": HIDDEN,
        "max_turn": MAX_TURN,
        "train_views": len(all_views),
        "train_streamer_views": len(skept),
        "train_era_views": len(ekept),
        "seed": args.seed + 30,
    }, model_path)

    _a_model, _a_names, _a_idx, _a_meta, a_logits = train_predict(
        streamer_train, streamer_test, args.seed)
    _b_model, _b_names, _b_idx, _b_meta, b_logits = train_predict(
        skept, ekept, args.seed + 10)
    human_views = streamer_test + ekept
    human_logits = list(a_logits) + list(b_logits)
    human_top_scores = max_scores(human_logits)
    threshold = float(np.median(human_top_scores))

    bot_records = []
    oov_rows = []
    human_names = set()
    for v in all_views:
        human_names.update(v["offers"])
        human_names.update(n for n, _ in v["board"])
    for label, filename in BOT_FILES:
        path = out / filename
        if not path.is_file():
            raise SystemExit(f"missing bot view file: {path}")
        raw, views, drops, names = load_bot_views(path, label)
        _model = all_model
        enc = encode(views, all_card_idx, all_meta_idx)
        logits, _refresh = predict(all_model, enc)
        bot_records.append((label, views, logits, raw, drops))
        oov = sorted(n for n in names["names"] if n not in human_names)
        oov_rows.append({
            "label": label, "path": path, "raw": len(raw), "kept": len(views),
            "drops": drops, "names": names, "oov": oov,
        })

    datasets = []
    for label, views, logits, _raw, _drops in bot_records:
        datasets.append((label, calc_cells(views, logits, threshold)))
    datasets.append(("humans streamers (A holdout)",
                     calc_cells(streamer_test, a_logits, threshold)))
    datasets.append(("humans Era (B)", calc_cells(ekept, b_logits, threshold)))
    datasets.append(("humans pooled",
                     calc_cells(human_views, human_logits, threshold)))
    by_label = {label: (views, logits)
                for label, views, logits, _raw, _drops in bot_records}

    human_offers, human_buys = card_buy_rates(human_views)
    bot_gap_views = by_label["lagmix-lag0"][0] + by_label["lagmix-lag2"][0]
    bot_offers, bot_buys = card_buy_rates(bot_gap_views)
    common = [n for n in human_offers
              if human_offers[n] >= 30 and bot_offers.get(n, 0) >= 30]
    gap_rows = []
    for name in common:
        hr = human_buys[name] / human_offers[name]
        br = bot_buys[name] / bot_offers[name]
        gap_rows.append({
            "card": name, "human_offers": human_offers[name],
            "human_buys": human_buys[name], "human_rate": hr,
            "bot_offers": bot_offers[name], "bot_buys": bot_buys[name],
            "bot_rate": br, "gap": hr - br,
        })
    top_human = sorted(gap_rows, key=lambda r: (-r["gap"], r["card"]))[:30]
    top_bot = sorted(gap_rows, key=lambda r: (r["gap"], r["card"]))[:15]

    human_pooled_mean, human_pooled_n = bought_score_stats(human_views, human_logits)
    human_stream_mean, human_stream_n = bought_score_stats(streamer_test, a_logits)
    human_era_mean, human_era_n = bought_score_stats(ekept, b_logits)
    score_rows = []
    for label, views, logits, _raw, _drops in bot_records:
        bot_mean, bot_n = bought_score_stats(views, logits)
        score_rows.append({
            "label": label, "bot_mean": bot_mean, "bot_n": bot_n,
            "human_mean": human_pooled_mean, "human_n": human_pooled_n,
            "stream_mean": human_stream_mean, "stream_n": human_stream_n,
            "era_mean": human_era_mean, "era_n": human_era_n,
        })

    lines = []
    lines.append("# Bot buys against the learned prior")
    lines.append("")
    lines.append("Generated by `.venv/bin/python train/evaltools/buy_prior.py --bot`.  "
                 "The plain context prior (no meta variant) was trained on all kept "
                 f"streamer and Era views: {len(all_views)} views "
                 f"({len(skept)} streamer, {len(ekept)} Era).  Saved as "
                 f"`{model_path.relative_to(ROOT)}`.")
    lines.append("")
    lines.append("Human comparison uses honest held-out predictions: the A model "
                 f"trained without the {len(streamer_test)} held-out streamer views "
                 f"and the B model trained on streamers and tested on all {len(ekept)} "
                 "Era views.  The high-confidence threshold is the median human top-1 "
                 f"prior score over those held-out human views: {threshold:.3f}.")
    lines.append("")

    by_name = {label: cells for label, cells in datasets}
    b0 = by_name["lagmix-lag0"]["overall"]
    b2 = by_name["lagmix-lag2"]["overall"]
    b1 = by_name["oneturn-lag0"]["overall"]
    hu = by_name["humans pooled"]["overall"]
    sc = {r["label"]: r for r in score_rows}
    lines.append("## Findings")
    lines.append("")
    lines.append(
        f"On views with a buy, the prior's top-1 is bought "
        f"{fmt(b0['top1_buy'])} / {fmt(b2['top1_buy'])} / {fmt(b1['top1_buy'])} "
        f"of the time for lagmix lag 0, lagmix lag 2 and oneturn, against "
        f"{fmt(hu['top1_buy'])} for held-out humans.  Over all views the top-1 "
        f"card is bought {fmt(b0['top1_all'])} / {fmt(b2['top1_all'])} / "
        f"{fmt(b1['top1_all'])} against {fmt(hu['top1_all'])}; above the human "
        f"median top-1 score it is {fmt(b0['top1_high'])} / {fmt(b2['top1_high'])} / "
        f"{fmt(b1['top1_high'])} against {fmt(hu['top1_high'])}.  The bots also roll "
        f"much less (roll share {fmt(b0['roll'])} / {fmt(b2['roll'])} / {fmt(b1['roll'])} "
        f"against {fmt(hu['roll'])}) and see fewer views per turn "
        f"({fmt(b0['views_per_turn'], 2)} / {fmt(b2['views_per_turn'], 2)} / "
        f"{fmt(b1['views_per_turn'], 2)} against {fmt(hu['views_per_turn'], 2)}).  "
        f"The bought cards also carry lower prior scores: "
        f"{sc['lagmix-lag0']['bot_mean']:.3f} / {sc['lagmix-lag2']['bot_mean']:.3f} / "
        f"{sc['oneturn-lag0']['bot_mean']:.3f} against "
        f"{human_pooled_mean:.3f} for held-out humans.")
    lines.append("")

    lines.append("## Bot view files and name matching")
    lines.append("")
    lines.append("| file | raw views | kept | turn below 1 | no board | no cards | buy not in view | unique names | names not in human logs | OOV offers | OOV board | OOV buys |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for r in oov_rows:
        d = r["drops"]
        nm = r["names"]
        lines.append(
            f"| {r['label']} | {r['raw']} | {r['kept']} | "
            f"{d.get('turn below 1', 0)} | {d.get('no board', 0)} | "
            f"{d.get('no cards on offer', 0)} | "
            f"{d.get('buy whose card is not in the view', 0)} | "
            f"{len(nm['names'])} | {len(r['oov'])} | "
            f"{sum(nm['offers'][n] for n in r['oov'])} | "
            f"{sum(nm['board'][n] for n in r['oov'])} | "
            f"{sum(nm['buys'][n] for n in r['oov'])} |")
    lines.append("")
    for r in oov_rows:
        if r["oov"]:
            lines.append(f"- {r['label']} OOV names: " + ", ".join(r["oov"]))
    lines.append("")
    lines.append("Offers are `offered` plus `spell`, with duplicate names in one view "
                 "collapsed.  Name matching is against every kept streamer and Era view, "
                 "not only the held-out views.  OOV names use the shared OOV embedding.")
    lines.append("")

    lines.append("## 2a. Prior top-1 is one of the bought cards (views with a buy)")
    lines.append("")
    lines.append("| dataset | 1-6 | 7-9 | 10+ | overall |")
    lines.append("|---|---:|---:|---:|---:|")
    for label, cells in datasets:
        vals = []
        for band in ["1-6", "7-9", "10+", "overall"]:
            c = cells[band]
            vals.append(rate_cell(c["top1_buy"], c["n_buy"]))
        lines.append(f"| {label} | " + " | ".join(vals) + " |")
    lines.append("")

    lines.append("## 2b. Prior top-1 card bought over ALL views")
    lines.append("")
    lines.append("| dataset | 1-6 | 7-9 | 10+ | overall |")
    lines.append("|---|---:|---:|---:|---:|")
    for label, cells in datasets:
        vals = []
        for band in ["1-6", "7-9", "10+", "overall"]:
            c = cells[band]
            vals.append(rate_cell(c["top1_all"], c["n_all"]))
        lines.append(f"| {label} | " + " | ".join(vals) + " |")
    lines.append("")
    lines.append(f"Restricted to views whose top-1 prior score is above the human "
                 f"median {threshold:.3f}:")
    lines.append("")
    lines.append("| dataset | 1-6 | 7-9 | 10+ | overall |")
    lines.append("|---|---:|---:|---:|---:|")
    for label, cells in datasets:
        vals = []
        for band in ["1-6", "7-9", "10+", "overall"]:
            c = cells[band]
            vals.append(rate_cell(c["top1_high"], c["n_high"]))
        lines.append(f"| {label} | " + " | ".join(vals) + " |")
    lines.append("")

    lines.append("## 2c. Roll share and views per turn")
    lines.append("")
    lines.append("| dataset | 1-6 | 7-9 | 10+ | overall |")
    lines.append("|---|---:|---:|---:|---:|")
    for label, cells in datasets:
        vals = []
        for band in ["1-6", "7-9", "10+", "overall"]:
            c = cells[band]
            vals.append(rate_cell(c["roll"], c["n_all"]))
        lines.append(f"| {label} | " + " | ".join(vals) + " |")
    lines.append("")
    lines.append("Mean views per turn (views / distinct game-turn pairs):")
    lines.append("")
    lines.append("| dataset | 1-6 | 7-9 | 10+ | overall |")
    lines.append("|---|---:|---:|---:|---:|")
    for label, cells in datasets:
        vals = []
        for band in ["1-6", "7-9", "10+", "overall"]:
            c = cells[band]
            vals.append(vpt_cell(c["views_per_turn"], c["n_all"], c["n_turns"]))
        lines.append(f"| {label} | " + " | ".join(vals) + " |")
    lines.append("")

    lines.append("## 3. Cards humans buy and the bot does not, turns 7+")
    lines.append("")
    lines.append("Human rates pool held-out streamers and Era.  Bot rates pool "
                 "lagmix lag 0 and lag 2.  Only cards with at least 30 offers on each "
                 "side are included.")
    lines.append("")
    lines.append("Largest gaps where humans buy more:")
    lines.append("")
    lines.append("| card | human offers | human buys | human rate | bot offers | bot buys | bot rate | gap |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|")
    for r in top_human:
        lines.append(f"| {r['card']} | {r['human_offers']} | {r['human_buys']} | "
                     f"{r['human_rate']:.3f} | {r['bot_offers']} | {r['bot_buys']} | "
                     f"{r['bot_rate']:.3f} | {r['gap']:+.3f} |")
    lines.append("")
    lines.append("Largest gaps where the bot buys more:")
    lines.append("")
    lines.append("| card | human offers | human buys | human rate | bot offers | bot buys | bot rate | gap |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|")
    for r in top_bot:
        lines.append(f"| {r['card']} | {r['human_offers']} | {r['human_buys']} | "
                     f"{r['human_rate']:.3f} | {r['bot_offers']} | {r['bot_buys']} | "
                     f"{r['bot_rate']:.3f} | {r['gap']:+.3f} |")
    lines.append("")

    lines.append("## 4. Mean prior score of bought cards")
    lines.append("")
    lines.append("Bot bought cards are scored with the saved all-data prior.  Human "
                 "bought cards are scored with their held-out model.  Each bought name "
                 "in a view counts once.")
    lines.append("")
    lines.append("| bot file | bot mean (cards) | humans pooled mean (cards) | streamers mean (cards) | Era mean (cards) | bot minus human |")
    lines.append("|---|---:|---:|---:|---:|---:|")
    for r in score_rows:
        lines.append(
            f"| {r['label']} | {r['bot_mean']:.3f} ({r['bot_n']}) | "
            f"{r['human_mean']:.3f} ({r['human_n']}) | "
            f"{r['stream_mean']:.3f} ({r['stream_n']}) | "
            f"{r['era_mean']:.3f} ({r['era_n']}) | "
            f"{r['bot_mean'] - r['human_mean']:+.3f} |")
    lines.append("")

    lines.append("## Choices")
    lines.append("")
    choices = [
        "Bot views with `turn` 0, a null board, no offered cards, or a bought card not "
        "on offer are dropped.  The other views are used as dumped.  The drops are "
        "counted in the first table.",
        "Bot offers are `offered` plus `spell`.  Duplicate names inside one view are "
        "collapsed, as in the human examples.",
        "Name matching uses all kept streamer and Era views, not only the held-out "
        "views.  Unmatched engine names are listed and use the shared OOV embedding.  "
        "They cannot enter the per-card human-versus-bot table without human offers.",
        "The saved all-data prior is the plain context model, no meta variant, trained "
        "on all kept streamer plus Era views with the same architecture, loss, "
        "hyperparameters and 10 epochs as the earlier report.  It is saved as "
        "`train/runs/buy-prior/buy_prior_all.pt` with its vocabulary and metadata.",
        "Human comparison predictions are honest held-out predictions: streamers use "
        "the A split holdout model, Era uses the B model trained on streamers only.  "
        "The pooled human row is those two sets together.",
        "The 2a, 2b and 2c tables use the same turn bands as the first report.  A view "
        "is in exactly one band, and overall pools the bands.",
        "The high-confidence threshold is one global number: the median top-1 prior "
        "score over pooled held-out human views.  The same threshold is applied to "
        "every dataset and band.  Human and bot scores come from different models "
        "(held-out for humans, all-data for bots), so this is a common relative "
        "confidence cut, not a calibrated probability.",
        "2b counts a no-buy view as a miss, because the top-1 card was not bought.  Ties "
        "for the top score use the expected fraction of the tied cards that were bought.",
        "2c uses only kept views.  `mean views per turn` is views divided by distinct "
        "(game, turn) pairs present in that dataset and band, not by all turns in the "
        "game.  Human roll share counts views that ended by refresh/roll, bot roll share counts `ended == roll`.",
        "The per-card table uses turns 7 and later, unique offered names per view, human "
        "rates pooled over held-out streamers and Era, and bot rates pooled over lagmix "
        "lag 0 and lag 2.  It keeps only cards with at least 30 offers on each side.  "
        "The two lists are sorted by human rate minus bot rate: the first 30 positive, "
        "the first 15 negative.",
        "The mean purchased-card prior score uses the bot's all-data model for bot buys "
        "and the honest held-out models for human buys.  Each bought name in a view "
        "counts once.  The two sides are therefore both out of sample for their model, "
        "but the model scores are not on one shared scale.",
        "The bot report trains only A, B and the all-data prior.  It does not retrain "
        "the C channel folds, and it does not change the earlier report.",
    ]
    for i, text in enumerate(choices, start=1):
        lines.append(f"{i}. {text}")
    lines.append("")
    lines.append("## Files")
    lines.append("")
    lines.append("- `train/evaltools/buy_prior.py` (extended with `--bot`)")
    lines.append("- `train/runs/buy-prior/report-bot.md` (this report)")
    lines.append("- `train/runs/buy-prior/buy_prior_all.pt` (saved plain prior)")
    lines.append("")

    report = "\n".join(lines)
    (out / "report-bot.md").write_text(report)
    print(report)
    print(f"\nwrote {out / 'report-bot.md'}")


def main():
    global EPOCHS
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", type=Path, default=OUT_DIR)
    ap.add_argument("--epochs", type=int, default=EPOCHS)
    ap.add_argument("--seed", type=int, default=20261111)
    ap.add_argument("--bot", action="store_true",
                    help="train the all-data prior and write report-bot.md for the "
                         "dumped bot views; does not rebuild report.md")
    args = ap.parse_args()
    EPOCHS = args.epochs

    spaths, sraw, skept, sdrops, sdropbuy, sdup = load_dataset("streamer")
    epaths, eraw, ekept, edrops, edropbuy, edup = load_dataset("era")
    if args.bot:
        run_bot_report(args, skept, ekept)
        return
    sdata = data_row("streamers", spaths, sraw, skept, sdrops, sdropbuy, sdup)
    edata = data_row("Era", epaths, eraw, ekept, edrops, edropbuy, edup)

    streamer_train = [v for v in skept if not is_holdout(v["source"])]
    streamer_test = [v for v in skept if is_holdout(v["source"])]
    train_games = len({v["source"] for v in streamer_train})
    test_games = len({v["source"] for v in streamer_test})

    print(f"streamer views: {sdata['raw']} raw, {sdata['kept']} kept, "
          f"{sdata['with_buy']} with a buy; dropped {dict(sdrops)}")
    print(f"Era views: {edata['raw']} raw, {edata['kept']} kept, "
          f"{edata['with_buy']} with a buy; dropped {dict(edrops)}")

    results_a = run_scenario("A", streamer_train, streamer_test, args.seed)
    results_b = run_scenario("B", skept, ekept, args.seed + 10)
    channels = sorted({v["channel"] for v in skept})
    results_c = []
    for i, ch in enumerate(channels):
        tr = [v for v in skept if v["channel"] != ch]
        te = [v for v in skept if v["channel"] == ch]
        results_c.append((ch, run_scenario(f"C:{ch}", tr, te, args.seed + 20 + 2 * i)))

    model_b = results_b["base"]["model"]
    vocab_b = results_b["base"]["vocab"]
    idx_b = results_b["base"]["card_idx"]
    top = no_context_top(model_b, vocab_b, idx_b)
    contexts = board_contexts(skept)
    cases = board_cases(model_b, vocab_b, idx_b, contexts)

    out = args.out
    out.mkdir(parents=True, exist_ok=True)

    lines = []
    lines.append("# Buy prior from the streamer logs")
    lines.append("")
    lines.append("Generated by `train/evaltools/buy_prior.py` from the JSON logs only, "
                 "no engine and no bgpy.  Cards are matched by name.")
    lines.append("")

    lines.append("## Examples")
    lines.append("")
    lines.append("| log | games | raw views | kept views | with at least one buy | dropped: no previous board | dropped: no cards | dropped: buy not in view | duplicate-name views |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for d in (sdata, edata):
        lines.append(
            f"| {d['kind']} | {d['games']} | {d['raw']} | {d['kept']} | {d['with_buy']} | "
            f"{d['drops'].get('no previous logged board', 0)} | "
            f"{d['drops'].get('no cards on offer', 0)} | "
            f"{d['drops'].get('buy whose card is not in the view', 0)} | {d['duplicates']} |")
    lines.append("")
    lines.append(f"Streamer split by game name: {train_games} training games and "
                 f"{test_games} held-out games ({HOLDOUT_PERCENT}% hash bucket), "
                 f"{len(streamer_train)} training views and {len(streamer_test)} held-out views.")
    lines.append(f"Views dropped while containing a buy: streamers "
                 f"{dict(sdropbuy)}, Era {dict(edropbuy)}.")
    lines.append("A duplicate-name view offered the same name more than once.  Offers "
                 "are collapsed to unique names, so each offered name is one example.  "
                 "The raw multiset is still used to check that a buy was really in the view.")
    lines.append("")

    a = results_a["base"]["metrics"]["overall"]
    b = results_b["base"]["metrics"]["overall"]
    b16 = results_b["base"]["metrics"]["1-6"]
    b79 = results_b["base"]["metrics"]["7-9"]
    b10 = results_b["base"]["metrics"]["10+"]
    lines.append("## Verdict")
    lines.append("")
    lines.append(
        f"The context model beats popularity overall on held-out streamer games "
        f"(top-1 {fmt(a['top1_model'])} vs {fmt(a['top1_pop'])}, "
        f"AUC {fmt(a['auc_model'])} vs {fmt(a['auc_pop'])}).  It transfers to Era "
        f"overall (top-1 {fmt(b['top1_model'])} vs {fmt(b['top1_pop'])}, "
        f"AUC {fmt(b['auc_model'])} vs {fmt(b['auc_pop'])}), but not early: on Era "
        f"turns 1-6 it is flat or behind popularity (top-1 {fmt(b16['top1_model'])} "
        f"vs {fmt(b16['top1_pop'])}, AUC {fmt(b16['auc_model'])} vs "
        f"{fmt(b16['auc_pop'])}), while turns 7-9 and 10+ transfer "
        f"(AUC {fmt(b79['auc_model'])} vs {fmt(b79['auc_pop'])} and "
        f"{fmt(b10['auc_model'])} vs {fmt(b10['auc_pop'])}).  The trinket and "
        f"hero-power variant is flat or slightly worse overall in A and B, so the "
        f"extra context is not clearly useful.  So the logs do contain a learnable, "
        f"partly transferable buy prior, mainly on later turns.")
    lines.append("")

    lines.append("## Model")
    lines.append("")
    lines.append(f"Small CPU model: card embedding {EMB}, turn embedding {EMB}, board "
                 f"context = mean board card embeddings + turn embedding, score MLP "
                 f"2x{HIDDEN}, binary cross-entropy per offered card.  The meta variant "
                 "adds the mean of trinket and hero-power embeddings to the context.  "
                 f"Adam lr {LR}, {EPOCHS} epochs, batch {BATCH}, refresh-no-buy head "
                 f"weight {REFRESH_LOSS}.  Unseen names share one OOV embedding.  "
                 "The no-board context is the zero vector, plus the turn embedding.")
    lines.append("")

    lines.append("## A: streamers only, held-out streamer games")
    lines.append("")
    lines.append(f"Train {results_a['train_views']} views, test "
                 f"{results_a['eval_views']} views ({results_a['base']['eval_with_buy']} "
                 "with a buy).")
    lines.append("")
    lines.append(metric_table(results_a))
    lines.append("")
    lines.append(f"Refresh-no-buy AUC: base {fmt(results_a['base']['refresh_auc'])}, "
                 f"meta {fmt(results_a['meta']['refresh_auc'])}.")
    lines.append("")

    lines.append("## B: all streamers, all Era games (transfer)")
    lines.append("")
    lines.append(f"Train {results_b['train_views']} streamer views, test "
                 f"{results_b['eval_views']} Era views ({results_b['base']['eval_with_buy']} "
                 "with a buy).")
    lines.append("")
    lines.append(metric_table(results_b))
    lines.append("")
    lines.append(f"Refresh-no-buy AUC: base {fmt(results_b['base']['refresh_auc'])}, "
                 f"meta {fmt(results_b['meta']['refresh_auc'])}.")
    lines.append("")

    lines.append("## C: leave one channel out")
    lines.append("")
    lines.append("Each row trains on the other four channels and tests on all games of "
                 "the named channel.  `overall` is that channel's whole test set.")
    lines.append("")
    lines.append("| channel | band | views | top1 random | top1 popularity | top1 model | top1 model+meta | AUC popularity | AUC model | AUC model+meta |")
    lines.append("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for ch, r in results_c:
        for band in ["1-6", "7-9", "10+", "overall"]:
            b = r["base"]["metrics"][band]
            m = r["meta"]["metrics"][band]
            lines.append(
                f"| {ch} | {band} | {b['n']} | {fmt(b['random'])} | "
                f"{fmt(b['top1_pop'])} | {fmt(b['top1_model'])} | {fmt(m['top1_model'])} | "
                f"{fmt(b['auc_pop'])} | {fmt(b['auc_model'])} | {fmt(m['auc_model'])} |")
    lines.append("")
    lines.append("Refresh-no-buy AUC by test channel (base / meta): "
                 + ", ".join(f"{ch} {fmt(r['base']['refresh_auc'])}/{fmt(r['meta']['refresh_auc'])}"
                             for ch, r in results_c) + ".")
    lines.append("")

    lines.append("## What the model prefers with no context")
    lines.append("")
    lines.append("Top 25 card names from the B base model, empty board at turn 9:")
    lines.append("")
    lines.append("| rank | card | score |")
    lines.append("|---:|---|---:|")
    for rank, (score, name) in enumerate(top[:25], start=1):
        lines.append(f"| {rank} | {name} | {score:.3f} |")
    lines.append("")

    lines.append("## Where the board changes the ranking")
    lines.append("")
    lines.append(f"Card names scored at turn 9 on real streamer turn-9 boards "
                 f"({len(contexts)} distinct non-empty boards).  The card itself is not "
                 "on either board.  `*` marks a golden board card.  Rank is among the "
                 "model's streamer card vocabulary.")
    lines.append("")
    if cases:
        lines.append("| card | high score (rank) | board where it scores high | low score (rank) | board where it scores low | gap |")
        lines.append("|---|---:|---|---:|---|---:|")
        for p, hv, lv in cases:
            lines.append(
                f"| {p['card']} | {p['hi_score']:.3f} ({p['hi_rank']}/{p['n']}) | "
                f"{board_text(hv)} | {p['lo_score']:.3f} ({p['lo_rank']}/{p['n']}) | "
                f"{board_text(lv)} | {p['gap']:.3f} |")
    else:
        lines.append("No pair of distinct board contexts was available.")
    lines.append("")

    lines.append("## Choices")
    lines.append("")
    choices = [
        "Views: streamer start shop plus every `refresh` event's `new_shop`; Era "
        "`start.tavern` plus every `roll` action's `tavern`.  Shop snapshots between "
        "those boundaries are not separate views, and a mid-turn card added to the "
        "Tavern by a spell or hero power does not start one.  A view with any buy "
        "whose name was not in that view is dropped whole, not just that buy.",
        "Streamer start board: the previous logged turn's `board`.  The first logged "
        "turn uses an empty board only when it is turn 1; other first turns are dropped "
        "because no previous board was logged.  Era uses `start.board`.",
        "Board context records golden flags, but the model uses names only, as the brief "
        "specified one embedding per card name.  Offered spell flags are recorded when "
        "the log gives them, and are not model inputs.",
        "Trinket and hero-power context uses the streamer turn's logged trinkets and "
        "hero power and Era's `start.trinkets` and `start.hero_powers`.  A missing "
        "`#None` hero power is skipped.  The meta variant pools trinkets and hero "
        "powers into one mean.",
        "Repeated offered names are collapsed to one offered name, so the random "
        "baseline is 1 / unique names on offer.  A bought name counts as one positive "
        "however many copies were bought.  The raw multiset is kept only to validate "
        "buys.",
        "The popularity baseline is the empirical buy rate per offered name over the "
        "training views (buys / times offered), with 0 for a name never offered in "
        "training.  Top-1 ties are scored as the expected chance that a random member "
        "of the tied top set was bought; AUC uses average ranks for ties.",
        "The random top-1 baseline is the expected 1 / cards on offer, averaged over "
        "the evaluated views, not a sampled number.",
        "The highest-tier baseline is skipped because neither log carries a per-card "
        "tier, and the brief says to skip it when the logs do not give one.",
        "Game split: SHA1 of the file name, first 8 hex digits modulo 100, held out "
        "when under 15.  All views of a game go to the same side.  A and B use the same "
        "split rule; B ignores it because it trains on all 449 streamer games.",
        "Turn bands are exactly 1-6, 7-9 and 10+.  A view at turn 10 or later is clipped "
        "to the turn-40 embedding.  Overall rows pool the bands.",
        "AUC is computed only on views with at least one buy, as asked.  A band with no "
        "positive or no negative offered cards reports n/a.",
        "Three trainings A, B and C each fit both variants: base and base plus "
        "trinket/hero-power mean.  The base model is the primary table column; the meta "
        "variant is shown next to it.  The refresh head target is 1 only when the view "
        "ended by refresh and had zero buys; its AUC is over all evaluated views, not "
        "just the views with a buy, and it is not used by any ranking metric.",
        "Model choices: 32-d embeddings, 64-unit two-layer score MLP, 10 epochs, Adam "
        "with lr 0.01 and weight decay 1e-4, batch 128, BCE without class weights, "
        "shared OOV embedding, empty board context zero, CPU only.  There is no "
        "validation split or early stopping.",
        "The no-context top 25 comes from the B base model at turn 9 with an empty "
        "board, scored over the streamer training card vocabulary.  OOV is not listed.",
        "The board-change list uses distinct non-empty streamer boards logged at turn 9 "
        "and scores every vocabulary card at turn 9.  It prefers cards absent from both "
        "chosen boards, ranks high on one and low on the other, with distinct cards and "
        "board pairs; if too few pass the strict rank filter, it fills the rest by "
        "score gap.",
        "Dropped views are excluded from training, evaluation and popularity counts.  "
        "Seeds are fixed, so a rerun on the same logs gives the same numbers.",
    ]
    for i, text in enumerate(choices, start=1):
        lines.append(f"{i}. {text}")
    lines.append("")
    lines.append("## Files")
    lines.append("")
    lines.append("- `train/evaltools/buy_prior.py` (this script, ignored by git)")
    lines.append("- `train/runs/buy-prior/report.md` (this report, ignored by git)")
    lines.append("")

    report = "\n".join(lines)
    (out / "report.md").write_text(report)
    print(report)
    print(f"\nwrote {out / 'report.md'}")


if __name__ == "__main__":
    main()
