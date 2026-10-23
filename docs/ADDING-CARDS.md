# Adding cards and tribes

A working reference for whoever adds the next tribe. Rules are in
`DESIGN.md` Part 3 (they win over everything here), the code map and
principles in `docs/ENGINE.md`. Function names, no line numbers: grep them.

Files: `cards.toml` (data), `crates/bgcore/src/cards.rs` (parsing, `Effect` and
trigger enums), `game.rs` (shop phase), `fight.rs` (combat), `minion.rs`.

## Workflow

1. Card text, stats, tier: `ref/lobby5.tsv`; golden card: `ref/hsjson-cards.json`
   (`battlegroundsPremiumDbfId` is the golden's `dbfId`; script with `python3 -I`).
   Pool cards: `isBattlegroundsPoolMinion` / `isBattlegroundsPoolSpell`, drop
   `isBattlegroundsDuosExclusive`. Golden stats not double the base: `golden_stats`.
2. Express it in `cards.toml` with an existing field (index below). Only if none
   fits: add a field to `RawCard`/`RawEffect` and the enum in `cards.rs`, parse it in
   `CardDb::from_toml`, handle it where it fires.
3. Generated cards (tokens, spells it makes) go in `cards.toml` too, `[[token]]` or
   `[[spell]] token = true`, never in the pool (`pool_copies = 0`).
4. Test with the real card (section 4), then `cargo test --workspace`,
   `cargo clippy --workspace --all-targets`, `cargo build --release -p bgbench`,
   `./target/release/bgbench fuzz 20000`, `./target/release/bgbench games 10`.
5. Never `cargo fmt` (the repo is not rustfmt-formatted).

The owner's tracker sim `~/projects/bg/bg/engine.py` is a good reference for how a
card fights (read only); DESIGN.md wins on disagreement.

## 1. Hook index

Every card is `Card` in cards.rs. "Parts" means the minion plus every Magnetized
card inside it (`Minion::parts()`); effects always iterate parts and a golden part
doubles per its own golden flag. `run_effect` exists twice: `Game::run_effect`
(shop, takes `pos`, `golden`, `runs`) and `Side::run_effect` (fight). An `Effect`
variant that a fight can run must be in `Effect::runs_in_fights`; the parser rejects
Rally/Deathrattle/Start of Combat/Avenge/etc. effects that are not.

| Kind | cards.toml field | Fires in | Example |
|---|---|---|---|
| Battlecry | `battlecry = {..}` | `Game::battlecry_or_choose_one` (via Play), Brann's `brann_mult` runs, `fire_after_battlecry` after | Southsea Busker |
| Choose One | `choose_one = [a, b]` | `battlecry_or_choose_one` -> `PendingChooseOne` -> `resolve_choose` | Intrepid Botanist |
| Deathrattle | `deathrattle = {..}` | `Side::fire_deathrattles` (count via `deathrattle_fires`, Titus), called by `Side::resolve` | Locked-up Mutineer |
| Rally | `rally = {..}` | `attack_triggers` in fight.rs (before the hit); shop copy via `Effect::TriggerRally` | Bigwig Bandit |
| Start of Combat | `start_of_combat = {..}` | `run_start_of_combat` | Diremuck Forager |
| End of turn | `end_of_turn = {..}` | `Game::run_end_of_turn` (Drakkari loops it), called by `finish_shop` | Surfing Sylvar |
| Start of turn | `start_of_turn = { gold }` | `Game::start_turn` | (grep start_of_turn) |
| Avenge (N) | `avenge = { every, effect }` | `Side::avenge_tick`, called by `resolve` | Relentless Deflector |
| Before / after a friendly attack | `whenever_friendly_attack`, `after_friendly_attack` (`attacker_tribe`, `attacker_rally`, `attacker_another`) | `attack_triggers` (after=false/true) from `attack_swings` | Roaring Recruiter |
| Cleave | `cleave = true` | `attack_swings` + `cleave_targets` | Blade Collector |
| Damage dealt milestone | `damage_milestone = { total, effect }` | `Side::count_damage` | Treasure Parrot |
| Takes damage | `when_damaged = {..}` | `Side::when_damaged` | Very Hungry Winterfinner |
| Summon in combat | `Effect::Summon`, `Deathrattle::Summon` | `Side::summon` (7 limit, Watch hook, summon triggers) | Cord Puller |
| After you summon (combat) | `after_summon_give_stats = { times }`, `on_summon_beast` | `Side::summon` | Stalwart Kodo |
| Summon from hand | `summon_from_hand = "murloc"` | `Side::summon_from_hand`; fight needs the hand (`Card::reads_hand`) | Expert Aviator |
| Magnetize / Mech played | `on_play_mech`, `on_play_or_magnetize_mech`, `on_cast_spell_on_mech`, `aura = polarizing_beatboxer` | `magnetize_to`, `trigger_mech_played_or_magnetized`, `beatboxers_copy`, `Minion::magnetize` | Spark Snapper |
| Cast a Tavern spell | `on_cast_spell`, `on_cast_spell_on_self`, `on_cast_spell_on_tribe` | `trigger_on_cast_spell`, `trigger_on_cast_spell_on_target`; one call per cast (a Bounty cast twice calls it twice) | Timecap'n Hooktail |
| Spell cast event counter | `on_event = { event = "spells_cast" }` | `cast_spell` -> `fire_event(SpellsCast)` | Felboar |
| Gold spent / hero damage / sold / bought / Elementals played | `on_event = { event = "gold_spent" / "hero_damage" / "minions_sold" / "cards_bought" / "elementals_played", every, effect }` | `Game::fire_event` (count in `improvements`, remainder kept, can fire several times) | Gunpowder Courier |
| Sell | `on_sell = {..}` | `Game::sell_minion` (`OnSell`) | Greedy Conniver |
| Buy | `Action::Buy`/`BuySpell` in `apply`; triple via `check_triple` | `apply` | none yet |
| Play a minion of a tribe | `after_play_tribe = { tribe, effects }` | `fire_after_play_tribe` | Wrath Weaver |
| While in hand, after a tribe is played | `while_in_hand` | `trigger_in_hand_on_play` | Bream Counter |
| After you trigger a Battlecry | `after_battlecry` | `fire_after_battlecry` | Kalecgos |
| Discover (the effect) | `discover_spell`, `discover_tribe_if_another`, `Effect::Discover*`, spell `discover = ..` | `discover_queue` of `DiscoverRequest`, `maybe_start_discover`, `resolve_choose` | Rodeo Performer |
| After you Discover | `after_discover = {..}` | `fire_after_discover` (end of `resolve_choose`) | Hooktusk |
| Hero takes damage | `after_hero_damage = [..]`, `DamageHero` | `take_hero_damage`, `take_shop_damage` (can kill), `can_pay_health` | Wrath Weaver |
| Refresh | `health_refreshes`, `RefreshKind` | `refresh_shop`, `fill_shop`, Roll in `apply` | Malchezaar |
| After the Tavern is Refreshed | `refresh_buff = [a, h]` (effect and spell effect) | `Player.refresh_buff`, `apply_refresh_buff` (end of `refresh_shop`, `refresh_with_spells`, `refresh_with_tribe`); a fight adds through `SideOutcome::refresh_buff_gain` | Waveling |
| Free Refreshes | `free_refreshes = n` | `Player.free_rolls`, used first by Roll | Refreshing Anomaly |
| Sell, improving a shared count | `on_sell = { effect = { improving_counter = "baller", buff } }` | `Effect::BuffAllImproving`, `Counter::Baller` | Fire Baller |
| Played minion of a tribe gets Divine Shield | `play_shield = "elemental"` | `give_play_shield` (right after the board insert, before the Battlecry), `Minion.temp_shield`, cleared in `start_turn` | Ichoron the Protector |
| After you buy a minion | `after_buy = { buff, multiply, golden_multiply }` | `after_buy_effects` in `Action::Buy`, before `check_triple`; uses `Minion.activated` as the once-per-turn flag | Stone Age Slab |
| Arm for the next buy | `activate = { effect = { arm_next_buy = true } }` | `Effect::ArmNextBuy` (armed in `improvements`), `after_buy_effects`, cleared in `finish_shop` | Living Prison |
| Triple wildcard | `triple_wildcard = "elemental"` | `check_triple_tracking` (`TriplePlan`) | Elemental of Surprise |
| Excess damage on a kill | `overkill = true` | `attack_swings` in fight.rs (golden hits both neighbours) | Wildfire Elemental |
| Spell target that reads the Tavern or this turn | `half_of_highest`, `per_played_this_turn` | `SpellTarget::HalfOfHighestTavern`, `BuffPerTribePlayed`, `Player.elementals_played_turn` | Arcane Absorption |
| After a friendly minion is Reborn | `on_reborn = { banshee = [a, h] }` / `{ phantom = true }` | `Side::note_reborn`, called by `Side::resolve` (fights and shop destroys) | Barrier Banshee |
| Destroy a friendly minion in the shop | `Effect::DestroyChosenDiscover`, `GiveRebornDestroy`, `SpellTarget::Butchering` | `Game::destroy_in_shop` -> `fight::shop_destroy` | Maw Caster |
| Player bonus "wherever they are" | `undead_bonus = [a, h]` effect, `on_cast_spell = { undead_bonus }`, `buff_per_kind_death` | `Game::add_undead_bonus`, `add_knight_deaths`, `Player::make_card`, `Side::summon` | Nerubian Deathswarmer |
| Activate (N) | `activate = { cost, effect }` | `Action::Activate` in `apply`; legality in `is_legal` (needs_* checks on `Effect`) | Suspicious Prisonguard |
| Golden played from hand | `buff_per_golden_played`, `after_discover` scaling | `Action::Play` -> `add_golden_played`, `Player.golden_played` | Maritime Extortionist |
| Tavern buff (this game / this turn) | `Effect::TavernBuff`, `tavern_buff` | `Player::add_tavern_buff`, `apply_tavern_buffs`, `end_turn_tavern_buffs` | Champion of Sargeras |
| Consume Tavern minion | `consume = "chosen"/"random"/"highest_health"` | `start_consume`, `consume`, `PendingShopTarget` | Mind Muck |
| Aura (counts, not stats) | `aura = "brann"/"titus"/"drakkari"/"balinda"/"proud_privateer"/..` | `brann_mult`, `drakkari_mult`, `balinda_mult`, `bounty_mult`, `Side::deathrattle_fires` | Proud Privateer |
| Per-game player counter | `buff_per_deathrattle`, `improve_counter`, `spell_bonus`, volumizer/beetle bonus | fields on `Player` (`counters()` hands them to fights) | Tasty Lobster |
| Per-minion counter | `improvements` (u16) | tier of Patient Scout, event counts, damage milestones, Lockbox turns | Patient Scout |
| Stat threshold | `attack_threshold` | `check_attack_thresholds`, `Side::gain_at` | Scarlet Survivor |
| Keep combat gains | `permanent_keep = "self"/"adjacent_dragons"`, `SideOutcome::permanent_*` | `Side::gain_at`, `apply_side_outcome` | Tarecgosa |
| Cards gained in a fight | `get = "X"`, `random_*` effects | `SideOutcome.cards`, `apply_side_outcome` -> `give_generated` | Headhunter Gryphon |
| Spell effect (no target) | `[[spell]] effect = { gold = 2 }` etc | `cast_spell_inner` -> `execute_spell_effect` (`SpellEffect`) | Tavern Coin |
| Spell effect (needs a minion) | `effect = { target = "chosen", .. }` | `cast_spell_inner` -> `PendingTarget` -> `apply_target` (`SpellTarget`) | Tavern Dish Banana |

Choices the player makes mid-action (`PendingTarget`, `PendingShopTarget`,
`PendingChooseOne`, Discover): an `Effect` that asks must be in `Effect::needs_choice`;
event triggers, end of turn, after_battlecry, after_discover and `after_hero_damage`
effects cannot ask (the parser refuses). `offer_target` auto-picks when there is
exactly one legal target and drops the effect when there is none.

## 2. Recipes (copy this card)

| To build | Copy | Notes |
|---|---|---|
| Battlecry: buff one friendly minion (of a tribe) | Forest Rover / Tinkerer (`target = "chosen"`, `tribe`) | cannot pick itself |
| Buff all / others / self / random | `target = "all"/"others"/"self"/"random"` | `fixed_stats` minions are skipped |
| Buff one minion of each type (one buff per minion) | The Last One Standing (`buff_each_type`), Misplaced Tea Set | `cards::pick_one_per_type`; all-type minions are every tribe |
| Buff scaled by a counter that improves | Tasty Lobster (`improve_counter`) | player counter `Counter`, kept through fights via `improve_gain` |
| Per-minion "N left" counter | Gunpowder Courier (`on_event`), Treasure Parrot (`damage_milestone`) | triple keeps the highest count |
| Stat per thing counted game-long | Falling Sky Golem (`buff_per_deathrattle`), Maritime Extortionist | wherever-it-is: update shop, hand, board (`add_golden_played`, `add_deathrattles_triggered`) |
| Scale by gold spent this turn | Lovesick Balladist (`per_gold_spent`) | `Player.gold_spent_turn` |
| Get a generated card | Weapons Forge (`get`), Chromadrake effects | no pool copy, hand slot needed, `give_generated` |
| Get a random pool minion (generated) | `random_minion`, Highkeeper Ra (`random_tier_minion`) | `CardDb::random_pool_minion`, `random_tier_minion` |
| Get a pool minion that takes a copy | Discover (`DiscoverRequest`), Silent Deliverer (`draw_golden`) | pool copies come out of `Game.pool`, go back on sell |
| Discover | Rodeo Performer (spell), Patient Scout (minion by tier), `discover_tier7_minion` | queue + `maybe_start_discover`; unpicked minions go back |
| Tier 7 draw | Greedy Conniver | `give_tier7_minion`, `discover_tier7_minion` |
| Golden: doubles numbers | any card, `mul` in `run_effect` | "twice" (fixed count, max over copies): Brann/Balinda/Proud Privateer. "An extra time" (stacks): Titus |
| Golden count differs from x2 | golden text says "twice" vs doubled stats | read the golden card; `golden_doubles_count` for summons, `golden_stats` for stats |
| Make a minion golden on the board | Elite Navigator (`make_golden_max_tier`) | no reward, not counted as played; `golden_up` |
| Golden played gives no reward | Aureate Laureate (`always_golden`, `no_triple`), `Minion.no_reward` | |
| Summon in combat | Cord Puller / Den Mother style (`summon`) | `golden_doubles_count` for Beetle style |
| Summon a copy / from hand | Flighty Scout, Expert Aviator | fights need the hand passed (`reads_hand`) |
| Deal damage in combat | Obsidian Ravager (`damage_target_and_neighbor`), Blade Collector (cleave) | `rally_damage`, `hit`; Divine Shield/Venomous honoured |
| Keep stats after the fight | Tarecgosa (`permanent_keep`), Eudora (`keep_golden`), Persistent Poet | `SideOutcome.permanent_stats` by start slot |
| Keep a counter after the fight | Lurking Leviathan, Treasure Parrot | `permanent_improvements` |
| Cast a named spell | Gatekeeper Amalgam (`cast`) | counts as a real cast |
| Trigger another card's Battlecry/Rally | Kelp Keeper, Sky-hatch Runaway | `TriggerBattlecry`, `TriggerRally` |
| Spells that share a pool slot | Volumizers (`volumizer_buff`), Bounties (`bounty = true`) | `CardDb.bounties`, only the first is in `spells` |
| Unplayable held card with a countdown | Lockbox (`lockbox = 5`) | counter in `improvements`, `tick_lockboxes` |
| Pay Health instead of Gold | Hasty Excavation (`pay_health`), Malchezaar | `can_pay_health`; Health payments never count as gold |
| Dual / all tribes | Treasure Parrot (`tribes = [..]`), `tribe = "all"` | `Tribes` bit set; every filter asks `has` |
| Effect only while in hand | Bream Counter, Choral Mrrrglr | |
| Stats that the Tavern can't change | Fishbait (`fixed_stats`) | |

## 3. New-tribe checklist

1. `cards.rs`: add the variant to `Tribe`, add it to `Tribe::EVERY` (rules use this) and
   `tribe()` parse, and bump the length in `EVERY: [Tribe; N]`; widen `Tribes::ALL`
   ("all types") by one bit (`u8` mask: at most 8 tribes, and Undead is the eighth, so
   the mask is full: a ninth tribe needs `Tribes(u16)` and `Tribe::bit`). `Tribe::ALL` stays the five-tribe list the observation uses until the
   observation run: do not touch it before then.
2. Tribe loops that must use `EVERY`: `Tribes::random_type`, `pick_one_per_type`,
   `bots::preferred_tribe`, sandbox `screen.rs` (`tribe_name` match, `parse_tribe`,
   `card_tribes`), Planar Telescope and Friendly Bounty counts in `game.rs`.
   grep `Tribe::ALL` and `Tribe::EVERY` to check nothing else needs it.
3. Tests that use an unknown tribe as the bad value: `tests/beast_mech.rs`
   (`attacker_tribe = "..."`) and `tests/d1a.rs` (`a_card_needs_exactly_one_of_tribe_and_tribes`)
   use a tribe that does not exist ("quilboar" now; "pirate" and "elemental" are real).
   Also raise `MAX_TAVERN_BUFFS` in game.rs (one Tavern buff per tribe filter and
   duration, so 2 x (tribes + 1), 18 with eight tribes); a push past it panics.
4. `tests/tier7.rs` Last One Standing test counts picks per type: an all-type minion is
   every tribe, so a new tribe changes its expected count. Re-check it. An all-type
   minion is also a member of the new tribe for every "your <tribe>" rule (Amalgam,
   Par-tea Guest and The Last One Standing count as Undead).
   `tests/deathrattles.rs` puts Titus next to the Deathrattle card under test: a card
   whose Start of Combat destroys its neighbours (Stitched Salvager) needs a filler
   between them (the test does that for any card with a Start of Combat).
5. Observation: layout frozen (`obs.rs` header, `TRIBES = 6` slots, `final_tribes`,
   `board_tribes`, bgpy constants). Do not add slots in a tribe run. `N_CARDS` in bgpy
   is `cards.len()` and grows with every card: old checkpoints stop loading, expected.
   New `TargetKind` variants need an arm in `obs::target_kind` (share a slot).
   New `Effect`/`SpellEffect` variants do not need obs changes unless a match is exhaustive.
6. `cards.toml`: pool minions go in the `[[minion]]` section (before tokens; ids
   `0..pool_cards` are the pool). Max 256 pool minions (`MAX_POOL_CARDS`; draws store
   ids as `u8`). Dual-tribe cards of older tribes get `tribes = [..]` edited in place.
7. `rules.toml`: pool copies per tier are by tier (`copies_per_tier`); nothing per
   tribe. Bonus keywords list is global. No change for a new tribe.
8. Pool slots: cards that share one pool slot set `pool_slot` in `from_toml`
   (Volumizers). A new group needs the same handling in `draw_filtered`.
9. `record.rs`: a new `Minion` field needs an `Option` in `FileMinion` plus the
   to/from conversions (done for `no_reward`).
10. Sandbox: `--tribe X` rigs the Tavern and every "any minion" draw
    (`Card::rig_ok`); new draw helpers must take `rig` (`draw_rigged`, `pick_rigged`,
    `draw_golden`). Update the usage string in `bgsandbox/src/main.rs`.
11. New `Effect` variants: add to `needs_choice` / `needs_any_minion` / `runs_in_fights`
    as fits, to the shop `Game::run_effect` (put fight-only ones in its last arm), to
    `Side::run_effect` if it runs in fights (it ends with `_ => {}`).
12. Update `docs/ENGINE.md` where your work makes it wrong, and the tests list there.
13. Do not edit `DESIGN.md`, `CLAUDETODO.md`, `TODO.md` unless asked; note unsure rules
    in your report (they go to `docs/UNSURE.md` by the coordinator).

## 4. Test helpers

One file per tribe in `crates/bgcore/tests/`; each defines its own small helpers
(copy the block from `tests/pirates.rs` or `tests/tier7.rs`, they are the newest).

| Need | Helper (copy from) |
|---|---|
| Card data | `Data::bundled().cards` (`db()`), `Data::bundled()` |
| A game | `Game::new(Arc::new(Data::bundled()), seed)` (`game(seed)`); one player sandbox: `Game::new_sandbox(data, seed, Some(tribe), free_rolls)` |
| A minion value | `db.make(db.by_name("X").unwrap(), golden)` (`mk`) |
| Board / hand from the pool | `put_on_board(g, p, "X", golden)`, `put_in_hand(...)` (golden takes 3 copies; they keep the pool exact) |
| A card in hand by name | `g.give(p, "X")` (spells and tokens are generated, minions come from the pool) |
| Set the Tavern | `set_shop(g, p, &["A", "B"])` |
| Act as player p | `act(g, p, Action::..)` (freezes the others until it is p's turn), `play(g, p, hand, pos)` |
| End the turn / all turns | `act(g, p, Action::EndTurn)`; `end_all_turns(g)` ends every player's turn and runs the fights |
| Set gold, tier, counters | write `g.players[p].gold`, `.tier`, `.free_rolls`, `.shop_spell` directly (a `shop_spell` above the player's tier trips `check_invariants`) |
| Force who fights whom | read `g.players[0].opponent` after the turn starts, clear boards, put a killer on that opponent |
| A fight | `bgcore::fight_with(db, &a, &b, [Counters::default(); 2], max_attacks, &mut Rng::new(seed), &mut watch)`; with hands: `fight::fight_with_hands` |
| Watch a fight | impl `fight::Watch` (`attack`, `death`, `summon`) |
| A wall / a killer | Microbot with attack 0 / health 1000, or attack 50 / health 100 |
| What a fight left over | `Outcome.sides[i]`: `cards_gained()`, `permanent_stats`, `lockboxes_gained()`, ... |
| The pool is exact | `g.check_invariants().unwrap()` after any setup that touches cards |
| A whole random game | `bots::random_action(&g, &mut rng)` loop with `check_invariants` |

Helper traps: after any action `current_player` moves on, so call `turn_of(g, p)`
before asking `g.is_legal(..)`; a targeted spell sets `pending_target` even with one
legal target, so answer it with `act(g, p, Action::Target(i))`; a Golden minion played
from hand opens a Triple Reward Discover (`Action::Choose(0)`) before anything else is
legal; Cord Puller has Divine Shield (do not use it as a "no shield" minion); a card
bought, played or given in a test can triple with the copies already in hand.

Run one test file: `cargo test -p bgcore --test pirates`; one test: add its name;
`-- --nocapture` for prints. Whole suite: `cargo test --workspace`.
Test card databases for rule tests: `tests/data/*.toml` (`CardDb::from_toml`); use them
for parse-error tests.

## 5. Gotchas

- **Fuzz never sees tier 7.** The Tavern stops at tier 6 (`MAX_TIER`), so tier 7 cards
  only appear through a card that names tier 7. The fuzzer reaches them only through
  such cards; test tier 7 paths directly.
- **Pool accounting is exact and checked after every fuzz action.** Drawn cards carry
  `pool_copies` (1, or up to 3 for a Golden minion); generated cards and tokens have 0
  and never touch the pool. Selling returns `pool_copies` (`give_back`). A Golden
  minion made on the board keeps its 1 copy. Discover options leave the pool when
  offered and the unpicked ones go back in `resolve_choose`. Cards that leave a player
  (death, knockout) go back in `knockouts`/`settle_dead`.
- **`Minion` is `Copy`, fixed size; no heap in fights or shop actions** (`arrayvec`).
  A new `Minion` field must be set in `CardDb::make` (only constructor), recorded in
  `record.rs`, and survive `Minion::magnetize`/triple merges (triple: `check_triple`
  sums buffs, takes the highest `improvements`).
- **`SideOutcome` is `Copy` with fixed arrays** (cards, lockboxes, permanent stats by
  start slot). Anything a fight gives the owner goes through it and
  `apply_side_outcome`.
- **Fights are pure.** No `Game` access: counters come in through `Counters`
  (`Player::counters`), gains come out through `SideOutcome`.
- **One RNG.** Draw order changes replays; adding a draw to an existing path moves
  every seeded test that crosses it. Tests that pin a seed may need a new seed.
- **Rally runs before the hit and can kill; the swing then fizzles** (`attack_swings`).
- **Deathrattle counts go through `Side::deathrattle_fires`** (Titus). Do not loop
  `fire_deathrattles` yourself.
- **Tavern spell bonus** is added to every stat spell effect (`da`, `dh` in
  `execute_spell_effect`/`apply_target`), including the other stat.
- **Spells and the spell slot:** `CardDb::spells` is the draw list; use
  `random_spell*` (they resolve the Bounty slot), never index it yourself.
- **Hand slots:** generated cards are lost when the hand is full; effects that give
  cards must check `hand.is_full()` (`give_generated_tracking`).
- **Gold:** only `fire_event(GoldSpent, n)` counts spending. Any new payment must call it.
- **A minion in the Tavern is not friendly** ("a minion" reaches board and Tavern,
  "friendly" only the board, never the hand): `TargetKind::takes_shop`.
- **Existing tests change when a tribe lands** (all-type minions, tribe counts); see
  the checklist. Fix expectations only after confirming the rule, and report it.
- **Do not trust JSON pool flags blindly** (Rot Hide Gnoll is flagged but not in
  the pool; `docs/card-data.md` has the game's own flags). The "(N left!)" display can disagree with the text (Gunpowder Courier);
  DESIGN.md records the owner's answer.

## 6. Systems the Pirate run added

- **Pirates (tribe 6).** `Tribe::EVERY` is every tribe the rules know; `Tribe::ALL`
  stays the five the observation has slots for. `Tribes::ALL` includes Pirate.
- **Golden minions.** Played from hand, a Golden minion adds to
  `Player.golden_played` (Extortionist, Hooktusk) and gives a Triple Reward unless
  `Minion.no_reward` (Aureate Laureate is `always_golden`, Silent Deliverer's
  minions). `TargetKind::MakeGolden` makes a board minion Golden with no reward and
  no count (Elite Navigator, Captain Sanders).
- **Golden draws from the pool.** `draw_golden` (game.rs) takes up to three copies for
  one Golden minion (Lockbox, Silent Deliverer).
- **Gold spent.** Every payment goes through `fire_event(GoldSpent, n)`, which also
  adds to `Player.gold_spent_turn` (Lovesick Balladist). Health payments and free
  Refreshes never count.
- **Lockbox.** A generated spell in hand whose `improvements` is the turns left.
  `give_lockbox(p, sooner)` gets one or shortens the held one, `tick_lockboxes` runs
  at the end of the turn, before that turn's fight (Drakkari doubles it). A
  Deathrattle gets one through `SideOutcome::lockboxes`.
- **Bounties.** The five share one slot of every spell draw: `CardDb.spells` holds
  only the first, `random_spell*` turn a drawn Bounty into one of the five
  (`bounties`), "get a random Bounty" is `CardDb::random_bounty`. `bounty_mult` is
  Proud Privateer's cast count; each cast is a spell cast for triggers and counters.
- **Cleave.** `Card.cleave`: `attack_swings` hits the two enemies next to the target
  with the attacker's Attack and no damage back (Blade Collector).

## 7. Systems the Elemental run added

- **Elementals (tribe 7).** `Tribe::EVERY` has seven tribes, `Tribes::ALL` is
  `0b1111111` (one bit left in the `u8` mask). Flaming Enforcer is Demon and
  Elemental (`tribes = [..]`). The observation still has five tribe slots.
- **The Refresh amount.** One `Player.refresh_buff` that Waveling's
  Deathrattle (fight or shop), En-Djinn Blazer and Easterly Winds add into;
  every Refresh except the turn-start roll gives all of it to one random
  non-spell, non-`fixed_stats` Tavern minion. No random number is drawn while it is 0,
  so seeded games without these cards are unchanged.
- **A second player counter.** `N_COUNTERS` is 2: `Counter::Baller` is the
  count of Ballers sold. Tests that spell out `improve` arrays need two entries.
- **Elemental plays.** In `Action::Play`, right after the board insert:
  `Player.elementals_played_turn` (Conflagration, reset in `start_turn`) and
  `give_play_shield`, then, before the Battlecry, `fire_event(ElementalsPlayed)`
  (Unbound Tempest) and Ultraviolet Ascendant's `after_play_tribe` with
  `exclude_played = true`. Neither counts the minion being played (DESIGN.md);
  the older Pirate and Demon `after_play_tribe` cards still do. Only minions
  played from hand count.
- **Triples with a stand-in.** `check_triple_tracking` builds a `TriplePlan`:
  three plain copies first, else two of a card plus one stand-in, else one
  plus two. Each source minion's buffs are measured against its own card,
  the stand-in keeps only keywords it was given, and its pool copy goes back
  to the pool.
- **Spells counted twice.** `apply_target` fires one extra `SpellsCast` event for
  every extra Balinda cast on a friendly minion (Proud Privateer's extra Bounty
  casts were already counted in `cast_spell`).
- **Fights.** Wildfire's splash is in `attack_swings` (`Card.overkill`);
  `Effect::BuffOthersPerImprovement` reads the host's `improvements`
  (Ultraviolet Ascendant).

## 8. Systems the Undead run added

- **Undead (tribe 8).** `Tribe::EVERY` has eight tribes and `Tribes::ALL` is
  `0b11111111`, the whole `u8` mask. Prosthetic Hand is Mech and Undead
  (`tribes = [..]`) and `magnetize_to_undead = true` lets it merge into an
  Undead (`Game::can_magnetize`, used by `is_legal` and `Action::Play`).
- **Player bonuses are baked into stats.** "Your Undead have +N this game
  (wherever they are)" is `Player.undead_bonus`; `Player::make_card` adds it to every
  Undead it makes (draws, Discovers, generated, triples), and `Game::add_undead_bonus`
  adds a gain to the Tavern, the hand and the board. In a fight `Counters.undead_attack`
  and `undead_health` do the same for minions made in it: `Side::summon` adds them,
  `Side::summon_baked` (hand minions, Salvager's exact copy) does not, and
  `SideOutcome::undead_gain` brings an in-fight gain back (the game gives it to the
  board only after a fight; a shop destroy already changed the board). Eternal Knight is
  the same with `Card.buff_per_kind_death` and `Player.knight_deaths`
  (`Counters.knight_deaths`, `SideOutcome::knight_deaths`, `Game::add_knight_deaths`).
  A new "wherever it is" bonus should copy this pair.
- **Reborn comes back through `summon`.** `Side::resolve` makes the plain card with
  1 Health, so the bonuses of `summon` apply (a Reborn Undead with +1 Health is 2
  Health) and `taken` keeps the maximum at the full value, then `Side::note_reborn`
  runs the `Card.on_reborn` cards (Barrier Banshee, Snazzy Phantom; never the Reborn
  minion itself).
- **Destroying in the shop.** `fight::shop_destroy` builds a one-sided `Side`, sets
  the victims to 0 Health and calls `Side::resolve`, so Deathrattles (Titus too),
  Reborn, Knight deaths, Banshee and Phantom all run as in a fight. It returns the new
  board, where a tracked slot ended up (Dead Bellringer's own slot moves when Skeletons
  appear on its left), the `SideOutcome` and the minions that died (their pool copies go
  back to the pool in `Game::destroy_in_shop`). `Side.outside` makes Plaguerunner give
  +4 instead of +2. Avenge does not count shop deaths (`Side.count_avenge`).
  Maw Caster (`TargetKind::DestroyDiscover`), Dead Bellringer
  (`TargetKind::RebornDestroy`), Butchering (`SpellTarget::Butchering`) and Tomb
  Turning's death all use it.
- **Tomb Turning.** The Discovered minion has `Minion.doomed`
  (`DiscoverRequest::DoomedTribeMinion`, `Player.discover_doomed`). Played, the flag
  stays on the board minion until `Game::resolve_doomed` finds no pending choice or
  Discover left, then destroys it. `finish_shop` clears the flag in hand.
- **Fight-only Effects.** `GiveRebornRandom` (Mummifier; `Side.dr_card` is the dying
  card, so "different" excludes copies), `SummonAttacker` plus `Side.immediate` and
  `run_immediate` (Eternal Tycoon's Knight swings after the attack that killed),
  `GetPlainCopy` (`Side.start_cards`, Lichling Hoarder), `GetRandomMinionHeld` and
  `SummonHeld` (`Side.held`, Deathly Striker: what it got is taken back out of
  `SideOutcome.cards` when summoned), `DestroyLeftNeighbor` and `SummonSalvaged`
  (`Side.salvage`, Stitched Salvager).
- **Spell triggers.** `OnCastSpell::UndeadBonus` (Forsaken Weaver) is called once per
  cast by `trigger_on_cast_spell`, so a double cast counts twice. Casts inside a fight
  (Runic Arcanist's Shiny Ring) trigger it through `Side::on_spell_cast`, and the gain
  leaves the fight in `SideOutcome::undead_gain`. A second cast that does nothing
  (Balinda on Butchering or Channel the Devourer, `stop_after`) is not a spell cast.
- **Tests.** `tests/undead.rs`; a fight with a victim that has Taunt and 1 Health, or
  Taunt "fodder" minions that die one per attack, is the way to script who dies when.
