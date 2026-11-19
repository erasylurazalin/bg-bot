# How the engine and training fit together

A map of the code for whoever works on it next. The rules themselves are in
`docs/rules/design.md`.

## Layout

```
cards.toml            every card: minions, tokens, spells, golden behaviour
rules.toml            every rule number: gold, costs, pool copies, damage caps, limits
heroes.toml           the built heroes and their powers (see "Heroes")
trinkets.toml         the built trinkets, lesser and greater (see "Trinkets")
darkgifts.toml        the 47 Dark Gifts: windows, weights, host conditions, the
                      offer tiers by turn (see "Dark Gifts")
peak_boards.toml      hand-made peak boards for `bgbench peak` (guesses, see below)
data/comps/comps.toml the comp library the planner bot plays (see "The comp planner")
crates/
  bgcore/             the rules: no I/O, no Python
    src/cards.rs      card data: TOML parsing into Card, Effect, triggers
    src/heroes.rs     hero data: heroes.toml into Hero, HeroPower, PowerEffect
    src/trinkets.rs   trinket data: trinkets.toml into Trinket, Act, Rule, Soc
    src/darkgifts.rs  gift data: darkgifts.toml into Gift, GiftCond, the offer table
    src/game/          one 8-player game: shop phase, actions, pairing, combat, knockouts;
                      one file per system: mod.rs (types, constructors, step, legality,
                      advance), player.rs, heroes.rs, shop.rs, play.rs, effects.rs,
                      spells.rs, discover.rs, targets.rs, triples.rs, trinkets.rs, turn.rs,
                      darkgifts.rs, invariants.rs
    src/fight.rs      the fight: a pure function of two boards, an RNG and counters
    src/minion.rs     Minion (Copy, fixed size), Keywords, Magnetic attachments
    src/secrets.rs    the Secret zone: the four combat Secrets, the Ice Block,
                      the order they were got in (see "Secrets")
    src/action.rs     the flat action space and its index mapping
    src/obs.rs        the observation a player sees, as fixed-size f32
    src/bots.rs       random, standard-curve and rush-curve bots
    src/comps.rs      the comp library: cards, lobby, enablers, abort, level
    src/planner.rs    the comp planner bot (see "The comp planner")
    src/overlay.rs    the comp overlay: forced rules and logit bonuses (see "The comp overlay")
    src/rules.rs      Rules from rules.toml
    src/rng.rs        xoshiro256++, written here so replays never change
    src/record.rs     recorded sandbox games: TOML writer and the loader back into Minions
    src/real_boards.rs the boards of Era's real games: file parsing, card id
                      mapping, the turn buckets a real seat draws from, the
                      paired fights of one turn
    src/fight_check.rs the fight check: one real fight seated in the engine
                      (power and trinkets included), played many times, and
                      the three files it writes (docs/real-boards.md "The
                      fight check")
    src/shop_replay.rs the shop replay: one logged shop state loaded into a
                      game, one step played in it, the three files it writes
                      (docs/real-boards.md "The shop replay"), and a whole
                      turn played forward from its own start
    src/search.rs     what the shop-turn search needs: the dominance
                      comparison of two positions, the full pool a playout's
                      Refresh draws from, and the coming fight against the
                      boards of Era's real games (see "The shop-turn search")
    src/lobster.rs    the lobster search mode's fixed Tavern and Gryphon Beast
                      streams (see "The lobster search mode")
    tests/            one file per tribe (beast_mech, dragons, murlocs, demons, pirates, elementals, undead, quilboar, aberration, neutrals),
                      heroes.rs (the hero frame: pick, armor, the 22 powers, the two spells),
                      trinkets.rs (the trinket frame: the two offers, the guarantee and the
                      discount, the slots, the hooks and the built cards),
                      prizes.rs (the Darkmoon Prizes: the Discover, the Triple Prize,
                      one test per Prize),
                      heroes_batch1.rs, heroes_batch2.rs, heroes_batch3.rs,
                      heroes_batch4.rs (Tickatus), heroes_frame2.rs,
                      secrets.rs (the Secret zone, the four Secrets in a fight,
                      Prestidigitation),
                      darkgifts.rs (the gift table, the button, the offers, the landing,
                      the countdowns and the gift hooks),
                      tier7.rs (tier 7 in the engine and its seven minions),
                      deathrattles.rs (a loop over every Deathrattle card),
                      lobby.rs (the 5-tribe lobby and every draw that respects it),
                      core_rules.rs (the generic rules on the real data), fight.rs,
                      game.rs, repeats.rs (golden-making once, a repeated Choose One,
                      the repeat queue on Battlecry triggers),
                      real_boards.rs (real-board seats; its small fixture
                      folder tests/data/real-boards stands in for boards/real),
                      fight_check.rs (one real fight seated and played, the
                      fixture folder through the checker, the three files),
                      shop_replay.rs (a logged step replayed from its state,
                      the fixture folder in tests/data/actions),
                      sandbox.rs; parser tests build small TOML strings inline
  bgbench/            benchmarks, fuzzer, bot matches, the peak-board balance matrix,
                      `bgbench collect-boards` (the real board files),
                      `bgbench collect-actions` and `bgbench check-actions` (the
                      real shop actions: src/collect_actions.rs, src/check_actions.rs;
                      a user timer runs the first two every 15 minutes)
  bglog/              Power.log reading, a port of the older Python tracker:
                      the parser and entity table (src/parser.rs, game.rs,
                      entity.rs), the Battlegrounds views
                      (src/views.rs: boards, hands, hero powers, trinkets,
                      rewards, Deity, counters, Dark Gifts, Secrets, Rapid
                      Reanimation's mark, Upper Hands, the player-level numbers),
                      the options and choices the client prints (src/options.rs:
                      what it offered me, what it offered as a Discover or a
                      Choose One, and the option I took) and the Tracker that
                      follows a log into fights (src/tracker.rs); card names and
                      tribes from ref/hsjson-cards.json (src/cards.rs).
                      `bgbench powerlog` and `bgbench tracker` dump a log, to diff
                      against the older Python tracker
  bgpy/               PyO3 bindings: a batch of games stepped together, the
                      search's copies (SearchBatch, see "The shop-turn search"),
                      and Era's turns seated step by step (EraReplay,
                      EraTurnPath, RealFights, see "Era's turns, with Era's
                      luck")
  bgsandbox/          terminal sandbox: play shop phases alone, boards recorded to boards/
train/
  model.py            policy and value network
  card_text.py        card text to token ids for the network (--card-text, --power-text)
  ppo.py              PPO self-play, evals, checkpoints
  real_boards.py      loads boards/real for training (card id to name, summary)
  search.py           the shop-turn search: candidates, cycles, the two
                      judges, playouts (see "The shop-turn search")
  eval_search.py      the search's evaluation: N games plain against the
                      search, and the Blue Shell, Warden and cycle-rate tests
  eval_overlay.py     the comp overlay's per-comp evaluation: N games, B, real or network
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
rules behind each are in docs/rules/design.md Part 2.

- **Choices the player makes mid-action.** `PendingTarget` (a minion on
  the board or in the Tavern), `PendingShopTarget` (a Tavern slot),
  `PendingChooseOne`, and Discover (`DiscoverRequest`). While one is open,
  the only legal actions answer it. A choice no answer can satisfy (no card
  reaches one today) fizzles in `step`, through `Game::choice_stuck`: the
  effect is lost, the normal actions stay legal, and one warning line goes
  to stderr.
- **Activate (N).** Pays gold, once per turn per minion (`Minion.activated`,
  cleared at the start of the owner's turn), with or without a target. The
  Gold leaves the purse on the click, but what watches Gold spent waits for
  the choice the Activate opened: `Action::Activate` hands it to
  `Player.pending_gold_spent` and `maybe_start_discover` fires it once
  nothing is left to answer, because the log counts the Gold on the click
  and gives the card at the pick (a Sky Admiral Rogers going `[2, 0, 9, 0]`
  to `[9, 1, 9, 0]` on the Activate and its Bounty landing in the hand at
  the pick, `Hearthstone_2026_10_26_00_48_15-g3` turn 7 steps 29 and 30).
- **Triggering in the shop.** Rally (Sky-hatch Runaway), Battlecries
  (Kelp Keeper, Young Murk-Eye; counted for Kalecgos, doubled by Brann),
  Deathrattles (Deathstrider after a Fishbait attack).
- **One count for Deathrattles.** `Side::deathrattle_fires` in fight.rs
  decides how many times a Deathrattle fires (Titus, golden Titus). Every
  path that fires one goes through it.
- **The triple check runs as the action ends.** "Check Triples" is one hidden
  step of the game, not part of the summon or the gain that made the third
  copy, and it waits while a choice is open. `Game.board_triple_check` (a
  summon may have left a third copy on the board) and
  `Game.pending_triples` (cards gained whose own check waits: a Graverobber's
  copy, a Discover's pick) are run by `Game::run_pending_triple_checks` at the
  end of `Game::step`, only while no choice is open. The board's triples go
  first, then the gained cards in the order they came, so a Battlecry's copy
  keeps the lower hand slot and the Golden card its destroy leaves joins
  after it (log 2026_10_30_19_45_15-g2 turn 5 step 4), and three Skeletons
  stay on the board until the Discover that is still open is answered (log
  2026_10_24_15_25_26-g6 turn 8 steps 14 to 16). A triple whose copies are all
  in a full hand still merges, since those copies free their slots first, and
  so does one that has to take a copy off the board with ten cards in hand:
  the merge is one action that takes all three copies and puts the Golden one
  in the hand, so the slots it frees are free before the Golden card needs one
  (log 2026_10_24_04_08_00-g1 turn 10 step 25). Three copies all on the board
  are the exception: they need a slot for the Golden card that they do not
  free themselves, so with ten cards in hand the check puts the card on
  `Player.triples_waiting` and `Game::retry_waiting_triples` (the last thing
  `run_pending_triple_checks` does, so a card an action gained keeps its hand
  slot first) asks again as every later action ends: the merge happens as soon
  as an action takes a card out of the hand (`docs/rules/design.md` "A full
  hand"; Era, 2026-11-04). Only a refused card is asked again, never the whole
  hand and board: the game's own check is queued by what happens, and a state
  that holds three copies the game has not merged yet stays that way until the
  next step that asks (log 2026_10_24_15_25_26-g6 turn 8 step 15, the Discover
  answered, leaves the Skeletons where they are; step 16 merges them). The one
  place that sweeps is the start of a turn: a fight leaves its survivors
  without any check of their own, so `Game::check_triples_at_turn_start` (from
  `dark_gift_start_of_turn`, after the cards a fight handed over got their
  check) asks once about every card in the hand and on the board that has its
  copies, merging them if the hand has room and putting them on
  `Player.triples_waiting` if it does not. A copy an open choice still points
  at holds its place: a Choose One left open keeps the board's copies where
  they stand. The Golden card's stats are its
  golden card's printed ones plus each copy's buffs; a copy's damage is not
  one of them, since the game sums the copies and then heals the Golden one (log
  `2026_10_24_00_56_06-g1` turn 8 step 21: the merged Plaguerunner takes ATK 25
  and DAMAGE 1, then DAMAGE 0). `Minion` keeps no damage, so a copy's Health
  below its card's printed stats reads as no buff. A copy's own Holy Vanguard
  aura is no buff either, and it is in the copies' stats like any other stat
  (`Player::make_card` puts it on the card's own +30/+30, +60/+60 for a
  Golden one, while the Health is 15 or less), so the merge's difference
  against the card's base leaves it out and the Golden card, made at the same
  Health, has its own once (log `Hearthstone_2026_10_28_22_11_50` game 5 turn
  10 step 12: the Golden Holy Vanguard is 208/116, where counting the three
  copies' auras said 238/146). `Minion::vanguard_aura` is what that pair is
  worth to the card, so `Game::sync_vanguard` can take it off exactly once
  when the Health goes back above 15: it is called from every Health change
  (damage, a Health price, a rewind, a fight's damage) and at a turn's start,
  and it walks the board, the hand and the Tavern only when the Health's side
  of 15 changed (`Player::vanguard_on` holds the state the zones are in).
- **A Health price comes before the card.** A Tavern spell bought for Health
  pays before it joins the hand, so a card the damage triggers gains gets the
  lower hand slot (Eredar Escapist's two Corrupted Cupcakes, log
  2026_10_24_15_25_26-g4 turn 11 step 10). Gold, and the triggers that read
  it, come after the spell is in hand (a Lockbox from a Gold-spent rule, log
  2026_11_02_00_10_22-g5 turn 11 step 12).
- **Counters.** Per minion: `improvements` (Leviathan, Hopebringer,
  Patient Scout, the "N left" cards, a Sacrificial Wrathguard's Activates);
  a triple keeps the highest of the three. Per player: `Player.improve` with `Counters.improve` inside a
  fight (Tasty Lobster), the Tavern spell bonus, the Beast bonus from a
  shop Goldrinn, the Fodder queue. Gains made inside a fight come back
  through `SideOutcome`. The shop replay loads the per-minion "N left"
  count off a logged card's first number for every card whose `on_event`
  trigger takes more than one event (the card data says which, and what the
  number means), and the "left this turn" counts a trinket keeps (Warcry
  Totem's free Battlecry minions, Transcribing Typewriter's copies, Magicfin
  Sticker's uses, Guiding Candle's Tier 6 Refreshes, Wand of Divination's
  third spell) card by card, since the same number means something else on
  each trinket (docs/real-boards.md "The shop replay"). The same pass reads
  the counts that are the game's own: Goldenizer Supply's turns left in its
  first number, and a Magicfin Mycologist's teachings left (no number means
  none, and a fresh card would otherwise teach once more than the log did).
  A Malchezaar's Health Refreshes left are the same kind of number (its
  first, no number at all when both are spent), and so are a Thorned
  Trailblazer's combine left this turn (its first, none when spent) and a
  Stone Age Slab's after-you-buy use (its third, `Minion.activated`), and so
  is the Demon Fodder
  queue, which the game keeps on its own player enchant (`fodder_refreshes`,
  one number per coming Refresh). The card itself also carries the Attack and
  Health its Blood Gems gave it (`gem_attack`/`gem_health`, the log's
  `4270`/`4271` pair on the minion): the loader seats them as
  `Minion.gem_attack`/`gem_health` in the board, the hand and the Tavern, so
  a replayed Gem Confiscation takes the neighbours' pair onto its target as
  the log does. A minion's own Magnetization count is on the card too
  (`magnetizations`), and it is the game's own count, the log's unnamed tag
  `3670`: one for every Magnetic that merged into the minion and one for the
  minion itself when the minion is Magnetic (the collector takes that one off,
  since `Minion.magnetizations` counts what merged in). It is seated as
  `Minion.magnetizations` so Utility Drone's end of turn has a count to
  read: the game's Drone reads that same tag, and one `Magnetic` enchantment
  is not one Magnetization (a Satellite's stats all pile into one
  `BG31_171te`, so a Glambot with 94 Magnetizations shows a single
  enchantment). The player's "your Volumizers have +A/+H this game (wherever they
  are)" is on no player entity in the log, only on each Volumizer's own
  `Volumized` enchantment: the loader seats the pair the collector reads
  there as `Player.volumizer_bonus`, and the next Volumizer played adds to
  it and hands the increment to every Volumizer in the shop, the hand and
  the board. The price the game charges for a Tavern
  card when it is not the rule's is on the card too (`cost_override` for
  Chillmere Mosaic's (1), `cost_reduce` for a free buy), and the loader turns
  a Tavern that all costs the same into the engine's own state: the shop's
  price (`tp.tavern_price`) or a free buy charge (`free_minion_left`). A
  Tavern spell's own price goes in as the player's next-spell discount
  (`Player.spell_discount`), from the same marks or, when the card has
  none, from what the step's `paid` says the log charged
  (`seat_spell_paid`). The
  spell a Magicfin token was taught is a dbf id in the token's first
  number, so `shop_replay::seat_taught_spells` (with `ref/bg-cards.json`)
  fills `ShopMinionSpec::taught` before the replay runs, and the loader hands
  it to `Minion.taught_spell`. "The last Tavern spell you cast"
  (`Player.last_spell`, which Cataclysmic Harbinger and the Galaxy's Lens
  copy) is in no logged state: `shop_replay` reads it off the file's own
  history, the last `play` step of a Tavern spell before the step being
  replayed, because only the player's own casts count.
- **The two counters the engine itself adds to.** The log's Tavern spell
  bonus (`TAVERN_SPELL_*_INCREASE`) is the total the game gives every spell:
  a board aura (Humon'gozz), a trinket's standing rule (Honeycomb Ring,
  Multilayered Shield) and its "this turn" part are already inside it. The
  loader takes the engine's own part back out (`Player::total_spell_bonus`
  minus `Player.spell_bonus`), so the total is the log's number and a Tavern
  spell cast in a replayed turn is exactly what the log shows. What the
  turn's own spells improved goes into `tp.spell_bonus_turn`
  (`seat_spell_bonus_turn`, the counter against the one the turn started
  with), which is the engine's own part for "this turn only"; it does not
  change the total, so the engine's live improvement inside one step still
  orders as the engine does. The Blood Gem
  counter is the other way round: `gem_attack`/`gem_health` leave out a
  trinket's from-hand bonus (Surveyor Portrait's +6/+6 is written on the Gem
  cards, not on the player), so the loader runs that `on_pick` act
  (`Act::GemBonusHand` into `Player.gem_hand_attack/gem_hand_health`) while
  `Act::GemBonus` stays in the logged state. The log evidence is under
  docs/real-boards.md "The file", at `counters`.
- **The hand in combat.** `fight_with_hands` passes a side's hand only when
  something on that side reads it. Murlocs summoned from hand come back
  unchanged; Winterfinner's hand buffs are kept. A real board file's `waits`
  is Diremuck Forager's summons still owed when its Start of Combat found the
  board full: it becomes `Counters.waits`, and `Side::spawn_when_space` takes
  the highest-Attack Murloc out of that side's hand through the same
  `summon_from_hand` the card's own trigger uses.
- **A full hand.** `Player.hand` holds `HAND` (10) cards; `add_to_hand` is
  the one way in. With a full hand the card is lost, which is what a card got
  during the shop phase or in combat does (Sky Admiral Rogers' Bounty, a
  fight's Blood Gems), except at the start of the turn: `Player.hand_queue_on`
  is set through `start_turn`'s own effects (hero powers, trinkets, Dark
  Gifts) and a card got then waits in `Player.hand_queue` instead, joining
  the hand first in first out as soon as a slot frees (`drain_hand_queue`,
  run after every action and at the end of the start of turn). A queued card
  is not in the hand yet, so nothing that reads "when you get a card" fires
  until it joins (docs/rules/design.md "A full hand", Era 2026-11-02).
- **Refresh kinds.** `RefreshKind { TurnStart, Explicit, Effect }`: only an
  explicit Refresh (the player's action) takes queued Fodders.
- **Hero health.** `take_hero_damage` runs the "after your hero takes
  damage" reactions (rewinds included) for damage taken in the shop; combat
  damage just lowers Health and reacts to nothing; effect damage in the shop can
  knock a hero out mid-turn; `can_pay_health` gates Health costs.
- **Tavern buffs** "this game" and "this turn", consuming Tavern minions,
  stealing them, Fishbait and Demon Fodder as generated Tavern cards.
- **Generated cards** (`pool_copies = 0`): tokens, Chromadrakes, Fodder,
  Fishbait, and the trinket-only minions of docs/rules/design.md ("Generated minions
  outside the pool": Tide Oracle Morgl and the rest, in the
  "--- Trinket-only minions" block of cards.toml). They never touch the
  pool.
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
- **A dead minion does not hold a board slot.** A summon made by an
  after-attack trigger while a minion is at 0 Health (before `resolve` runs)
  takes the left-most dead minion out of `Side.ms` into `Side.held_dead`,
  and `Side::resolve` runs that death after the summon. Power.log 274478 is
  the case: a golden Ravaging Scorpid's Deathrattle, fired by Deathstrider,
  filled the slot of the Headhunter Gryphon that had just died. A Deathrattle
  that both summons and buffs Beetles (`Turquoise Skitterer`) summons first
  and buffs second, as its log shows (Power.log 2026_11_11 152571): the new
  Beetle stands at the old bonus and the buff lands after Banana Slamma.
- **One pick per type.** `cards::pick_one_per_type` is the pick Misplaced
  Tea Set and The Last One Standing share: one random minion per type, each
  minion at most once.
- **Maximum Health in a fight.** `Fm.taken` counts the damage a minion's
  Health took, so its maximum is Health plus `taken` (Stalwart Kodo).
- **Magnetize hooks.** Every Magnetization goes through
  `Game::magnetize_to`, which also runs `beatboxers_copy` (Polarizing
  Beatboxer, `Aura::PolarizingBeatboxer`), so a new "whenever you Magnetize"
  card hooks in there. A Magnetic played from hand goes through
  `magnetize_onto_played_mech` first: the "whenever you play a Mech" hooks
  (`on_play_mech`, Spark Snapper) run against the played minion while it is
  a minion of its own, so its Satellite joins it before it merges into the
  host, and every copy of it (Beatboxers) carries the Satellite
  (`10_29_22_20_04-g3` turn 13 step 10). Drone Duplicator's extra
  Magnetization (`Minion.extra_magnetize`, from `Effect::ExtraMagnetization`
  and read by the collector off the game's own `4945` tag, see
  `docs/real-boards.md` "The action file") then repeats the merge that many
  times, each repeat counted as a fresh Volumizer (its own "the first time
  this is played or Magnetized" fires again, so the Volumizer bonus grows)
  and with its own `beatboxers_copy`; the first merge's Beatboxer copies
  come last, as in the log (`10_29_22_20_04-g3` turn 14 step 20), and the
  repeats re-run the "whenever you Magnetize a Mech" buffs without firing
  the played-Mech hooks a second time (`10_27_04_30_04-g2` turn 10 step 18:
  a 24/20 Volumizer with a Satellite into a Drone Duplicator happens twice).
  Every copy the Beatboxers take is minted afresh, so a Volumizer's copy is
  as much bigger as the copies before it in the same step raised "your
  Volumizers have +A/+H" (`10_29_22_20_04-g3` turn 14 step 20: two
  Beatboxers take three copies each of one golden Green Volumizer and the
  log's two come out +4140 and +4146 bigger, three even copies +4122); the
  pair a step's copies paid is handed on to the next `beatboxers_copy` call.
- **Pirates, Golden minions, gold spent, Lockbox, Bounties, Cleave:** see
  `docs/adding-cards.md` section 6.
- **Undead** (the Undead bonus, Reborn in the shop, shop destroys, Knight
  count, Phantom chain): see `docs/adding-cards.md` section 8.
- **Quilboar** (Blood Gems and the gem value, Tavern spells versus plain
  spells, the Triple Reward spell, set-versus-add, discard, both effects,
  the Juggernaut Golem): see `docs/adding-cards.md` section 9.
- **Aberration** (the Deity, one discard path, one hand-insert hook, linked
  cards): see `docs/adding-cards.md` section 10.
- **Elementals** (refresh amount, free Refreshes, Baller count, Surprise
  triples, Slab, Prison, Wildfire): see `docs/adding-cards.md` section 7.
- **Legal actions.** `Game::legal_mask` only asks `is_legal` for slots that
  can be legal (a test checks it against `is_legal` everywhere). Use it
  rather than looping `is_legal` over every action.

## The lobby

Every game plays with exactly 5 of the 10 tribes (docs/rules/design.md "The lobby's
tribes"). `Game::unstarted` rolls them with the game's RNG (`Tribes::random_lobby`)
and stores them on `Game.lobby`; the same seed always rolls the same lobby. The
pool starts with copies only of lobby cards (`Card::lobby_ok`: a minion with a
lobby tribe, or a neutral or spell not tied to a tribe outside it; a neutral or
spell tied through cards.toml `lobby = "<tribe>"`, the game data's
`lobby_subsets`, is in only with that tribe; all-type minions are always in),
`check_invariants` checks the rest stay at 0, and every card draw takes the
lobby as its filter: the Tavern rolls, Discovers, "get a random X", random
spells, random tier minions, random Choose One cards (minions and Tavern
spells alike: `random_choose_one_card`), Bounties, Chromadrakes,
Volumizers, Robust Evolution and the draws inside a fight (the lobby travels
there in `Counters.lobby`; `Tribes::ALL` draws from everything, which is what a
fight without a game behind it uses). Nothing outside the lobby can appear, and
a draw with no in-lobby candidate returns None (the effect fizzles). The Deity
exists only when Aberration is in the lobby. A draw filter is just a `Tribes`
mask, so a future trinket or hero effect can pass a wider one for its own draw.
A Timewarped minion (`cards.toml timewarp = "minor"` or `"major"`, the 2-cost
Timeworn Candelabra lists) is a token outside the pool table, and
`Card::lobby_ok` is false for it, so no normal Tavern, Discover or random-minion
draw can hand one out. The Candelabra's own Discover is
`DiscoverRequest::Timewarp(side)` and `CardDb::random_timewarp`, which reads
the marker and has no lobby argument: 19 built Minor and 39 built Major cards.

Tests and the sandbox can fix the lobby: `Game::new_with_lobby(data, seed,
lobby)` (a test may pass more than 5 to reach its cards) and
`Game::new_sandbox(data, seed, Some(lobby), free_rolls)`. The bots'
`preferred_tribe` counts only lobby tribes.

## Heroes

`heroes.toml` (docs/rules/design.md "Heroes") holds the built heroes: armor, extra
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
- **Forced learner heroes:** `Game::new_random_heroes_forced` and the
  `_forced` forms of the real-seat, Era-seat, scenario and Era-start
  constructors take (seat, allowed heroes) pairs. Each such seat gets one of
  its heroes with the game's RNG, and those heroes leave the deck, so no other
  seat is dealt one; a list shorter than the seats repeats a name after every
  one has been used. The empty list is the old deal exactly, RNG included.
  `bgpy.VecEnv(learner_heroes=[..])` resolves hero names and applies it to
  every learner seat, refusing an unknown name by name; `ppo.py
  --learner-heroes` passes the list to training and every eval. An Era-start
  seat keeps its logged hero, so it and a forced hero equal to it are left
  out.
- **Health and armor**: take_hero applies armor, extra Health and the power.
  Every hero damage takes armor first (`take_hero_damage`, the combat damage
  in `combat`, Health costs via `can_pay_health`); a rewind gives back what
  was taken, armor as armor (`last_damage_armor`). Armor Stash sets armor.
- **Powers are their own table.** `HeroDb.powers` holds every built power and
  `PowerState.of` indexes it: a hero's `power` is the index of the power it
  starts with (its own number, since the starting powers come first), and a
  `[[power]]` entry is a power no hero starts with (Sulfuras, the four
  Invocations, Galewing's flightpaths, the King of X family). That is what
  lets one power turn into another without the engine knowing any hero by
  name. `HeroDb::power_by_card_id` finds one by the game data's
  `hero_powers` id; `becomes`, `returns_to` and `choose` in heroes.toml name
  powers by card_id and are resolved to indexes at load.
- **The two powers**: `Player.power` is the MAIN `PowerState` (which power,
  uses left this turn, used this game, unlocked, one progress counter, the
  power it last completed into its chooser) and `Player.power2` the SECOND
  one, held after Genn's King of Duality or the Reinvigorating Light trinket.
  `Action::HeroPower` presses the main button and `Action::HeroPower2` the
  second (`power_usable` / `power_usable2`, `press_power(p, second)`); each
  has its own uses, cost and state, and Unmasked Identity changes only the
  main one. Both are reset at the start of the player's turn in
  `power_start_of_turn`.
- **Powers that change**: `BuyMilestone` counts bought cards in
  `power_bought_card` (the two buy paths; a steal never comes through them)
  and swaps the power past its count; `EndOfTurnEnds` runs in
  `power_end_of_turn`; a chooser (`ChooseSubPowers`, its `options`) fills
  `power_offer` on a press, and `pick_power` puts the pick in the slot the
  offer names. A chosen power with a `parent` completes after
  `complete_turns` turns (counted in `power_start_of_turn`) or after the next
  fight (`power_after_fight`) and then goes back to its chooser, which
  remembers it in `PowerState.last_pick` so Galewing cannot offer it again.
  A power in a family (`family` in heroes.toml, the ten King powers) rotates
  in `rotate_family_power` to a random other power of the family whose tribe
  is in the lobby, never the tribe it has now.
- **Choices at the end of the turn.** `finish_shop` answers everything the
  player left open (`resolve_open_choices`, first option that can be taken)
  before and after the end-of-turn effects, so no prompt outlives the turn:
  that includes a Discover an end-of-turn effect itself opens (a target or a
  cast, a trinket's), which is still sitting in `discover_queue` when the
  effects finish. A Discover a Battlecry an end-of-turn trigger runs never
  asks: it is answered at random (see "A Discover a Battlecry that was not
  played opens").
- **Skipped turns**: a power with `skip_turns` leaves only Freeze and End
  Turn legal for the player's first turns (`Player.skipped`, kept by
  `refresh_skip`, read by `turn_skipped` in `is_legal` and `legal_mask`); the
  fight still happens, and a choice the skipped turn opened can still be
  answered (Choose stays legal beside Freeze and End Turn, which resolves
  it). `TurnDiscovers` queues a `TierLockedMinion` Discover per tier at the
  start of that turn (Faelin's at the start of turn 1), and the pick lands
  in hand with `Minion.locked_tier` set: it can be buffed in hand but not
  played, discarded or pulled into a fight from the hand until the Tavern
  reaches that tier. `assign_hero` runs the current turn's power work when a
  test forces a hero into a game that has already started.
- **Passives hook into the existing triggers**: FreeRefreshPerTurn and
  Bananarama in `power_start_of_turn`, GoldAfterUpgrade in `Action::Level`,
  GoldAfterBuyTribe in `Action::Buy`, GoldNextTurnAfterSell in `sell_minion`,
  and the four Start of Combat effects through `Counters` (`soc_leftmost`,
  `soc_rightmost`, `soc_double_left`, `soc_earth` with its token card,
  `soc_lightning`) at the top of `run_start_of_combat`, before any minion's.
  `Player::counters` reads both held powers, and a power that triggers twice
  (Valdrakken Wind Chimes, Ancient Wishbone: docs/rules/heroes.md "Start of Combat
  powers that trigger twice") doubles the three counts through
  `Player::tick_power_effect`; the two keyword masks keep the single grant,
  since the same keywords twice are the same keywords.
- **Buttons that reuse a card path** (the 2026-10-30 batch): SpellDiscount
  writes `Player.spell_discount`, the same field Ominous Seer sets and
  `pay_for_spell` uses up; GetRandomSpell draws through
  `CardDb::random_spell` and `give_generated` (no pool copy);
  TriggerBattlecry offers `TargetKind::TriggerBattlecry`, the Kelp Keeper
  path, and `power_usable` asks `has_battlecry_target` (the same helper an
  Activate uses); DiscardGetRandomMinion opens `pending_discard` with its
  reward, the Mangled Bandit path, whose `source` is None for a power;
  RefreshWithSpells calls `refresh_with_spells`, the Saloon's Finest fill.
  A power can also give a linked group: GetLinkedMinions draws with
  `CardDb::random_any_minion` and hands it to `give_linked`, so playing one
  discards the rest (Dark Ritual).
- **Discovering a hero power** (Unmasked Identity, Finley's Adventure!, King
  of Duality, the Reinvigorating Light trinket): `discover_hero_power(p,
  excludes, to_second, count)`, one engine function taking the source's
  exclusion list as hero indexes (`HeroDb::hero_indices`, from cards.toml's
  `exclude_heroes` for the spell, from `HeroDb::hero_power_excludes` for a
  power, from `power_excludes` on the hero for Finley). Offers are
  `Player.power_offer` (power indexes), answered with `Choose`; the pick
  lands in the main slot or, with `power_offer_second`, in the second one,
  as a fresh `PowerState`. `count` of 2 is King of Duality: the first pick is
  the second power, the second pick the main one. Finley's runs right after
  his pick, before turn 1.
- **Hero batch 1 (2026-11-01)**, the gold and price, counter, Discover and
  generate powers (the rules are in docs/rules/heroes.md "Hero batch rules
  (Era, 2026-11-01)"):
  - **The press price is not the card's.** `PowerState.cost_delta` is a
    running change to it and `Game::power_cost(p, second)` is the one read
    (legality, payment, the observation's `SCALAR_POWER_COST`): Lead Explorer
    takes +1 per press, Pirate Parrrrty! takes 1 off per Pirate bought and
    spends the discount on the press, and a power with `cheaper_each_turn`
    (The Perfect Crime, The Galaxy's Lens) drops 1 at every end of turn. It
    never goes below 0.
  - **The bot sees every power's count.** Each power slot now carries
    `SCALAR_POWER_PROGRESS`/`SCALAR_POWER_LEFT` (and the same pair for the
    second slot): `obs::power_progress` fills them from the power's own
    `PowerState` and the shared counters, so a power that counts something
    adds no scalar. A counting power reports the count and what is left of it
    (Natural Balance, Glaive Ricochet, Verdant Spheres, Gone Fishing, Heroic
    Inspiration, For the Horde!, Buried Treasure, Stir the Pot, Upbeat
    Harmony, BUY, INSECT!, Reliquary Research's every third spell), the
    growing ones the level reached (Sharpen Blades) or the Gold they pay
    (Piggy Bank), Choose Your Champion the Gold spent and the Gold to go, a
    chosen power its countdown (Galewing), Lucky Roll its lock in both. A card
    a power hands out locked in hand is a hand slot like any other: the
    slot's `SLOT_LOCKED` says it is locked (see below).
  - **Presses a game.** `HeroPower.uses_per_game` is 1 for "Once per game"
    (heroes.toml's `once_per_game = true` still parses, and sets it) and 3
    for Three Wishes; `uses_per_game = 0` means no limit. A passive whose
    count is used up stops counting.
  - **The prices a power sets.** `PowerEffect::Prices` (Manastorm) is read by
    `Game::power_prices`, and from there by `minion_price`, `refresh_cost`
    (Roll's legality and payment) and `upgrade_price` (the tier's price as it
    stands plus the extra: Level's legality and payment, the observation, the
    sandbox). Tavern spells keep their own cost.
  - **One counter block.** `Player.counts` (`HeroCounts`) holds what a power
    counts over the game: cards bought, minions bought, Tiers bought (a spell
    by its Tier), minions sold, friendly attacks, Gold spent this game and
    Tavern spells bought. The counters grow at the hooks that already exist:
    `power_bought_card` (both buy paths and the spell slot, `Action::Buy` and
    `Action::BuySpell`), `power_sold_minion` in `sell_minion`,
    `power_elemental_played` where `elementals_played` grows, `power_attacks`
    in `apply_side_outcome` (the fight's `friendly_attacks`), and
    `power_gold_spent` from `fire_event(GoldSpent)` (never a Health price).
    A power reads the counter it needs at its press (`BuffPerBuys`) or keeps
    its own cycle in `PowerState.progress` (`RandomTribeAfterSells`,
    `CardAfterBuys`, `TripleRewardAfterTiers`, `TavernBuffAfterBuys`,
    `CopyBuyAfterMinions`).
  - **Offers that are not one pool draw.** `maybe_start_discover` deals three
    new requests itself: `PotTypes` (Stir the Pot: one minion per thrown
    type), `DeadWarband` (Kel'Thuzad's Kitty) and `DeadLastFight` (Reclaimed
    Souls, plain copies). The last two fill `Player.discover_minions`, the
    ready-made minions `resolve_choose` hands over (`discover_minions`, with
    no pool copy moving; `abandon_discover` returns none). That path takes the
    `discover_damage_from` damage too, so a Battlecry's Discover that lands as
    a ready-made minion (a shop replay seats it so) still deals what it owes. `Champion` deals
    three Tier 7 minions and the pick lands in hand with
    `Minion.locked_gold` set (`discover_locked_gold`), the shared lock of
    docs/rules/design.md "Cards locked in hand". `MagneticMinion` is a plain draw with a
    filter (Tinker).
  - **The last fight's dead** are `SideOutcome.dead_cards` (one list per
    side, in death order, tokens included); `combat` puts both sides' into
    `Player.dead_last_fight`. A hero's death hands its warband to
    `Game::kitty_watch` from `knockouts` and `knock_out_mid_turn`, before the
    cards go back: `Game.kitty_warbands` keeps every warband of the round with
    the round it died in, and at the start of the next round each holder
    queues one `DeadWarband(i)` per warband, back to back (heroes die at the
    end of a fight, so several can die at once). The warbands are dropped once
    that round has started for everyone.
  - **What the leaderboard shows** is taken in `combat` before any fight:
    `Leaderboard::of` over each living player's board fills
    `Player.leaderboard` (the top tribe and its count, nothing for a neutral
    or empty board, "mixed" when two tribes tie for the count), and it holds
    until the next combat. Each fight then shifts one `FightResult` (the
    outcome and the damage dealt or taken) into the front of `Player.fights`,
    the three the observation carries (docs/rules/design.md "The leaderboard").
  - **A random Tavern spell** (Puzzle Box): `Game::cast_random_spell` draws
    with `CardDb::random_castable_spell` (never a destructive one), skips a
    spell with nothing to land on (`Game::spell_castable`, which
    `cast_spell_inner` shares), casts it, and answers its target and its
    Choose One at random.
  - **Buttons that pick a minion**: `TargetKind::RemoveTierBelow` (Trash for
    Treasure: not a sale, the pool copy goes back), `SwapWithShop` (Swap,
    Lock, & Shop It: the Tavern minion takes the board slot, no buy and no
    play) and `ThrowInPot { count }` (Stir the Pot: the minion is gone for
    good, its copies go to `Game.consumed`). The Perfect Crime's leftovers go
    the same way. A Tavern pick is `Player.pending_shop_target` (Consume, and
    a Fishbait's "a card in the Tavern"): the Fishbait counts the spell slot,
    one past the minion slots, and takes its place (`Game::put_fishbait`);
    Consume eats minions only.
  - Reliquary Research reads `Game::spell_price` (0 for every third Tavern
    spell bought) in both spell buy paths, and The Galaxy's Lens hands over a
    fresh copy of `Player.last_spell`.
  - **Cards locked in hand** (docs/rules/design.md) are one lock in the engine:
    `Minion.locked_in_hand(tier, gold_spent)` over `locked_turns`,
    `locked_tier` and `locked_gold`, read by `is_legal` (Play),
    `valid_discard`, the fight's summon-from-hand (`Counters.gold_spent`) and
    the observation's `SLOT_LOCKED` (the turn count, or 1 while a tier or
    Gold lock holds it). Every card that locks reuses it.
  - **Hero batch 2 (2026-11-01)**, the stats, target and Tavern powers
    (the rules are in docs/rules/heroes.md "Hero batch rules (Era,
    2026-11-01)"):
    - **A standing rule is a player field.** `Game::sync_power_flags` reads
      both power slots into `Player.shop_fewer` (Stay Frosty's one fewer
      minion), `freeze_end`, `extra_dragon` (Dream Portal), `double_time`
      (the Tavern Coin it names), `hat` (Hat Trick), `enhance_times`
      (Enhancification), `twice_nice` and `ticket_chance` (Come One, Come
      All!). It runs whenever a power arrives (the pick, `set_power`) and at
      the start of every turn, so `Player::shop_size`, `minion_price` and the
      shop paths read one field and never the power table. A power that
      presses into its own price (Stay Frosty's minions cost 2) goes through
      `Game::power_prices` like Manastorm's.
    - **One after-Refresh hook.** `Game::after_refresh_powers` runs at the
      end of every Refresh (`refresh_shop`, `refresh_with_spells`,
      `refresh_with_battlecries`, `refresh_with_tribe`, Temporal Tavern's own
      fill, the turn-start roll when the Tavern changed, Bob's Burgles): it
      marks Darkmoon Tickets on the new cards, gives Twice as Nice's copy
      (the highest-Tier minion, frozen, into one of the normal slots the fill
      left empty for it) and Enhancification's two random Bonus Keywords.
    - **Frozen on its own.** `Minion.frozen` is a per-card Freeze (Twice as
      Nice freezes the two cards it names): the turn-start roll in
      `start_turn` keeps those cards and drops the rest, the same one-turn
      rule `Player.frozen` follows. `Player.frozen` stays the whole Tavern.
    - **Two-step picks.** `Player.two_step` remembers the first answer of a
      pick that needs two: Spirit Swap (two minions, any minion, the Tavern's
      included; the attacks swap at once and come back at the next turn's
      start through `Minion.temp_attack`) and Devour (the sold minion, whose
      stats `Player.devour_stats` carries to the second pick, after
      `sell_minion` has paid out Gold and the sell triggers).
    - **Until the next turn.** `Minion.temp_reborn` (Reborn Rites) and
      `Minion.temp_attack` come off at the start of the player's next turn, on
      the board and in hand; a Reborn the card prints stays.
    - **A Discover that lands in the Tavern.** `Player.discover_into_shop`
      (slot + 1) sends the pick into that Tavern slot instead of the hand:
      Galakrond's Greed Discovers from exactly one Tier higher into its slot,
      and the pick lands in the Tavern, so it gets the Tavern's standing
      "this game" buffs like any card dealt there (Era, 2026-11-02). Its
      button asks only for a Tavern minion, so with every Tavern minion
      already at Tier 6 the press spends and nothing happens
      (`11_01_04_27_54-g2` turn 8). The old
      card stays on show in the slot until the pick lands (the game's own
      client keeps it there too); the pick then puts it back in the pool,
      buffs and all. Arcane
      Alteration replaces a card on the board or in the Tavern with a pool
      card of the same Tier, keeping the stats the old card had gained.
      Anything else that puts a card into the Tavern goes through
      `Player::apply_tavern_buffs` (the Refresh, Evolving Tavern, a
      Surprise Portrait's Elementals, a Fishbait), since the standing buffs
      belong to whatever sits in the Tavern.
    - **Hats and Tickets.** `Minion.hats` counts Dancin' Deryl's +1/+1 hats
      (already in the stats): a played minion gets one, a sold minion's move
      to a random friendly minion, and a triple adds the copies' up.
      `Minion.tickets` is 1 for a Tavern card carrying a Darkmoon Ticket, 2
      for one rolled for without getting one (so a card that stays put is not
      rolled again), and 0 once it leaves the Tavern (`add_to_hand` clears
      it). `Player.tickets` counts the Tickets taken; the third gives a
      Triple Reward spell whose Discover tier is the player's own
      (`Game::take_ticket`).
    - **What the bot sees of them.** `Minion.hats`, `Minion.tickets` and
      `Minion.frozen` are three new card-slot features (`SLOT_HATS`,
      `SLOT_TICKETS`, `SLOT_FROZEN`, `SLOT_FEATURES` 86 to 89), and
      `obs::power_progress` reports Tavern Lighting's level, Saturday
      C'Thuns!' repeats and Come One, Come All!'s Tickets in the power's own
      progress/left pair.
    - **The next warband at the start of the round.** `Game::pair` copies each
      player's opponent's board into `Player.next_board`, so I Spy (and the
      button's legality) reads the warband as it stood before anyone played.
      A replay has no pairing to copy from: the shop replay seats the warband
      of the opponent I last fought there, the closest board its file holds
      (docs/real-boards.md "The shop replay").
    - **A power that arms an end of turn.** `PowerState.armed` is set by
      Saturday C'Thuns!'s press and spent in `power_end_of_turn`, where the
      repeats are the power's own turn count. `HeroPower.swaps_to`
      (`swaps_each_turn` in heroes.toml) is how Major Hymn becomes Minor Hymn
      at the end of every turn, except in a Power of the Storm slot: the power
      Master Nguyen borrowed is handed back at the start of the next turn, and
      the game ends the turn still holding it (`11_02_00_10_22-g6` turn 6
      ends on Major Hymn).
    - `cast_spell` ends by dropping a target prompt nothing can answer, so a
      spell's own "after you cast" triggers can never leave the player
      stranded.
- **Hero batch 3 (2026-11-01)**, the fight, kill and rest powers
  (the rules are in docs/rules/heroes.md "Hero batch rules (Era,
  2026-11-01)"):
  - **A power's Start of Combat is a `Counters` field.** `Player::counters`
    fills `wingmen`, `wax_warband`, `all_will_burn`, `phylactery`, `glory_of_combat`,
    `sprout_it_out`, `frostwolf_fervor`, `stormpike_strength`, `tentacular`,
    `broodmother`, `minions_sold` and `count_damage` from the two power slots,
    and the fight reads them: Wingmen before every other Start of Combat
    effect (`run_start_of_combat`, a trinket's Boon of Beetles included),
    then the rest in the hero-power step. `Player::power_effect` skips a power
    that has not unlocked (Frostwolf Fervor's turn 7). A passive power's tick runs
    `tp.power_triggers` times (`Player::tick_power_effect`); a press that
    fights (Embrace Your Rage, I'll Take That!, Rapid Reanimation) arms the
    slot instead and uses `Player::armed_power_effect`, so it counts once per
    press. `PowerState.armed` is cleared at the start of every turn.
  - **Something waits for a board slot.** `Side::spawn_when_space` is the one
    place that summons what a power holds back, and `Side::resolve` calls it
    after every death: Rapid Reanimation's queued exact copies (`Side.reanimate`,
    in the slot the minion stood in), Lock and Load's stored minion (summoned
    then pushed on `Side.immediate`, so it attacks at once), Frostwolf
    Fervor's and Stormpike Strength's one copy of the highest Attack or
    Health (`Side.pick_highest`, ties at random), Tentacular's Tentacle, and
    Diremuck Forager's owed summons (`Side.waits_left`, from
    `Counters.waits`, the highest-Attack Murloc out of the hand through
    `Side::summon_from_hand`).
    `Side::exact_copy` is the copy: the minion as it stands, buffs, keywords
    and gifts kept, its damage carried over (a 10/10 at 4 Health copies as
    10/4, and `Side::summon_exact_copy` keeps the original's `taken` so its
    maximum Health stays 10), no pool copy (docs/rules/design.md "Exact copies").
    `Side::summon_saved_copy` is the one summon path for a saved copy, so
    Frostwolf Fervor, Stormpike Strength, Rapid Reanimation and Stitched
    Salvager all copy the same way.
  - **A power's token.** Menagerist's Amalgam, Avatar of N'Zoth's Fish and
    Pilot the Shredder's Sneed's New Shredder are cards.toml `[[token]]`s and
    a `start_minion = "Name"` power puts one on the board when the hero is
    taken (and when such a power is picked at turn 0). Heroes.toml writes it
    that way; the token takes no pool copy.
  - **A Deathrattle granted for the fight.** `Counters.phylactery` gives the
    lowest-Attack minion `GrantedDr::Phylactery` (Attack and maximum Health to
    its other minions when it dies); `Side.granted` holds every such grant and
    `fire_deathrattles` now fires all of them for a minion rather than the
    first, so two triggers of the same Start of Combat give two Deathrattles.
    A Fish of N'Zoth takes a granted Deathrattle too.
  - **A press with a guess in it.** Friendly Wager and Detective for Hire
    open a two-way choice: `Player.wager_offer` (the two seats) and
    `Player.holmes_offer` (the two card ids and which is right) are checked by
    `is_legal` and `Game::is_choice_open`, answered by `Action::Choose(k)` in
    `resolve_choose`, settled in `Game::settle_wagers` after the round's
    fights (3 Tavern Coins for the winner, `gold_next_turn` 1 for a tie) and
    read by the observation (`SCALAR_PENDING_WAGER`, `SCALAR_WAGER_OFFER`,
    `SCALAR_WAGER_PICK`, `SCALAR_PENDING_HOLMES`, `SCALAR_HOLMES_OFFER`, the
    "empty" flags). `Player.wagers` holds the unanswered guesses for the round.
  - **Counters that only a fight can grow.** `apply_side_outcome` feeds
    `power_attacks` (Demon Hunter Training), `power_friendly_deaths` (MechGyver,
    shop destroys included), `power_damage_dealt` (Bloodfury, from
    `SideOutcome.damage_dealt`, the damage friendly minions deal in the fight)
    and `power_broodmother_gain` (Onyxia's Whelps, `SideOutcome.broodmother_gain`).
    Bloodfury becomes Nether Portal at its milestone and the portal is a
    `random_minions_each_turn` power, so the Demons arrive in
    `power_start_of_turn`.
  - **Conviction's stances** are `[[power]]` entries in heroes.toml, listed in
    the power's `choose`, with `effect = { stance = [count, attack, health] }`.
    `Player.conviction` is what the three add up to; the offer opens in
    `power_start_of_turn` (turn 2 on, three options, United left out at 7
    minions) and `pick_power` adds a Stance instead of replacing a power.
  - **What a third power needs.** A power whose effect the pick must run at
    once is fired from `pick_power` when it came from Power of the Storm:
    Master Nguyen's slot (`Player.storm_power`) is re-offered every turn
    (`deal_power_offer` deals two options, same exclusions, never Power of the
    Storm back), and Unmasked Identity clears the flag.
- Bots answer the pick, a power Discover and the two guesses in
  `answer_pending`, and press a
  legal button (either one) with gold that would be left over (Gonna Be
  Rich! waits for turn 8). A skipped turn gets a Freeze and then an End Turn.
  `bgbench` and `bgpy` games play with heroes.

## Secrets

The Great Akazamzarak's Prestidigitation and the four combat Secrets
(docs/rules/heroes.md "Secrets", Era's rules). `src/secrets.rs` holds the zone:
the Secrets are neither pool cards nor minions, so there is no table for
them. There are four kinds (`VENOMSTRIKE_TRAP`, `AUTODEFENSE_MATRIX`,
`PACK_TACTICS`, `REDEMPTION`) and a fifth kind in the zone, the Ice Block a
Safety Patch gives.

- **The zone.** `Player.secrets` (`Secrets`) is five `u8` kinds and a length:
  the order they were got is the order they fire in, and nothing allocates. A
  Secret stays until it fires. `combat_len`/`all_combat` never count the Ice
  Block, which is why Prestidigitation never offers it and still works with
  the save in hand. The Ice Block keeps working exactly as it did from
  `Player.ice_block`: the fatal combat damage in `game/turn.rs` and
  `heroes.rs::take_hero_damage` ask `ice_block()` and remove the kind instead
  of clearing a flag; `SCALAR_ICE_BLOCK` is unchanged.
- **A fight gets the zone as counters.** `Player::counters` copies it into
  `Counters.secrets`, and `Side::new` copies it again into `Side.secrets`, so
  a fight keeps its own copy and pays one field test when it has none.
  `Side::fire_secret` removes a fired kind and sets its bit in
  `SideOutcome.secrets_fired`, and `apply_side_outcome` takes those kinds out
  of the player's zone for good after the fight.
- **The attack Secrets** run in `Side::attack_secrets`, called from
  `attack_swings_at` after the Rally and "whenever a friendly minion attacks"
  triggers and before the hit (so Autodefense Matrix's Divine Shield blocks
  it): Autodefense Matrix sets the keyword, Pack Tactics summons a copy of the
  attacked minion to its right with 3/3 base stats and Venomstrike Trap a 2/3
  Venomous Cobra there (`CardDb::cobra_token`, cards.toml's "Cobra" token).
  Both summons go through `Side::summon`, so the "wherever this is" bonuses
  land on the copy exactly as they do on any summoned minion: the Undead
  bonus, an Eternal Knight's deaths, Goldrinn's Beasts, a trinket's "your
  minions have". A Secret that would summon stays in the zone when the board
  is full.
- **Redemption** runs `Side::redemption` in `Side::resolve` after the dead
  minion's Deathrattles and its Reborn: `CardDb::make` of its base card, 1
  Health, summoned where it stood. With no room the copy waits in
  `Side.secret_wait` and `spawn_when_space` puts it down as soon as a slot
  opens; the Secret is only spent when the copy lands, and a death that comes
  while a copy is owed does not replace it.
- **Secrets are combat only.** `Side.outside` (a Tavern attack, a shop
  destroy) turns every Secret hook off: they are the client's combat Secrets,
  and a shop destroy is not a fight.
- **Prestidigitation** is `PowerEffect::DiscoverSecret`: free, once a turn,
  from turn 1, and `power_usable` refuses it while all four combat Secrets are
  up. A press fills `Player.secret_offer` (`ArrayVec<u8, CHOICES>`, the kinds
  still missing, all of them rather than the three of a Discover), answered
  with `Action::Choose` exactly as `power_offer` is, and `pick_secret` adds
  the kind at the end of the zone. `obs::power_progress` reports the Secrets
  held and the ones still missing.
- **The observation** carries the player's own zone only: `SCALAR_SECRET` is
  four flags (one per combat Secret) and `SCALAR_SECRET_OFFER` the kinds an
  open offer gives (kind + 1). An opponent's zone is written nowhere: the
  client hides it, and so does this.

## Trinkets

`trinkets.toml` (docs/rules/design.md "Trinkets and Dark Gifts (2026-10-30)") holds the
built trinkets: card_id, slot, price, the lobby tribes it belongs to and its
effects, as data (`crates/bgcore/src/trinkets.rs`: the engine never matches a
trinket by name). Only built trinkets are in the table, so the offer pool is
exactly what exists; a trinket's tribe must be in the lobby to be offered.

- **The slots.** `Player.trinkets` is `[TrinketState; 3]`: the lesser slot,
  the greater one, and the trinket a hero power becomes (Marin the Manager,
  Buttons). A `TrinketState` holds the trinket (index + 1, 0 empty), its "every
  N" counter, its uses left this turn and a `done` flag for a `once` rule.
  `Game::set_trinket(p, slot, Some(index) | None)` is the one way to fill a
  slot (the pick uses it, and so do tests and the sandbox): it sets the state
  and refreshes `Player.trinket_events`.
- **The offer.** `open_trinket_offers` runs in `start_turn`: at
  `trinket_lesser_turn` (6) for an empty lesser slot, at `trinket_greater_turn`
  (9) for the greater one, and at its `unlock_turn` for a power whose effect is
  `PowerEffect::TrinketOffer(slot)` (5 and 8). It deals `TRINKET_CHOICES` (4)
  distinct options into `Player.trinket_offer` as `(trinket, price)` pairs,
  with `trinket_offer_slot`, answered by `Action::ChooseTrinket(k)`.
  `deal_trinket_offer` does the guarantee (a tribe with
  `trinket_lesser_need`/`trinket_greater_need` minions on the board, Murlocs
  counting in hand too, gets one of its trinkets; one qualifying tribe at
  random) and the weights of `rules.toml` for the other three, and writes each
  price: a tribe trinket whose tribe falls short costs `trinket_discount`
  less. A trinket marked `out_of_pool` (patch 37.0.2's removals) is never
  offered, and one whose `banned_tribes` meets the lobby is left out
  (`replace_trinket_at` applies both too). `pick_trinket` pays the price (Gold, so "Gold spent" counters see it),
  fills the slot, starts an "every N turns" counter at 1 and runs `on_pick`. A
  power's pick consumes the power (`power_usable` is false afterwards). The
  offer can't be declined: `is_legal` blocks the shopping actions while it is
  open but lets Freeze and End Turn through, and `finish_shop` buys the
  cheapest option for a player who ends the turn with it open. A choice a
  trinket's own start-of-turn effect opened is answered first
  (`Game::other_choice_open`).
- **The hooks.** The lists are the ones in `trinkets.toml`'s header:
  `on_pick`, `start_of_turn`, `end_of_turn`, `on_buy`, `on_buy_spell`,
  `on_play`, `on_sell`, `on_refresh`, `on_cast_spell`,
  `on_cast_spell_on_minion`, `on_discard`, `on_gold_spent` (once per
  spending, whatever the amount) and `rule` (event, every, once, acts).
  A trinket has no host minion, so the frame calls
  `Game::trinket_event(p, event, card, amount)` where the card path already
  fires (see the list below). Each hook starts from the player's
  `trinket_events` mask: `Trinket::event_mask` (the events its lists and its
  rule listen for, `HAS_TRINKET`, `STARTS_TURN`, `ENDS_TURN`) is ORed over the
  slots by `Game::set_trinket`, so an event that no held trinket wants costs
  one `u16` test and never touches the shared card table. A matching hook
  copies each `Act` out of that table by index instead of cloning a `Vec`, and
  nothing in these paths clones the shared `Arc<Data>`: both were 3x of
  contention across threads. `check_invariants` re-derives the mask, so a
  slot written behind `set_trinket` is caught in tests. The call sites are: `Action::Buy` (Buy, BuySpell), `Action::Play` (Play, with the tribe
  filter), `sell_minion` (Sell), the Refresh sites (Refresh, the same places
  "after the Tavern is Refreshed this game" fires), `cast_spell` and
  `cast_spell_on` (SpellCast for every spell, CastSpell for a Tavern spell
  only: see "Spells versus Tavern spells" below),
  `trigger_trinket_on_cast_spell_on_minion` and the Tavern arm of
  `apply_target_shop` (CastSpellOnMinion, after the minions' own watchers for
  that cast and after the spell's own effect: a Honeycomb Ring's this-turn
  +1/+1 lands after the Misplaced Tea Set the cast made the Amalgam cast; one
  event per cast on either path, so a Hot-Air Surveyor's extra Gem cast counts
  twice), `discard_card` (Discard) and `fire_event(GoldSpent)`.
  The turn flow calls `trinket_start_of_turn` (offers, uses back, the Turns
  counter, the start-of-turn effects) and `trinket_end_of_turn` (the
  end-of-turn effects, then an "every N turns" rule that reached N). A hook
  list with `uses_per_turn` is capped per turn. Both kinds are end-of-turn
  effects, so a `drakkari_end` trinket's list and the payout of its rule run
  once more per Drakkari Enchanter (Goldenizer Supply with one on the board
  hands out two Goldenizers), and `run_end_of_turn` calls
  `trinket_end_of_turn` before the board's minions, the order the log shows.
- **A seated trinket** (a logged shop state, a real-board seat) never ran its
  pick. `Player::hold_trinket` puts the trinket in its slot and sets only what
  a fight reads (the power's extra triggers, the Hammer's numbers), because it
  has no `Game`. The shop replay then calls
  `Game::seat_trinket_pick_state`, which runs the `on_pick` acts that leave a
  rule standing through `run_trinket_act` (the code the pick runs, so the two
  cannot drift): a `CardMod`, Lovely Locket, War Drum, the Tavern's shape, the
  Gold cap, the offer turns, the standing counters. The acts the logged state
  already carries are skipped: cards, Gold, stats given once, the counters the
  file gives and the Secret zone. `Game::seat_trinket_act` is where that split
  lives, so a new `Act` goes on one side of it.
- **The effects.** `Act` mirrors the effect keys of cards.toml (`get`,
  `get_gems`, `cast`, ...) and `Game::run_trinket_act` runs each through the
  same `Effect`/`SpellEffect` arm the card path runs, with no host slot (`pos`
  0) and no golden multiplier. `Act::Cast` is the one exception: a trinket's
  spell is the player's cast, while the same `cast` key on a minion is the
  minion's (`cast_named_spell` with `by_minion` true), so it calls
  `cast_named_spell` itself.
  The keys with no card behind them are the ones trinkets.toml's header lists;
  `Act` names the card each was written for.
  The first five were `upgrade_discount`, `buff_tavern_this_turn` (Cursed
  Crystal's per-Refresh buff must not be remembered and stack),
  `make_golden_random`, `last_spell_copies` and `discover_deathrattle`
  (Deathly Phylactery; a
  `SpellEffect::Discover(DiscoverKind::DeathrattleMinion)`).
- **Getting cards (block 2).** `on_get` is the hook for "whenever you get a
  <tribe>": `add_to_hand` is the one place a card enters the hand (a buy, a
  generated card, a Discover, a copy), so it fires `Event::GetTribe(t)` there
  and a `get_tribe` filter names the tribe (Trusty Crowbar). The same place
  fires the card-side `EventKind::CardGot(t)`, which a minion's `on_event`
  counts ("Whenever you get 4 Pirates": Doubloon Grifter).
  `discover_tier` is a Discover of one fixed tier;
  `discover_remember` plus `get_remembered` are Portable Factory: the pick's
  card id is kept in `TrinketState.card` (written in `resolve_choose`, which
  also gives Sinstone Sticker's extra copy for the first N Discovers of a
  turn, `tp.discover_copies`, refilled from `discover_copies_base` at the
  start of each turn). `copy_buys` is Transcribing Typewriter's count of
  exact copies of the next minions bought, spent in `Action::Buy`.
  `start_every` runs a start-of-turn list only every N turns (Azeroth Model
  Globe, Lens Case, Conch Portrait); `rule = { event = "turns", every = N }`
  still fires at the end of the turn it reaches N. `warband_whistle` makes the
  next Refresh hold plain copies of the warband (`fill_shop`),
  `triple_need_two_pirate` lowers a Pirate triple to two (`check_triple_tracking`),
  `magnetize_ends` Magnetizes a card to the two ends of a tribe's minions
  (Accord-o-Tron Portrait) and `lockbox_sooner` takes turns off every Lockbox
  the player gets.
- **Spellcraft (block 3).** A Spellcraft spell is a `[[spell]]` with
  `spellcraft = true`, `token = true` and `tavern_spell = false`: a plain
  spell like a Blood Gem (type SPELL in the game data, so "after you cast a
  spell" counts it and "Tavern spell" counters don't) that leaves the hand at the end of the turn if it was
  not cast, dropped in `finish_shop` without firing a discard. The trinket
  hands it out with `on_pick` and `start_of_turn` `get` lists, so it comes
  back every turn. Floating Candle Set's Spellcraft spell is the "Floating
  Candle" token (`cards.toml`: `target = "chosen", trigger_deathrattle`): it
  fires a friendly minion's Deathrattle outside combat through
  `fight::shop_trigger_deathrattles`, with the minion standing and the summon
  landing where it stands.
- **The Tavern's shape (block 3).** `Player::shop_size` is the one Tavern
  size: `rules.shop_size(tier)`, or six with Glowing Gauntlet, plus one for
  Innkeeper's Stein (`extra_high_tier`: a minion of the tier above, none at
  tier 6) and one for Electrode Attractor (`extra_magnetic`), never past
  `SHOP - 1` minions because the seventh slot is the spell (docs/rules/design.md).
  `fill_shop` draws the extra slots and takes its tier bounds from
  `tavern_tiers` (Bob-blehead's `min_tier`, Guiding Candle's
  `tier6_refreshes`, which the turn-start roll counts down too when the
  Tavern changed). The spell slot is the Tavern's last card: `fill_shop`
  deals it whenever fewer than `SHOP` cards are there, so a fill that takes
  all seven leaves no spell. The spell also follows the tier range where the
  rules name one: a Guiding Candle Refresh deals a Tier 6 Tavern spell
  (`random_tier_spell`), every other Refresh one up to the player's tier. A
  Refresh with more Demon Fodders than the
  tier's slots grows the Tavern to hold them all (six Fodders at tier 4 put
  the spell seventh, seven take every card), and each Fodder's refilled slot
  draws inside the same tier range as the Refresh, Tier 6 alone during one of
  Guiding Candle's (`feed_fodders`, given the range `fill_shop` drew in).
  A Fodder feeds once the Refresh is over, never inside its fill: the
  caller (`Action::Roll`, or a hero power's own Refresh) runs
  `feed_refresh_fodders` after the "after the Tavern is Refreshed" buffs, so
  the Fodder carries Waveling's amount, the Blood Gem Barrage and a
  trinket's `on_refresh` buff into the Demon it feeds
  (log 2026_10_27_04_30_04-g3 turn 8 step 2: a 2/2 Fodder feeds as 8/8,
  its own stats plus the Barrage's 6/6). The queue's own number for one Refresh
  stops at `SHOP`: six queued and three more show as 7, not 9
  (`add_fodder_to_refresh`, log 2026_11_01_17_59_13-g1 turn 9 step 16). The Warband Whistle and Bob's Burgles
  (`burgle_last_opponent`) hold one plain copy per warband minion the same
  way (docs/rules/design.md "Demon Fodder", 2026_11_01_04_27_54-g2 turn 11,
  2026_11_02_00_10_22-g7 turn 8 step 40). `Game::refill_tavern` runs after
  every action and tops a Gauntlet Tavern back up: a bought, consumed or
  stolen minion by a minion, the spell by a spell. `minion_price` reads the
  set prices (`magnetic_cost`, `tavern_price`).
- **The hero power (block 3).** `tp.power_triggers` runs a press's effect
  twice and a passive power's tick twice (one payment); `power_extra_uses`
  raises the per-turn uses (`power_start_of_turn` adds it to both held
  powers, the pick adds one at once); `power_gold` pays Gold after each press.
  The `uses_left` invariant allows the extra uses, on either power.
- **The Mystery Cube.** Its effect belongs to the slot: `TrinketState.cube`
  (plus `cube_choices`) is set by the `offer_replace` act and kept when
  another trinket takes the slot. `Game::deal_cube_offer` deals the two free
  Lesser options, never the trinket the slot holds and never one marked
  `cube = false` (the one-time effects and the self-replacing ones);
  `refresh_trinket_mask` then gives the slot `STARTS_TURN`, so
  `trinket_start_of_turn` runs the held trinket's own start-of-turn list
  first and the new offer after it. An offer that has to wait for another
  one (the turn 9 Greater offer) is remembered in `tp.cube_pending` and dealt
  when that one is answered.
- **In a fight.** A trinket has no host minion, so its in-fight effects are a
  bundle the fight owns: `Counters.trinkets` is a `FightTrinkets`
  (`crates/bgcore/src/fight.rs`), `Copy`, allocation-free, with no pointer
  into the shared table, built by `Player::counters` (`fight_trinkets`) from
  the three slots in order. `trinkets.toml` holds the vocabulary as data
  (`fight = [{ when = ..., ... }]`): `when` is `soc` (Start of Combat, the
  trinket step, lesser before greater, before any minion's own), `avenge`
  (`every = N` friendly deaths this fight), `summon`, `attack` (once per
  swing), `death`, `reborn` or `awaken` (the Deity). `tribe`/`neutral`/`rally`
  filter the minion a trigger is about; `pick` (`all`, `ends`, `random`,
  `lowest_attack`, `rally`, a bare `card`) picks the minions a Start of
  Combat or Avenge effect reaches. `cap` is a per-fight use limit, `once`
  fires it once, `grow` adds the trinket's own "improve this" number, and
  `improve` raises that number for good. The hooks are `Side::trinket_hook`
  at the fight's own points (`run_start_of_combat`, `Side::resolve` for
  deaths/Avenge/Reborn, `summon_with`, `attack_triggers`, `try_awaken`), and
  a side without trinkets answers each of them from one `u8` (the bundle's
  length). A Deathrattle a trinket grants for the fight (Hoggy Bank, Sky
  Golem Portrait) is kept by uid in `Side.granted` and fires with the
  minion's own in `fire_deathrattles`; `first_deathrattle` (Deathly
  Phylactery) adds one run in `deathrattle_fires`, combat only.
- **What comes back.** `SideOutcome` carries the fight's trinket gains the way
  every other fight gain travels: `gem_gain` and `spell_bonus_gain` (Avenge
  rewards), `free_refresh_gain` (Herding Horn), `trinket_improve` by slot
  ("improve this"), and `friendly_deaths`, `death_tribes` and `summon_tribes`
  for the shop-side "after N friendly minions die / Beasts die / Beasts
  summoned" counters.
  `apply_side_outcome` applies them and fires `trinket_event` with the new
  shop events `Event::Death` (and `death_<tribe>`) and `Event::Summon` (and
  `summon_<tribe>`), so those counters count fight deaths, fight summons and
  shop destroys (docs/rules/design.md: shop deaths count for death counters) in one
  slot counter that carries across turns. A shop destroy counts deaths but
  not Avenge: a trinket's Avenge is Avenge, and its counts start from 0 in
  every fight.
- **Cards a fight gives the hand.** `SideOutcome.cards` (a Rally's Beast, a
  Deathrattle's card, GetLockbox's Lockbox) is handed over in
  `apply_side_outcome` through `give_generated`, and the trinket and hero
  power hooks it fires there (those same "after N deaths / summons"
  counters, MechGyver's deaths, Friendly Wager's coins) hand theirs over
  through `add_to_hand` like every card. Nothing triples during a fight
  (docs/rules/design.md "Nothing triples during combat"), so `Game.defer_triples`
  is set while that carry-over runs: `add_to_hand` records each minion card
  on `Player.triples_next_turn` (`defer_triple`) and `check_triple_tracking`
  merges nothing. `dark_gift_start_of_turn` runs those checks as the next
  turn starts, so the golden lands in the hand in the shop phase, and the
  golden's own Triple Reward follows from playing it as usual.
- **Block 4: the player-level hooks.** The frame grew the events this block
  needed instead of a field per card: `on_card_play` (any card played from the
  hand, a Blood Gem or a Banana included), `on_battlecry` (once per Battlecry
  trigger, so Brann's double fires it twice), `rule` on `play_battlecry` (a
  minion with a Battlecry played), `on_gem` and `rule` on `gem` (a Blood Gem
  cast from the hand), `on_attack` and `rule` on `attack` (a friendly swing in
  a fight, once per swing, so Windfury counts 2: `SideOutcome::friendly_attacks`),
  `on_destroy` and `rule` on `destroy` (`Game::destroy_in_shop`, one event per
  dead minion, `destroy_card` narrows it to one card), `on_magnetize`,
  `on_cast_spell_on_shop` (with `tp.shop_hit` naming the slot the spell hit),
  `on_consume` and `rule` on `hero_damage`. `death_card` narrows a death
  counter to one card, `SideOutcome::death_cards` brings those deaths home
  (the bulk `friendly_deaths` fire carries no card, so a plain death counter
  never counts them twice), `alternate` makes the two buy lists take turns
  (`TrinketState.value` is the step, Trickster's Sleeve) and `drakkari_end`
  runs the end-of-turn list once per Drakkari. A Fishbait that dies in a
  Tavern attack is a Tavern card with no side of its own, so
  `fight::tavern_attack` reports that death with the attacking side's
  `death_cards` (Snarky Portrait counts it like a friendly death).
  `Player.trinket_events` is a `u32` for them; the mask test is still the one
  thing every hook pays.
- **Counting Battlecries (docs/rules/design.md "Counting Battlecries").** One place
  counts a Battlecry's triggers: `battlecry_or_choose_one`, through
  `Game::count_battlecries`, which adds `runs` (the trigger count just below)
  to `Player.tp.battlecries` and `Player.battlecries_triggered` and grows
  every "for each Battlecry" minion by that many stacks: Dark Paradox t6,
  Murky Sticker's `buff_leftmost_two_per_battlecry` and the Battle Scars
  gift. Only a Battlecry counts: a minion without one counts nothing when
  played, and a Choose One minion counts nothing (its branch returns first).
  A Battlecry that leaves a choice open (a target, a Discover, a Choose One)
  counts when the pick is made, as the log shows (17_59_26-g1 turn 8: the
  Tavern's Paradox grows at the Discover pick, not on the play), so what it
  owes waits in `Player.pending_battlecries` and `maybe_start_discover` adds
  it once nothing is left to answer. `Game::battlecry_triggers` says, read
  only, what a Battlecry of a card would trigger now; the shop replay uses
  it to pay a pick step what the play step before it owed.
- **Battlecry multipliers (block 4).** One place counts a Battlecry's
  triggers: `battlecry_or_choose_one`. It is Brann's `brann_mult` (set or add)
  plus `tp.war_drum` (War Drum's two extra, spent from that trinket's own
  `uses_left` by `take_war_drum`) plus `tp.dragon_battlecry` for a Dragon
  (Dragon's Eye). `Game::run_battlecry_at(p, pos, random)` triggers a
  Battlecry outside a play (Young Murk-Eye Sticker, Hackerfin Portrait, a Dark
  Gift's Echoing Voice), and
  with `random` it answers the target the Battlecry asked for at random out of
  the board and the Tavern (`answer_target_at_random`,
  `valid_battlecry_shop_target`: only a plain "a minion" buff reaches the
  Tavern that way). A Discover it opens gives a random card too, with no
  Discover offered (see "A Discover a Battlecry that was not played opens").
  A Battlecry never panics on a slot that a hook before it
  moved: the lookup is a `get` (2026-10-31).
- **Trinkets that cast and trigger (block 4).** `Game::cast_spell_on` casts a
  named card on a given friendly minion as a real cast (Emergency Gearblade,
  Spell-powered Wrench: Repair Job). `trigger_one_deathrattle` and
  `trigger_deathrattles_in_shop` fire a friendly minion's Deathrattle outside
  combat through `fight::shop_trigger_deathrattles` (a one-sided `Side`,
  `outside = true`, Avenge off), so summons land where the minion stands and
  the gains come back through `apply_side_outcome` (Wolfhead Flail, Floating
  Candle Set). The Lovely Locket hangs off `apply_target`'s spell arm: after a
  spell cast on a friendly minion, the same `PendingTarget` runs once on
  another friendly minion picked at random, ignoring the spell's own target
  rules, guarded by `tp.locket_copy` so it never chains. The copy is the
  trinket's cast, not the player's, so the minion it lands on answers with
  none of its own "whenever you cast a spell on this": while
  `tp.locket_copy` is set `apply_target` skips
  `trigger_on_cast_spell_on_watchers` and `trigger_on_cast_spell_on_mech`.
  It fires no "after you cast a spell" half either (`owe_after` is false
  while `tp.locket_copy` is set), though a Tavern spell's copy still fires
  the board's "whenever you cast a Tavern spell" cards. Read from the raw log
  (`Hearthstone_2026_11_02_00_10_22-g2`, Power.log): turn 12 steps 24 to 31
  has three Hot-Air Surveyors, so each Gem play is six casts (six POWER
  blocks of the hand's Gem, 772266 onwards) and the Locket fires on every
  one of them, six copies per play. Every real cast has its two Glambot
  triggers before its own POWER block (772280 and 772634, then the cast at
  772821); a copy has none: the two that land on the board's second Glambot
  (772873, 773521) give it +1/+1 each and no Satellite, and the pair after
  them (772939, 773293) belongs to the next cast. Not one of the game's 140
  copies spawns a trigger block (no task list has a copy as its parent), and
  turn 9 shows the "after" half too: Felboar fires 7 times for the turn's 7
  real casts and none for its 6 copies.
- **Spells versus Tavern spells (`cast_spell`, fixed 2026-10-31).** Two
  events, and every card or trinket that reacts to a cast is on the one its
  own words name (docs/rules/design.md "Read the card's exact words, every time"):
  `Event::SpellCast` fires for every spell cast (a Tavern spell, a Blood Gem,
  Gem Day, a Banana, a Spellcraft spell), `Event::CastSpell` for a
  `tavern_spell = true` card only. The trinket lists are `on_any_spell` and
  `on_cast_spell`; the rule events are `spell_cast` and `cast_spell`. A
  Triple Reward is "a 0-cost plain spell (not a Tavern spell)"
  (docs/rules/design.md "Triples"), so its cast is a spell cast and not a
  Tavern spell (fixed 2026-11-04): it fires `EventKind::AllSpellsCast` (the
  Felboar's counter and its consume) and `Event::SpellCast`, and never
  `EventKind::SpellsCast`, `Event::CastSpell` or `Card.on_cast_spell`. The
  raw log has it twice: `Hearthstone_2026_11_02_00_10_22-g4` turn 8, the
  reward played at Power.log 1128876 with the Felboar's TRIGGER at 1129386,
  and `Hearthstone_2026_11_03_15_22_36-g3` turn 8, where a Timecap'n Hooktail
  on the board (Tavern spells only) never fires. The minion side: `Card.on_cast_spell` stays Tavern-only (every
  card that has it says "Tavern spell"); `EventKind::SpellsCast` and
  `Player.spells_cast` (the observation's "Tavern spells cast this game", and
  Faceless Converter's per-Tavern-spell improvement) stay Tavern-only, and
  `EventKind::AllSpellsCast` is the "after you cast N spells" counter (Felboar)
  with no player scalar. `on_cast_spell_on_self` and `on_cast_spell_on_tribe`
  were always any-spell (they fire from `trigger_on_cast_spell_on_watchers`).
- **The two halves of a spell-event trigger.** A card's `on_event` for a
  spell event fires either as the spell is cast ("whenever", `TriggerWhen::Now`)
  or once it has resolved (`after = true` in cards.toml,
  `TriggerWhen::Resolved`). `cast_spell_as` fires the "whenever" half once per
  cast (the first at the play, the rest in `apply_target`'s cast loop, so a
  Balinda double cast sees two) and the "after" half once per cast too, at
  the end of the cast it belongs to: `apply_target`'s cast loop fires it when
  that cast left no prompt open, so a three-cast Gem play (Balinda and a
  Hot-Air Surveyor) gives Felboar its third count, and eats the Tavern minion
  with that cast's Tavern buff on it, between the play's first and second
  casts (log `2026_11_02_00_10_22-g4` turn 9 step 39). Whatever a cast leaves
  owed (a Discover it opened, a choice, a target) is fired by
  `fire_cast_after_choice`, which `maybe_start_discover` calls once no prompt
  is left: a spell with no target whose own effect opens a Discover keeps its
  "after" half until the card is picked too (Fandral's Fortune, Hired
  Headhunter: log `2026_10_24_17_59_26` turn 10, the pick at line 252294 and
  the Felboar's consume only at 252362). A Choose One spell's "after"
  half waits the same way: its option and
  its target come first (docs/rules/design.md "Eating a minion with a
  spell"). Once that target is picked, the casts of the option are the
  play's casts: the "whenever" half of the first one fires then (it was
  owed to the choice, `apply_target` fires it before the first run) and
  every cast's "after" half follows its own run in the loop, so a golden
  Balinda's three casts of Alliance Flag give a Felboar three counts and
  let it eat the Tavern minion with only the first cast's Tavern buff on it
  (log `Hearthstone_2026_11_02_00_10_22-g4` turn 14 step 24: three POWER
  blocks, both Felboars' "After you cast 3 spells" counters one per block).
  `CastAfterChoice::casts` is that many, not the one cast the play counted.
  The Dark Gift count is on that half too (`CastAfterChoice::count_owed`,
  the `buff_per_tavern_spell` gifts Spell Siphon): the game grows those
  minions once the spell has resolved, so a Discover spell's pick comes first
  (log `2026_10_27_04_30_04` turn 7: Planar Telescope cast at Power.log
  68792, the Discover picked at 69231, the Spell Siphon tag at 69283). The
  "whenever" half stays on the play in both cases, and for a Triple Reward it
  does not: the game takes that whole cast at the pick, so its
  `CastAfterChoice` is seated with `at_play: false`
  (`Hearthstone_2026_11_02_00_10_22-g4` turn 8: the reward played at 1128876
  moves no counter, and the pick at 1129386 carries the Felboar's own and its
  consume together). `Card.on_cast_spell_after`
  is the same split for a minion's own "After you cast a Tavern spell"
  (Forsaken Weaver): `trigger_on_cast_spell(p, TriggerWhen::Now)` fires the
  `on_cast_spell` cards as the spell is cast, the `Resolved` call fires the
  `on_cast_spell_after` ones with the rest of the "after" half. The shop
  replay logs a pick as its own step, so the step that answers it loads a
  state with nothing owed: `shop_replay::seat_cast_after_pick` seats the
  cast's `CastAfterChoice` from the step's `source` spell again (for a Triple
  Reward's pick, `step.kind = "triple_reward"`, both halves). In
  a fight, `Effect::CastSpell` fires the Tavern counter and the Tavern-only
  minion triggers only for a Tavern spell, and the trinket's `when = "cast"`
  hook for every spell. Replica Cathedral's "your first spell each turn casts
  an extra time" covers any spell, a Blood Gem included (`gem_casts`).
- **The order inside one cast (2026-11-04).** Two things the logs pin down:
  the target's "whenever you cast a spell on this" watchers run before the
  spell's own effect, and a Tavern buff that effect gives lands after the
  cast's "after you cast" triggers. Everything else stays where it was, the
  "after" half included: it still fires once the cast has resolved, and still
  waits for a pick the cast opened.
  - **The target's watchers come first.** `apply_target`'s cast loop calls
    `trigger_on_cast_spell_on_watchers` (the `on_cast_spell_on_tribe` watchers
    and the target's own `on_cast_spell_on_self`) before the spell's own
    effect, so a Tavern buff the target's watcher gives is on the Tavern's
    minions when the effect picks among them:
    `Hearthstone_2026_10_24_15_25_26-g4` turn 11 step 23, a
    Methodical Madness on a Devilish Distractor (Power.log 1220476), where the
    Distractor's TRIGGER block and the Tavern's +1/+2 stat pushes come first,
    the spell's POWER block then consumes at those buffed 77/80, and the log's
    Distractor gains +117/+121 rather than the +77/+79 of the pair's pre-spell
    stats. The two spells that eat or destroy their own target (Channel the
    Devourer, Butchering) run it themselves in `fire_cast_triggers_early`, with
    the board's "whenever you cast a spell" cards, before they eat
    (docs/rules/design.md "Eating a minion with a spell"). A spell that moves
    its target off the board (Repeat Customer, Raise the Stakes) fires the
    watchers too, now that they come before the effect. After the effect
    `trigger_trinket_on_cast_spell_on_minion` fires the Honeycomb Ring class
    of trinket, as `Hearthstone_2026_11_03_15_22_36-g4` turn 10 step 27 shows
    (the Gem's stats on the target, then the trinket).
  - **The spell's own Tavern buff comes last.** A Tavern buff the spell's own
    effect prints ("give minions in the Tavern +2/+2 this game",
    `SpellEffect::BuffTavernPermanent`, and Eonar's Favor's
    `SpellTarget::TavernBuffType`) goes through
    `Player::add_tavern_buff_late`: the buff is held in `Player.late_tavern`
    and `Player::flush_tavern_buffs` puts its stats on the Tavern's minions
    once the cast's "after you cast" triggers have run (end of `cast_spell_as`,
    end of `fire_cast_after_choice`, and once per action in `step`). So a
    Felboar eating for the same cast takes its minion at its pre-spell stats:
    `Hearthstone_2026_10_28_22_11_50-g4` turn 11 step 41 (Power.log 1946359),
    where Staff of Enrichment's POWER block sets the Shop Buff to 22/22 for six
    Tavern minions, the Felboar's TRIGGER block then consumes the Tichondrius
    at 52/46 with no buff on it, and only then do the other Tavern minions get
    the +22. A Tavern buff a trigger of the same cast gives (the Distractor's
    "whenever you cast a spell on this", `Effect::TavernBuff`) is not late: it
    lands at once, which is what the target's watcher above reads. An "after
    you cast" trigger cannot simply be moved before the effect instead: a
    Blood Gem's own stats on a Tavern minion do reach a Felboar eating for the
    same cast (`felboar_eats_the_tavern_minion_the_same_gem_buffed` in
    tests/quilboar.rs, log `2026_11_02_00_10_22-g4` turn 9 step 39), so only
    the Tavern buff waits.
- **A spell a minion casts for you is not your cast (Era 2026-11-02).**
  `Effect::CastSpell` (Gatekeeper Amalgam's Misplaced Tea Set, an Air
  Revenant's Easterly Winds, a Crimson Vindicator's Rally spell) and
  `Effect::CastTaughtSpell` (a Magicfin token's spell) go through
  `cast_named_spell` with `by_minion` true; `Game::cast_spell` stays the
  player's own cast.
  The spell still runs and buffs as printed, spell power included, but none of
  the "spells you cast" bookkeeping is its: no `EventKind::AllSpellsCast` or
  `EventKind::SpellsCast` (so no Felboar count, no `spells_cast`), no
  `spells_cast_turn`, no trinket `spell_cast` or `cast_spell` event, no
  `trigger_on_cast_spell`, `trigger_on_cast_spell_on_watchers` or
  `trigger_on_cast_spell_on_mech`, so neither Timecap'n Hooktail nor the
  Amalgam's own "on this" fires for the Tea Set. The flag travels with the
  prompt a taught spell can open (`PendingTarget.by_minion`,
  `QueuedOption`), so `apply_target` and `apply_choose_option` also skip the
  extra-cast counters and the "cast on a Tavern minion" trinket events for
  it. The player's own extra casts (Balinda, a Hot-Air Surveyor's extra Gem
  cast) are the player's and do not reach a minion's cast either:
  `cast_spell_inner` gives a `by_minion` cast one cast, whatever Balinda or a
  Surveyor would add, and the log's Magicfin Apprentice casting Fortify with
  Balinda on the board gives +3 Health and not +6
  (`Hearthstone_2026_11_03_00_29_06-g1` turn 11 step 17). A trinket's own `cast` act is the player's cast: `Act::Cast` calls
  `cast_named_spell` with `by_minion` false, as does `cast_spell_on`; the
  Lovely Locket's copy inherits the flag of the cast it copies. In a fight the
  same `Effect::CastSpell` keeps its own `Side` counters.
  **An Activate's leftover Gems are casts (`Game::cast_gem_on`).** Victorious
  Geomant's "Get 6 Blood Gems. Cast any that don't fit on your left-most
  minion" casts them: the player's own Activate pays for them, so
  `Effect::GetGemsCastOverflow` goes through `cast_gem_on` for each one that
  finds no hand slot, which fires the "whenever" and "after you cast a spell"
  halves and the target's own "whenever you cast a spell on this" and
  "on a Mech" watchers. The log's golden Geomant casts four on a Gatekeeper
  Amalgam and the Amalgam's four Misplaced Tea Sets run, so it gains +76/+124
  and not the Gems' +56/+104 (`Hearthstone_2026_10_24_17_59_26-g1` turn 12
  step 40). They are not Gems played from the hand, so a Hot-Air Surveyor adds
  no cast and no from-hand bonus reaches them: one cast each. They do target a
  friendly minion, so a Balinda Stonehearth casts each twice (golden: three
  times): the plain Geomant of `Hearthstone_2026_11_03_15_22_36-g4` turn 12
  step 8 gives the golden Bramble Tunneler +2/+4 with a Gem worth +1/+2, and
  the golden Sanguine Refiner of `Hearthstone_2026_10_28_22_11_50-g1` turn 12
  step 22 takes +40/+72 with a Gem worth +20/+36. Each is a cast of its own,
  its "after you cast a spell" triggers included (that step's Felboar counts
  them apart).
  **A Choose One minion's play casts nothing.** The option a minion's Battlecry
  picks is not a spell at all, so `apply_choose_option` fires
  `trigger_on_cast_spell` only for a Choose One whose `source` is none (a
  Choose One spell the player cast); the option's own numbers still get the
  Tavern bonus. The log has it both ways: a Crater Miner's, a Fearless
  Foodie's, a Sly Infiltrator's and a Snare Trapper's play give a Vicious
  Mindslasher and a Timecap'n Hooktail nothing
  (`Hearthstone_2026_11_03_00_29_06-g1` turn 8,
  `Hearthstone_2026_10_30_19_45_15-g3` turn 7 step 9), while the Blood Gem
  Barrages a Veteran Brigand's own option casts still count, three of them
  (`11_03_00_29_06-g1` turn 8 step 29: +3/+6).
  **A spell a minion casts gets no Choose One choice (Era 2026-11-04).** The
  Choose One option of a taught spell is picked for the player at random
  (`cast_spell_inner`, which never opens a `PendingChooseOne` for a minion's
  cast) and each cast picks its own, so Brann's two Battlecry triggers make
  two separate random casts. A Discover it opens is a normal Discover: it is
  offered to the player like any other and counts for "after you Discover"
  cards. The log shows the game doing exactly that
  (`Hearthstone_2026_10_28_02_28_32` at 02:41: a Magicfin token's Boundless
  Potential offers three Tavern spells and Era picks one). A target the spell
  needs is still the player's pick, and every cast of the one taught spell
  lands on it (the Tavern slots included, see `resolve_shop_target`).
- **A repeated targeting spell or Battlecry keeps the one target.** Every
  "how many times" count is `cards::how_many_times` ("Set versus add"), and a
  spell or Battlecry that asks for a target asks once: the repeats land on
  the minion picked for the first (docs/rules/design.md "Set versus add", Era
  2026-11-04). A played minion's targeted Battlecry does this: `run_effect`
  puts the count in `PendingTarget.runs` and `apply_target` runs the effect
  that many times on the one pick. A named spell does it through
  `cast_named_spell`: it passes the casts it still owes to `cast_spell_as`
  (`repeats`), and `cast_spell_inner` folds them into the prompt's `runs`
  beside Balinda's own count, so a Battlecry doubled by Brann asks for one
  target and casts the spell twice on it. A spell that removes its target
  stops the loop (`stop_after`).
- **The repeat queue.** What a repeat still owes after its first run asked a
  question waits in `Player.repeats`, a fixed `ArrayVec<QueuedRepeat, 8>` (no
  heap), and `Game::run_queued_repeats` runs one run at a time. An entry is
  either a `RepeatWhat::Battlecry` (that minion's Battlecry triggered again:
  a golden Kelp Keeper, Drakkari on Young Murk-Eye's neighbours) or a
  `RepeatWhat::ChooseOne` (the option the player picked, or a new random one
  for each cast a minion makes). The first run of an entry asks the player,
  and its answers are kept on the entry: `resolve_target` (a board slot) or
  `resolve_shop_target` (a Tavern one) writes the target into
  `QueuedRepeat.at`, and `resolve_choose` writes the Choose One option into
  `QueuedRepeat.option`. Every repeat lands on them, and one whose own
  Battlecry asks for a target the first run never picked answers at random
  (`answer_target_at_random`). `QueuedRepeat.random_discover` marks the
  entries a Battlecry that was not played makes (a Kelp Keeper's Activate, an
  end-of-turn trigger; see "A Discover a Battlecry that was not played opens"
  below): the Discover requests such a run queues go into
  `Player.repeat_discover`, and `maybe_start_discover` answers each with a
  random card of the offer instead of showing it, `Player.discover_silent`
  keeping `fire_after_discover` from firing. A different minion's Battlecry has its own
  entry with its own first pick, so a second neighbour's prompt never replaces
  the one before it. `maybe_start_discover` runs the queue once nothing else
  is open, before the "after" triggers of the Battlecry or cast, and
  `Player.repeating` keeps the driver from re-entering itself when it answers
  a prompt. Cleared with the other pending choices at the end of a turn, in
  `fizzle_stuck_choice` and by the replay's state loader.
- **A Discover a Battlecry that was not played opens is random and silent.**
  A Battlecry an Activate, a hero power or an end-of-turn trigger runs is not
  a Battlecry you played, and a Discover it opens is answered for you: Kelp
  Keeper's Activate on a Rodeo Performer, and Young Murk-Eye, its Sticker, a
  Hackerfin Portrait or Echoing Voice running one at the end of the turn, give
  a random card from that Discover's pool with no Discover offered, and that
  card is not a Discover for "after you Discover" cards (docs/rules/design.md
  "Shop replay answers" and "Set versus add", Era 2026-11-03 and 2026-11-04).
  `Game::run_queued_repeats` counts the requests such a run queues
  (`QueuedRepeat.random_discover`, then `Player.repeat_discover`), and
  `Game::run_battlecry_at` counts them the same way for the triggers that do
  not go through the queue, holding the driver off (`Player.repeating`) until
  the run has asked and answered its target. `maybe_start_discover` picks one
  of the offers with the RNG, and `Player.discover_silent` stops
  `fire_after_discover`, so Hooktusk and Master Marauder do not react. A
  Battlecry you play yourself, Brann's double included, keeps its normal
  Discover, and so does a Magicfin token's taught spell when the player plays
  the token. The counting takes every request the run queues, a taught spell a
  token casts included: docs/rules/design.md "Set versus add" makes a taught
  spell's Discover a normal pick, while an end-of-turn trigger's Discovers are
  folded into the turn's own step (a Rodeo Performer's two picks with Echoing
  Voice, `Hearthstone_2026_10_24_17_59_26-g1` turn 8; a Magicfin token's
  Boundless Potential at `Hearthstone_2026_10_28_02_28_32` 02:41:12, inside a
  Young Murk-Eye trigger), so the replay has no step for the pick either way
  and the trigger's Discovers all go random (open question for Era: whether
  the taught-spell rule should win there).
- **A repeated Choose One remembers the pick.** The player picks the option
  and its target once, and every cast after the first uses the SAME option on
  the SAME target (docs/rules/design.md "Set versus add", Era 2026-11-04).
  `resolve_choose` writes that option into the queue entry's `QueuedRepeat.option`
  beside the target `resolve_target` writes into `at`, and
  `run_queued_repeats` takes it from there (`Game::repeat_choose_option`);
  only a cast with no pick of its own (a spell a minion casts) chooses at
  random. A Battlecry's repeats are the ones that reach a Choose One spell
  (`cast_spell_inner`): `PendingChooseOne.repeats` is how many casts the one
  choice stands for, each of them counted as a cast in `cast_spell_as`
  (`CastAfterChoice.casts`), and the runs themselves are a
  `RepeatWhat::ChooseOne` entry in the queue. Balinda is not one of them:
  `apply_choose_option` runs the option she doubles again, the same option on
  the same target, which is what the log shows: turn 12 step 0 of
  `Hearthstone_2026_11_02_00_10_22-g4`, Alliance Flag cast twice with the
  second option on one minion and one "spells cast". A Choose One minion's
  choice is its Battlecry, so a trigger that repeats it (a golden Kelp Keeper)
  goes through the same queue and the same remembered option. A Choose One
  played from the hand is not a Battlecry and Brann never repeats it
  ("Counting Battlecries").
- **Making a minion Golden happens once.** A repeat still counts as a trigger
  or a cast, but it makes no second minion Golden (docs/rules/design.md "Set
  versus add"). So `Effect::MakeGoldenChosen` puts only the card's own count
  in `PendingTarget.runs` (`count` times `mul`, so a golden Elite Navigator
  picks two Pirates), never the Battlecry's repeats, and Reno Jackson's power
  offers one pick whatever made it trigger twice. A
  `SpellTarget::MakeGolden` cast twice (Eyes of the Earth Mother with
  Balinda) is guarded by `!m.golden`, and the second cast still counts.
  `golden_up` (game/shop.rs) is the one way a minion turns Golden where it
  stands, and every maker calls it: Golden Touch, Eyes of the Earth Mother,
  Captain Sanders, Elite Navigator, Gilding, the trinkets and Raise the Stakes.
  It adds the golden card's printed stats and the second share of the counters
  the card scales with "wherever it is" (the same `Player::wherever_stats`
  `Player::make_card` applies, read with `mul` 2), and keeps the minion's
  buffs, which do not double. The logs say both halves: a Tavern Dark Paradox
  t6 with five Battlecries behind it goes 18/18 to 36/36
  (`Hearthstone_2026_10_24_17_59_26-g1` turn 8), a Tavern Banana Slamma at
  51/51 (its own 3/6 plus +48/+45 of buffs) goes 54/57
  (`Hearthstone_2026_11_03_00_29_06-g5` turn 9), and a 330/296 Brann goes
  332/300 under Captain Sanders (`Hearthstone_2026_10_28_02_28_32-g1` turn 11).
- **The numbers that grow (block 4).** `tp.spell_bonus_per_type`
  (Multilayered Shield: the bonus per different friendly minion type) and
  `tp.spell_bonus_turn` (Honeycomb Ring: this turn only, cleared in
  `start_turn`) are added in `Player::total_spell_bonus`. `Player.discarded`
  is the Hammer of Twilight's step: `tp.hammer_base` plus `tp.hammer_per` per
  card discarded, given to the board when a minion lands
  (`hammer_on_play`, the pick, every discard) and, in a fight, to a minion
  summoned there through `Counters.aura_attack`/`aura_health`.
  `Act::DoubleValue` doubles a trinket's own number (Weighted Gauntlet).
  `Player.spells_cast_turn` is the Inductive Gyroblade's growth step.
- **The portraits that rewrite a card (block 4).** `TrinketPowers.card_mods`
  is a small fixed array of `(card, CardMod)`: the trinket's data names the
  card, and the card's own effect reads the rewrite where it runs. `AlsoAttack`
  (`after_hero_damage_mods`), `AlsoHealth` and `AlsoNeighbors`
  (`trigger_on_cast_spell`, and `consume` for Flaming Enforcer) and `AlsoBoard`
  (the `Effect::TavernBuff` arm). `tp.choose_one_all` is Trailblazer Sticker
  (every Choose One card has both effects). `tp.souvenir` makes Souvenir Stand
  copy the next Greater trinket picked (`pick_trinket`).
- **The fighter's leftovers (block 4).** `fight = [{ when = "soc", fermenter = 3 }]`
  destroys the left-most minions at Start of Combat and keeps them in
  `Side.fermenter`; when the side's last minion dies, `Side::resolve` summons
  them back exactly as they were. `gem_golem` summons a token with the Blood
  Gem stats of the minion that just died (`Side.last_dead`, `Side.dead_at`).
  `when = "cast"` fires on a spell cast in the fight, and `permanent = true`
  on a `buff` act means the minions keep it after the fight
  (`FightAct::PermanentStats` -> `gain_permanent_at`, Bewitched Ribbon's
  bigger in-combat numbers).
- **The observation** carries the three slots (trinket index + 1), their
  counters, the open offer (its slot and the four options with prices) as own
  scalars, and each opponent's three slots as their scalars 30 to 32.
  `OBS_SIZE` grew with them and `train/model.py` embeds the trinket ids in a
  table of their own (`N_TRINKETS`). Hero frame 2 put the offered powers just
  before the hero offer, so the trinket block moved from 173 to 183. Block 5
  added four scalars at the end (`SCALAR_ICE_BLOCK`, `SCALAR_IMMUNE`,
  `SCALAR_PRIZE_OFFER_TIER`, `SCALAR_TRIPLE_PRIZE`), so `SCALARS` grew by 4
  and `train/model.py` picks them up on its own (its scale list is built from
  `bgpy.SCALARS`).
- **Bots** pick a trinket in `answer_pending`: the cheapest one of a tribe the
  board already has, else the cheapest. Tests: `tests/trinkets.rs`; the shared
  test helpers answer an open offer (and any Discover a trinket opened) before
  passing the turn.
- **The Darkmoon Prizes (block 5).** A Prize is a spell or a minion with
  `prize = <tier 1 to 4>` in cards.toml (`Card.prize`), never in the pool:
  `CardDb.prizes` is the four lists a `DiscoverRequest::Prize(tier)` deals
  three different cards from (`deal_...` in `maybe_start_discover` via
  `CardDb::random_prize`), and the pick lands in hand with `pool_copies = 0`
  (`resolve_choose` reads a Prize's offer as holding no pool copy;
  `abandon_discover` skips a token for the same reason). The 31 Prize spells
  are plain spells (`tavern_spell = false`), and All-Seeing Elder is the one
  Prize minion (a `[[token]]` with `prize = 4`). `Act::DiscoverPrize`,
  `Act::RandomPrize` and the `SpellEffect`s of the same name are the ways in.
  What a Prize leaves behind that outlasts the cast lives in
  `Player.prize` (`PrizePowers`): the extra Tavern slot (New Recruit, a flag
  that `Player::shop_size` counts and `fill_shop` draws in; its +2/+2 is not
  its own, it goes into `Player.refresh_buff`, the after-Refresh buff
  Waveling and Easterly Winds give, so the two stack), the free Refreshes
  per turn (Rocking and Rolling, Open Bar), this turn's Tavern spell discount
  (Reserve Prices, read by `spell_price` and `Player::spell_cost`), this
  turn's extra Battlecry triggers (Big Brann Play, read by
  `battlecry_or_choose_one`), a set minion price (Friends and Family
  Discount, read by `Game::minion_price`) and the card that comes back every
  end of turn (The Unlimited Coin, given in `run_end_of_turn`). A minion can
  be given "At the end of your turn, gain +4/+4" for good (Gruul Rules) in
  `Minion.end_turn_gain`, read by `run_minion_end_of_turn`. Corrupted Tome's
  Triple Prize is `tp.triple_prize`: `give_triple_reward` hands that card out
  instead of a Triple Reward, so every Triple Reward source goes through it
  (a Golden minion, Natural Balance, Heroic Inspiration). The observation
  carries the Prize tier an open Discover offers (`SCALAR_PRIZE_OFFER_TIER`)
  and whether Triple Rewards became Triple Prizes (`SCALAR_TRIPLE_PRIZE`) on
  top of the Discover's own card ids. Tests: `tests/prizes.rs`, one per
  Prize plus Balinda's extra casts.
- **The last trinkets (block 5).** Every Triple Reward becomes a Triple Prize
  (`Act::TriplePrize`). Tickatus Sticker is a `discover_prize = 3` at the pick
  and `start_every = 3`. The Curator Sticker hands out a Golden Mishmash (a
  trinket-only token, `buddy = true`) and a 10/10 Amalgam; Mishmash's
  "whenever your Amalgam gains stats, this gains them twice" is
  `Game::note_amalgam_gains`, run after every action beside Makeshift
  Master's watch (which marks the card it chose, `Minion.watched`, so it
  follows a Tavern minion I buy and then play): the player keeps a snapshot of every board slot
  (`Player.amalgam_seen`) and the gains an Amalgam made land on each Mishmash
  once per `Card.amalgam_gain_copy` (golden doubles it). In a fight it is
  `Side::amalgam_gain_at`, called from `gain_at` and `gain_permanent_at`: what
  an Amalgam gains in combat lands on every Mishmash of that side for the rest
  of the fight through the ordinary `gain_at`, so it is gone when the fight
  ends unless a Persistent Poet next to it keeps it (Mishmash is every type,
  so it is a Dragon for the Poet). Safety Patch sets
  `Player.ice_block`; the fatal damage path (the fight damage loop in
  `finish_fights` and `take_hero_damage`) prevents the hit that would knock
  the hero out and sets `Player.immune` for the rest of the turn, cleared at
  the start of the next one, and the observation carries both
  (`SCALAR_ICE_BLOCK`, `SCALAR_IMMUNE`). `Act::Surprise` is the Surprise
  Portrait's 5% roll on every buy, sell and Refresh, rolled at the top of
  `trinket_event` (never twice in a row, `tp.surprise_last`); an Elemental of
  Surprise arrives generated, and a Refresh either adds one to the Tavern or
  fills every minion slot with them. `Act::YoggWheel` picks one of the
  trinket's `wheel` acts, so the Wheel's six results are data in
  trinkets.toml; its results are `Act::RandomPrize`, `Act::CurseOfFlesh`,
  `Act::DevouringHunger`, `Act::RodOfRoasting`, `Act::GoldenTavern` and
  `Act::CastRandomSpells` (Puzzle Box's `cast_random_spell`).
  `Act::MechaDemon` hands out `CardDb.mecha_demons` (the four tokens with
  `mecha_demon = true`), generated; their `magnetize_to_tribes` (a `Tribes`
  on the card, Mech by default) is what `can_magnetize` reads, so they merge
  onto Mechs and Demons. Bronzebeard Portrait's tribe change is
  `CardMod::AlsoTribes`: it writes `Minion.extra_tribes` on the copies the
  player has (the pick) and on every later one (`Player::make_card`), and
  `CardDb::tribes_of` ORs it in, so every tribe check in the shop and in a
  fight sees it; `most_common_card_type` is the one place that reads the cards
  alone, for the Dark Discovery's "your type" slot. Two of the reviewed rules
  needed no code of their own: a Prize is a spell, so `balinda_mult` already
  gives a Prize that targets a friendly minion `PendingTarget.runs = 2` (or 3
  with a golden Balinda), and a "then double it" buff doubles once per cast;
  and `evolve_tavern` draws the Tavern spell one tier higher too, which the
  invariant allows the way it allows Robust Evolution's minion. The greater Eternal
  Portrait is an `on_pick` Golden Eternal Knight and a `rule` on `death_undead`
  that buffs every Eternal Knight in hand and on the board. Tickatus' Prize
  Wall is `PowerEffect::PrizeWall { every: 4 }`: a run-tested
  `DiscoverRequest::Prize` at the start of turn 4, 8, 12, 16 and every four
  turns after (Tier = turn / 4, at most 4), with the power's `progress`
  holding the turns to the next Prize and `obs::power_progress` reporting the
  tier coming and those turns. Tests: the block 5 section at the end of
  `tests/trinkets.rs` (one per card) and `tests/heroes_batch4.rs` for
  Tickatus.
- **The compasses, Minion Bait and the Poet Portrait (block 6).** A trinket
  can lock a minion type in at the pick. `lock_type` (`Act::LockType`) reads the board's
  most common type through `crate::darkgifts::most_common_type`, the same read
  the Dark Discovery's "your type" slot uses (the all-type minions count for
  every tribe, a tie is broken at random, an empty or all-neutral board gives
  none), and stores it in `TrinketState.tribe`, which `set_trinket` clears
  with the rest of the slot. `random_most_common_type` hands out minions of
  the type the slot holds and only reads the board when it holds none (a
  Colorful Compass therefore keeps its type however the board changes, and a
  copy a portrait made over it still does something). Minion Bait uses the
  same lock (`refresh_extra_type` in `trinkets.toml`,
  `Act::RefreshExtraType`) and adds two minions of the type to every Refresh
  it makes: the normal fill leaves the last two minion slots before the spell
  for them, so at Tiers 4 to 6 the Tavern holds four normal minions, the two
  extras and the spell (log `Hearthstone_2026_11_08_03_26_06-g4` turns 6 to
  13). A frozen Tavern is not Refreshed and gets none. The extras draw in the
  Refresh's own tier range and take their pool copies like any Tavern card.
  The Gold-plated Compass's `golden_bought` is an `on_buy` act
  (`Act::GoldenBoughtType`): the
  minion just bought is `TrinketPowers.bought_card`, written beside
  `bought_stats` just before the Buy event, so the act makes that hand minion
  Golden with `golden_up` and spends the slot's type. It is a Golden bought and
  played from the hand, so it gives its Triple Reward like any other
  (docs/rules/design.md "Triple rewards"): nothing sets `Minion.no_reward`. A
  loaded state's pick never ran, so the shop replay reads the type the game
  writes into the trinket's first number (the race code `race_code_tribe`
  names) in `seat_trinkets`, and `shop_replay::compass_spent` empties the slot
  for a step whose file shows the Golden minion it named already bought (a
  plain Tavern card Golden in the hand, with no copies behind it for a triple).
  The Poet Portrait's token,
  Timewarped Poet, is Persistent Poet's keep for every friendly Dragon:
  `permanent_keep = "all_dragons"` is `PermanentKeep::AllDragons`, which
  `Side::keep_mul` reads on every living minion of the side, the keeper's own
  gains included; a golden one doubles them, as goldens do for
  `adjacent_dragons`, and a keyword a Dragon gains in the fight is kept the
  same way. The observation carries each slot's locked type in
  `SCALAR_TRINKET_TRIBE` (a tribe index + 1, 0 for none). Tests: the block 6
  section at the end of `tests/trinkets.rs`, and `tests/dragons.rs` for the
  keep.

## Dark Gifts

`darkgifts.toml` (docs/rules/darkgifts.md; docs/rules/design.md "Trinkets and Dark Gifts")
holds the 47 gift cards of the game data: each gift's card_id, its turn
window (tags 4853/4854), its weight (4856, 50 half as likely) and the
condition its host has to meet. What a gift *does* is a token in cards.toml
(the "Dark Gifts" section), so a gift whose text matches a card's runs
through exactly that card's code. The engine never matches a gift by name:
`crate::darkgifts::GiftDb` is the table, `game/darkgifts.rs` the rules.

- **A gift on a minion.** `Minion.gifts` is a fixed `[GiftOn; MAX_GIFTS]` (8)
  with a count, so `Minion` stays `Copy`; `GiftOn` packs the gift's effect
  card in ten bits and a six-bit counter (a countdown, the turn Steady Growth
  landed). `Minion::parts()` yields the minion, its attachments and then its
  gifts (always as `golden: false`, so a gift never doubles), which is what
  makes the fight and the shop run a gift's Start of Combat, Deathrattle,
  Rally and end of turn effects with no code of their own. Gifts stack
  (docs/rules/darkgifts.md): `push_gift` attaches a second copy, which fires a
  second time, except for the gifts that set a state (`Card::gift_unique`:
  the keyword gifts, Amalgamation, Toreth's Blessing's hits and Gilding),
  which `push_gift_unique` drops. Past `MAX_GIFTS` a new gift is not attached.
  A triple takes the gifts of all three copies, a Magnetized Mech keeps its
  own (the magnetic minion's are lost) and a sold minion's go with it.
  Countdown gifts come along like any other and both pay: the golden Cadaver
  Caretaker of `Hearthstone_2026_10_26_00_48_15-g2` turn 7 carries two
  Replication enchantments and hands over two plain copies.
  `CardDb::tribes_of(&Minion)` is the one tribe read that knows about
  Amalgamation ("has all minion types"). Every tribe check on a minion goes
  through it (board, hand, the Tavern, a fight, an observation, a bot's
  score, a target's validity, a trinket's "when you play a Dragon"); a
  `Card`'s own `.tribes` is only for card definitions, where no minion
  exists yet (a pool draw, an offer, a Discover filter, the Dark Discovery's
  "your type" slot). `Tribes::is(tribe)` is that test with `None` meaning
  "any minion".
- **The landing** is `game::darkgifts::land_gift`: the effect card's stats
  (its `stats` are what the gift adds), its keywords, `shield_hits` for
  Toreth's Blessing, Gilding's `golden_up`, the "for each ... this game"
  counts taken now, and the counter. Everything else the gift does is read
  later from its card by the existing paths; `Minion::has_gift` is the one
  flag test a fight pays for a gift with no trigger.
- **The button.** `Action::DarkGift` (the last global action); `dark_gift_usable` is the rule (rules.toml's `dark_gift_cost`,
  `_uses` and `_turn`, once a turn, 3 uses a game, locked until turn 3).
  A press pays, opens `DiscoverRequest::DarkGift(DarkGiftPlan)`, and
  `deal_dark_gift_offer` draws the three offers. The plan is the source's:
  the button and Xavius use this turn's row of `[[offer]]`, Ominous Stone a
  fixed tier of the player's type, Wax Lance tier 7, Sinister Invitation the
  button's tiers Golden, Void Power tier 5 with its gifts picked as turn 7
  and the pick locked in hand until turn 7. Each offer takes a pool copy
  (`Player.discover_copies`, three for a Golden offer, none for the Dagger's
  copies), gives the rest back on the pick, and `pl.discover_gifts` carries
  each offer's gift so the observation shows it before the pick.
- **The pool the offers come from** is `GiftDb::pool_allows` (`pool_allows_for`
  with the player's most common board type from `most_common_card_type`): never
  a spell or a token, never a Magnetic minion, one with "when you sell this" or
  one whose effect works in hand, no Battlecry or Choose One minion before turn
  5, darkgifts.toml's `exclude` list (Leeroy the Reckless, Deadly Spore), and
  darkgifts.toml's `require_common_type` list after `require_common_type_after`
  (patch 37.0.3: after turn 9, Tyrael, Falling Sky Golem, Cataclysmic Harbinger
  and De-volition-ist are offered only if one of their tribes matches the
  board's most common type).
- **Who gets a gift.** `GiftDb::pick` chooses at random among the gifts whose
  window holds the turn and whose condition the host meets, never one already
  on another offer of the press and never one whose darkgifts.toml `exclude`
  list names the host (Furtiveness on Relentless Deflector, patch 37.0.2);
  the three weight-50 gifts count half. A Golden
  offer (Sinister Invitation) also leaves out every gift that makes its host
  golden (`Gift.makes_golden`, from the effect card's own flag: Gilding): the
  minion is golden already (Era 2026-11-03,
  `crates/bgcore/tests/darkgifts.rs` `no_sinister_invitation_offer_ever_holds_gilding`).
- **Countdowns.** `dark_gift_end_of_turn` (called from `run_end_of_turn`)
  ticks Incubation, Replication and Affinity and runs the countdown's effect
  at 0. A gift marked `repeats = true` in darkgifts.toml (Replication and
  Affinity) stays on the minion and starts its count again; the rest
  (Incubation) comes off. It ticks once a turn whatever a Drakkari
  Enchanter does: the Drakkari makes the *payout* run twice on the turn it
  fires, never the count. Incubation is the one payout that waits: its count
  runs out at the end of the turn, the gift is marked `PAYOUT_PENDING` and
  stays, and `dark_gift_start_of_turn` doubles the minion as the next turn
  starts (log `Hearthstone_2026_10_28_19_47_59-g1` turn 7: the end of the
  turn sets `TAG_SCRIPT_DATA_NUM_1` to 0 with the minion still 12/16, and
  the next turn's board is 24/32). `dark_gift_start_of_turn` (called from
  `start_turn`) also gives every Time Turning minion its end of turn effects
  again (`run_minion_end_of_turn`, the same code the end of the turn runs).
  Replication's `plain_copy` payout hands over the base card, never the host
  as it stands: `Player::make_card` with no golden, no buffs, no added
  keywords and no gift. It goes into the hand unmerged, because the turn is
  over and nothing triples during the fights: the card is recorded on
  `Player.triples_next_turn` (`defer_triple`, the list a fight's own grants
  use) and `dark_gift_start_of_turn` runs its `check_triple` as the next
  turn starts, so the three copies become the golden in hand there, gifts
  and all (Era 2026-11-03; every logged payout that made a third copy shows
  the golden at the turn's start).
- **The other hooks.** `dark_gift_card_played` ("whenever you play a card":
  Dexterity, Sharpened Sword, Toughened Shield) runs from `Action::Play`;
  `dark_gift_count` grows the "for each Battlecry / Tavern spell / discard"
  gifts from `battlecry_or_choose_one`, the Tavern-spell branch of
  `cast_spell` and `discard_card` (Death's Embrace rides the existing
  `buff_per_deathrattle`), on the board, in the hand and in Bob's Tavern
  (a Tavern Dark Paradox t6 grows too, "wherever this is");
  `dark_gift_on_acquire` gives Double Vision's plain
  copy in `add_to_hand`; Efficiency doubles an Activate in `Action::Activate`.
- **In a fight.** New `StartOfCombat` arms: Resistance, Hostility,
  Transcendence, Admiration, Worship (reads `Counters.deity`), Jaws of Death,
  Doppelganger. `deal_damage`/`hit` spend `Minion.shield_hits` for Toreth's
  Blessing, the attack loop reads Invulnerability, the reborn path rebuilds
  Persisting Horror's copy with its stats, Bonus Keywords and gifts, and
  `Counters.bonus_keywords` is rules.toml's mask for it.
- **Sources.** The four gift trinkets are `Act::DarkGift { tier, yours,
  golden }` and `Act::DarkGiftCopies` (Maldraxxus Dagger: plain copies of
  three of your own minions, gifted or not, each with a gift picked fresh for
  it by the normal rules, the one offer that may copy a Magnetic minion); the
  two heroes are
  `PowerEffect::DarkGiftStart` (Void Power) and `DarkGiftEvery { turns }`
  (Feel Devastation, counted in the power's `progress`).
- **Sunken Persistence** ("this minion's Spellcrafts are permanent") has no
  effect: no built minion hands its owner a Spellcraft spell, so nothing can
  read the flag.
- **Dark Paradox** ("a different Dark Gift, stats and Tier each game") is
  five pool cards in cards.toml (`paradox = true`, named after the game
  data's card-id suffixes: t3 Tier 2, t4 Tier 5, t5 Tier 4, t6 Tier 3, t9
  Tier 6, all-type so in every lobby). Their gift is the card's own text,
  not a gift attached by the gift machinery: `on_card_play` reads the card
  itself (a golden one doubles it), `shield_hits`/`golden_shield_hits` set
  `Minion.shield_hits` in `CardDb::make` (the Tier 5 golden takes 4 hits,
  not 6), `buff_per_battlecry` is applied by `Player::make_card` and grown by
  `dark_gift_count` (the golden gets +6/+6), the Rally reuses
  `Effect::GetMostCommonTypeMinion` once per `mul` (the golden gets two), and
  the Deathrattle reuses `Deathrattle::SummonShaped` with the dead minion's
  maximum Health times `mul` (the Tier 6 golden's ONE Golem has double
  stats). `Game::unstarted` puts exactly ONE of the five in the pool, with
  its tier's usual copies, the same for every player; `Game.paradox` holds
  it, `check_invariants` reads the same field, and the pick uses its own RNG
  seeded from the game seed so no other draw moves. Dark Discovery never
  offers any of them (darkgifts.toml's `exclude`). The observation's
  `SCALAR_PARADOX` is that card id + 1.
- **The observation** writes each minion's gifts as effect card id + 1 in the
  eight `SLOT_GIFTS` slots after the attachments (the model embeds them in
  the card table, `SLOT_IDS` in `train/model.py`), the button's uses,
  whether it was pressed this turn, whether it can be pressed and whether the
  open Discover is a gift offer as four own scalars after the trinket block.
  `SLOT_FEATURES` went 78 to 86, and hero batch 2 took it from 86 to 89
  (hats, ticket, frozen after the gift slots; OBS_SIZE 7145 to 7292). Hero
  batch 3 took the scalars from 208 to 224 and the card slot from 89 to 90
  (the reanimate mark; OBS_SIZE 7292 to 7385): free minion buys, Conviction,
  Lock and Load's stored minion, the two guesses and the "empty" flags. The
  scalars have grown since, so OBS_SIZE read 7400 before Poisonous.
  Poisonous (2026-11-04) added the card slot's eighth keyword, after stealth
  (`SLOT_FEATURES` 90 to 91, OBS_SIZE 7400 to 7477): a checkpoint from
  before it breaks too.
- **Real-board seats** carry the `dark_gifts` the files record: the loader
  attaches them (their stats are already in the file's attack and health),
  with the counter set as if the gift had landed on that board's turn.
- **Bots** press the button in `curve_action` when 3 Gold is left over.
  Tests: `tests/darkgifts.rs`.

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
0), `--boards DIR`. The screen shows your hero, armor and power (both powers
once a second one is held), and a power that is a chooser or a Discover
lists what it offers; `hp` presses the main button, `hp2` the second one.
`dg` presses the Dark Discovery button and a Discover shows each offer's
Dark Gift; `gift 2 fort` gives board minion 2 the gift whose name holds
"fort" (the command is not an action, so `undo` does not take it back).
Commands are read from stdin (`help` lists them). Every
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

## Real-board seats

A seat can play the boards of Era's real games (`boards/real/*.json`, see
`docs/real-boards.md`) instead of shopping: at the start of each of its turns
it takes a fresh random board of that turn (the file's attack, health, golden
and keywords, Dark Gift effects lost), that player's hand and the side-wide
numbers it had at the first attack, and its turn is over. The boards are not
pool cards: they hold no pool copies and nothing goes back when they die or
the seat is knocked out, so pool accounting stays exact, and neither do the
minions in the hand.

- **Building one:** `Game::new_random_heroes_real(data, seed, &seats, boards)`
  marks the seats before the heroes are dealt, so a real seat gets no hero
  (no power, no hero armor, and it uses up no hero from the deck; the pick
  phase skips it too) and plays with a flat 30 Health and 10 armor whatever
  its boards. It takes damage like anyone (armor first) and is knocked out
  at 0 Health.
- **The boards:** `RealBoards` (src/real_boards.rs) is built once from specs
  and shared by every game (`Arc`). A fresh random board each turn from all
  boards of that turn, Era's and opponents' alike; turn 13 and later draw
  from every board of turn 13 or later; a turn before 13 with no boards of
  its own uses the nearest earlier turn that has some. Minions whose name
  matches no card are dropped (about 1%: Dark Paradox, which the files name
  without saying which of its five versions it was, buddies, timewarped
  minions, and a few that only sat in a hand).
- **What a board carries:** `RealBoard` holds the minions, the hand and a
  `RealCounters` (Blood Gem size, Beetle and Undead bonuses, the Tavern spell
  bonus, spells cast, Eternal Knight deaths, the `Counter` counts) plus its
  `Deity`. In `start_turn` the seat's `Player` takes them with
  `take_real_counters`, which also resets the numbers a fight's outcome can
  move, so nothing accumulates: a different player is drawn every turn and
  its numbers replace the last one's. A number the log did not show is the
  base value (`RealCounters::default`), never "unknown", and Health, armor
  and tier stay the seat's own (30, 10 and the board's). A minion's current
  numbers become `Minion.improvements` for the cards whose scaling the
  engine models that way, and a Tasty Lobster's become the player's
  `tasty_lobster` count (the list is in `docs/real-boards.md`). A minion the
  power Rapid Reanimation chose carries the file's `reanimate` as
  `Minion.reanimate`, which the fight reads.
- **One side as it went into a fight:** `RealBoard` also holds the Health,
  armor, hero power (with the minion Lock and Load owes the board), trinkets,
  quest rewards and Secret zone the file carries, and
  `Player::from_real_board` seats the board, the hand, the tier, the numbers
  and the Deity on a player. The fight check
  (`src/fight_check.rs`) adds the power, the trinkets, the Secret zone,
  Ozumat's Tentacle sales (`Player::hold_power`, `Player::hold_trinket`) and
  the file's `waits` (Diremuck Forager summons it still owes) as
  `Counters.waits` on top; a real seat takes none of that yet (no hero, flat
  30 Health and 10 armor). This is the seam a real seat would use to play with
  its hero power and trinkets later.
- **The hand:** a real seat's hand reaches the fight through
  `fight_with_hands`, which already passes each side's hand whenever a card
  on either board reads it; the minions come out of it with no pool copies,
  as the board's do.
- **Loading:** `bgcore::real_boards::parse_board_files` parses the JSON texts
  and maps card ids through `ref/bg-cards.json` (`CardIdMap`: a golden id,
  the game's `_G` and `TB_BaconUps_*` forms, maps to the base card). The
  callers read the files: `train/real_boards.py` for training (it prints the
  boards per turn and the unmatched minions by name), `bgbench --real-boards
  DIR [--real-seats N]` for fuzz and games, and the tests use the small
  fixture folder `crates/bgcore/tests/data/real-boards/`.
- **In training:** the lineup grows a sixth number,
  `learner,snapshot,standard,rush,random,real` (default `2,2,0,0,0,4`); `bgpy.VecEnv(n, seed, lineup, real_boards=..)`
  takes the loaded specs, and real seats are played in Rust like the bot
  seats. `ppo.py` adds a `vs7 real` eval (one learner against 7 real seats,
  `place_vs_real`) whenever real boards are loaded. `ppo.py
  --learner-heroes NAMES` fixes every learner seat's hero in training and in
  every eval; `train/fixed_eval.py` and `train/evaltools/hero_check.py` take
  the same flag.
- **The Era seat:** `Game::new_random_heroes_era` marks one real seat to play
  Era's own boards of one finished game (`who: me`, nearest earlier turn, the
  last board again past the game's end) and leaves that game out of every
  other real seat's pool. `bgbench eraseat` runs it (`--runs N` per finished
  game or `--games N` with a random era game each time) and
  `bgpy.VecEnv(era_game=(source, game))` exposes the same seat to Python.
- **The lobby filter:** `RealBoards::set_lobby_filter(true)` keeps only
  boards whose minions fit the game's lobby (neutral always, a typed minion
  when any type is in). A turn with fewer than `MIN_FILTERED_BOARDS` (8)
  eligible boards widens to the nearest earlier turn with 8, then the nearest
  later one, then the fullest bucket; no eligible board anywhere falls back
  to the old draw. `draw_filtered` is the seat's draw and `filter_stats`
  counts own-turn, widened and fallback draws. On `bgbench`, `ppo.py`,
  `eval_search.py` and `eval_overlay.py` the flag is `--real-lobby-filter`;
  on `bgpy.VecEnv` it is `real_lobby_filter=True`. Off is the old draw
  exactly.
- **The lag:** `RealBoards::set_lag(k)` (default 0) makes a real seat on
  turn t draw a board of turn `max(1, t - k)`, through the same late-turn,
  nearest-turn, lobby-filter and widening rules. The Era seat (`draw_era`) is
  not lagged. On `bgpy.VecEnv` it is `real_lag=K`; lag 0 is the old draw
  exactly.
- **The first lagged turn:** `RealBoards::set_lag_from(k)` (default 1) makes
  the lag apply only when `t >= k`; before that a real seat draws turn t as
  at lag 0. So `lag_from=7, lag=2` plays the full board through turn 6 and
  the turn 5 board on turn 7. It applies to the fixed lag and to the
  per-game mix alike, and the floor at turn 1 stays. On `bgpy.VecEnv` it is
  `real_lag_from=N`; `fixed_eval.py` takes `--real-lag-from N` and writes it
  in its report and results, and `ppo.py` takes it for the training env and
  the mixed-lag eval and prints it with the mix. The default 1 lags every
  turn and is the old draw exactly.
- **The per-game lag mix:** `RealBoards::set_lag_mix(&[(lag, weight)])` makes
  every game draw its real seats' lag at game start from that weighted list.
  The game's lag is fixed for the whole game (`Game.lag`, read from
  `bgpy.VecEnv.real_lags()`). The draw is deterministic from the game's seed
  and uses an RNG of its own, so it never moves the game's other draws: a game
  that draws lag 0 is byte for byte the old game. `set_lag`/`real_lag` still
  give a fixed lag, and an empty mix uses it. On `bgpy.VecEnv` it is
  `real_lag_mix=[(0, 1), (1, 1), (2, 1)]`; `ppo.py` takes
  `--real-lag-mix 0:1,1:1,2:1`, logs each eval's share of games and place per
  lag, and keeps its `vs7 real` score at lag 0. The fixed starting bodies of
  scenario and Era-start games are setup, not a real seat's turn draw, so
  `new_scenario` and `new_era_start` place them unlagged; every later turn
  draw uses the game's lag.
- **The late-start lag:** a late-start game draws its lag like a normal game
  unless `era_start_lag` is set. Then that fixed lag is the game's lag
  whatever the mix or fixed lag says, while normal games in the same `VecEnv`
  keep drawing from the mix. On `bgpy.VecEnv` it is `era_start_lag=K` (None
  keeps today's draw; it needs `era_starts`). `ppo.py` takes
  `--era-start-lag K`, uses it for training and for the in-run holdout Era
  eval, and prints it on the Era starts line; the vs7 real eval and `best.pt`
  stay lag 0 from normal starts. `fixed_eval.py` needs no separate flag: its
  `--real-lag` is a fixed lag with no mix, so it already covers late starts.
- **The lag in the observation:** `--lag-in-obs` (off by default) writes
  `Game.lag` as one extra own scalar at `obs::SCALAR_LAG`, after every old
  scalar, so every own slot moves one place later and the observation is one
  value wider (`OBS_SIZE_WITH_LAG`). Off is the old `OBS_SIZE` observation
  byte for byte, which is what an old checkpoint reads. `bgpy` exposes
  `set_lag_in_obs(on)` to switch its layout constants before the arrays and
  network are built.

## The shop-turn search

`train/search.py` plays shop turns with a search instead of the network's single
move: at each shop decision it tries every legal move and every short cycle on
copies of the game, judges them, and plays the best. Play time only; nothing in
training changes. The idea is Era's (`docs/notes/era-strategy.md`, "Gold-neutral
loops"): buying and selling a card for value, the Tortollan Blue Shell after a
loss (+2 Gold), Brann with Kalecgos and a Draconic Warden (two Chromadrakes for
free, the Dragons buffed).

- **The copies** are `bgpy.SearchBatch` slots: `Game` clones with their own RNG
  and pool, stepped by the actions a candidate tries. The real game is stepped
  only with the action the search picks, and the rest of the picked line
  follows as a plan (`Searcher.plans`), so the copy's work never touches the
  game or its accounting. A copy may put every pool copy back before a Refresh
  (`crate::search::refresh_from_full_pool`), because a playout models what a
  real player knows rather than the exact pool; that is the one state where
  `check_invariants` does not hold, and `Game::pool_untracked` marks it.
- **Judge 1, dominance** (`crate::search::Position`): a cycle *dominates* when
  nothing about the position got worse and something got better. It compares
  the player's own fields one by one, Gold and the counters as "at least as
  much", the Tavern upgrade cost as "no higher", the board minion by minion
  (same card, same order, Attack, Health, keywords and `improvements` at least
  as good, not Golden turned plain), the hand as "every card still there with
  stats at least as good, a card gained is a gain" (the engine's Blue
  Chromadrake gives Tavern spells to hand), and everything not listed exactly
  as it was; the Tavern's contents, the freeze, `recent_buys` and the action
  count are ignored. The doc comment on `Position` lists every field.
- **Judge 2, real trades** (`Searcher.score`): a move or a cycle that costs
  Gold is played out to the end of the turn, `--playouts` times. After the
  candidate's own first move the network plays at `--playout-temperature`
  (default 0, greedy: sampled playouts were the passivity bug, 2026-11-07),
  for at most `--playout-actions` shop actions (default 60). Move, Freeze and
  End Turn are not played in a playout: a greedy network loops on board moves,
  and ending the turn there would run the coming fight and start the next
  turn. The playout stops if its turn changes. End Turn itself is exact: the
  copy's shop turn is ended with `end_shop_turn` (no fight) and scored as it
  stands, while the other playouts finish. Each finished turn is scored as the
  network's value of the position plus `--fight-weight` times the Health the
  coming fight is expected to cost against `--fights` boards of Era's real
  games for the turn (preferring the next opponent's tier and leaderboard
  tribe, the whole bucket when fewer than 5 match), fought with
  `fight_with_hands` exactly as `Game::combat` fights a real seat. The same
  boards and the same seeds for every candidate of one decision.
- **The network's prior** is the greedy action at the forced mask. A candidate
  must beat its score by `--margin` (default 0, which still breaks ties toward
  the network) or the search takes the prior. The forced mask (`ppo.force_mask`)
  also restricts the candidates, so the search cannot skip a forced Dark
  Discovery or level that the plain player must press.
- **`--replan`** says when judge 2 runs: `every` (the default) at every shop
  decision, `turn` once per turn (about ten times cheaper, the rest of the
  turn is the network with the dominance cycles), or `cycle` never
  (dominance only).
- **`train/eval_search.py`** is the measurement: one learner against seven
  real-board seats, plain network and search over the same seeds, plus the Blue
  Shell test, the Warden test and the cycle rate over Era's logged turns
  (`bgpy.shop_replay_turns`, `bgpy.sandbox_position`). `train/era_turns.py
  --sample N` plays N turns spread evenly over the seatable turns (every
  k-th, all comps), not the first N in file order; 0 plays all.

## The lobster search mode

`docs/briefs/lobster-search.md` is its own mode, off in every normal game:
the Tavern and Headhunter Gryphon's random Beasts come from streams fixed by a
seed, so a line is reproducible and a search sees its future. The mode is
`LobsterMode` in `src/lobster.rs`, an `Option` on `Player` (`Player.lobster`).

- **The streams:** a draw is a pure function of the seed, the turn, the
  stream's index and the slot, so nothing another path did can change it. The
  Tavern's index is 0 at the start of the turn, then 1 for every explicit
  Refresh and every Refresh effect (a sold Snarky Shark); the Gryphon's Beast
  index is 0 at the start of the turn, then one per random Beast. A normal
  Tavern card is weighted like a fresh pool at its tier; a normal Beast is
  uniform over the pool's Beasts, the engine's `random_pool_minion` rule.
- **The Shark knob:** before each normal draw, a per-mille roll can replace it
  with a Snarky Shark or a Lurking Lionfish, equal odds. `p` is 0, 0.1, 0.25
  or 0.5. The mode neither reads nor consumes the pool, and an extra Shark or
  Lionfish is a generated token: `pool_untracked` is set, so the normal
  invariant checker stays out of this mode. A bought or sold normal card does
  not change what a later stream index holds.
- **The turn:** `Game::next_shop_turn_without_fight` ends every open shop turn
  and runs the start of the next turn without pairing or a fight, which is the
  mode's turn step. `Action::EndTurn` and the rule cap do not auto-finish a
  mode turn: the search stops expanding at
  `rules.max_actions_per_turn` and ends the turn itself.
- **The counters:** `LobsterMode` counts every Fishbait the left-most Beast
  attacked and every Deathstrider deathrattle fire (`SideOutcome`'s new
  `deathstrider_fires`), so search turns and the logged-turn replay can report
  them. The Tasty Lobster count stays the player's `improve[TastyLobster]`.
- **The log switches:** `LobsterMode.stream_tavern` and `stream_beasts` turn
  the fixed streams off for the logged-turn replay, which puts the log's own
  Tavern in place and lets the Gryphon use the engine RNG. A Shark's Refresh
  in that replay still places and attacks a Fishbait even though the stream is
  off; that is replay plumbing, not a normal game rule.
- **The search:** `crates/bgbench/src/bin/lobster_search.rs` is a beam over the
  legal shop actions of turns 10 to 13. It scores a state by the Tasty Lobster
  counter (triggers so far), then board Attack plus Health; fewer actions only
  breaks a tie. Every state is also an end-of-turn candidate. Duplicate states
  merge on the player, turn, lobby and RNG, and a bounded heap keeps the best
  beam width. A move is only tried to the front of the board, and a minion is
  only played to the front or the end; two moves cannot follow each other. A
  state that has not raised triggers or board stats in 60 actions is an end
  candidate but is not expanded. No rule looks at a card name.
- **The compact node:** a search copy stores `ShopStateCapture` (the player,
  RNG, turn, pool and pending-triple flags, from `Game::capture_shop_state`)
  instead of a 157 KiB `Game`; one mutable game is restored to the parent for
  each action. The end-of-turn carry is capped at 4,096 states even when the
  beam is wider.
- **The NRPA search:** `crates/bgbench/src/bin/lobster_nrpa.rs` replaces the
  beam with Nested Rollout Policy Adaptation over the same mode. A move code
  is the action kind plus the card it acts on, never a bare slot. The default
  policy is one global weight per move code with a uniform prior, and end turn
  is a code like any other; a level shifts its weights toward that level's best
  line. A rollout samples legal moves by a softmax with a small uniform chance,
  and runs one turn or turns 10 to 13 as one line. The control run puts Era's
  logged turn 10 Taverns in place of the seeded stream. It writes `nrpa.md`
  and `nrpa.json` under `train/runs/lobster-search/`.
- **The run:** `cargo run --release -p bgbench --bin lobster_search` with no
  options runs the brief's grid. It writes `report.md` and `results.json`
  under `train/runs/lobster-search/`, with Fishbait attacks and Deathstrider
  fires per turn. The worker pool uses a 32 MiB stack and a path is an `Arc`
  chain with an iterative drop.
- **The turn 10 replay:** `cargo run --release -p bgbench --bin
  lobster_replay` seats each of Era's logged turn 10 steps from the log's own
  state, plays the shop replay's actions in the mode, and writes
  `train/runs/lobster-search/replay-turn10.md`.

## The comp planner

`crates/bgcore/src/planner.rs` is a bot that plays a comp written as data
(`data/comps/comps.toml`, loaded by `crates/bgcore/src/comps.rs`). The plan is
`docs/notes/comp-planner.md`; the builds are `docs/briefs/comp-planner-1.md`
and `docs/briefs/comp-planner-2.md`. The effects still live in `cards.toml`:
the planner may name cards, as the scripted bots do, but no rule matches a
name.

- **The file:** `[defaults]` holds the shared `level` curve (the tier target
  for turn 1, 2, ...), `dying_health`, the Health at or below which the
  planner stops leveling, and `health_loss_penalty` (the overlay's negative
  bonus for a Health cost with no Soul Rewinder or Ashen Corruptor on the
  board). One `[[comp]]` per comp: `name`, `tribes` (the card tribes it
  plays), `lobby` (every tribe it needs in the lobby; empty for any lobby),
  `draft`, `fallback`, `commit_turn` (first and last turn it may commit),
  `level` (an own curve; absent inherits `[defaults]`, `level = []` means no
  curve), `scales` (`none`, `beetles` or `stats`), `core` cards with a weight
  and a `scale` flag, `early` (cards and `strong_stats`), `signals` (cards, a
  tribe with a minimum count, a hero, a trinket, or a keyword, each with a
  weight), `enabler` (entries of `all` groups: every group needs one of its
  cards held, and `copies = N` asks for N distinct cards of every group; a
  comp has any one entry), `abort` (`after_turn` or `min_tier`, with no
  enabler held; one-way), `give_up` (`after_turn`, `progress_below`, `to`)
  and `not_in_comp`. Every card, hero and trinket is checked against the
  loaded data; an unknown name fails the load with the comp and the name.
  `data/comps/comps.toml` holds 26 `draft = true` entries from JeefHS's
  current guides, chosen by Era on 2026-11-09: every comp with a card-based
  commit except the seven APM or economy comps (Quilboar - APM Shop Buff,
  Undead - APM Undead, Mechs - APM Magnetic, Aberrations - APM Deity,
  Pirates - APM Golden, Elementals - Living Azerite, Murlocs - Tidecaller).
  Murlocs - Venom Scam is the fallback, `level = []`, and is listed last.
  File order is tier: the two S-tier comps first, then Tier 2 and Tier 3, so
  the overlay's first-active tiebreak prefers the stronger comp. Abort is
  used only where the guide gives a latest turn (Demons - Deathrattle/Shop
  Buff turn 6, Dragons - Copebringer turn 8, Undead - Eternal Knight turn 4)
  and for Era's Self Damage rule. The shipped default is the
  standard curve: Era's own timing (tier 4 on turn 6, tier 5 on turn 8) was
  measured at 7.27 against 7.02 for this one, so it is left in
  `docs/notes/mining/sure-plays.md` for a comp to override.
- **One decision:** answer an open choice (a comp card first), apply the
  standing rules, level when the comp's curve or the default curve says so
  (unless Health is at or below `dying_health`; an unaffordable level is
  skipped and tried again on the next decision), buy a sure-buy spell, cast a
  spell already in hand, make room on a full board (the lowest comp weight
  card goes, never a core card), take a dominating cycle
  (`search::Position`), then score the legal moves. Turns 1 to 3 play the
  standard curve bot, so the opening board has known tempo.
- **The score:** `strong + w(turn) * comp progress`. `strong` is the average of
  `FIGHTS` (16) `search::fight_outcome` fights against the real boards of the
  turn, preferring the next opponent's tier and leaderboard tribe through
  `search::fight_boards`; without real boards it is a stat reading.
  `comp progress` is the core weights on the board (full weight) and in hand
  (half), triples of core cards, the early cards, a comp-tribe body, keyword
  and signal cards, the player-wide beetle or improvement scaling, less
  `not_in_comp` cards. `w(turn)` is 0.10 on turns 0 to 3, 0.25 on 4 to 6,
  0.50 on 7 to 9, 1.00 on 10 to 12 and 2.00 after. The weight is 10 points per
  progress point. A full board refuses a non-core buy unless it is the third
  copy for a triple, a Magnetic merge, or a card the make-room rule is already
  replacing a weaker card with. A core card in hand or in the Tavern always
  gets a slot. A Refresh is worth 1.0 when the Gold would otherwise be left
  over, so the planner looks again instead of ending with unspent Gold; a Hero
  Power is worth 2.0, so a free or useful power is pressed before that
  Refresh. `bgbench planner` prints the measured decision time and the
  end-of-turn hand and Gold.
- **Standing rules:** after a lost fight, a Tortollan Blue Shell in the Tavern
  is forced as its three ordinary actions, buy, then play, then sell (+2
  Gold); the planner keeps a small line marker so a Shell that was already on
  the board is not sold. On a full board the lowest comp weight card is sold
  first, and the line is skipped when every card on the board is core. The
  same rule makes room for a core card. Leaf Through the Pages, Wealthy Bounty
  and Fandral's Fortune are
  bought whenever offered and affordable. A hand spell is cast at once unless
  it is one Era holds (`HOLD_SPELLS`: Them Apples, Sludge Corrosion,
  Methodical Madness, Hasty Excavation, Corrupted Coin) or a stat buff with
  Gold still to spend (those wait until the buying is done). Trinket, hero and
  Discover offers prefer the comp's signals. The constants are in
  `planner.rs` (`FIGHTS`, `CANDIDATES`, `COMMIT_MIN`, `REPLACE_MARGIN`,
  `TRIBE_PROGRESS`, `HOLD_SPELLS`, `STANDARD_OPENING_TURNS`) and the two
  defaults are in `data/comps/comps.toml`.
- **`bgbench planner --comps PATH --games N --vs real [--seat planner|standard|rush]`:**
  one seat 0 bot against seven real-board seats. It prints average place,
  top-4 and first rates, the comps committed to and their average commit turn,
  the share of final boards with at least half the comp's core weight and at
  least two core cards, Shell cycles on the turns the rule applied, the median
  tier turns, the end-of-turn cards-in-hand and Gold (overall and by turn
  bucket, with the most held spell and minion names), the decision time and
  ten final boards in full.
- **Python:** `bgpy.Planner(comps, seed, real_boards)` has `action(path)` for
  `EraTurnPath`, `reset(seed)`, `comp`, `committed_turn`, `shell_cycles`,
  `shell_chances` and `level_turns`. `train/era_turns.py --planner PATH` adds
  the `planner` player, with `--players plain,planner` to measure it beside
  plain. `bgbench fuzz ... --planner PATH` lets seat 0 play the planner while
  the fuzzer checks the invariants after every action.

## The comp overlay

`crates/bgcore/src/overlay.rs` is the rules-on-top-of-a-network layer
(`docs/briefs/comp-overlay-1.md`): a forced action or nothing, plus a logit
bonus per action. It is not a bot. The network plays by default; a comp
becomes ACTIVE when one of its enabler entries is complete (every group has
its `copies` cards on the board or in the hand), all its `lobby` tribes are in
the lobby, and its `abort` rule has not fired. Several active comps are
resolved by file order: the first one wins. `OverlayState` keeps the Shell
line and one abort bit per comp per seat and game, so an aborted comp is never
turned back on.

- **The comp fields:** `enabler` (entries of groups that turn the comp on),
  `payoff` (the core cards that matter) and `filler` (the add-ons bought only
  to feed it). Beetles: two enabler entries, Ravaging Scorpid or Turquoise
  Skitterer; payoffs Banana Slamma, Turquoise Skitterer, Headhunter Gryphon
  and Ravaging Scorpid; filler Forest Rover, Buzzing Vermin, Lurking Leviathan
  and Sewer Lord. Murlocs - Venom Scam has no enabler: the overlay cannot
  measure no_future or outscaled, so it is the planner's `fallback` only.
- **Health rule (Era):** any action that costs the hero Health while no Soul
  Rewinder or Ashen Corruptor is on the board gets `health_loss_penalty` (a
  large negative bonus, not a mask). It applies with no active comp and to
  every comp and card, and covers a Health-priced Tavern minion or spell, a
  Refresh paid in Health, playing a Demon with Wrath Weaver on the board,
  Imposing Percussionist's Discover and Graveyard Shift's card steal.
- **Forced:** the Tortollan Blue Shell after a lost fight is three ordinary
  forced actions, buy, then play, then sell. A small `ShellLine` marker per
  seat records the turn and the board slot the line put it in, so a Shell
  that was already on the board is not sold. On a full board an active comp
  sells a card that is not in the comp first; with no active comp the sale is
  skipped. A Discover or Choose One is forced only for a card that
  `completes_enabler` a comp whose lobby tribes are all present; half of a
  two-card entry is left to the network. An active comp takes a payoff, then
  a filler. The enabler/offered flags are in the overlay stats array.
- **Bonuses:** while a comp is active, buying or playing a payoff is worth
  `B`, a filler `B/2`, and a full board sells a non-comp card for `B` when a
  comp card waits in the hand or Tavern. Selling an enabler or a payoff is
  masked with `-inf`. With no active comp the bonus array is all zero apart
  from the Health rule.
- **`bgpy.VecEnv`:** `set_overlay(comps_path, B)` loads the rules,
  `clear_overlay()` turns them off, `overlay(forced, bonus, flags)` fills the
  arrays (n by 8 seats, `forced` -1 or an action, `bonus` n by 8 by
  `N_ACTIONS`, `flags` n by 8 by `OVERLAY_FLAGS`, now 7: the active comp's
  index plus one is `F_COMP`), and `remember_boards()` must be called
  after `observe` and before `step` when `final_boards()` is needed.
  `final_boards()` returns the last completed game's board card ids per seat.
  `scenario_bodies()` returns the board a scenario seat started with, for the
  scenario eval's body-sold measure. Only the current player's row is computed,
  because `Game::is_legal` is current-player only; the other seats are
  neutral.
  `overlay_comp(comp)` fills every seat's active comp index or -1 without
  changing a Shell line or latching an abort, and `overlay_probe(forced,
  flags)` gives the overlay's advice on a copy of the state. The second is
  how eval measures a network with the wheels off: the comp input stays on,
  but no forced move or bonus is applied. `overlay(forced, bonus, flags)`
  remains the training call, the only one that advances the state.
- **Training wheels (`train/ppo.py`):** the network takes the active comp as
  an extra one-hot input (`n_comps` is a model shape setting, 26 for the
  current comp file). During training the overlay's forced moves and bonus
  steer the behaviour policy; an aux cross-entropy on the network's own
  logits (no bonus) teaches the overlay's choice at those rows, and both fade
  with `--wheels-full`/`--wheels-end` games (`--aux-weight` is lambda). The
  rollout stores the bonus compactly as four bitmasks (B, B/2, the Health
  penalty, -inf) plus those raw values, so the update rebuilds the exact
  scaled bonus the behaviour policy acted under. Forced rows are excluded
  from the policy gradient; the aux loss is what teaches them. The checkpoint
  stores the games played, so a `go.sh` restart continues the fade instead of
  putting the wheels back to full. Evals set the overlay to B 0 and only
  probe it, so the comp line (`comp_active_share`, `comp_end_share`,
  `shell_rate`, `first_share`) is wheels-off.
- **`train/eval_overlay.py --games 2048 --bonus 8`** runs the network seat
  against seven real boards (`--opponents network` plays seven network seats
  instead). It prints the place with a 95% interval, top-4, first, Shell
  legal/taken/rate, enabler offered/taken/active and active turn, the active
  games' comp-board share and place, the forced full-board sells, and a per
  comp table: games active, activation turn, boards ending as the comp and
  their share and place. `--real-lobby-filter` draws a real seat's boards only
  from the game's lobby. It also prints the overlay call's cost per step
  (relative while a training run is on the machine) and a 512-game
  overlay-off check, which is the network-only baseline. A network with no
  comp input and B 0 still uses `train/eval_search.py`'s own `play` function;
  a comp-input network uses the same loop with the overlay probed only, so
  its comp input stays on.

## Scenario starts

`crates/bgcore/src/scenario.rs` and `Game::new_scenario` are the training
mode from `docs/briefs/comp-starts.md`: a game can begin on a later turn with
a real board already on the board and a comp's core cards in hand, so the
network gets to practice playing an assembled comp. It is off unless asked
for; normal games never read it.

- **What a scenario game gets:** a turn drawn from `MIN..MAX` (uniform), a
  lobby that holds the comp's `lobby` tribes and every core card's own tribe
  or lobby tie (so every core card is in the pool), and one real board of
  that turn as each scenario seat's body. The body comes from `RealBoards`
  with the lobby filter, exactly as a real seat draws it, so its minions hold
  no pool copies. The seat's hand is replaced by the comp's `core` cards,
  highest `weight` first and at most `HAND` of them, each taken out of the
  pool. Tier is the higher of the scenario deadline curve (`0,0,0,0,3,3,4,5`,
  tier 6 from turn 9) and the comp's highest core card; Health is uniform 15
  to 35 after the turn's start-of-turn effects; Gold is the normal Gold for
  the turn; the hero is drawn as usual. Scenario seats get no Trinkets and
  no Dark Gifts, and the Dark Discovery action is illegal for them. Real
  seats play as they do now.
- **The comp input:** `OverlayState.scenario_comp` names the comp a seat was
  started with. When several comps are active, `Overlay::active_comp` reports
  that one if its normal enabler check passes; otherwise the file-order rule
  is unchanged. This keeps a scenario comp from losing the tie to an earlier
  comp whose enabler happens to be a subset (Beetles against Tasty Lobstah,
  or Unbound Tempest against Demons - Shop Buff's Brann).
- **`bgpy.VecEnv`:** `scenario_comps` is the comp file,
  `scenario_names` the comp names a game may start with, `scenario_share`
  the share of new games that are scenario games (0: off), and
  `scenario_turn_min`/`scenario_turn_max` the turn range. The choice is
  drawn from the game seed with its own RNG, so a scenario game is
  deterministic. `scenario_bodies()` gives the starting body for the eval.
- **`train/ppo.py`:** `--scenario-comps "A;B;C"`, `--scenario-share S` and
  `--scenario-turns MIN,MAX` turn it on. The eval block adds one scenario
  eval per comp and a pooled one (`scen_<comp>_place`, `_top4`, `_first`,
  `_end`, `_body_sold` in `metrics.csv`): one learner against seven real
  seats from scenario starts, wheels off. `_body_sold` is the share of the
  starting body's card copies no longer on the final board.
- **`bgbench fuzz|games`:** `--scenario-comps "A;B"`, `--scenario-file PATH`,
  `--scenario-share PCT` and `--scenario-turns MIN,MAX`, with
  `--real-boards DIR` for the bodies. The fuzzer then checks pool accounting
  after every action in scenario games too.

## Era starts

`crates/bgcore/src/era_start.rs` and `Game::new_era_start` are the training
mode from `docs/briefs/era-starts.md`: a game can begin on a turn Era really
played, with Era's own board, hand, Gold, tier, Health, hero, trinkets and
counters at the start of that turn. It is off unless asked for; normal games
never read it.

- **The positions:** every turn in a range of `boards/actions/*.json`, seated
  exactly as the shop replay seats the turn's first step
  (`shop_replay::turn_start_game`). A position whose seating reports anything
  missing is dropped and counted with its reason. `EraStarts` builds the list
  once and shares it (`Arc`), like `RealBoards`. The list is split by source
  game, by a stable hash of the file name and game number (15 in 100 held
  out), so adding a file does not move another game's side.
- **What an Era game gets:** the source game's lobby, the position's turn, and
  one network seat with the logged board, hand, Gold, tier, upgrade cost,
  Health and armor, hero and hero-power state, trinkets, Dark Gifts, and the
  player counters the replay seats. Its board and hand minions hold no pool
  copies, exactly like a scenario body. The logged Tavern and frozen cards
  are not kept: the engine rolls the start of that turn. The position is
  already past the turn's start-of-turn effects, so only the Tavern is rolled
  for that seat on the first turn; later turns run normally. Trinkets and
  Dark Gifts are on, not off as in a scenario game.
- **The other network seats:** a real board of that turn through the lobby
  filter as their body, with an empty hand, the board's tier and recorded
  Health and armor (the scenario's 15 to 35 Health when the file has none).
  Their heroes are dealt as usual. Real seats play a fresh real board each
  turn as in a scenario game; a scenario real seat has a flat 30 Health and
  10 armor, and an Era-start real seat takes the recorded Health and armor of
  a random real board of the start turn instead. Unlike the shop replay, an
  Era start is a normal game: its seven opponents are alive from the first
  observation (`SCALAR_ALIVE = 8`), with their own Health, armor, tier and the
  next opponent already set. Their seen boards, leaderboard lines and fight
  histories are empty until each has been fought, because the logged position
  carries no history for them.
- **Choosing:** each new game is an Era start with probability `share`, drawn
  from the game seed with its own RNG. The position is uniform over the
  allowed positions of the turn range and split, and which network seat gets
  it is drawn from the same RNG. `EraSplit` is `train`, `holdout` or `all`.
- **`bgpy.VecEnv`:** `era_starts` is the action folder, `era_share` the share
  (0: off), `era_turn_min`/`era_turn_max` the turn range, `era_split` the
  side to draw from, and `era_start_lag` the fixed lag of every late-start
  game (None: the same draw as a normal game). `era_bodies()` gives each
  game's Era starting board, `era_seats()` the seat, `era_turns()` the start
  turn, and `era_start_report()` the positions per turn and the dropped ones.
- **`train/ppo.py`:** `--era-starts DIR`, `--era-share S` (default 0),
  `--era-turns MIN,MAX` (default 7,11) and `--era-start-lag K` (default:
  draw like a normal game); training draws from the train split. When on, the
  eval block adds one eval from the holdout split (`era_place`, `era_top4`,
  `era_first`, `era_body_kept` in `metrics.csv`), one learner against seven
  real seats, wheels off, at the same era-start lag as training.
  `era_body_kept` is the share of the starting board's card copies still on
  the final board. The normal `vs7 real` eval and `best.pt` are unchanged.
- **`train/fixed_eval.py`:** `--era-starts DIR --era-turns MIN,MAX
  --era-split holdout|train|all` starts every game from Era's position, one
  game per seed. `--real-lag` fixes the lag of every real seat and, because
  there is no mix, of every late start too; there is no separate Era-start lag
  flag. It adds body kept, place split by starting turn, and the Era seat's
  board total at the start turn and 2, 4 and 6 turns later. The default
  folder is `boards/frozen/actions-2026-11-10/`, checked against its
  `MANIFEST.txt`. Without Era flags the old fixed eval is unchanged.
  `--trinket-pick own|popular|random` (default `own`) forces every learner
  trinket pick: own is the network, popular the offered trinket with the most
  clean streamer picks (ties by offer slot), random uniform over the offer
  from a generator seeded from the game seed. It overrides only a
  `ChooseTrinket` action; an End Turn with the offer open still takes the
  engine's cheapest option.
- **`bgbench fuzz|games`:** `--era-starts DIR`, `--era-share PCT` and
  `--era-turns MIN,MAX`, with `--real-boards DIR` for the bodies. The fuzzer
  then checks pool accounting after every action in Era-start games too.

## Era's turns, with Era's luck

`train/era_turns.py` measures the bot on Era's own turns (the compiled plan's
main stick, `docs/notes/training-plans.md`): for every logged shop turn from
turn 3 on it puts the bot in Era's position at the start of the turn and lets
it play, then fights the end-of-turn board against the board Era really fought
next. While the bot takes Era's steps, every random outcome is the log's: the
state before each decision is seated from the log (`shop_replay::seat_step`),
so a Refresh shows Era's Tavern and a Discover offers Era's cards. The first
action that is not Era's ends that, and from then on the engine draws, its rng
seeded from the turn's seed (a repeat's).

- **The player is `bgpy.EraTurnPath`** (`crates/bgpy/src/lib.rs`, built from a
  `bgpy.EraReplay`, one action file): `observe`/`describe`/`state` read the
  current position, `era_action` is Era's next logged decision as an action
  index, `act` plays one action and returns whether the turn is still on her
  path, `left_at`/`left_why` say where it left, `finish` ends the shop turn
  without the fight, and `batch` gives a `SearchBatch` at the position for the
  search. `bought`/`played`/`sold` count the turn's cycles and `levelled` the
  tier button.
- **The lobby is live by default:** `bgpy.era_replays` and
  `EraReplay.from_file` seat each turn's `lobby` block, so the seven opponents
  are the log's living players with their hero, Health, armor, tier,
  placement and last-seen boards. `lobby=False` is the explicit old dead-seat
  sandbox. The frozen set `boards/frozen/actions-2026-11-15/` carries the
  block (same 91 games as `actions-2026-11-10`, re-collected from the logs);
  `train/era_turns.py`, `train/evaltools/demo_dataset.py` and
  `train/evaltools/trinket_picks.py` default to it.
- **Seating every step alone** means a step whose actions the engine refuses
  ends the turn's path where it happens: `missing` carries the replay's own
  words for it. `reset` keeps the log's state before every decision; the
  fidelity pass (`play_era`) turns it off and plays Era's steps forward from
  the turn's start instead, which is what `diffs` compares against the log's
  end state (`shop_replay::compare`).
- **`shop_replay::SeatedStep::play`/`play_on`** is the replay's own step
  runner, split out of `run_once` so a caller can play one logged step (or
  carry a turn forward): the replay's report is byte for byte what it was.
  `bgbench shopreplay` and the search still use `replay_step`, which seats and
  plays one step without help.
- **Whole-turn replay:** `shop_replay::replay_whole_turn` seats one logged turn
  from its own start, puts each step's logged Tavern in place before the step
  (`place_tavern`, with the old Tavern's pool copies returned and the new
  cards' taken), and plays the logged steps in order with
  `SeatedStep::play_on` on the carried game. The step's seated prompts are put
  on the carried game too (`take_step_prompts`): a forced Discover or Dark
  Gift offer replaces the engine's own open offer (returning that offer's pool
  copies, which is what a multi-pick Discover needs), and a trinket or hero
  power offer is installed by its own table. After each step the log's hand
  (`force_hand`) and Tavern go back on the carried game with their pool copies
  moved, so a random card the engine drew is replaced by the one the log shows
  and a later Buy or Play names the same card as the log. It stops at the
  first step it cannot follow and returns the step index, kind and reason.
  `bgbench turn_replay` runs turns 7 to 12 of every action file at several
  seeds and writes `train/runs/engine-pays/turns.json`; `--trace` also writes
  the first diverging step of every stopped turn (`trace.json`) for
  `train/evaltools/replay_sort/sort.py`. Measurement only; normal games do not
  use any of it.
- **`bgpy.RealFights`** loads `boards/real` through
  `real_boards::parse_board_files`, so every board keeps the file and game it
  came from: `opponent(file, game, turn)` is the board Era really fought after
  that turn, `typical(turn, n)` the same evenly spread draw the search fights
  (`search::fight_boards` with no opponent hint), and `fight(batch, slot,
  which, seed)` plays the batch's board against those boards with one seed per
  fight, returning wins, ties, losses and the Health change
  (`search::fight_outcome`, the three-way result `fight_health_change` only
  had as a number). `final_tribe` is the leaderboard's top tribe of Era's last
  board, the comp label for a game the comps run has no name for.
  `RealBoards::places_at`/`indexed` keep a bucket's boards tied to their place
  in the spec list.
- **`bgpy.BoardFight`** (`crates/bgpy/src/fight.rs`) fights two arbitrary
  boards from the tracker (`~/projects/bg-overlay`). Each board is a JSON object in a
  board file's shape (one entry of its `boards` array, so `real_boards`'s
  parser is the only parser) with `turn`, `tier`, `minions`, `hand`,
  `hand_hidden`, `counters`, `hero_power`, `trinkets`, `rewards` and `deity`.
  The constructor seats both sides with `fight_check::prepare`, so hero
  powers, trinkets, rewards, Dark Gifts and hands work as in the fight check.
  `fight(n, seed)` plays them `n` times and returns win/tie/loss shares (for
  the first board), the mean damage it deals when it wins, the mean damage it
  takes when it loses, the same two means over every fight, the same four
  damage figures uncapped (the tier plus surviving minion tiers before the
  turn's cap: `damage_dealt_uncapped`, `damage_taken_uncapped`,
  `damage_dealt_uncapped_mean`, `damage_taken_uncapped_mean`), the pieces the
  engine does not build, and the boards' turn. `missing()`/`complete()` say
  the same before a fight. `built_card_names()` lists what the card data
  builds, for the tracker's coverage report. The first attacker is the
  engine's rule (more minions first, a coin flip when even): the tracker
  cannot pass the log's first attacker.
- **bg's copy of the engine:** the tracker's venv is Python 3.14 (its GTK
  window needs the system PyGObject), so it needs its own build of bgpy.
  `tools/bg-rebuild.sh [COMMIT]` exports a commit with `git archive`, builds
  the wheel for bg's Python and installs it there; a local post-commit hook
  (`.git/hooks/post-commit`, not in the repo) runs it in the background when
  a commit touches `crates/`. Only committed code reaches bg. Log:
  `target/bg-rebuild.log`; the installed commit is in
  `target/bg-rebuild/installed`. A running tracker keeps its old copy until
  it restarts.
- **The fight-state export:** `bgpy.VecEnv.record_fights(True)` makes each
  game keep every seat's last fight at the moment `Game::combat` fixes the
  boards, hands and counters (not at End Turn). `fight_snapshot(i, seat)`
  returns both sides as board-file JSON objects plus the paired seat, and
  `fight_export_limits()` lists what the shape cannot carry. A fight that
  ends the game survives the env reset. Off (the default) is one `Option`
  check per fight and no allocation; on is one box per game and an inline
  copy of the board and hand. `bgcore::fight_export` writes the JSON, so
  `bgpy.BoardFight` reads an exported pair exactly as a board file.
  `bgpy.VecEnv.fight_damage(dealt, taken)` fills each seat's last real
  fight's damage before the turn cap, dealt and taken, as two `[n, 8]` int32
  arrays; a knockout's raw hit survives the env reset, and a tie or a won
  ghost fight is 0. It always records: no flag, no allocation.
- **Not measurable:** a turn whose first step is a trinket pick whose offer
  holds a trinket the trinket table does not build cannot be seated (turns 6
  and 9 are mostly lost that way), and a step the loader cannot seat ends the
  path mid-turn; the report counts both.

## Live move suggester (`train/live/suggest.py`)

A passive helper for `bg-overlay` during hand-played games. It never sends input to Hearthstone or modifies game files.

- **How it works:** watches the newest `Hearthstone_*/Power.log` under `--logs` (default `~/hearthstone-linux/hearthstone/Logs`) through `./target/release/bgbench live-actions --live-log`, seats the latest shop state (including Turn 1 before the first fight, step 0 before the turn's first action, and unanswered Discover / Trinket / Power offers via `pending_choice`) using `bgcore::shop_replay::seat_live` and `bgpy.EraReplay.from_file().live()`, evaluates the policy checkpoint on CPU (`torch.set_num_threads(2)`), and serves v1 JSON lines (`hello`, `decision`, `unsupported`, `game_end`) over `$XDG_RUNTIME_DIR/bg-suggest.sock`.
- **Running it:**

```sh
.venv/bin/python train/live/suggest.py --ckpt train/runs/r1113-a1/best.pt [--socket PATH] [--logs DIR] [--top 3]
```

  Use `--once LOG` to score one log snapshot and print a single JSON event. Requires rebuilt `bgbench` (`live-actions`) and `bgpy` (`EraReplay.live` / `LivePosition`).
- **What it cannot see (and `value: null`):**
  - `value` is sent as `null`: trained checkpoints estimate turn-discounted return (`gamma=0.97` minus move costs and deadline penalties) rather than 1..8 placement.
  - The suggester is one of the paths that eats the real lobby. It calls `EraReplay.from_file(..., lobby=True)`, and every other replay of Era's positions (`era_replays`, the demo dataset, the trinket check, `era_turns.py`) now does the same by default; `lobby=False` is the old dead-seat opt-out. The action file's `lobby` block seats the seven opponents alive with their hero, Health, armor, tier, trinkets, next and last opponent, and the board and counters of the last fight with each (docs/real-boards.md "The action file"). What the log still cannot say stays zero: a player's leaderboard tribe line (`OPP_TOP_TRIBE`, `OPP_TRIBE_MIXED`, `OPP_TRIBE_COUNT`), their last three fights (`OPP_FIGHTS`), and the current board of a player I have not fought (only the last board I saw of them is seated). The suggester then zeroes each opponent's hero and trinkets (`OPP_HERO`, `OPP_TRINKET`) before the network sees them, because a network trained with lineup 1,0,0,0,0,7 only ever played real-board seats, which have none; `--keep-opp-heroes` keeps them.
  - `SCALAR_PARADOX` is drawn by the sandbox rather than parsed from `Power.log`, `dark_gift_uses` / `dark_gift_pressed` start at engine defaults, `SCALAR_LOBBY` holds only single-tribe minion types seen in the Tavern so far, `SCALAR_ACTIONS_LEFT` resets on each seated step, and remaining pool counts are omitted by `obs.rs` (only `SLOT_FROM_POOL` per card slot is present).

## Adding a card

See `docs/adding-cards.md` (hook index, recipes, new-tribe checklist, test helpers, gotchas).

## Card data snapshots (`ref/`, git-ignored)

- `ref/hsjson-cards.json`: the full HearthstoneJSON card file
  (api.hearthstonejson.com/v1/latest/enUS/cards.json), fetched 2026-10-22.
  Has tokens and golden versions. Some retired cards are missing from it.
- `ref/bg-pool.tsv`: the solo Battlegrounds pool from it.
- `ref/lobby5.tsv`: our lobby, built from the JSON (the rule is in
  docs/rules/design.md Part 2). The JSON's pool flags aren't perfect (Rot Hide Gnoll);
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
./target/release/bgbench fightcheck      # play Era's real fights, write boards/checks/
./target/release/bgbench shopreplay      # replay Era's real shop steps, write boards/checks/
./target/release/turn_replay             # whole-turn replay, train/runs/engine-pays/
./target/release/turn_replay --trace     # ... and the first diverging step of each stop
./target/release/bgbench collect-boards  # write Era's board files, boards/real/
./target/release/bgbench collect-actions # write Era's action files, boards/actions/
./target/release/bgbench check-actions   # add up the turns of boards/actions/
./target/release/bgbench planner         # the comp planner against real boards
.venv/bin/python train/eval_overlay.py --games 2048 --bonus 8 --real-lobby-filter
cargo run --release -p bgsandbox        # the sandbox, boards go to boards/
```

`fuzz` and `games` also take `--real-boards DIR [--real-seats N]` (default 2)
to seat real-board seats playing DIR's boards (docs/real-boards.md); without
a real boards/ folder, `crates/bgcore/tests/data/real-boards/` is a small
stand-in. Needs `ref/bg-cards.json` for the card id mapping. `--planner PATH`
on either gives seat 0 the comp planner. `--era-starts DIR --era-share PCT
--era-turns MIN,MAX` start that share of games from Era's logged positions
(see "Era starts"); the fuzzer checks pool accounting after every action in
them too.

`planner [--comps PATH] [--games N] [--vs real] [--seat planner|standard|rush]`
(see "The comp planner") plays one seat against seven real-board seats and
prints the placement and comp numbers, then ten final boards.

`fightcheck [--boards DIR] [--runs N] [--out DIR] [--flag PCT]` plays every
paired fight of DIR `--runs` times on every core and writes `fights.tsv`,
`cards.tsv` and `report.md` to `--out` (docs/real-boards.md "The fight
check"). It only reads the engine: nothing in a fight changes. Needs
`ref/bg-cards.json` for the card id mapping, like `--real-boards`.

`shopreplay [--actions DIR] [--seeds N] [--out DIR] [--card NAME]` replays
every step of Era's real shop phases from the logged state before it and writes
`steps.tsv`, `shop-cards.tsv` and `shop-report.md` to `--out` (docs/real-boards.md
"The shop replay"). It only reads the engine, and it needs no
`ref/bg-cards.json`: the action files name every card they hold. A step that
draws a random number is run `--seeds` times and judged by shape.

`games 10` on this machine (release, 32 threads, five-tribe lobbies with
heroes) was about 13,300 games a second after hero batch 3: one core 1.04 ms
a game, 1.2 us an action. The same build of `git archive HEAD` made 13,700,
so the batch costs 2.7%. `fuzz 100000` takes about 14 s.

After trinkets block 5 (2026-11-01) the same measure is about 12,800 games a
second (one core 1.05 ms a game, about 1.21 us an action); the committed tree
built beside it made 13,200, so the block costs about 1.5% to 3% (noise; the
one change on every tribe read is `CardDb::tribes_of` ORing
`Minion.extra_tribes`, the Bronzebeard Portrait's rule). `fuzz 100000` still
takes about 14 s.

Python side, from the repo root:

```
VIRTUAL_ENV=$PWD/.venv PATH=$PWD/.venv/bin:$PATH maturin develop --release -m crates/bgpy/Cargo.toml
cd train && ../.venv/bin/python ppo.py --name NAME --minutes 480 --stop-flat 90
../.venv/bin/python status.py runs/NAME
```

The venv is Python 3.12 with torch (cu128) and maturin, made with uv.
Deleting `.venv` undoes it.

## Training notes

- `train/bgt.py` (I link it as `bgt`) wraps these: `bgt` shows the run and
  its latest numbers, `bgt go HOURS [RUN]` starts or continues a run (it
  refuses while one is going; a new run copies the last run's flags and is
  named rDDMM by default), `bgt stop`, and two strengths beside full:
  `bgt half on|off` caps the run at 8 GB of GPU memory, 24 CPU threads and
  the lowest priority so the PC stays usable, and `bgt night on|off` gives
  it the whole GPU while the desktop is off (`train/night/bg-night`). The
  three strengths are exclusive: turning one on drops the others' flags from
  `runs/NAME-args.txt`, and a running run restarts with the time it has left.
  Both use `--vram-gb` and `--micro-batch` (the same update, run through the
  network in pieces); night also sets `--no-recompute` and `--obs-fp16`, and
  picks its micro-batch from the model shape in the run's args file. `bgt
  menu` is a full-screen menu that refreshes itself; `train/bgt.desktop`
  (linked into `~/.local/share/applications`) opens it in kitty.
- Night strength, for overnight runs with the desktop off, keeps what the
  recompute pass would throw away: `--no-recompute` calls the attn blocks
  directly instead of again in the backward pass (the same loss and
  gradients, bit for bit on CPU), and `--obs-fp16` stores the rollout's observations
  in float16, widened again just before the network. Float16 rounds nothing
  there: every observation is a whole number and the largest in 32,768 real
  observations was 472, below the 2048 float16 holds exactly. `--vram-gb 15`
  of the card's 16.3 leaves about a gigabyte for the driver, and night needs
  no room for Hyprland because `bg-day` puts a running run back to half
  strength before the desktop comes back. The micro-batch is 8192 for the
  attn default and 4096 for r1105-big's 4 layers of d256 (2048 for any other
  shape). Measured 2026-11-06 with the desktop off (nvidia-smi peak,
  seconds per update between updates 20 and 40): big full 15.2 GB 11.1 s,
  big night 4096 10.9 GB 10.5 s, big night 8192 out of memory at the 15 GB
  cap; small full 14.2 GB about 6.5 s, small night 8192 12.3 GB about the
  same. So night saves memory but hardly time (about 5% on the big model,
  not the fifth the plan guessed): the update is not limited by the
  recompute. Full's high peak is the caching allocator taking what is free.
  One small night test died in its first 20 updates on NaN action
  probabilities and its repeat ran clean; cause unknown, and go.sh would
  restart such a crash from the last checkpoint.
- Day to day, `./train/go.sh HOURS` continues the current run
  (`r1025-attn`) for that many hours and stops, detached, resuming from
  the last checkpoint after a crash; `./train/stop.sh` stops it, and
  `train/runs/go.log` says what happened. Each start reloads
  `boards/real`.
- One network plays every learner seat. Seats per game come from
  `--lineup` (learner, snapshot, standard bot, rush bot, random, real; the
  real seats play boards from `--real-boards`, default `boards/real` when
  the lineup has any).
- With more than one learner seat per game, a placement is credited only to
  a seat that has acted in the current rollout (`last[e, s] >= 0` in
  `ppo.py`). A seat knocked out before it acts again in a new rollout gets
  no reward for its place: 13.2% of learner placements, about a quarter of
  6th to 8th places (measured 2026-11-11 on the lineup `2,2,0,0,0,4`; the
  wasted samples are not biased given the state). With one
  learner seat (`1,0,0,0,0,7`) every placement arrives in the same step as
  the seat's End Turn, so nothing is lost; no fix yet for the wider lineups.
- The comp overlay is off in training unless `--overlay-comps FILE` is given
  (`--overlay-bonus B`, default 8). A PPO forced press (Dark Gift, level)
  wins over the overlay on that row.
- Scenario starts are off unless `--scenario-comps "A;B"` is given
  (`--scenario-share`, `--scenario-turns`; see "Scenario starts"). They need
  real boards and the overlay's comp file.
- Era starts are off unless `--era-starts DIR` is given (`--era-share`,
  default 0, and `--era-turns`, default 7,11; see "Era starts"). Training
  draws from the train split; they need real boards.
- Reward is placement only (+1 for 1st to -1 for 8th), plus an optional
  `--first-bonus` for 1st and `--second-bonus` for 2nd (both default 0, which
  is exactly the linear reward), minus 0.002 per Move action (`--action-cost`,
  `--cost-kinds move`) so it doesn't shuffle its board for free; buys, sells,
  plays and refreshes cost nothing extra, since gold already limits them and
  long gold-neutral loops are good play. Discounting is per turn, not per
  action; with per-action discounting nothing learned, because a turn can be
  up to 200 actions.
- `--fight-reward W` adds +W at a learner turn end when the fight after it is
  won, 0 for a tie and -W for a loss or a knockout (1st place is a win); W = 0
  is the old reward, and `metrics.csv` appends `fight_reward_per_turn`, the
  mean term per learner turn.
- `--fight-score W` (default 0) adds W * (uncapped damage dealt - uncapped
  damage taken) / 48 at the same learner turn end, with the engine's raw
  damage before the damage cap (`bgpy.VecEnv.fight_damage`), so a heavy loss
  differs from a light one; a tie is 0, and a knockout counts the raw hit
  that decided it. `--fight-score-from T` (default 1) pays only fights after
  turn T, and `--fight-score-fade F` (default 0) falls linearly to 0 over the
  last fraction F of the run's planned end (`--minutes`). `--fight-score` with
  `--value-warmup` is refused. `metrics.csv` appends `fight_score_per_turn`,
  the mean term per learner turn.
- `--win-score W` (default 0) adds +W for a fight won, W/2 for a tie and 0
  for a loss at the same learner turn end, from the observation's won/lost
  last-combat scalars (exact where damage is not: a won ghost fight deals 0,
  and a loss can take 0). `--win-score-from T` (default 9) pays only fights
  after turn T on; a knockout is a loss, and 1st place (the final fight won)
  pays W. `--win-score-fade F` (default 0) falls linearly to 0 over the last
  fraction F of the run's planned end (`--minutes`), like the fight score's
  fade. It may be used with `--value-warmup`. `metrics.csv` appends
  `win_score_per_turn`, the mean term per learner turn from turn T on.
- `--demo PATH --demo-weight W --demo-fade F` (default off) adds the
  demonstration loss from the Build A dataset
  (`train/evaltools/demo_dataset.py`): each PPO update adds W times the
  cross-entropy between the network's masked policy and Era's action on a
  fresh train-split demonstration minibatch of `--demo-batch N` rows (the
  PPO minibatch size when N is 0). The demonstration loss is a mean, so N
  changes the cost, not the meaning of W. The weight falls linearly to 0
  over the last fraction F of the run's planned end, by the same helper as
  the fight and win score fades, and the term is not added during
  `--value-warmup`. The dataset must have this
  build's observation width, built with `old_boards='keep'` and no lag; a
  mismatch is refused. Every eval prints the held-out masked top-1 agreement
  overall and on buys, plus the same count on a fixed train sample of the
  held-out size, and `metrics.csv` appends `demo_top1`, `demo_top1_buy` and
  `demo_top1_train`. The policy that acts is unchanged.
- `--demo-augment` (default off, needs `--demo`) makes each drawn
  demonstration row a different but equally true picture. It independently
  permutes the seven Tavern minion slots and the ten hand slots in the
  observation, remapping every action that names one (Buy and TargetShop;
  Play with or without a board position, and Discard) and the legal mask with
  the same permutation. It also replaces the next opponent's seven board
  slots and `OPP_TRIBES` with one of the dataset's K alternative boards,
  keeping the logged board with probability 1/(K+1); the named player's other
  scalars stay as logged, and a row whose logged next board is empty keeps it
  because there is no board to replace. The learner's own board is never
  touched. The alternatives come from
  `train/evaltools/demo_dataset.py --alt-boards K`,
  which encodes real boards of the same turn through `EraTurnPath.observe`
  (the engine's own observation code) and stores them as `alt_opp`. The
  build's `--real-lobby-filter` uses the trainer's lobby-fit rule; a late turn
  with fewer than K eligible boards repeats some of the same-turn boards. A
  dataset without `alt_opp` makes `--demo-augment` stop with a clear error
  rather than run the shuffles alone. `train/runs/demo-dataset-live-aug/demo.npz`
  is the live dataset with K = 8 and the filter on. Off, the draw is exactly
  the old one.
- `--demo-kind-weight W` (default 0, needs `--demo`; it may run with or
  without `--demo-weight`) adds W times the cross-entropy between the network's
  masked policy summed over the legal actions of each grouped action kind and
  Era's kind, on the same fresh train-split demonstration minibatch. The
  groups are buy (buy and buy_spell), sell, play (play and activate), roll,
  level, freeze, hero power (hero_power and hero_power2), move, choose
  (choose, target, target_shop, discard, choose_trinket and dark_gift), and
  end. It uses `--demo-fade` and is skipped during `--value-warmup`. With it
  on, every eval also prints the held-out and fixed-train top-1 kind
  agreement, logged as `demo_kind_top1` and `demo_kind_top1_train`, and
  startup prints BASE's held-out kind agreement once, for reference. The
  policy that acts is unchanged.
- `--buy-prior PATH --buy-prior-weight W --buy-prior-fade F` (default off)
  loads the streamer buy model (`train/evaltools/buy_prior.py`, the class and
  its loading and logit helpers are shared with `oneturn.py`). Each PPO update
  adds W times the cross-entropy from the model's softmax over the legal buy
  slots to the network's own softmax over the same slots, on rows with at least
  two legal buys. The 8 slots are the 7 Tavern minions and the Tavern spell,
  in the buy actions' order. The term is skipped during `--value-warmup`, and
  `--buy-prior-fade` uses `--demo-fade`'s fade to 0 at the run's planned end.
  Every eval also compares the network's top legal buy slot with the model's
  top legal buy slot on the learner decisions of the standard-bot eval, logs
  `buy_prior_top1`, and prints it in the update line; startup prints BASE's
  value on 128 standard-bot games once, for reference. The policy that acts is
  unchanged. With W = 0 nothing is loaded and the run is exactly as before.
- `--entropy-moves-as-one` counts all Move actions as one entropy atom (their
  summed probability) instead of one per move, so the bonus stops paying for
  board shuffles; moves stay legal and sampled, off is the old loss, and
  `metrics.csv` appends `entropy_plain` (the ordinary term) and
  `moves_per_turn` (learner moves over learner turn ends).
- The starting policy is uniform over action kinds, not over all actions,
  or it almost never ends a turn.
- Snapshots of itself are taken every 250 updates (at about 95 updates a
  minute, 10 was only 2 minutes of history).
- `train/fixed_eval.py CKPT [CKPT2 ...] --vram-gb 10 [--games 2048]`
  is the fixed yardstick for comparing checkpoints: one learner against
  seven real seats, exactly one game per seed, played to its end (unlike
  the eval inside `ppo.py`, which stops at the first N finished games and
  over-counts short ones). It runs in chunks of 512 envs and writes
  `train/runs/fixed-eval/NAME/report.md` and `results.json`; with two or
  more checkpoints it adds a paired table. `--temp T` (default 1.0, which
  is the old path; it must be above 0) divides the network's logits by T
  before sampling. Each checkpoint's report and `results.json` also carry
  the tier curve: share levelling on turn 2, and tier at the end of turns
  5 and 8 (over games alive after that turn), computed as
  `review2/tiercurve.py` does and shown against the plan's minimums of
  90%, 2.85 and 4.60 with PASS or FAIL. The real seats come from
  `boards/frozen/real-2026-11-10/`, a frozen copy checked against its
  `MANIFEST.txt` before the run. With `--era-starts` (default
  `boards/frozen/actions-2026-11-10/`, also manifest checked),
  `--era-turns` and `--era-split`, every game is an Era start and the
  report adds body kept, place by start turn and the Era board's totals.
- `train/evaltools/turn_motion.py A.pt B.pt ... --games N --vram-gb G`
  measures rolls, buys, sells and cycles on turns 10 to 13, one learner
  against seven real seats; `--cpu` runs the same measurement without the
  GPU. A cycle is a minion sold in the same turn it was
  bought. `cycles` counts every one; `cycles_fx` counts only minions whose
  cards.toml data has a Battlecry, a Choose One, a play trigger or an
  `on_sell` effect, so churn that pays nothing is separated from a cycle that
  does something. The report and `results.json` carry both lines and, per
  checkpoint, the ten most cycled card names with their counts and whether
  each counts for `cycles_fx`.
- Every eval writes a row to `runs/NAME/metrics.csv`: placement against
  each bot, a curve report (turn reaching tiers 3, 4, 5; share of rush,
  standard, economy rush and slow games), a tribe report, entropy and the
  action mix. With real boards loaded (real seats in the lineup, or
  `--real-boards` given), `place_vs_real` holds the one-learner-vs-7-real-
  seats eval. With scenario starts on, the `scen_...` columns hold the
  scenario eval per comp and pooled. With Era starts on, the `era_...`
  columns hold the holdout-split Era eval.
- `--stop-flat N` stops a run after N minutes without gains, but only when
  it also no longer beats its oldest snapshot.
- `--max-updates N` stops cleanly after update N, whichever of it and
  `--minutes` comes first. The stop uses the same end-of-update path as the
  minute limit, so `ckpt.pt`, `best.pt`, `metrics.csv` and the keep copies
  are written exactly as at the end of a `--minutes` run. `train/go.sh`
  passes it through with the run's other flags.
- Any change to the action space or observation breaks old checkpoints;
  start a new run, except where the change only appends actions or input
  columns. The cycle action (2026-11-08) is removed, so `N_ACTIONS` is 183
  again and every old index means what it meant before the cycle kind. A
  checkpoint or recording from the cycle build has 200 actions: `imitate.py`
  keeps the first 183 and drops examples whose action was a cycle, and a
  cycle-era checkpoint's action rows no longer match, so start a new run from
  one of those. The 2026-10-25 observation rewrite (all ten tribes, the
  lobby, the Deity, Blood Gems, attachments, and opponents' Deity, gem value
  and spell power as last seen; OBS_SIZE 2658 to 6362) broke every earlier
  checkpoint, expected. The heroes run the same day added own and opponents'
  heroes (a hero embedding beside the card one), armor, the power state, the
  hero offer during the pick and the `hero_power` action kind (OBS_SIZE to
  6389, SCALARS 173, OPP_SCALARS 30): every checkpoint before it breaks too.
  Hero frame 2 (2026-10-31) added the second power to the observation (six
  scalars right after the main power's), the powers an open Hero Power
  Discover or chooser offers (one per Choose slot, power index + 1, so the
  bot picks a power it can see; four for Bru'kan's Elements, three for a
  Discover), a fourth `choose` slot and the `hero_power2` action kind
  (SCALARS 195 to 199, N_ACTIONS 182): every checkpoint before it breaks as
  well. Hero batch 2 (2026-11-01) added hats, the Darkmoon Ticket mark and a
  frozen card to the card slot (SLOT_FEATURES 86 to 89, OBS_SIZE 7145 to
  7292), and hero batch 3 the reanimate mark and sixteen scalars
  (SLOT_FEATURES 89 to 90, OBS_SIZE 7292 to 7385), so every checkpoint before
  it breaks too. Poisonous (2026-11-04) added the eighth keyword slot to the
  card slot (SLOT_FEATURES 90 to 91, OBS_SIZE 7400 to 7477), which breaks them
  all again. The opponent view (2026-11-06) gave every opponent block the
  leaderboard's tribe line and their last three fights (OPP_SCALARS 33 to 54,
  OBS_SIZE 7477 to 7624). That one does not break them: `warm_start` copies
  the old columns of an input layer that only grew into their places and
  starts the new ones at zero, so an old checkpoint answers exactly as it did
  until it learns to use them, and `--resume` does the same (with a fresh
  Adam, whose state has the old shapes). A hero id and a power id share the
  model's one id embedding, so its
  table is `max(N_HEROES, N_POWERS)` wide and the offer slots are embedded
  from it too (`HERO_IDS` in train/model.py), not scaled as numbers.
- Games from `bgpy` play with heroes (`Game::new_random_heroes`, no pick),
  except real seats which get none; learner seats answer any power Discover
  with `choose`.
- `--model` picks the network: `mlp` (default, 3.1M parameters, all slots
  concatenated), `attn` (a 3-layer transformer over the card slots, 0.65M)
  or `pool` (the same slot tokens pooled per zone, 0.76M). The last two
  score per-slot actions from the token of the slot they act on.
  `--opp-boards` sets how `attn` sees opponent boards: `full` (every
  opponent minion a token, 85 tokens), `pooled` (each board pooled into
  its opponent's token, 36) or `next` (pooled, plus the next opponent's
  board in full, 43; the default). Both are saved in the checkpoint and
  `--resume` builds what the checkpoint holds. `attn` runs its layers in
  bf16, recomputes them in the backward pass to fit a minibatch of 8192
  (`--no-recompute` keeps the activations instead: the same math, more GPU
  memory, night strength's trade), and is compiled with torch.compile unless
  `--no-compile`. It does about
  20 updates a minute with `next` (10 with `full`) against `mlp`'s 70,
  `pool` about 35. `python model.py` checks that the per-slot logits
  follow their slots; `python model.py PATH` also loads that checkpoint with
  `warm_start` into this build and checks that it answers exactly as its own
  network did on the same positions, the grew-in columns at zero (how an old
  run crosses an observation that only grew).
- `--card-text` (2026-11-09) adds a vector read from each card's printed
  Battlegrounds text to its id embedding (the id embedding stays). `card_text.py`
  reads the text by name from `ref/hsjson-cards.json`, drops markup and
  `(N left!)` counters, makes `+N/+M` one `<buff>` token and every other
  number `<num>`, then builds a fixed table: one word embedding, one
  pre-norm self-attention layer over the card's words, the mean over its
  present words, projected to `emb`. The whole table is computed once per
  forward and indexed by card id; a textless card's vector is zero. The
  vocabulary and token table are saved in `model_opts`, so `--resume` and
  the eval scripts rebuild the model without reading `ref/`; `model.load` does
  read today's export once to widen the word table when the vocabulary grew,
  and falls back to the saved list when `ref/` is absent. A checkpoint
  without them has no text parameters and is unchanged. The shared slot
  encoder means all three models (`mlp`, `attn`, `pool`) read it.
- `--power-text` (2026-11-13) adds the printed text of every hero power and
  trinket to those id embeddings, through one learned scalar gate per table
  that starts at exactly 0: a checkpoint loaded into it answers as before
  until the gate learns. The rows are read by card id from `heroes.toml`
  (one starting power per hero, then the `[[power]]` entries) and
  `trinkets.toml`, share the card-text word embedding and vocabulary, and are
  saved in `model_opts` like the card table. `model.load(path,
  power_text=True)` and `warm_start` widen an older card vocabulary by word,
  start the new rows fresh and leave the gates at 0. Dark Gifts need no
  separate input: the observation shows them only as their effect card's id
  in the main card table (the gift slots on minions, and a Discover slot's
  gift field during an offer), so the card text already covers them.
  `fixed_eval.py --power-text`
  forces the fresh parts for the gate-0 check, and `ppo.py` prints the gate
  values at startup and in each update line. With `--init-from` a
  checkpoint that has no text, `--power-gate-init X` starts both fresh
  gates at X instead of 0 (a gate at exactly 0 barely moved in 5 hours).
- `--init-from PATH` starts a new run (fresh optimizer) from every
  parameter of another checkpoint of the same model whose name and shape
  match, for example `attn` with `full` into `next`; it prints what was
  fresh. `--warmup N` ramps the learning rate up over N updates.
- The card, hero and trinket embeddings carry over by name, not by position:
  a card added, removed or moved in `cards.toml` (a new patch, a token) keeps
  every other card's row, and only new cards start fresh. Every checkpoint
  saves `card_names`, `hero_names` and `trinket_names` for this; a trinket
  name can occur twice (a lesser and a greater version), so rows match by
  occurrence in that table's order. A checkpoint from before those names were
  saved has none; `--init-cards-rev REV` rebuilds them from `cards.toml`,
  `heroes.toml` and `trinkets.toml` at the commit it trained on
  (`runs/n1025-attn` trained on `7b545bd`). Without that flag, a missing
  table is rebuilt from the newest commit at or before the checkpoint's mtime
  whose table has the checkpoint's size. The card-text word embedding carries
  over by word too: the word list is the `card_text["vocab"]` every
  `--card-text` checkpoint saves, and a word it did not know starts fresh in
  `warm_start` or takes the padding row in `model.load`. The eval scripts'
  `model.load` remaps the same three tables and the text buffers (card,
  power and trinket ids and word ids) on load: a name it did not know gets
  the empty row (0), so the loaded network answers as it did on everything it
  knew. The fresh token rows are kept from today's export rather than copied
  from the checkpoint, since a moved word id would undo the vocabulary
  alignment. `--resume`
  still needs the same cards: after a card change, start a new run with
  `--init-from`.

## An older simulator

An older private simulator, in Python, models fights for about 325 cards
of every tribe, much of it checked by hand against real games. It is the
best reference for how a card fights. `docs/rules/design.md` wins where
the two disagree (for example, a per-type pick gives each minion at most
one buff, unlike the simulator's `_phalanx`).

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
  as-last-seen opponent Deities, opponents' gem value and spell power from
  the last fight with them, their leaderboard line (the top tribe and its
  count, or "mixed", taken for every living player at the start of each
  combat, so it changes once a round and not with the live board) and their
  last three fight results with the damage dealt or taken, plus "fight
  stats" (the shop Goldrinn bonus a Beast in hand gets when played; Holy
  Vanguard's +30/+30 is not one, it is in the slot's own Attack and Health
  while the Health is 15 or less).
  Relentless Deflector's Taunt while shielded isn't a feature; the
  card embedding has to learn it.
- An opponent's last-seen board is what the bot used to read an opponent
  from, and Era does not remember boards (docs/notes/era-strategy.md
  "Guessing opponents, and the turn-2 level"). `obs::OldBoards` decides how
  much of it goes in: `keep` writes every opponent's board however old (the
  observation before 2026-11-06), `drop` writes only the opponent fought
  last turn and the one fought this turn when that board is at most
  `OLD_BOARD_AGE` (2) turns old, zeros for the rest, their scalars and the
  new lines untouched. The size is the same either way, so the two can be
  compared on one network: `bgpy.VecEnv(.., old_boards="keep"|"drop")` and
  `--old-boards` in train/ppo.py (default `keep`, and the checkpoint's own
  on `--resume`).
