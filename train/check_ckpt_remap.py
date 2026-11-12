"""Check that a checkpoint loaded through the name remap answers exactly as
the network it was saved as.

    # once, with bgpy built at the old commit:
    PYTHONPATH=target/old-bgpy .venv/bin/python train/check_ckpt_remap.py \
        --write-obs target/ckpt-tables-old-obs.pt
    # then with today's bgpy:
    .venv/bin/python train/check_ckpt_remap.py train/runs/r1111-lagmix/best.pt \
        --obs target/ckpt-tables-old-obs.pt

--write-obs plays random legal games with the bgpy on PYTHONPATH and saves a
fixed set of observations and action masks. The old network is the
checkpoint's own embedding sizes and card-text table, loaded strictly. The new
one is `model.load` (remapped by name). Card ids in the old observations are
translated to today's ids by card name, because a new card can be inserted
before an existing spell; trinket ids are translated the same way. The script
prints the largest logit, masked action-probability and value differences, and
exits nonzero when any is 1e-5 or more.
"""

import argparse
import copy
import sys
from pathlib import Path

import numpy as np
import torch
from torch import nn

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "train"))

import bgpy
import model


def write_obs(path, n=256, steps=300, seed=3):
    """Random legal play from the bgpy on PYTHONPATH, one fixed set of rows."""
    import torch
    env = bgpy.VecEnv(n, seed, [4, 2, 1, 1, 0, 0])
    obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
    mask = np.zeros((n, bgpy.N_ACTIONS), bool)
    seat, kind = np.zeros(n, np.int64), np.zeros(n, np.int64)
    places = np.zeros((n, bgpy.PLAYERS), np.int8)
    tiers = np.zeros((n, bgpy.PLAYERS, bgpy.MAX_TIER), np.int8)
    tribes = np.zeros((n, bgpy.PLAYERS, bgpy.TRIBES), np.int8)
    reset = np.zeros(n, bool)
    rng = np.random.default_rng(seed)
    out = []
    for t in range(steps):
        env.observe(obs, mask, seat, kind)
        if t % 50 == 49:
            out.append((obs.copy(), mask.copy()))
        w = mask * rng.random(mask.shape)
        w[:, bgpy.END_TURN] *= 0.05
        w[:, model.SELL_A] *= 0.05
        env.step(w.argmax(1), places, tiers, tribes, reset)
    obs = torch.from_numpy(np.concatenate([o for o, _ in out])[-512:])
    mask = torch.from_numpy(np.concatenate([m for _, m in out])[-512:])
    torch.save({"obs": obs, "mask": mask}, path)
    print(f"wrote {path}: {len(obs)} observations from bgpy {bgpy.N_CARDS} cards, "
          f"{bgpy.N_TRINKETS} trinkets")


def _old_net(ck):
    """The checkpoint's network at its own table sizes, loaded strictly."""
    kind, opts = model.ck_model(ck)
    net = model.build(kind, **copy.deepcopy(opts))
    where = net if kind == "mlp" else net.inp
    for name in ("card", "hero", "trinket"):
        src = ck["net"][f"{'' if kind == 'mlp' else 'inp.'}{name}.weight"]
        emb = getattr(where, name).embedding_dim
        setattr(where, name, nn.Embedding(src.shape[0], emb))
    # The checkpoint's own card-text buffers fit these sizes; the strict load
    # below fills every row.
    net.load_state_dict(ck["net"])
    return net.eval()


def _occurrence_map(old_names, new_names):
    """old index + 1 -> new index + 1 (0 stays 0) by name and occurrence."""
    old_at = {}
    for i, n in enumerate(old_names):
        old_at.setdefault(n, []).append(i)
    used = {}
    lut = torch.zeros(len(old_names) + 1, dtype=torch.long)
    for i, n in enumerate(new_names):
        at = old_at.get(n)
        k = used.get(n, 0)
        if at is not None and k < len(at):
            lut[at[k] + 1] = i + 1
            used[n] = k + 1
    return lut


def _translate(obs, at, lut):
    ids = obs[:, at].long()
    obs[:, at] = lut[ids].float()


def translate_card_ids(obs, old_names, new_names):
    lut = _occurrence_map(old_names, new_names)
    for s in range(bgpy.OWN_SLOTS):
        for f in model.SLOT_IDS:
            _translate(obs, bgpy.SCALARS + s * bgpy.SLOT_FEATURES + f, lut)
    for at in model.SCALAR_IDS:
        _translate(obs, at, lut)
    for o in range(model.OPP):
        base = bgpy.OWN_END + o * bgpy.OPP_BLOCK + bgpy.OPP_SCALARS
        for s in range(bgpy.BOARD):
            for f in model.SLOT_IDS:
                _translate(obs, base + s * bgpy.SLOT_FEATURES + f, lut)


def translate_trinket_ids(obs, old_names, new_names):
    lut = _occurrence_map(old_names, new_names)
    for at in range(bgpy.SCALAR_TRINKET, bgpy.SCALAR_TRINKET + bgpy.TRINKET_SLOTS):
        _translate(obs, at, lut)
    for k in range(bgpy.TRINKET_CHOICES):
        _translate(obs, bgpy.SCALAR_TRINKET_OPTION + k * bgpy.TRINKET_OPTION_FEATURES, lut)
    for o in range(model.OPP):
        _translate(obs, bgpy.OWN_END + o * bgpy.OPP_BLOCK + bgpy.OPP_TRINKET, lut)


def check(path, obs_path):
    ck = torch.load(path, map_location="cpu", weights_only=False)
    old = _old_net(ck)
    new = model.load(path).eval()
    data = torch.load(obs_path, weights_only=False)
    obs, mask = data["obs"], data["mask"]
    old_obs = obs.clone()

    src_rows = ck["net"]["inp.card.weight"].shape[0] if "inp.card.weight" in ck["net"] \
        else ck["net"]["card.weight"].shape[0]
    old_cards = ck.get("card_names")
    if old_cards is None or len(old_cards) != src_rows - 1:
        old_cards = (model._infer_names(path, "cards", src_rows - 1) or (None,))[0]
        if old_cards is None:
            raise SystemExit(f"{path}: no card names and no matching cards.toml commit")
    translate_card_ids(obs, old_cards, model.current_names()["cards"])

    old_trinkets = ck.get("trinket_names")
    trows = (ck["net"]["inp.trinket.weight"].shape[0] if "inp.trinket.weight" in ck["net"]
             else ck["net"]["trinket.weight"].shape[0])
    if old_trinkets is None:
        old_trinkets = (model._infer_names(path, "trinkets", trows - 1) or (None,))[0]
    if old_trinkets is not None:
        translate_trinket_ids(obs, old_trinkets, model.current_names()["trinkets"])

    with torch.no_grad():
        old_logits, old_value = old(old_obs)
        new_logits, new_value = new(obs)
        old_prob = torch.softmax(old_logits.masked_fill(~mask, float("-inf")), -1)
        new_prob = torch.softmax(new_logits.masked_fill(~mask, float("-inf")), -1)
    d_logits = (old_logits - new_logits).abs().max().item()
    d_prob = (old_prob - new_prob).abs().max().item()
    d_value = (old_value - new_value).abs().max().item()
    print(f"logits {d_logits:.2e}, masked action probabilities {d_prob:.2e}, "
          f"value {d_value:.2e} on {len(obs)} observations")
    if max(d_logits, d_prob, d_value) >= 1e-5:
        raise SystemExit("the remapped network does not answer as the checkpoint did")


def main():
    a = argparse.ArgumentParser()
    a.add_argument("checkpoint", nargs="?")
    a.add_argument("--obs")
    a.add_argument("--write-obs")
    args = a.parse_args()
    if args.write_obs:
        write_obs(args.write_obs)
    if args.checkpoint:
        if not args.obs:
            raise SystemExit("--obs is needed to compare a checkpoint")
        check(args.checkpoint, args.obs)


if __name__ == "__main__":
    main()
