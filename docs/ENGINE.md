# How the engine and training fit together

A map of the code for whoever works on it next. The rules themselves are in
`DESIGN.md`, the card pool's reasoning in `docs/CONCEPT.md`.

## Layout

```
cards.toml            every card: minions, tokens, spells, golden behaviour
rules.toml            every rule number: gold, costs, pool copies, damage caps, limits
peak_boards.toml      hand-made peak boards for `bgbench peak` (guesses, see below)
crates/
  bgcore/             the rules: no I/O, no Python
    src/cards.rs      card data: TOML parsing into Card, Effect, triggers
    src/game.rs       one 8-player game: shop phase, actions, pairing, combat, knockouts
    src/fight.rs      the fight: a pure function of two boards, an RNG and counters
    src/minion.rs     Minion (Copy, fixed size), Keywords, Magnetic attachments
    src/action.rs     the flat action space and its index mapping
    src/obs.rs        the observation a player sees, as fixed-size f32
    src/bots.rs       random, standard-curve and rush-curve bots
    src/rules.rs      Rules from rules.toml
    src/rng.rs        xoshiro256++, written here so replays never change
    tests/            one file per tribe (beast_mech, dragons, murlocs, demons, neutrals),
                      deathrattles.rs (a loop over every Deathrattle card), fight.rs,
                      game.rs, d1a.rs; tests/data/ has test-only card files
  bgbench/            benchmarks, fuzzer, bot matches, the peak-board balance matrix
  bgpy/               PyO3 bindings: a batch of games stepped together
train/
  model.py            policy and value network
  ppo.py              PPO self-play, evals, checkpoints
  status.py           prints a run's metrics.csv as a table
ref/                  (git-ignored) Blizzard card data snapshots, see below
docs/                 concept, pool analysis, mechanics inventory (a snapshot from before
                      the five-tribe runs), this file
```

## Principles the code keeps

- **Effects are data.** Card text maps to `Effect` and trigger variants chosen
  in `cards.toml`. A variant can be specific to one card when the text is
  unique, but the rules code never matches card names.
- **No heap allocation in a fight or a shop action.** Boards, hands and shops
  are fixed-capacity (`arrayvec`). `Minion` is `Copy`.
- **Deterministic.** One seeded RNG per game; a seed plus the action list
  replays a game exactly (there is a test for it).
- **The fight is pure.** `fight_with` takes both boards and both players'
  counters and returns the outcome plus what carries over: permanent minion
  changes (Lurking Leviathan's improvement), counter changes (Beetle bonus),
  cards gained (Headhunter Gryphon, Rescue Bot). The game applies them after.
- **Pool accounting is exact.** `Game::check_invariants` holds pool copies +
  held copies + consumed copies (Magnetized into a host) equal to the
  starting copies. Generated cards (`pool_copies = 0`) never touch the pool.
  The fuzzer runs this after every action.

## Systems the five-tribe lobby added

What exists, so the next card reuses it instead of rebuilding it. The
rules behind each are in DESIGN.md Part 3.

- **Choices the player makes mid-action.** `PendingTarget` (a minion on
  the board or in the Tavern), `PendingShopTarget` (a Tavern slot),
  `PendingChooseOne`, and Discover (`DiscoverRequest`). While one is open,
  the only legal actions answer it.
- **Activate (N).** Pays gold, any number of times a turn, with or without
  a target.
- **Triggering in the shop.** Rally (Sky-hatch Runaway), Battlecries
  (Kelp Keeper, Young Murk-Eye; counted for Kalecgos, doubled by Brann),
  Deathrattles (Deathstrider after a Fishbait attack).
- **One count for Deathrattles.** `Side::deathrattle_fires` in fight.rs
  decides how many times a Deathrattle fires (Titus, golden Titus). Every
  path that fires one goes through it.
- **Counters.** Per minion: `improvements` (Leviathan, Hopebringer,
  Patient Scout, the "N left" cards); a triple keeps the highest of the
  three. Per player: `Player.improve` with `Counters.improve` inside a
  fight (Tasty Lobster), the Tavern spell bonus, the Beast bonus from a
  shop Goldrinn, the Fodder queue. Gains made inside a fight come back
  through `SideOutcome`.
- **The hand in combat.** `fight_with_hands` passes a side's hand only when
  something on that side reads it. Murlocs summoned from hand come back
  unchanged; Winterfinner's hand buffs are kept.
- **Refresh kinds.** `RefreshKind { TurnStart, Explicit, Effect }`: only an
  explicit Refresh (the player's action) takes queued Fodders.
- **Hero health.** `take_hero_damage` runs the "after your hero takes
  damage" reactions (rewinds included); effect damage in the shop can
  knock a hero out mid-turn; `can_pay_health` gates Health costs.
- **Tavern buffs** "this game" and "this turn", consuming Tavern minions,
  stealing them, Fishbait and Demon Fodder as generated Tavern cards.
- **Generated cards** (`pool_copies = 0`): tokens, Chromadrakes, Fodder,
  Fishbait. They never touch the pool.
- **Golden stats** that aren't double the base use `golden_stats`.
- **Legal actions.** `Game::legal_mask` only asks `is_legal` for slots that
  can be legal (a test checks it against `is_legal` everywhere). Use it
  rather than looping `is_legal` over every action.

## Adding a card

1. Copy its exact text, stats and tier from `ref/lobby5.tsv`, and the golden
   version from `ref/hsjson-cards.json` (a pool card's
   `battlegroundsPremiumDbfId` points at its golden card).
2. Express it in `cards.toml` with existing triggers and effects. If none
   fits, add a variant in `cards.rs` (parsing) and handle it where it
   fires (`game.rs` for the shop, `fight.rs` for combat).
3. Generated cards it makes (tokens, spells) also go in `cards.toml`, as
   tokens, with data from the JSON.
4. Test it with the real card in `crates/bgcore/tests/`.
5. `cargo test --workspace`, `cargo clippy --workspace --all-targets`,
   `cargo run --release -p bgbench -- fuzz 20000`.

`docs/mechanics-inventory.md` lists every mechanic the 5-tribe pool uses
and whether the engine has it (as of the D1a commit; runs after that fill
it in).

## Card data snapshots (`ref/`, git-ignored)

- `ref/hsjson-cards.json`: the full HearthstoneJSON card file
  (api.hearthstonejson.com/v1/latest/enUS/cards.json), fetched 2026-10-22.
  Has tokens and golden versions. Some retired cards are missing from it.
- `ref/bg-pool.tsv`: the solo Battlegrounds pool from it.
- `ref/lobby5.tsv`: our lobby, built from the JSON (the rule is in
  DESIGN.md Part 3). The JSON's pool flags aren't perfect: Pointy Arrow is
  a tier 1 pool spell though `isBattlegroundsPoolSpell` is false.
- Useful flags in the JSON: `isBattlegroundsPoolMinion`,
  `isBattlegroundsPoolSpell`, `isBattlegroundsDuosExclusive` (drop these),
  `battlegroundsAssociatedRaces` (the tribe a spell or neutral minion
  belongs to; drop it when that tribe isn't in the lobby), `techLevel`.

## Commands

```
cargo test --workspace
cargo clippy --workspace --all-targets
cargo build --release -p bgbench
./target/release/bgbench fight [n]      # ns per fight
./target/release/bgbench games [secs]   # games per second, one core and all cores
./target/release/bgbench fuzz [games]   # random play, invariants after every action
./target/release/bgbench match [games]  # bots against each other
./target/release/bgbench peak [n]       # peak boards of each build against each other
```

Python side, from the repo root:

```
VIRTUAL_ENV=$PWD/.venv PATH=$PWD/.venv/bin:$PATH maturin develop --release -m crates/bgpy/Cargo.toml
cd train && ../.venv/bin/python ppo.py --name NAME --minutes 480 --stop-flat 90
../.venv/bin/python status.py runs/NAME
```

The venv is Python 3.12 with torch (cu128) and maturin, made with uv.
Deleting `.venv` undoes it.

## Training notes

- One network plays every learner seat. Seats per game come from
  `--lineup` (learner, snapshot, standard bot, rush bot, random).
- Reward is placement only (+1 for 1st to -1 for 8th), minus 0.002 per
  action so it doesn't shuffle its board for free. Discounting is per
  turn, not per action; with per-action discounting nothing learned,
  because a turn can be 60 actions.
- The starting policy is uniform over action kinds, not over all actions,
  or it almost never ends a turn.
- Snapshots of itself are taken every 250 updates (at about 95 updates a
  minute, 10 was only 2 minutes of history).
- Every eval writes a row to `runs/NAME/metrics.csv`: placement against
  each bot, a curve report (turn reaching tiers 3, 4, 5; share of rush,
  standard, economy rush and slow games), a tribe report, entropy and the
  action mix.
- `--stop-flat N` stops a run after N minutes without gains, but only when
  it also no longer beats its oldest snapshot.
- Any change to the action space or observation breaks old checkpoints;
  start a new run.

## Known weak spots

- `peak_boards.toml` holds made-up boards, so `bgbench peak` means
  nothing yet. Planned: boards taken
  from build-forcing bots playing real games.
- The curve bots are simple yardsticks; beating them doesn't prove a curve
  is bad.
- The observation doesn't show the new tribes, Stealth, counters, pending
  choices or Holy Vanguard's conditional +30/+30 yet; that is the D4 step.
