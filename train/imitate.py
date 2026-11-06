"""Learn to copy Era's decisions (cloning step 3, docs/notes/training-plans.md
"Plan A").

`bgbench record` writes one example per step of Era's finished games into the
folder described in docs/briefs/clone-2-record.md: the observation the bot
would see, the legal mask and Era's action, plus one EndTurn example after
each turn. This script fits ppo.py's network to those actions (weighted
cross-entropy over the legal actions, weighted by Era's final place) and
saves a checkpoint ppo.py can start a normal run from:

    .venv/bin/python train/imitate.py --make-fake train/runs/fake-clone
    .venv/bin/python train/imitate.py --data train/runs/clone-data \\
        --out train/runs/clone-1
    .venv/bin/python train/ppo.py --name clone-1 --model attn \\
        --init-from train/runs/clone-1/best.pt

The network is built exactly as ppo.py builds it (model.py: model_args,
model_opts, build), so --model, --opp-boards and the --attn-* sizes mean the
same thing in both. Only the policy head is trained; the value head is left
as built (random), for PPO to learn from there.

Games, not examples, are split: about --holdout of them are held out with
--seed, so held-out accuracy measures play on games the network never saw.
An epoch is one pass over the training examples; after each one the held-out
top-1 and top-3 accuracy are measured, the best epoch is kept, and training
stops after --patience epochs without a better top-1 or at --max-epochs.

Output in --out: best.pt (model kind, model opts, card and hero names and
net, the keys ppo.py's --init-from reads) and log.tsv (one row per epoch:
epoch, train loss, held-out loss, held-out top-1, held-out top-3, minutes
since the start). The same epoch table, the best epoch and held-out accuracy
by step kind and by patch, and a baseline (always picking the most common
legal action kind) are printed and written to report.md.
"""

import argparse
import csv
import json
import time
from pathlib import Path

import numpy as np
import torch

import bgpy
from model import build, masked, model_args, model_opts

KINDS = [name for name, _ in bgpy.ACTION_KINDS]
KIND_OF = np.repeat(np.arange(len(KINDS)), [count for _, count in bgpy.ACTION_KINDS])
END_TURN = bgpy.END_TURN
# A line of meta.tsv, in the order `bgbench record` writes it.
META_COLS = ["i", "action", "game", "turn", "step", "kind", "place", "patch"]
# The patch values --make-fake cycles through (a recording's own come from
# the game logs; see docs/briefs/era-comps.md "The patch of each game").
FAKE_PATCHES = ["37.0", "37.0.2", "unknown"]


def args():
    a = argparse.ArgumentParser()
    a.add_argument("--data", default="train/runs/clone-data",
                   help="the folder bgbench record wrote: obs.f32, mask.u8, meta.tsv, header.json")
    a.add_argument("--out", default="train/runs/clone-1",
                   help="where best.pt, log.tsv and report.md go")
    a.add_argument("--make-fake", metavar="DIR",
                   help="write a fake recording in DIR (random observations and masks, ~20 games) "
                        "and exit, so this script can be tested before bgbench record exists")
    a.add_argument("--fake-games", type=int, default=20)
    a.add_argument("--fake-per-game", type=int, default=150)
    model_args(a, default="attn")
    a.add_argument("--holdout", type=float, default=0.1, help="share of games held out")
    a.add_argument("--seed", type=int, default=0, help="for the split and the shuffles")
    a.add_argument("--lr", type=float, default=3e-4)
    a.add_argument("--batch", type=int, default=256)
    a.add_argument("--max-epochs", type=int, default=50)
    a.add_argument("--patience", type=int, default=3,
                   help="stop after this many epochs without a better held-out top-1")
    a.add_argument("--device", help="torch device (default: cuda when there is one)")
    return a.parse_args()


def make_fake(folder, games=20, per_game=150, seed=7):
    """A fake recording in the format of docs/briefs/clone-2-record.md.

    Observations are random in [0, 0.5) (a real recording's numbers are
    mostly small, but random is enough to run every line of the script);
    masks are random with a 5% chance per action and always hold Era's
    action; Era's action itself is a random legal one, so nothing here can be
    learned and the accuracy on a fake folder only shows the plumbing works.
    meta.tsv has `games` games with every place 1 to 8 and all three patch
    values."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(seed)
    n = games * per_game
    obs = rng.random((n, bgpy.OBS_SIZE), np.float32) * 0.5
    mask = rng.random((n, bgpy.N_ACTIONS)) < 0.05
    mask[:, :bgpy.SHOP] = True
    mask[:, END_TURN] = True
    legal = [np.flatnonzero(row) for row in mask]
    action = np.array([row[rng.integers(len(row))] for row in legal], np.int64)
    mask[np.arange(n), action] = True
    kind = np.array([KINDS[KIND_OF[a]] for a in action])
    obs.tofile(folder / "obs.f32")
    mask.astype(np.uint8).tofile(folder / "mask.u8")
    lines = ["\t".join(META_COLS)]
    for i in range(n):
        game = i // per_game
        place = int(rng.integers(1, 9))
        patch = FAKE_PATCHES[game % len(FAKE_PATCHES)]
        lines.append("\t".join([str(i), str(int(action[i])), f"fake-g{game + 1}",
                                str(int(rng.integers(1, 13))), str(int(rng.integers(0, 9))),
                                str(kind[i]), str(place), patch]))
    (folder / "meta.tsv").write_text("\n".join(lines) + "\n")
    header = {"obs_size": bgpy.OBS_SIZE, "n_actions": bgpy.N_ACTIONS, "count": n,
              "games": games, "git": "fake", "skipped": {}}
    (folder / "header.json").write_text(json.dumps(header, indent=1) + "\n")
    print(f"wrote a fake recording in {folder}: {n} examples, {games} games, "
          f"{bgpy.OBS_SIZE} observations x {bgpy.N_ACTIONS} actions")


def read_meta(path, count):
    """meta.tsv as arrays, in the file's order. Refuses a file that doesn't
    hold `count` rows with the columns `bgbench record` promises."""
    with open(path, newline="") as f:
        rows = list(csv.DictReader(f, delimiter="\t"))
    if not rows or list(rows[0].keys()) != META_COLS:
        raise SystemExit(f"{path}: expected the columns {' '.join(META_COLS)}")
    if len(rows) != count:
        raise SystemExit(f"{path}: {len(rows)} rows, but header.json says {count} examples")
    meta = {c: [r[c] for r in rows] for c in META_COLS}
    if [int(i) for i in meta["i"]] != list(range(count)):
        raise SystemExit(f"{path}: the i column is not 0 to {count - 1} in order")
    return {
        "action": np.array([int(a) for a in meta["action"]], np.int64),
        "game": meta["game"],
        "turn": np.array([int(t) for t in meta["turn"]], np.int64),
        "kind": meta["kind"],
        "place": np.array([int(p) for p in meta["place"]], np.int64),
        "patch": meta["patch"],
    }


def load(folder):
    """obs, mask and meta of a recording, checked against this bgpy."""
    folder = Path(folder)
    if not (folder / "header.json").exists():
        raise SystemExit(f"{folder}: no header.json, so not a `bgbench record` folder "
                         f"(the default --data is train/runs/clone-data)")
    header = json.loads((folder / "header.json").read_text())
    missing = {"obs_size", "n_actions", "count"} - set(header)
    if missing:
        raise SystemExit(f"{folder}/header.json has no {', '.join(sorted(missing))}")
    if header.get("obs_size") != bgpy.OBS_SIZE or header.get("n_actions") != bgpy.N_ACTIONS:
        raise SystemExit(f"{folder}/header.json holds {header.get('obs_size')} observations and "
                         f"{header.get('n_actions')} actions, but this bgpy holds "
                         f"{bgpy.OBS_SIZE} and {bgpy.N_ACTIONS}: the data is from another build")
    count = int(header["count"])
    obs = np.fromfile(folder / "obs.f32", np.float32)
    mask = np.fromfile(folder / "mask.u8", np.uint8)
    if obs.size != count * bgpy.OBS_SIZE:
        raise SystemExit(f"{folder}/obs.f32 is {obs.size} floats, expected "
                         f"{count} x {bgpy.OBS_SIZE}")
    if mask.size != count * bgpy.N_ACTIONS:
        raise SystemExit(f"{folder}/mask.u8 is {mask.size} bytes, expected "
                         f"{count} x {bgpy.N_ACTIONS}")
    obs = obs.reshape(count, bgpy.OBS_SIZE)
    mask = mask.reshape(count, bgpy.N_ACTIONS) != 0
    meta = read_meta(folder / "meta.tsv", count)
    legal = mask[np.arange(count), meta["action"]]
    if not legal.all():
        bad = int((~legal).sum())
        print(f"skipping {bad} examples whose action is illegal in their own mask", flush=True)
        keep = legal
        obs, mask = obs[keep], mask[keep]
        for k in ("action", "turn", "place"):
            meta[k] = meta[k][keep]
        for k in ("game", "kind", "patch"):
            meta[k] = [v for v, ok in zip(meta[k], keep) if ok]
    return header, obs, mask, meta


def weights(place):
    """Era's decision: a 1st place counts 1.0, an 8th (9 - 8) / 8."""
    return (9.0 - np.clip(place, 1, 8)) / 8.0


def split_games(games, holdout, seed):
    """(train rows, held-out rows) with whole games on one side; about
    `holdout` of the games held out, at least one, never all of them."""
    uniq = np.array(sorted(set(games)))
    if len(uniq) < 2:
        raise SystemExit(f"need at least 2 games to hold any out, this recording has {len(uniq)}")
    rng = np.random.default_rng(seed)
    n_hold = min(max(1, int(round(len(uniq) * holdout))), len(uniq) - 1)
    held = set(uniq[rng.permutation(len(uniq))[:n_hold]])
    is_held = np.array([g in held for g in games])
    return np.flatnonzero(~is_held), np.flatnonzero(is_held), held


def policy_loss(logits, mask, action, w):
    """Weighted cross-entropy over the legal actions: the mean of -log p(Era's
    action), each example weighted by Era's place."""
    logp = torch.log_softmax(masked(logits, mask), 1)
    pick = logp.gather(1, action[:, None]).squeeze(1)
    return -(w * pick).sum() / w.sum().clamp(min=1e-9)


@torch.no_grad()
def evaluate(net, obs, mask, action, w, dev, batch=512):
    """Held-out loss (weighted like training), top-1 and top-3 accuracy (plain
    counts) and the predicted action of every example."""
    net.eval()
    pred = np.empty(len(action), np.int64)
    tot_loss, tot_w, top1, top3 = 0.0, 0.0, 0, 0
    for s in range(0, len(action), batch):
        sl = slice(s, s + batch)
        x = torch.from_numpy(obs[sl]).to(dev)
        m = torch.from_numpy(mask[sl]).to(dev)
        a = torch.from_numpy(action[sl]).to(dev)
        wt = torch.from_numpy(w[sl]).to(dev)
        logits = masked(net(x)[0], m)
        logp = torch.log_softmax(logits, 1)
        tot_loss += float(-(wt * logp.gather(1, a[:, None]).squeeze(1)).sum())
        tot_w += float(wt.sum())
        # how many logits beat Era's; illegal ones sit at -1e9, so a state with
        # fewer than three legal actions can still be a top-3 hit
        rank = (logits > logits.gather(1, a[:, None])).sum(1)
        top1 += int((rank == 0).sum())
        top3 += int((rank < 3).sum())
        pred[sl] = logits.argmax(1).cpu().numpy()
    net.train()
    n = len(action)
    return {"loss": tot_loss / max(tot_w, 1e-9), "top1": top1 / n, "top3": top3 / n, "pred": pred}


def first_legal(mask, cand):
    """Each row's first legal action of the kinds `cand` marks (falling back
    to its first legal action when it has none of them)."""
    cand = cand & mask
    return np.where(cand.any(1), cand.argmax(1), mask.argmax(1))


def baselines(mask_val, action_val, action_train):
    """Three bots with no network, for scale: (name, prediction, note).

    - the most common kind of action Era takes in the training games, picked
      as that kind's first legal action (the brief's baseline);
    - per state, the legal action kind with the most legal actions;
    - always the single most common action index in the training games
      (EndTurn, almost certainly)."""
    kind_count = np.bincount(KIND_OF[action_train], minlength=len(KINDS))
    common = int(kind_count.argmax())
    same_kind = KIND_OF[None, :] == common
    per_state = np.zeros((len(mask_val), len(KINDS)), np.int32)
    for k in range(len(KINDS)):
        per_state[:, k] = (mask_val & (KIND_OF[None, :] == k)).sum(1)
    index_count = np.bincount(action_train, minlength=bgpy.N_ACTIONS)
    common_action = int(index_count.argmax())
    return [
        (f"always {KINDS[common]} (the most common kind of Era's training actions)",
         first_legal(mask_val, same_kind)),
        ("always the legal kind with the most legal actions",
         first_legal(mask_val, KIND_OF[None, :] == per_state.argmax(1)[:, None])),
        (f"always action {common_action} ({KINDS[KIND_OF[common_action]]})",
         np.full(len(action_val), common_action, np.int64)),
    ]


def by_group(groups, hit):
    """[(group, count, accuracy)] over the examples, busiest group first."""
    order = sorted(set(groups))
    rows = [(g, sum(1 for x in groups if x == g),
             float(np.mean(hit[[i for i, x in enumerate(groups) if x == g]])))
            for g in order]
    return sorted(rows, key=lambda r: (-r[1], r[0]))


def main():
    cfg = args()
    if cfg.make_fake:
        make_fake(cfg.make_fake, cfg.fake_games, cfg.fake_per_game)
        return
    cfg.model = cfg.model or "attn"
    torch.manual_seed(cfg.seed)
    dev = torch.device(cfg.device or ("cuda" if torch.cuda.is_available() else "cpu"))
    data = Path(cfg.data)
    out = Path(cfg.out)
    out.mkdir(parents=True, exist_ok=True)
    t0 = time.time()

    header, obs, mask, meta = load(data)
    bad_place = int(((meta["place"] < 1) | (meta["place"] > 8)).sum())
    if bad_place:
        print(f"warning: {bad_place} examples have a place outside 1 to 8, "
              f"their weight is clamped to that range", flush=True)
    w_all = weights(meta["place"]).astype(np.float32)
    train_idx, val_idx, held = split_games(meta["game"], cfg.holdout, cfg.seed)
    if len(val_idx) == 0:
        raise SystemExit(f"no held-out examples: {len(set(meta['game']))} games and "
                         f"--holdout {cfg.holdout}")
    act_tr = meta["action"][train_idx]
    obs_va, mask_va = obs[val_idx], mask[val_idx]
    act_va, w_va = meta["action"][val_idx], w_all[val_idx]
    patches = {p: meta["patch"].count(p) for p in sorted(set(meta["patch"]))}
    print(f"{data}: {len(obs)} examples from {len(set(meta['game']))} games "
          f"(git {header.get('git')}), patches {patches}, skipped {header.get('skipped')}")
    print(f"split by game with seed {cfg.seed}: {len(train_idx)} training and {len(val_idx)} "
          f"held-out examples ({len(set(meta['game'])) - len(held)} and {len(held)} games)",
          flush=True)

    # The card and hero names ppo.py puts in its checkpoints, from the engine
    # itself, so --init-from can match embedding rows by name.
    env = bgpy.VecEnv(1, cfg.seed, [4, 2, 1, 1, 0, 0])
    card_names, hero_names = env.card_names(), env.hero_names()
    opts = model_opts(cfg.model, opp_boards=cfg.opp_boards, d=cfg.d, layers=cfg.layers,
                      heads=cfg.heads, ff=cfg.ff)
    net = build(cfg.model, len(card_names), **opts).to(dev)
    print(f"network: {cfg.model} {opts}, {sum(p.numel() for p in net.parameters())} parameters "
          f"on {dev}", flush=True)
    opt = torch.optim.Adam(net.parameters(), lr=cfg.lr)
    rng = np.random.default_rng(cfg.seed)

    rows, best_epoch, best_score = [], 0, None
    for epoch in range(1, cfg.max_epochs + 1):
        net.train()
        order = train_idx[rng.permutation(len(train_idx))]
        tot_loss, tot_w = 0.0, 0.0
        for s in range(0, len(order), cfg.batch):
            idx = order[s:s + cfg.batch]
            x = torch.from_numpy(obs[idx]).to(dev)
            m = torch.from_numpy(mask[idx]).to(dev)
            a = torch.from_numpy(meta["action"][idx]).to(dev)
            wt = torch.from_numpy(w_all[idx]).to(dev)
            loss = policy_loss(net(x)[0], m, a, wt)
            opt.zero_grad(set_to_none=True)
            loss.backward()
            opt.step()
            tot_loss += float(loss.detach()) * float(wt.sum())
            tot_w += float(wt.sum())
        ev = evaluate(net, obs_va, mask_va, act_va, w_va, dev)
        minutes = (time.time() - t0) / 60
        rows.append((epoch, tot_loss / max(tot_w, 1e-9), ev["loss"], ev["top1"], ev["top3"],
                     minutes))
        print(f"epoch {epoch:3d}  train {rows[-1][1]:.4f}  held-out {ev['loss']:.4f}  "
              f"top-1 {ev['top1']:.1%}  top-3 {ev['top3']:.1%}  {minutes:5.1f} min", flush=True)
        score = (ev["top1"], -ev["loss"])
        if best_score is None or score > best_score:
            best_score, best_epoch = score, epoch
            torch.save({"net": {k: v.detach().cpu() for k, v in net.state_dict().items()},
                        "model": cfg.model, "model_opts": opts,
                        "card_names": card_names, "hero_names": hero_names,
                        "epoch": epoch, "val_top1": ev["top1"], "val_top3": ev["top3"],
                        "val_loss": ev["loss"],
                        "imitate": {"data": str(data), "examples": len(obs),
                                    "train": len(train_idx), "holdout": len(val_idx),
                                    "games_held_out": len(held), "lr": cfg.lr,
                                    "batch": cfg.batch, "seed": cfg.seed}},
                       out / "best.pt")
        elif epoch - best_epoch >= cfg.patience:
            print(f"stopping: held-out top-1 has not improved for {epoch - best_epoch} epochs",
                  flush=True)
            break

    if best_epoch == 0:
        raise SystemExit("no epoch ran: --max-epochs must be at least 1")
    with open(out / "log.tsv", "w", newline="") as f:
        log = csv.writer(f, delimiter="\t")
        log.writerow(["epoch", "train_loss", "val_loss", "val_top1", "val_top3", "minutes"])
        for r in rows:
            log.writerow([r[0], f"{r[1]:.6f}", f"{r[2]:.6f}", f"{r[3]:.6f}", f"{r[4]:.6f}",
                          f"{r[5]:.2f}"])

    # The report is measured with what best.pt holds, read back from the file
    # so the checkpoint itself is checked on the way.
    net.load_state_dict(torch.load(out / "best.pt", map_location=dev)["net"])
    ev = evaluate(net, obs_va, mask_va, act_va, w_va, dev)
    hit1 = ev["pred"] == act_va
    kind_va = [meta["kind"][i] for i in val_idx]
    patch_va = [meta["patch"][i] for i in val_idx]
    bases = baselines(mask_va, act_va, act_tr)
    base_hits = [(name, float(np.mean(pred == act_va))) for name, pred in bases]
    by_kind = by_group(kind_va, hit1)
    by_patch = by_group(patch_va, hit1)

    lines = [
        f"# Imitation run: {data} -> {out}",
        "",
        f"- examples: {len(obs)} ({len(train_idx)} training, {len(val_idx)} held out)",
        f"- games: {len(set(meta['game']))}, {len(held)} held out (seed {cfg.seed}, "
        f"--holdout {cfg.holdout})",
        f"- patches: " + ", ".join(f"{p} {n}" for p, n in patches.items()),
        f"- network: {cfg.model} {opts}, {sum(p.numel() for p in net.parameters())} parameters",
        f"- training: Adam lr {cfg.lr}, batch {cfg.batch}, weight (9 - place) / 8, "
        f"cross-entropy over the legal actions, policy head only",
        f"- device: {dev}",
        "",
        "## Epochs (log.tsv)",
        "",
        "| epoch | train loss | held-out loss | top-1 | top-3 | minutes |",
        "|---|---|---|---|---|---|",
        *[f"| {r[0]} | {r[1]:.4f} | {r[2]:.4f} | {r[3]:.1%} | {r[4]:.1%} | {r[5]:.1f} |"
          for r in rows],
        "",
        "## Best epoch",
        "",
        f"- epoch {best_epoch}: held-out top-1 {ev['top1']:.1%}, top-3 {ev['top3']:.1%}, "
        f"loss {ev['loss']:.4f} ({len(val_idx)} examples on games it never saw)",
        "",
        "## Baseline",
        "",
        "- the brief's baseline, always picking the most common legal action kind: "
        f"{base_hits[0][1]:.1%}",
        *[f"- {name}: {acc:.1%}" for name, acc in base_hits[1:]],
        f"- so the network's {ev['top1']:.1%} is {ev['top1'] / max(base_hits[0][1], 1e-9):.1f}x "
        "the brief's baseline",
        "",
        "## Held-out top-1 accuracy by step kind",
        "",
        "| kind | examples | top-1 |",
        "|---|---|---|",
        *[f"| {k} | {n} | {a:.1%} |" for k, n, a in by_kind],
        "",
        "## Held-out top-1 accuracy by patch",
        "",
        "| patch | examples | top-1 |",
        "|---|---|---|",
        *[f"| {p} | {n} | {a:.1%} |" for p, n, a in by_patch],
        "",
        f"- minutes: {rows[-1][5]:.1f}",
    ]
    report = "\n".join(lines) + "\n"
    (out / "report.md").write_text(report)
    print(report)
    print(f"wrote {out}/best.pt, {out}/log.tsv and {out}/report.md")


if __name__ == "__main__":
    main()
