# How the engine and training fit together

A map of the code for whoever works on it next. The rules themselves are in
`DESIGN.md`.

## Layout

```
cards.toml            every card: minions, tokens, spells, golden behaviour
rules.toml            every rule number: gold, costs, pool copies, damage caps, limits
heroes.toml           the built heroes and their powers (see "Heroes")
peak_boards.toml      hand-made peak boards for `bgbench peak` (guesses, see below)
crates/
  bgcore/             the rules: no I/O, no Python
    src/cards.rs      card data: TOML parsing into Card, Effect, triggers
    src/heroes.rs     hero data: heroes.toml into Hero, HeroPower, PowerEffect
    src/game.rs       one 8-player game: shop phase, actions, pairing, combat, knockouts
    src/fight.rs      the fight: a pure function of two boards, an RNG and counters
    src/minion.rs     Minion (Copy, fixed size), Keywords, Magnetic attachments
    src/action.rs     the flat action space and its index mapping
    src/obs.rs        the observation a player sees, as fixed-size f32
    src/bots.rs       random, standard-curve and rush-curve bots
    src/rules.rs      Rules from rules.toml
    src/rng.rs        xoshiro256++, written here so replays never change
    src/record.rs     recorded sandbox games: TOML writer and the loader back into Minions
    tests/            one file per tribe (beast_mech, dragons, murlocs, demons, pirates, elementals, undead, quilboar, aberration, neutrals),
                      heroes.rs (the hero frame: pick, armor, the 14 powers, the two spells),
                      tier7.rs (tier 7 in the engine and its seven minions),
                      deathrattles.rs (a loop over every Deathrattle card),
                      lobby.rs (the 5-tribe lobby and every draw that respects it),
                      core_rules.rs (the generic rules on the real data), fight.rs,
                      game.rs; parser tests build small TOML strings inline
  bgbench/            benchmarks, fuzzer, bot matches, the peak-board balance matrix
  bgpy/               PyO3 bindings: a batch of games stepped together
  bgsandbox/          terminal sandbox: play shop phases alone, boards recorded to boards/
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
rules behind each are in DESIGN.md Part 2.

- **Choices the player makes mid-action.** `PendingTarget` (a minion on
  the board or in the Tavern), `PendingShopTarget` (a Tavern slot),
  `PendingChooseOne`, and Discover (`DiscoverRequest`). While one is open,
  the only legal actions answer it.
- **Activate (N).** Pays gold, once per turn per minion (`Minion.activated`,
  cleared at the start of the owner's turn), with or without a target.
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
  damage" reactions (rewinds included) for damage taken in the shop; combat
  damage just lowers Health and reacts to nothing; effect damage in the shop can
  knock a hero out mid-turn; `can_pay_health` gates Health costs.
- **Tavern buffs** "this game" and "this turn", consuming Tavern minions,
  stealing them, Fishbait and Demon Fodder as generated Tavern cards.
- **Generated cards** (`pool_copies = 0`): tokens, Chromadrakes, Fodder,
  Fishbait. They never touch the pool.
- **Golden stats** that aren't double the base use `golden_stats`.
- **Tier 7.** `CARD_MAX_TIER` (7) is the highest card tier, `MAX_TIER` (6)
  the highest Tavern tier: the Tavern, leveling and every "up to your
  tier" draw stop at 6. Tier 7 minions are in the pool (`copies_per_tier`
  has seven entries) and only an effect that names the tier makes one:
  `Game::give_tier7_minion` (generated, no pool copy) and
  `Game::discover_tier7_minion` (a Discover, the pick takes its copy) are
  the helpers, `CardDb::random_tier7_minion` the draw. The Pirate run's
  Greedy Conniver is the first user.
  The four tier 7 spells are in `cards.toml` but every spell draw caps at
  `MAX_TIER`, so none is offered. A generated spell (Lockbox) has tier 0.
- **Rally runs before the hit, and can kill.** `attack_swings` resolves the
  deaths a Rally caused before the attack lands; when the target died, the
  swing fizzles (no hit, no retaliation) but "after a friendly minion
  attacks" still fires. Obsidian Ravager is the first Rally that damages.
- **One pick per type.** `cards::pick_one_per_type` is the pick Misplaced
  Tea Set and The Last One Standing share: one random minion per type, each
  minion at most once.
- **Maximum Health in a fight.** `Fm.taken` counts the damage a minion's
  Health took, so its maximum is Health plus `taken` (Stalwart Kodo).
- **Magnetize hooks.** Every Magnetization goes through
  `Game::magnetize_to`, which also runs `beatboxers_copy` (Polarizing
  Beatboxer, `Aura::PolarizingBeatboxer`), so a new "whenever you Magnetize"
  card hooks in there.
- **Pirates, Golden minions, gold spent, Lockbox, Bounties, Cleave:** see
  `docs/ADDING-CARDS.md` section 6.
- **Undead** (the Undead bonus, Reborn in the shop, shop destroys, Knight
  count, Phantom chain): see `docs/ADDING-CARDS.md` section 8.
- **Quilboar** (Blood Gems and the gem value, Tavern spells versus plain
  spells, the Triple Reward spell, set-versus-add, discard, both effects,
  the Juggernaut Golem): see `docs/ADDING-CARDS.md` section 9.
- **Aberration** (the Deity, one discard path, one hand-insert hook, linked
  cards): see `docs/ADDING-CARDS.md` section 10.
- **Elementals** (refresh amount, free Refreshes, Baller count, Surprise
  triples, Slab, Prison, Wildfire): see `docs/ADDING-CARDS.md` section 7.
- **Legal actions.** `Game::legal_mask` only asks `is_legal` for slots that
  can be legal (a test checks it against `is_legal` everywhere). Use it
  rather than looping `is_legal` over every action.

## The lobby

Every game plays with exactly 5 of the 10 tribes (DESIGN.md "The lobby's
tribes"). `Game::unstarted` rolls them with the game's RNG (`Tribes::random_lobby`)
and stores them on `Game.lobby`; the same seed always rolls the same lobby. The
pool starts with copies only of lobby cards (`Card::lobby_ok`: a minion with a
lobby tribe, or a neutral or spell not tied to a tribe outside it; a neutral or
spell tied through cards.toml `lobby = "<tribe>"`, the game data's
`lobby_subsets`, is in only with that tribe; all-type minions are always in),
`check_invariants` checks the rest stay at 0, and every card draw takes the
lobby as its filter: the Tavern rolls, Discovers, "get a random X", random
spells, random tier minions, random Choose One cards, Bounties, Chromadrakes,
Volumizers, Robust Evolution and the draws inside a fight (the lobby travels
there in `Counters.lobby`; `Tribes::ALL` draws from everything, which is what a
fight without a game behind it uses). Nothing outside the lobby can appear, and
a draw with no in-lobby candidate returns None (the effect fizzles). The Deity
exists only when Aberration is in the lobby. A draw filter is just a `Tribes`
mask, so a future trinket or hero effect can pass a wider one for its own draw.

Tests and the sandbox can fix the lobby: `Game::new_with_lobby(data, seed,
lobby)` (a test may pass more than 5 to reach its cards) and
`Game::new_sandbox(data, seed, Some(lobby), free_rolls)`. The bots'
`preferred_tribe` counts only lobby tribes.

## Heroes

`heroes.toml` (DESIGN.md "Heroes") holds the built heroes: armor, extra
Health, the tribes a hero is tied to (offered only with one of them in the
lobby) and the power, as data (`PowerEffect` in heroes.rs: the engine never
matches a hero by name). Games with heroes come from `Game::new_heroes` (or
`new_heroes_with_lobby` / `new_sandbox_heroes`); `Game::new` and friends
build hero-less games so old tests pass unchanged, and
`Game::new_sandbox_with_hero` / `Game::assign_hero` force a hero.

- **The pick** runs at turn 0: `open_hero_picks` deals `hero_choices`
  (rules.toml, 2) offers per player without replacement (round by round, so
  a short hero table still feeds everyone), the players answer with
  `Action::Choose` in shuffled order (nothing else is legal), and turn 1
  starts once all have picked (`advance`). Player.hero_offer holds the offer.
- **Health and armor**: take_hero applies armor, extra Health and the power.
  Every hero damage takes armor first (`take_hero_damage`, the combat damage
  in `combat`, Health costs via `can_pay_health`); a rewind gives back what
  was taken, armor as armor (`last_damage_armor`). Armor Stash sets armor.
- **The power**: `Player.power` is a `PowerState` (whose power, uses left
  this turn, used this game, unlocked, one progress counter) reset each turn
  in `start_turn`. `Action::HeroPower` presses a button power
  (`Game::power_usable` is the legality, `press_power` pays and runs it);
  targeted powers open the pending-target flow like spells (`offer_target`).
  Passives hook into the existing triggers: FreeRefreshPerTurn and the
  uses/unlock reset in `start_turn`, GoldAfterUpgrade in `Action::Level`,
  GoldAfterBuyTribe in `Action::Buy`, GoldNextTurnAfterSell in `sell_minion`,
  SocLeftmost through `Counters.soc_leftmost` at the top of
  `run_start_of_combat` (before any minion's).
- **Discovering a hero power** (Unmasked Identity, Finley's Adventure!):
  `discover_hero_power(p, excludes)`, one engine function taking the source's
  exclusion list (hero card_ids, in cards.toml for the spell, on the hero for
  Finley). Offers are `Player.power_offer`, answered with `Choose`; the pick
  replaces the power with a fresh `PowerState`. Finley's runs right after his
  pick, before turn 1. Room for trinkets and Dark Gifts sits on Player
  (`trinkets`, `dark_gifts`), unused.
- Bots answer the pick and a power Discover in `answer_pending`, and press a
  legal button with gold that would be left over (Gonna Be Rich! waits for
  turn 8). `bgbench` and `bgpy` games play with heroes.

## The sandbox

`Game::new_sandbox(data, seed, lobby, free_rolls)` is a one-player game for
playing a build and recording its boards: only player 0 lives, there is no
pairing, and each turn ends in `sandbox_combat`: the player fights a copy of
their own board, hand and counters with the real fight, only the player's side
outcome is applied, and Health never changes in combat. Shop damage still
counts and ends the game at 0, and the game ends at the turn limit. The latest
fight (board and hand as they fought, result, side outcome) is in
`Game.sandbox.last_fight`. Normal games never set `sandbox`.

With `lobby: None` the game rolls its 5 tribes from the seed like any game.
With `Some(lobby)` the lobby is fixed: the screen shows it, and the pool is
exactly that lobby's cards (one tribe plays like a rigged Tavern used to: only
that tribe and neutral minions show up). There are no free Refreshes unless
`free_rolls` says so.

```
cargo run --release -p bgsandbox                            # a rolled 5-tribe lobby
cargo run --release -p bgsandbox -- --tribe murloc          # a one-tribe lobby
cargo run --release -p bgsandbox -- --tribe beast,mech,dragon,murloc,demon
```

Flags: `--tribe A[,B,...]` (1 to 5 distinct tribes: the lobby), `--hero
NAME` (play that built hero instead of picking from the offer), `--label
TEXT`, `--seed N` (random and printed by default), `--free-rolls N` (default
0), `--boards DIR`. The screen shows your hero, armor and power; `hp` presses
the button. Commands are read from stdin (`help` lists them). Every
end of turn rewrites the recording from the game's history, so `undo` (which
replays the seed and all actions but the last) also removes an undone turn
from the file.

The label is the build you played. Give `--label`, or type `label <text>` at
any time, or answer the question at the end (quit, hero death, turn limit;
a terminal only: with piped input it stays `unlabeled`). During the game the
file is `boards/game-<seed>.toml`; at the end it is renamed to
`boards/<label>-<seed>.toml`, or `boards/<tribes>-<label>-<seed>.toml` with a
fixed lobby (the tribe names joined by dashes). An existing file is never
overwritten, a suffix `-2`, `-3` is added. The header holds `rigged`, the
`tribe` when fixed, the label, seed, `free_rolls` and date. `bgcore::record::load_file` reads a file back into
per-turn boards and hands of `Minion`s. Each turn holds the board that went
into the mirror fight (after end-of-turn effects, before the fight's permanent
gains). Card text on the `?` command comes from `ref/lobby5.tsv` when it
exists.

## Adding a card

See `docs/ADDING-CARDS.md` (hook index, recipes, new-tribe checklist, test helpers, gotchas).

## Card data snapshots (`ref/`, git-ignored)

- `ref/hsjson-cards.json`: the full HearthstoneJSON card file
  (api.hearthstonejson.com/v1/latest/enUS/cards.json), fetched 2026-10-22.
  Has tokens and golden versions. Some retired cards are missing from it.
- `ref/bg-pool.tsv`: the solo Battlegrounds pool from it.
- `ref/lobby5.tsv`: our lobby, built from the JSON (the rule is in
  DESIGN.md Part 2). The JSON's pool flags aren't perfect (Rot Hide Gnoll);
  `docs/card-data.md` and `tools/export_cards.py` read the game's own.
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
cargo run --release -p bgsandbox        # the sandbox, boards go to boards/
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
  start a new run. The 2026-10-25 observation rewrite (all ten tribes, the
  lobby, the Deity, Blood Gems, attachments, and opponents' Deity, gem value
  and spell power as last seen; OBS_SIZE 2658 to 6362) broke every earlier
  checkpoint, expected. The heroes run the same day added own and opponents'
  heroes (a hero embedding beside the card one), armor, the power state, the
  hero offer during the pick and the `hero_power` action kind (OBS_SIZE to
  6389, SCALARS 173, OPP_SCALARS 30): every checkpoint before it breaks too.
- Games from `bgpy` play with heroes (`Game::new_heroes`), pick included;
  learner seats answer the pick and any power Discover with `choose`.
- `--model` picks the network: `mlp` (default, 3.1M parameters, all slots
  concatenated), `attn` (a 3-layer transformer over the card slots, 0.65M)
  or `pool` (the same slot tokens pooled per zone, 0.76M). The last two
  score per-slot actions from the token of the slot they act on.
  `--opp-boards` sets how `attn` sees opponent boards: `full` (every
  opponent minion a token, 85 tokens), `pooled` (each board pooled into
  its opponent's token, 36) or `next` (pooled, plus the next opponent's
  board in full, 43; the default). Both are saved in the checkpoint and
  `--resume` builds what the checkpoint holds. `attn` runs its layers in
  bf16, recomputes them in the backward pass to fit a minibatch of 8192,
  and is compiled with torch.compile unless `--no-compile`. It does about
  20 updates a minute with `next` (10 with `full`) against `mlp`'s 70,
  `pool` about 35. `python model.py` checks that the per-slot logits
  follow their slots.
- `--init-from PATH` starts a new run (fresh optimizer) from every
  parameter of another checkpoint of the same model whose name and shape
  match, for example `attn` with `full` into `next`; it prints what was
  fresh. `--warmup N` ramps the learning rate up over N updates.

## Agent briefs and references

Tribe runs are done by subagents from briefs in `docs/briefs/` (one file
per run: house rules, the cards with their text, the owner's rules, tests,
done-means). Write the brief only after the owner has answered the unsure
rules; DESIGN.md Part 2 holds the answers.

`~/projects/bg/bg/engine.py` is the owner's tracker fight simulator in
Python, with combat effects for about 325 cards of every tribe, much of it
checked by hand against real games. It's the best reference for how a
card fights. DESIGN.md wins where they disagree (for example, a per-type
pick gives each minion at most one buff, unlike its `_phalanx`).

## Model comparison (night 2026-10-23)

150 minutes each, same settings. Place vs 7 bots, standard / rush:
mlp 2.01 / 1.98 (934k games), pool 2.38 / 2.12 (409k), attn with
`--opp-boards next` 3.30 / 2.86 (228k, its last minutes shared the GPU).
Per game the three are close; attn levels fastest. mlp wins per minute
because it's about 4x faster. None learned standard curves or tribe
boards: the opponents and the reward are the bottleneck, not the network.
mlp stays the default until a head-to-head says otherwise.

## Known weak spots

- `peak_boards.toml` holds made-up boards, so `bgbench peak` means
  nothing yet. Planned: boards taken
  from build-forcing bots playing real games.
- The curve bots are simple yardsticks; beating them doesn't prove a curve
  is bad. They answer every pending choice (falling back to the first legal
  action), use Activates (the ones that add stats or cards before rolling,
  the rest with leftover gold), keep 10 Health when paying with Health,
  roll for queued Fodders when a Demon is on the board, and pick targets
  by the effect (the weakest minion for Channel the Devourer and Robust
  Evolution, the biggest gain for Tyrael's 50/50). They never reorder
  the board.
- The observation covers every tribe and the heroes (layout at the top of
  `obs.rs`): own and opponents' heroes and armor, the power state,
  the hero offer during the pick, the lobby's five tribes as a multi-hot,
  ten-wide tribe multi-hots, every
  keyword, per-minion counters (Blood Gem stats, attachments, links,
  Ichoron's one-turn shield, doomed picks), player counters (gem value,
  Undead bonus, Knight deaths, discards, spells cast, ...), own and
  as-last-seen opponent Deities, and opponents' gem value and spell power
  from the last fight with them, plus "fight stats" (Holy Vanguard's +30/+30
  at 15 Health or less, the shop Goldrinn bonus a Beast in hand gets when
  played). Relentless Deflector's Taunt while shielded isn't a feature; the
  card embedding has to learn it.
