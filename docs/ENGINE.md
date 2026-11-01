# How the engine and training fit together

A map of the code for whoever works on it next. The rules themselves are in
`DESIGN.md`.

## Layout

```
cards.toml            every card: minions, tokens, spells, golden behaviour
rules.toml            every rule number: gold, costs, pool copies, damage caps, limits
heroes.toml           the built heroes and their powers (see "Heroes")
trinkets.toml         the built trinkets, lesser and greater (see "Trinkets")
darkgifts.toml        the 47 Dark Gifts: windows, weights, host conditions, the
                      offer tiers by turn (see "Dark Gifts")
peak_boards.toml      hand-made peak boards for `bgbench peak` (guesses, see below)
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
    src/action.rs     the flat action space and its index mapping
    src/obs.rs        the observation a player sees, as fixed-size f32
    src/bots.rs       random, standard-curve and rush-curve bots
    src/rules.rs      Rules from rules.toml
    src/rng.rs        xoshiro256++, written here so replays never change
    src/record.rs     recorded sandbox games: TOML writer and the loader back into Minions
    src/real_boards.rs the boards of Era's real games: file parsing, card id
                      mapping, the turn buckets a real seat draws from
    tests/            one file per tribe (beast_mech, dragons, murlocs, demons, pirates, elementals, undead, quilboar, aberration, neutrals),
                      heroes.rs (the hero frame: pick, armor, the 22 powers, the two spells),
                      trinkets.rs (the trinket frame: the two offers, the guarantee and the
                      discount, the slots, the hooks and the built cards),
                      prizes.rs (the Darkmoon Prizes: the Discover, the Triple Prize,
                      one test per Prize),
                      heroes_batch1.rs, heroes_batch2.rs, heroes_batch3.rs,
                      heroes_batch4.rs (Tickatus), heroes_frame2.rs,
                      darkgifts.rs (the gift table, the button, the offers, the landing,
                      the countdowns and the gift hooks),
                      tier7.rs (tier 7 in the engine and its seven minions),
                      deathrattles.rs (a loop over every Deathrattle card),
                      lobby.rs (the 5-tribe lobby and every draw that respects it),
                      core_rules.rs (the generic rules on the real data), fight.rs,
                      game.rs, real_boards.rs (real-board seats; its small fixture
                      folder tests/data/real-boards stands in for boards/real),
                      sandbox.rs; parser tests build small TOML strings inline
  bgbench/            benchmarks, fuzzer, bot matches, the peak-board balance matrix
  bgpy/               PyO3 bindings: a batch of games stepped together
  bgsandbox/          terminal sandbox: play shop phases alone, boards recorded to boards/
train/
  model.py            policy and value network
  ppo.py              PPO self-play, evals, checkpoints
  real_boards.py      loads boards/real for training (card id to name, summary)
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
  the only legal actions answer it. A choice no answer can satisfy (no card
  reaches one today) fizzles in `step`, through `Game::choice_stuck`: the
  effect is lost, the normal actions stay legal, and one warning line goes
  to stderr.
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
  Fishbait, and the trinket-only minions of DESIGN.md ("Generated minions
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
  `Player::counters` reads both held powers.
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
  generate powers (docs/briefs/heroes-batch1.md, the rules in docs/heroes.md
  "Hero batch rules (Era, 2026-11-01)"):
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
    no pool copy moving; `abandon_discover` returns none). `Champion` deals
    three Tier 7 minions and the pick lands in hand with
    `Minion.locked_gold` set (`discover_locked_gold`), the shared lock of
    DESIGN.md "Cards locked in hand". `MagneticMinion` is a plain draw with a
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
    the same way.
  - Reliquary Research reads `Game::spell_price` (0 for every third Tavern
    spell bought) in both spell buy paths, and The Galaxy's Lens hands over a
    fresh copy of `Player.last_spell`.
  - **Cards locked in hand** (DESIGN.md) are one lock in the engine:
    `Minion.locked_in_hand(tier, gold_spent)` over `locked_turns`,
    `locked_tier` and `locked_gold`, read by `is_legal` (Play),
    `valid_discard`, the fight's summon-from-hand (`Counters.gold_spent`) and
    the observation's `SLOT_LOCKED` (the turn count, or 1 while a tier or
    Gold lock holds it). Every card that locks reuses it.
  - **Hero batch 2 (2026-11-01)**, the stats, target and Tavern powers
    (docs/briefs/heroes-batch2.md, the rules in docs/heroes.md
    "Hero batch rules (Era, 2026-11-01)"):
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
      (the highest-Tier minion, frozen, outside the normal slots) and
      Enhancification's two random Bonus Keywords.
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
      Galakrond's Greed puts the old card back in the pool and Discovers from
      exactly one Tier higher into its slot, with no Tavern buffs. Arcane
      Alteration replaces a card on the board or in the Tavern with a pool
      card of the same Tier, keeping the stats the old card had gained.
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
    - **A power that arms an end of turn.** `PowerState.armed` is set by
      Saturday C'Thuns!'s press and spent in `power_end_of_turn`, where the
      repeats are the power's own turn count. `HeroPower.swaps_to`
      (`swaps_each_turn` in heroes.toml) is how Major Hymn becomes Minor Hymn
      at the end of every turn.
    - `cast_spell` ends by dropping a target prompt nothing can answer, so a
      spell's own "after you cast" triggers can never leave the player
      stranded.
- **Hero batch 3 (2026-11-01)**, the fight, kill and rest powers
  (docs/briefs/heroes-batch3.md, the rules in docs/heroes.md "Hero batch
  rules (Era, 2026-11-01)"):
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
    Health (`Side.pick_highest`, ties at random) and Tentacular's Tentacle.
    `Side::exact_copy` is the copy: undamaged, buffs and keywords kept, no
    pool copy.
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

## Trinkets

`trinkets.toml` (DESIGN.md "Trinkets and Dark Gifts (2026-10-30)") holds the
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
  less. `pick_trinket` pays the price (Gold, so "Gold spent" counters see it),
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
  only: see "Spells versus Tavern spells" below), `trigger_on_cast_spell_on_target`
  (CastSpellOnMinion), `discard_card` (Discard) and `fire_event(GoldSpent)`.
  The turn flow calls `trinket_start_of_turn` (offers, uses back, the Turns
  counter, the start-of-turn effects) and `trinket_end_of_turn` (the
  end-of-turn effects, then an "every N turns" rule that reached N). A hook
  list with `uses_per_turn` is capped per turn.
- **The effects.** `Act` mirrors the effect keys of cards.toml (`get`,
  `get_gems`, `cast`, ...) and `Game::run_trinket_act` runs each through the
  same `Effect`/`SpellEffect` arm the card path runs, with no host slot (`pos`
  0) and no golden multiplier. Five keys have no card behind them:
  `upgrade_discount`, `buff_tavern_this_turn` (Cursed Crystal's per-Refresh
  buff must not be remembered and stack), `make_golden_random`,
  `last_spell_copies` and `discover_deathrattle` (Deathly Phylactery; a
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
  back every turn. Floating Candle Set is not built: triggering a Deathrattle
  outside combat is a shop system the engine does not have (docs/KNOWN-GAPS.md).
- **The Tavern's shape (block 3).** `Player::shop_size` is the one Tavern
  size: `rules.shop_size(tier)`, or six with Glowing Gauntlet, plus one for
  Innkeeper's Stein (`extra_high_tier`: a minion of the tier above, none at
  tier 6) and one for Electrode Attractor (`extra_magnetic`), never past
  `SHOP - 1` minions because the seventh slot is the spell (DESIGN.md).
  `fill_shop` draws the extra slots and takes its tier bounds from
  `tavern_tiers` (Bob-blehead's `min_tier`, Guiding Candle's
  `tier6_refreshes`, which the turn-start roll counts down too when the
  Tavern changed). `Game::refill_tavern` runs after every action and tops a
  Gauntlet Tavern back up: a bought, consumed or stolen minion by a minion,
  the spell by a spell. `minion_price` reads the set prices
  (`magnetic_cost`, `tavern_price`).
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
  shop destroys (DESIGN.md: shop deaths count for death counters) in one
  slot counter that carries across turns. A shop destroy counts deaths but
  not Avenge: a trinket's Avenge is Avenge, and its counts start from 0 in
  every fight.
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
  runs the end-of-turn list once per Drakkari.
  `Player.trinket_events` is a `u32` for them; the mask test is still the one
  thing every hook pays.
- **Battlecry multipliers (block 4).** One place counts a Battlecry's
  triggers: `battlecry_or_choose_one`. It is Brann's `brann_mult` (set or add)
  plus `tp.war_drum` (War Drum's two extra, spent from that trinket's own
  `uses_left` by `take_war_drum`) plus `tp.dragon_battlecry` for a Dragon
  (Dragon's Eye). `Game::run_battlecry_at(p, pos, random)` triggers a
  Battlecry outside a play (Young Murk-Eye Sticker, Hackerfin Portrait), and
  with `random` it answers the target the Battlecry asked for at random out of
  the board and the Tavern (`answer_target_at_random`,
  `valid_battlecry_shop_target`: only a plain "a minion" buff reaches the
  Tavern that way). A Battlecry never panics on a slot that a hook before it
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
  rules, guarded by `tp.locket_copy` so it never chains.
- **Spells versus Tavern spells (`cast_spell`, fixed 2026-10-31).** Two
  events, and every card or trinket that reacts to a cast is on the one its
  own words name (DESIGN.md "Read the card's exact words, every time"):
  `Event::SpellCast` fires for every spell cast (a Tavern spell, a Blood Gem,
  Gem Day, a Banana, a Spellcraft spell), `Event::CastSpell` for a
  `tavern_spell = true` card only. The trinket lists are `on_any_spell` and
  `on_cast_spell`; the rule events are `spell_cast` and `cast_spell`. A
  Triple Reward is not cast at all (it is a reward you open) and is skipped in
  `cast_spell`. The minion side: `Card.on_cast_spell` stays Tavern-only (every
  card that has it says "Tavern spell"); `EventKind::SpellsCast` and
  `Player.spells_cast` (the observation's "Tavern spells cast this game", and
  Faceless Converter's per-Tavern-spell improvement) stay Tavern-only, and
  `EventKind::AllSpellsCast` is the "after you cast N spells" counter (Felboar)
  with no player scalar. `on_cast_spell_on_self` and `on_cast_spell_on_tribe`
  were always any-spell (they fire from `trigger_on_cast_spell_on_target`). In
  a fight, `Effect::CastSpell` fires the Tavern counter and the Tavern-only
  minion triggers only for a Tavern spell, and the trinket's `when = "cast"`
  hook for every spell. Replica Cathedral's "your first spell each turn casts
  an extra time" covers any spell, a Blood Gem included (`gem_casts`).
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
  Master's watch: the player keeps a snapshot of every board slot
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

## Dark Gifts

`darkgifts.toml` (docs/darkgifts.md; DESIGN.md "Trinkets and Dark Gifts")
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
  (docs/darkgifts.md): `push_gift` attaches a second copy, which fires a
  second time, except for the gifts that set a state (`Card::gift_unique`:
  the keyword gifts, Amalgamation, Toreth's Blessing's hits and Gilding),
  which `push_gift_unique` drops. Past `MAX_GIFTS` a new gift is not attached.
  A triple takes the gifts of all three copies, a Magnetized Mech keeps its
  own (the magnetic minion's are lost) and a sold minion's go with it.
  `CardDb::tribes_of(&Minion)` is the one tribe read that knows about
  Amalgamation ("has all minion types").
- **The landing** is `game::darkgifts::land_gift`: the effect card's stats
  (its `stats` are what the gift adds), its keywords, `shield_hits` for
  Toreth's Blessing, Gilding's `golden_up`, the "for each ... this game"
  counts taken now, and the counter. Everything else the gift does is read
  later from its card by the existing paths; `Minion::has_gift` is the one
  flag test a fight pays for a gift with no trigger.
- **The button.** `Action::DarkGift` (the last action, `N_ACTIONS` grew by
  one); `dark_gift_usable` is the rule (rules.toml's `dark_gift_cost`,
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
- **The pool the offers come from** is `GiftDb::pool_allows`: never a spell
  or a token, never a Magnetic minion, one with "when you sell this" or one
  whose effect works in hand, no Battlecry or Choose One minion before turn
  5, and darkgifts.toml's `exclude` list (Leeroy the Reckless, Deadly
  Spore).
- **Who gets a gift.** `GiftDb::pick` chooses at random among the gifts whose
  window holds the turn and whose condition the host meets, never one already
  on another offer of the press; the four weight-50 gifts count half.
- **Countdowns.** `dark_gift_start_of_turn` (called from `start_turn`) ticks
  Incubation, Replication and Affinity and runs the countdown's effect at 0
  (the gift then comes off), and gives every Time Turning minion its end of
  turn effects again (`run_minion_end_of_turn`, the same code the end of the
  turn runs).
- **The other hooks.** `dark_gift_card_played` ("whenever you play a card":
  Dexterity, Sharpened Sword, Toughened Shield) runs from `Action::Play`;
  `dark_gift_count` grows the "for each Battlecry / Tavern spell / discard"
  gifts from `battlecry_or_choose_one`, the Tavern-spell branch of
  `cast_spell` and `discard_card` (Death's Embrace rides the existing
  `buff_per_deathrattle`); `dark_gift_on_acquire` gives Double Vision's plain
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
  Lock and Load's stored minion, the two guesses and the "empty" flags.
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
  `tasty_lobster` count (the list is in `docs/real-boards.md`).
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
  `learner,snapshot,standard,rush,random,real` (default `4,2,1,1,0,0`, so
  old commands keep working); `bgpy.VecEnv(n, seed, lineup, real_boards=..)`
  takes the loaded specs, and real seats are played in Rust like the bot
  seats. `ppo.py` adds a `vs7 real` eval (one learner against 7 real seats,
  `place_vs_real`) whenever real boards are loaded.

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

`fuzz` and `games` also take `--real-boards DIR [--real-seats N]` (default 2)
to seat real-board seats playing DIR's boards (docs/real-boards.md); without
a real boards/ folder, `crates/bgcore/tests/data/real-boards/` is a small
stand-in. Needs `ref/bg-cards.json` for the card id mapping.

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

- Day to day, `./train/go.sh HOURS` continues the current run
  (`r1025-attn`) for that many hours and stops, detached, resuming from
  the last checkpoint after a crash; `./train/stop.sh` stops it, and
  `train/runs/go.log` says what happened. Each start reloads
  `boards/real`.
- One network plays every learner seat. Seats per game come from
  `--lineup` (learner, snapshot, standard bot, rush bot, random, real; the
  real seats play boards from `--real-boards`, default `boards/real` when
  the lineup has any).
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
  action mix. With real boards loaded (real seats in the lineup, or
  `--real-boards` given), `place_vs_real` holds the one-learner-vs-7-real-
  seats eval.
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
  it breaks too. A hero id and a power id share the model's one id embedding, so its
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
  bf16, recomputes them in the backward pass to fit a minibatch of 8192,
  and is compiled with torch.compile unless `--no-compile`. It does about
  20 updates a minute with `next` (10 with `full`) against `mlp`'s 70,
  `pool` about 35. `python model.py` checks that the per-slot logits
  follow their slots.
- `--init-from PATH` starts a new run (fresh optimizer) from every
  parameter of another checkpoint of the same model whose name and shape
  match, for example `attn` with `full` into `next`; it prints what was
  fresh. `--warmup N` ramps the learning rate up over N updates.
- The card and hero embeddings carry over by name, not by position: a
  card added, removed or moved in `cards.toml` (a new patch, a token)
  keeps every other card's row, and only new cards start fresh. Every
  checkpoint saves `card_names` and `hero_names` for this. A checkpoint
  from before 2026-10-25 has none; `--init-cards-rev REV` rebuilds them
  from `cards.toml` and `heroes.toml` at the commit it trained on
  (`runs/n1025-attn` trained on `7b545bd`). `--resume` still needs the
  same cards: after a card change, start a new run with `--init-from`.

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
