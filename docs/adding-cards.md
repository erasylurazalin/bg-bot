# Adding cards and tribes

A working reference for whoever adds the next tribe. Rules are in
`docs/rules/design.md` Part 2 (they win over everything here), the code map and
principles in `docs/engine.md`. Function names, no line numbers: grep them.

Files: `cards.toml` (data), `crates/bgcore/src/cards.rs` (parsing, `Effect` and
trigger enums), `game/` (shop phase, one file per system, see `docs/engine.md`),
`fight.rs` (combat), `minion.rs`.

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
   `./target/release/bgbench fightcheck` plays the real fights the card was in
   (docs/real-boards.md "The fight check"); a card whose size comes from a
   number in the log needs its line in `real_boards.rs`'s `improvements`, or
   the check plays it at the wrong size and flags its fights.
   `./target/release/bgbench shopreplay --card NAME` does the same for the shop
   phase: every real step the card was in, against the engine (docs/real-boards.md
   "The shop replay").
5. Never `cargo fmt` (the repo is not rustfmt-formatted).

An older private simulator is a good reference for how a card fights (read
only); `docs/rules/design.md` wins on disagreement.

## 1. Hook index

Every card is `Card` in cards.rs. "Parts" means the minion plus every Magnetized
card inside it (`Minion::parts()`); effects always iterate parts and a golden part
doubles per its own golden flag. `run_effect` exists twice: `Game::run_effect`
(shop, takes `pos`, `golden`, `runs`) and `Side::run_effect` (fight). An `Effect`
variant that a fight can run must be in `Effect::runs_in_fights`; the parser rejects
Rally/Deathrattle/Start of Combat/Avenge/etc. effects that are not.

The shop-phase functions named below live in `crates/bgcore/src/game/`, one file
per system (see `docs/engine.md` for the map): `run_effect` and event triggers in
effects.rs, spells in spells.rs, hero powers in heroes.rs, the Tavern and pool in
shop.rs, Discover and Choose One in discover.rs, pending targets in targets.rs,
triples in triples.rs, turn flow and combat in turn.rs, and `step`/`apply` and
legality in mod.rs.

| Kind | cards.toml field | Fires in | Example |
|---|---|---|---|
| Battlecry | `battlecry = {..}` | `Game::battlecry_or_choose_one` (via Play), Brann's `brann_mult` runs, `fire_after_battlecry` after | Southsea Busker |
| Choose One | `choose_one = [a, b]` | `battlecry_or_choose_one` -> `PendingChooseOne` -> `resolve_choose`; a repeat takes the option the player picked (`QueuedRepeat.option`) on the same target; with no option the state can answer, the minion is played and the choice is skipped (as for a Choose One spell) | Intrepid Botanist |
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
| Magnetize / Mech played | `on_play_mech`, `on_play_or_magnetize_mech`, `on_cast_spell_on_mech`, `aura = polarizing_beatboxer` | `magnetize_to` (a Magnetic played from hand first runs the played-Mech hooks in `magnetize_onto_played_mech`, so its Satellite joins it before the merge), `trigger_mech_played_or_magnetized`, `beatboxers_copy`, `Minion::magnetize` | Spark Snapper |
| Cast a Tavern spell (the exact words) | `on_cast_spell`, `on_cast_spell_on_self`, `on_cast_spell_on_tribe` | `trigger_on_cast_spell` (Tavern spells only), `trigger_on_cast_spell_on_watchers` (any spell, one per cast); one call per cast (a Bounty cast twice calls it twice). The minions' watchers run before the spell's own effect, so a Tavern buff the target's watcher gives is on the Tavern when the effect picks among it (log `Hearthstone_2026_10_24_15_25_26-g4` turn 11 step 23), and a spell that takes its target off the board runs them itself in `fire_cast_triggers_early` (the `Devourer` and `Butchering` arms, docs/rules/design.md "Eating a minion with a spell"). The trinkets for that cast (`Event::CastSpellOnMinion`, Honeycomb Ring) run after the effect: `trigger_trinket_on_cast_spell_on_minion`. A Tavern spell a minion casts for you off this cast is therefore sized without the +1/+1 this cast's improvement gives, and the play's next cast gets it (log `Hearthstone_2026_11_03_15_22_36-g4` turn 10 step 27: 12/13 then 13/14) | Timecap'n Hooktail |
| Cast any spell, Tavern spell or not | `EventKind::AllSpellsCast` for a minion's `on_event` counter (Felboar), `Event::SpellCast` for a trinket (Bewitched Ribbon, Bloodbound Earrings) | `Game::cast_spell` / `cast_spell_on`, once per spell cast (a Blood Gem, Gem Day, a Banana, a Spellcraft spell, a Triple Reward, which is a plain spell and not a Tavern spell); the extra casts of one spell count too (`apply_target`). A spell a minion's effect casts for you (`cast` on a minion, `cast_taught`) is not your cast: `cast_named_spell` with `by_minion` runs it and nothing here sees it (Era 2026-11-02) | Felboar |
| "Whenever" versus "After" you cast a spell | `on_event = { event = "all_spells_cast" / "spells_cast", after = true }` for "After you cast ..."; no `after` for "Whenever you cast ...". A minion whose own text says "After you cast a Tavern spell" (Forsaken Weaver) uses `on_cast_spell = { after = true, ... }`, which fires from `trigger_on_cast_spell(p, TriggerWhen::Resolved)`, the other half of the "whenever" one | `Game::cast_spell` fires the `TriggerWhen::Now` half of the card triggers as the spell is cast (once per cast, before each cast's own effect) and the `TriggerWhen::Resolved` half once that cast has resolved: at once for a spell that leaves nothing open, at the end of each cast in `apply_target`'s cast loop for one that asks for a target (so a two- or three-cast play fires it between its casts), and from `fire_cast_after_choice` (`maybe_start_discover`) for a Choose One option, a Discover the cast's own effect opened, or whatever else a cast left open. A "whenever" trigger therefore sees the Tavern before the spell touches it, and an "after" one the minion the spell just buffed (a Blood Gem's stats), though not a Tavern buff that same cast's own effect gave: `Player::add_tavern_buff_late` holds those until the "after" triggers have run (docs/engine.md "The order inside one cast") | Living Azerite ("whenever", reads the Tavern), Felboar ("after", consumes the buffed minion) |
| Spell cast event counter | `on_event = { event = "spells_cast" }` (add `after = true` when the card says "After") | `cast_spell` -> `fire_event(SpellsCast)`, the counter at the play and the triggers per half | Felboar |
| Gold spent / hero damage / sold / bought / Elementals played / discarded | `on_event = { event = "gold_spent" / "hero_damage" / "minions_sold" / "cards_bought" / "elementals_played" / "discarded", every, effect }` | `Game::fire_event` (count in `improvements`, remainder kept, can fire several times; an every-1 trigger leaves `improvements` alone so a self-improving effect can use it) | Gunpowder Courier |
| Sell | `on_sell = {..}` | `Game::sell_minion` (`OnSell`) | Greedy Conniver |
| Buy | `Action::Buy`/`BuySpell` in `apply`; triple via `check_triple` (a bought minion) or at the end of the action (`Game::run_pending_triple_checks`). A Tavern spell's Health price is paid before the spell joins the hand, its Gold price after | `apply`, `pay_for_spell` | none yet |
| Play a minion of a tribe | `after_play_tribe = { tribe, effects }` (never counts the minion being played: its own play marks it `Minion::just_played` while the trigger waits; runs once the play's Battlecry, and whatever it asks, has resolved). `golden_twice = true` when the golden text says ", twice": the whole list then runs twice with the printed numbers, so a golden Wrath Weaver's two hits are two hero-damage events (a Soul Rewinder takes +2 Health for each) | `fire_after_play_tribe`, `resolve_after_play` | Wrath Weaver |
| While in hand, after a tribe is played | `while_in_hand` | `trigger_in_hand_on_play` | Bream Counter |
| After you trigger a Battlecry | `after_battlecry` | `fire_after_battlecry` | Kalecgos |
| Discover (the effect) | `discover_spell`, `discover_tribe_if_another`, `Effect::Discover*`, spell `discover = ..` | `discover_queue` of `DiscoverRequest`, `maybe_start_discover`, `resolve_choose`. A Discover a Battlecry an Activate triggers opens is the exception: it gives a random card of its pool with no offer (`QueuedRepeat.random_discover`, `Player.repeat_discover`; docs/rules/design.md "Shop replay answers") | Rodeo Performer |
| After you Discover | `after_discover = {..}` | `fire_after_discover` (end of `resolve_choose`), skipped for the random card such an Activate's Discover gives (`Player.discover_silent`, Era 2026-11-03/04) | Hooktusk |
| Hero takes damage | `after_hero_damage = [..]`, `DamageHero` | `take_hero_damage`, `take_shop_damage` (can kill), `can_pay_health` | Wrath Weaver |
| Refresh | `health_refreshes`, `RefreshKind` | `refresh_shop`, `fill_shop`, Roll in `apply` | Malchezaar |
| After the Tavern is Refreshed | `refresh_buff = [a, h]` (effect and spell effect) | `Player.refresh_buff`, `apply_refresh_buff` (end of `refresh_shop`, `refresh_with_spells`, `refresh_with_tribe`, and in `start_turn` when the turn-start roll changed the Tavern); a fight adds through `SideOutcome::refresh_buff_gain` | Waveling |
| Limited uses per turn | `choose_one_cover` (Trailblazer combines), `health_refreshes` (Malchezaar's Health Refreshes), `teaches_spell` (Mycologist teachings) | `Minion.uses_left`, set from `Minion::turn_uses` at start of turn and when the minion is played; spent by `take_trailblazer_cover`, `spend_health_refresh`, `magicfin_teach`. Uses belong to the minion: sold with it. A golden from a triple starts with fresh uses, but it keeps the copies' pending "this turn" effects, adding: Drone Duplicator's held extra Magnetizations (`extra_magnetize`, summed) and Living Prison's held arms (`improvements`, summed) | Thorned Trailblazer |
| Free Refreshes | `free_refreshes = n` | `Player.free_rolls`, used first by Roll | Refreshing Anomaly |
| Sell, improving a shared count | `on_sell = { effect = { improving_counter = "baller", buff } }` | `Effect::BuffAllImproving`, `Counter::Baller` | Fire Baller |
| Played minion of a tribe gets Divine Shield | `play_shield = "elemental"` | `give_play_shield` (right after the board insert, before the Battlecry), `Minion.temp_shield`, cleared in `start_turn` | Ichoron the Protector |
| After you buy a minion | `after_buy = { buff, multiply, golden_multiply }` | `after_buy_effects` in `Action::Buy`, before `check_triple`; uses `Minion.activated` as the once-per-turn flag | Stone Age Slab |
| Arm for the next buy | `activate = { effect = { arm_next_buy = true } }` | `Effect::ArmNextBuy` (armed in `improvements`: a held arm is one share of the next bought minion's stats, a golden's own press two; arms add up, and a triple's golden keeps the copies' held arms), `after_buy_effects`, cleared in `finish_shop` | Living Prison |
| Triple wildcard | `triple_wildcard = "elemental"` | `check_triple_tracking` (`TriplePlan`) | Elemental of Surprise |
| Excess damage on a kill | `overkill = true` | `attack_swings` in fight.rs (golden hits both neighbours) | Wildfire Elemental |
| Spell target that reads the Tavern or this turn | `half_of_highest`, `per_played_this_turn` | `SpellTarget::HalfOfHighestTavern`, `BuffPerTribePlayed`, `Player.elementals_played_turn` | Arcane Absorption |
| After a friendly minion is Reborn | `on_reborn = { banshee = [a, h] }` / `{ phantom = true }` | `Side::note_reborn`, called by `Side::resolve` (fights and shop destroys) | Barrier Banshee |
| Destroy a friendly minion in the shop | `Effect::DestroyChosenDiscover`, `DestroyChosenGetCopy`, `GiveRebornDestroy`, `SpellTarget::Butchering` | `Game::destroy_in_shop` -> `fight::shop_destroy` | Maw Caster |
| Player bonus "wherever they are" | `undead_bonus = [a, h]` effect, `on_cast_spell = { undead_bonus }`, `buff_per_kind_death` | `Game::add_undead_bonus`, `add_knight_deaths`, `Player::make_card`, `Side::summon` | Nerubian Deathswarmer |
| Activate (N) | `activate = { cost, effect }` | `Action::Activate` in `apply`; legality in `is_legal` (needs_* checks on `Effect`) | Suspicious Prisonguard |
| Activate with a discard | `activate = { cost, discard = true, effect }` | opens `pending_discard`, answered by `Action::Discard`; needs a discardable card | Mangled Bandit |
| A Blood Gem lands on this | `on_gem_played_on_self = 1` | `Game::roogug_on_gem` (shop: `gem_on_board` / `roogug_reacts`) and `Side::roogug_on_gem` (fight: `gem_at`), for every Gem but another Roogug's | Geomagus Roogug |
| A Choose One card each turn gets both effects | `choose_one_cover = 1` | one combine in `Minion.uses_left` per Trailblazer on the board, spent by `take_trailblazer_cover`, `resolve_choose_options` | Thorned Trailblazer |
| After a Choose One card is played | `after_choose_one_play = 2` | `Player.hogrider_pending`, fired by `Game::resolve_hogrider` once no choice is open (from `step` and `finish_shop`) | Turbo Hogrider |
| Rally: summon a stat copy that attacks first | `rally = { summon_stat_copy = "Golem" }` | `attack_triggers` + `attack_swings_at` (fight); shop copy summons only | Jailbird Juggernaut |
| Golden played from hand | `buff_per_golden_played`, `after_discover` scaling | `Action::Play` -> `add_golden_played`, `Player.golden_played` | Maritime Extortionist |
| Tavern buff (this game / this turn) | `Effect::TavernBuff`, `tavern_buff` | `Player::add_tavern_buff`, `apply_tavern_buffs`, `end_turn_tavern_buffs` | Champion of Sargeras |
| Consume Tavern minion | `consume = "random"` (the game picks), `"highest_health"` (a Health tie picks at random, no random number without one), `"chosen"` (the player picks: `PendingShopTarget`, unused by any card today) | `start_consume`, `consume`, `PendingShopTarget` | Mind Muck, Corrupted Cupcakes (random), Flaming Enforcer (highest health) |
| Aura (counts, not stats) | `aura = "brann"/"titus"/"drakkari"/"balinda"/"proud_privateer"/..` | `brann_mult`, `drakkari_mult`, `balinda_mult`, `bounty_mult`, `Side::deathrattle_fires` | Proud Privateer |
| Per-game player counter | `buff_per_deathrattle`, `improve_counter`, `spell_bonus`, volumizer/beetle bonus | fields on `Player` (`counters()` hands them to fights) | Tasty Lobster |
| Per-minion counter | `improvements` (u16) | tier of Patient Scout, event counts, damage milestones, Lockbox turns | Patient Scout |
| Stat threshold | `attack_threshold` | `check_attack_thresholds`, `Side::gain_at` | Scarlet Survivor |
| Keep combat gains | `permanent_keep = "self"/"adjacent_dragons"/"all_dragons"`, `SideOutcome::permanent_*` | `Side::gain_at`, `apply_side_outcome` | Tarecgosa, Timewarped Poet |
| Cards gained in a fight | `get = "X"`, `random_*` effects | `SideOutcome.cards`, `apply_side_outcome` -> `give_generated` | Headhunter Gryphon |
| Spell effect (no target) | `[[spell]] effect = { gold = 2 }` etc | `cast_spell_inner` -> `execute_spell_effect` (`SpellEffect`) | Tavern Coin |
| Spell effect (needs a minion) | `effect = { target = "chosen", .. }` | `cast_spell_inner` -> `PendingTarget` -> `apply_target` (`SpellTarget`) | Tavern Dish Banana |
| A card is discarded, any source | `on_event = { event = "discarded" }`; on the discarded spell itself `on_discard = "cast_twice"` / `"max_gold_2"` | `Game::discard_card` (Activates, linked cards, Mysterious K'Thir): pool copy back, the `Discarded` event, then `OnDiscard` (two real Tavern spell casts, or maximum Gold) | Cutthroat K'Thir |
| A card is added to your hand | `on_card_added = { buff }` | `Game::add_to_hand`, the one path into a hand (buys, Gems, Triple Rewards, Discover picks, generated, Lockbox, `Game::give`) | The Shadow of Doubt |
| You get a card of a tribe | `on_event = { event = "got_pirate", every = 4, effect }` (the event is `got_<tribe>`); a trinket listens with `get_tribe` + `on_get` | `Game::add_to_hand` fires `EventKind::CardGot(tribe)` and `trinkets::Event::GetTribe(tribe)` for every tribe the added card has | Doubloon Grifter, Trusty Crowbar |
| After this attacks and kills | `on_kill = "max_stats_to_hand"` (`KillReward`) | `attack_swings` in fight.rs, at the hit that takes the defender below 1 Health; "maximum stats" are its Attack and its Health before the hit plus what it had already taken, so a Venomous or Poisonous kill reads right | Tide Oracle Morgl |
| Stats for every Elemental played / time Magnetized this game | `buff_per_elemental_played`, `buff_per_magnetize` (`[a, h]`, doubled when golden) | `Player::make_card` reads `Player.elementals_played` (the play path) and `Player.magnetizations` (`Game::note_magnetized`, every `Minion::magnetize` the shop does) | Flourishing Frostling, Dr. Boom's Monster |
| Selling a minion of a tribe gives its stats to the ones that stay | `on_sell = { gain_sold_stats = "<tribe>" }` (empty string for any) | `Game::sell_minion` after the sell: the minion with the text is still on the board, so it is scanned there, not on the sold card | Arcane Behemoth |
| Gain the Deathrattle of a friendly Deathrattle minion that dies | `gains_friendly_deathrattles = true` | `Side::gain_deathrattles` (from `Side::resolve`) fills `Side.gained_dr` by uid; `Side::fire_deathrattles` fires them with the minion's own, once each | Fish of N'Zoth |
| Choose a minion and become a copy of it | `battlecry = { become_copy = true }` | `Effect::BecomeCopyChosen` -> `TargetKind::BecomeCopy` (board or Tavern) -> `Game::become_copy_of`: an exact copy, no pool copy, no Triple Reward | Faceless Manipulator |
| Get a plain copy of a minion | `Effect::PlainCopyRandomFriendly` (`plain_copy_random_friendly`), a spell `effect = { target = "chosen", plain_copy = 3 }` (`SpellTarget::PlainCopy`), a gift's `countdown = { effect = "plain_copy" }` (Replication) | a plain copy is `give_generated` (the card at its printed stats, no pool copy): the same shape Disguised Graverobber's copies use. The countdown's payout makes the card with `Player::make_card` instead and adds it to the hand unmerged: its `check_triple` waits in `Player.triples_next_turn` for the start of the next turn, since nothing triples during the fights (Era 2026-11-03) | Reflective Pendant, Duplicating Lens |
| Give your Deity stats | `buff_deity` (+`per_spell`), `buff_self_and_deity`, `buff_deity_and_improve`, spell `buff_deity`, `on_cast_spell = { buff_self_and_deity / buff_all_and_deity }` | `Game::run_effect` / `execute_spell_effect` / `trigger_on_cast_spell`; in a fight `Side::buff_deity`, the gain leaves through `SideOutcome::deity_gain` | Joyous |
| Linked cards | `linked_spells = 2`, `linked_minions = "aberration"` (golden links four) | `Game::give_linked`, `Minion.linked`; `Action::Play` discards the rest of the group (a golden four after two plays) | Wandering Willbreaker |
| The god tokens | token `deity = "cthun"` / `"yshaarj"` | `Game::unstarted` picks one god per lobby onto `Player.deity`; fights: `Side::note_deity_death`, `try_awaken`, C'Thun's split, `Effect::SummonDeadAberrations` | C'Thun |
| A neutral or spell tied to one tribe | `lobby = "<tribe>"` (the export's `lobby_subsets`) | `Card.lobby_ok` filters it into the pool and every draw; it only exists in lobbies with that tribe | Butchering |
| Set your Armor to N | `effect = { set_armor = N }` | `cast_spell_inner` -> `SpellEffect::SetArmor` | Armor Stash |
| Discover a new Hero Power | `effect = { discover_hero_power = true, exclude_heroes = [..] }` | `cast_spell_inner` -> `Game::discover_hero_power` (offers in `Player.power_offer`, answered with Choose) | Unmasked Identity |
| Hero power (button) | `heroes.toml` `power = { cost, uses_per_turn, once_per_game, uses_per_game, unlock_tier, unlock_turn, cheaper_each_turn, effect }` | `Action::HeroPower` -> `Game::press_power` (`power_usable` is the legality, `Game::power_cost` the price); a targeted power opens the pending-target flow | Lich Baz'hial |
| Hero power (passive) | `passive = true` + the effect | the trigger it hooks: `FreeRefreshPerTurn` in `start_turn`, `GoldAfterUpgrade` in `Action::Level`, `GoldAfterBuyTribe`/`GetTribeCutCost`/`CardAfterBuys`/`TripleRewardAfterTiers`/`TavernBuffAfterBuys`/`CopyBuyAfterMinions` in `power_bought_card`, `GoldNextTurnAfterSell` and `RandomTribeAfterSells` in `sell_minion`, `UpgradeDiscountAfterElementals` where an Elemental is played, `TripleRewardAfterAttacks` in `apply_side_outcome`, `Champion`/`KittyDiscover` at a death, `HandCopyEvery` in `power_end_of_turn`, `CastRandomSpell` in `power_start_of_turn`, `SocLeftmost` through `Counters.soc_leftmost` (before any minion's Start of Combat) | Nozdormu |
| Hero power that sets the Tavern's prices | `effect = { prices = { minion, refresh, upgrade_extra } }` (`PowerEffect::Prices`) | `Game::power_prices`, read by `minion_price`, `Game::refresh_cost` (Roll), `Game::upgrade_price` (Level), the observation and the sandbox | Millhouse Manastorm |
| Hero power that counts what the player does | `Player.counts` (`HeroCounts`) grown by `power_bought_card`, `power_sold_minion`, `power_elemental_played`, `power_attacks`, `power_gold_spent`; a per-cycle count lives in `PowerState.progress` | Sharpen Blades reads the shared count at its press, the rest keep their own cycle | Edwin VanCleef |
| Hero power that presses for more or less | `PowerState.cost_delta` + `Game::power_cost`; `cheaper_each_turn = true` for the -1 a turn ones | Lead Explorer's `discover_my_tier_costs_more`, Pirate Parrrrty!'s `get_tribe_cut_cost`, The Perfect Crime and The Galaxy's Lens | Elise Starseeker |
| Hero power that Discovers from something that is not the pool | a `DiscoverRequest` dealt by `maybe_start_discover` itself: `MagneticMinion`, `PotTypes([u8; 3])`, `DeadWarband`, `DeadLastFight`, `Champion` | `deal_pot_offer`, `deal_dead_warband`, `deal_dead_last_fight`, `deal_champion`; the last three fill `Player.discover_minions` (ready-made minions, no pool copy) | Tinker, Stir the Pot, Kel'Thuzad's Kitty |
| Hero power that casts a random spell | `effect = { cast_random_spell = true }` (`PowerEffect::CastRandomSpell`) | `Game::cast_random_spell`: `CardDb::random_castable_spell` (no destructive spell), `Game::spell_castable`, then a random target and a random Choose One option | Puzzle Box |
| Hero power that asks for a card to discard | `effect = { discard_get_random_minion = "<tribe>" }` (`PowerEffect::DiscardGetRandomMinion`) | `press_power` opens `Player.pending_discard` with `source: None` (a power has no board slot), answered by `Action::Discard`; the reward runs in `resolve_discard`, the same path as an Activate with `discard = true` | Incubate |
| Hero power that triggers a Battlecry | `effect = { trigger_battlecry = true }` (`PowerEffect::TriggerBattlecry`) | `press_power` offers `TargetKind::TriggerBattlecry` (Kelp Keeper's path); `power_usable` needs a target, `has_battlecry_target` | Snicker-snack |
| Hero power that gives linked cards | `effect = { get_linked_minions = 2 }` (`PowerEffect::GetLinkedMinions`) | `press_power` draws with `CardDb::random_any_minion` and calls `give_linked`, so playing one discards the rest | Dark Ritual |
| Hero power that is a card effect | `spell_discount = 1`, `get_random_spell = true`, `refresh_with_spells = true` (`PowerEffect::SpellDiscount`, `GetRandomSpell`, `RefreshWithSpells`) | `press_power` writes the same player field, draws through `CardDb::random_spell` + `give_generated`, or calls `refresh_with_spells` (Saloon's Finest) | Arcane Knowledge, Blessing of the Nine Frogs, Mana Per Minute |
| Hero power that is a trinket offer | `effect = { trinket_offer = "lesser" }` (`PowerEffect::TrinketOffer(slot)`) | `trinket_start_of_turn` deals the offer at the power's `unlock_turn` into the power slot; the pick turns the power into the trinket and `power_usable` goes false | Fantastic Treasure, Growing Collection |
| Hero power that changes the Tavern's shape | `stay_frosty = 2`, `extra_dragon = true`, `twice_as_nice = true`, `enhancification = 2`, `darkmoon_tickets = [33, 3]` | `Game::sync_power_flags` writes `Player.shop_fewer`/`freeze_end`/`extra_dragon`/`twice_nice`/`enhance_times`/`ticket_chance`; `Player::shop_size` and `Game::fill_shop` read them, and `Game::after_refresh_powers` runs at the end of every Refresh (Twice as Nice's copy, the Tickets, the keywords) | Sindragosa, Ysera, Varden Dawngrasp, Enhance-o Mechano, Silas Darkmoon |
| Hero power that lasts until the next turn | `temp_reborn = true`, `spirit_swap = true`, plus `Minion.temp_reborn` and `Minion.temp_attack` | the press offers `TargetKind::TempReborn` / `SpiritSwapFirst`; both come off in `start_turn` (board and hand), a Reborn the card prints stays | The Lich King, Vol'jin |
| Hero power that needs two picks | `spirit_swap = true`, `devour = true` | `Player.two_step` holds the first answer, the second prompt is a fresh `offer_target`, and `Devour` carries the sold minion's stats in `Player.devour_stats` after `sell_minion` | Vol'jin, Mutanus the Devourer |
| Hero power that puts a card where another one stood | `replace_shop_higher = true`, `replace_same_tier = true` | `TargetKind::ReplaceHigher` (old card back to the pool, `Player.discover_into_shop` sends the pick into its slot) and `TargetKind::ReplaceSameTier` (`Game::replace_tavern_same_tier` / `replace_board_same_tier`, the old card's stat gain moves onto the new one) | Galakrond, Malygos |
| Hero power that gives a card temporary state | `imprison = 2`, `steal_shop_as_stats = [2, 2]`, `hat_trick = [1, 1]` | `Minion.locked_turns` (the shared lock of docs/rules/design.md "Cards locked in hand"), a stat set on the way into hand, and `Minion.hats` (already in the stats; a sale moves them in `sell_minion`, a triple adds them in `check_triple`) | Maiev Shadowsong, Xyrella, Dancin' Deryl |
| Hero power that arms an end of turn | `saturday_cthun = [1, 1]`, `swaps_each_turn = "<power card_id>"` | `PowerState.armed` is spent in `power_end_of_turn` (repeats = the power's `progress`, one more every turn); `HeroPower.swaps_to` replaces the power at the end of each turn | C'Thun, Inge the Iron Hymn |
| Hero power that Discovers from something that is not the pool, part 2 | `ispy_next_opponent = true`, `burgle_last_opponent = true` | `DiscoverRequest::IspyNextOpponent` dealt by `deal_ispy_offer` from `Player.next_board` (copied by `Game::pair` at the start of the round); Bob's Burgles fills the Tavern from `Player.seen` instead, one plain copy per warband minion up to the Tavern's seven cards, and a warband of seven leaves no spell slot (`burgle_last_opponent`, the shape the Warband Whistle has in `fill_shop`). The shop replay seats both from the file's `last_opponent_board` (docs/real-boards.md "The shop replay") | Scabbs Cutterbutter, Tess Greymane |
| Hero power that changes what makes a Golden minion | `double_time = true, card = "Tavern Coin"` | `Player.double_time`: `check_triple` needs two copies, and `Game::give_play_reward` (the two play sites) hands the named coin instead of a Triple Reward | Mister Clocksworth |
| Trinket (the frame) | `trinkets.toml` `[[trinket]]` with `name`, `card_id`, `slot`, `cost`, `tribes`, effects | `trinket_start_of_turn` deals the offers, `Action::ChooseTrinket` picks (`pick_trinket`), the hooks call `Game::trinket_event` | see section 2c |
| Trinket that runs a card effect | the effect keys of cards.toml inside a hook list (`on_pick`, `start_of_turn`, `end_of_turn`, `on_buy`, `on_play`, `on_cast_spell`, ...) | `Game::run_trinket_act` runs the same `Effect`/`SpellEffect` arm with `pos` 0 and no golden multiplier | Sellemental Portrait (`get`) |
| Trinket every N events / turns | `rule = { event = "cast_spell", every = 4, acts = [..] }` (or `"turns"`) | the slot's `count` in `TrinketState`; `trinket_event` for the events, `trinket_end_of_turn` for turns (the pick turn counts as 1) | Bloodbound Earrings |
| Trinket's event mask (the hot path) | `Trinket::event_mask` over the lists and the rule; `Game::set_trinket` writes `Player.trinket_events` | every hook tests one bit of that `u16` first, then copies its `Act`s out of the shared table by index: no `Arc<Data>` clone, no `Vec` clone | Water Wheel (a capped hook that needs the start of the turn) |
| Trinket in a fight | `fight = [{ when = "soc", tribe = "undead", keywords = ["taunt"] }]` (or `when = "avenge"`, `"summon"`, `"attack"`, `"death"`, `"reborn"`, `"awaken"`) | `Player::counters` -> `Counters.trinkets` (a `Copy` `FightTrinkets`), run by `Side::trinket_hook` at the fight's own hook points | Holy Mallet, Quilligraphy Set, Mama Bear Sticker |
| Trinket whose reward is a shop action after N fight deaths / summons | `rule = { event = "death", every = 8, acts = [..] }` (`death_<tribe>`, `summon`, `summon_<tribe>`) | the fight reports `SideOutcome::friendly_deaths` / `summon_tribes`; `apply_side_outcome` fires `trinket_event`, and a shop destroy counts its dead the same way | Bleeding Heart, Wildfeather Duster |
| Trinket that counts something inside a fight | `{ when = "avenge", every = 5, ... }`, `cap = N` (per fight), `once = true`, `grow` (adds the trinket's `value`) | the bundle entry's own `count`/`uses`, reset every fight | Quilligraphy Set, Reinforced Shield, All-Purpose Kibble |
| Trinket that hands out a card | `get = "X"`, `get_golden = "X"`, `one_of = ["X", "Y"]`, `random_battlecry = n`, `plain_copy_random_friendly = n`, `copy_last_discarded = true`, `discover_tier = N` | `Game::run_trinket_act`; a minion no card has yet is a `[[token]]` in cards.toml ("--- Trinket-only minions") | Morgl Portrait, Baller Portrait, Rockin' Music Box |
| Trinket that remembers a Discover | `discover_remember = 4` (+ `discover_remember_typed = true`) and `get_remembered = 1` in a later list | `Act::DiscoverRemember` sets `tp.remember_in`; `resolve_choose` writes the pick into `TrinketState.card` | Portable Factory |
| Trinket every N turns at the start of the turn | `start_every = 2` (the start-of-turn list only) | `trinket_start_of_turn` counts the slot's `count` and skips the list between (a `rule` with `event = "turns"` still fires at the end of the turn it reaches N) | Azeroth Model Globe, Lens Case |
| Trinket that changes how another rule works | `copy_buys = n` (Transcribing Typewriter), `warband_whistle = true`, `triple_need_two_pirate = true`, `magnetize_ends = "X"` + `tribe`, `lockbox_sooner = n`, `discover_copy_bonus = n`, `gems_each_type = n` | `TrinketPowers` on `Player` (`copy_buys`, `whistle`, `pirate_triple`, `lockbox_sooner`, `discover_copies`), read by `Action::Buy`, `fill_shop`, `check_triple_tracking`, `give_lockbox`, `resolve_choose` | Transcribing Typewriter, Designer Eyepatch, Accord-o-Tron Portrait |
| Spellcraft spell | a `[[spell]]` with `spellcraft = true`, `token = true`, `tavern_spell = false`, `cost = 0`, and the effect; the trinket hands it out with `on_pick = [{ get = "X" }]` and `start_of_turn = [{ get = "X" }]` | `Card.spellcraft`; `finish_shop` drops every uncast Spellcraft spell from the hand (not a discard); it is a plain spell, not a Tavern spell (type SPELL in the data) | Vibrant Bubble, Jailer Sticker |
| Trinket that changes the Tavern's shape | `gauntlet = true` (six minions and a spell, refilled at once), `extra_high_tier` / `extra_magnetic` (one extra slot), `tier6_refreshes = 2` (Guiding Candle), `min_tier = 3` (Bob-blehead), `magnetic_cost = 2` | `TrinketPowers` on `Player`; `Player::shop_size` is the one size (capped at `SHOP - 1` minions), `shop::fill_shop` draws the extra slots and `tavern_tiers` the bounds, `Game::refill_tavern` tops the Gauntlet's Tavern up after every action, `minion_price` reads the set prices | Glowing Gauntlet, Innkeeper's Stein, Electrode Attractor, Guiding Candle, Bob-blehead |
| Trinket that changes the hero power | `power_triggers = 2` (the effect runs twice per press, once per payment), `power_extra_uses = 1`, `power_gold = 1` | `TrinketPowers::power_*`, read by `press_power` (the effect arms and the passive hooks in `start_turn`, `Action::Level`, `Action::Buy`, `sell_minion`) and the uses refill | Ancient Wishbone, Sous Chef Sticker |
| Trinket whose effect stays with the slot | `offer_replace = 2` (Mystery Cube): it marks the slot (`TrinketState.cube`) and deals two free Lesser options, at the pick and then at every start of turn, after the held trinket's own start-of-turn list; `cube = false` keeps a trinket out of those offers | `Act::OfferReplace` -> `Game::deal_cube_offer` (`Offer::Cube`: never the held one, never a `cube = false` one); `refresh_trinket_mask` gives the slot `STARTS_TURN`; `pick_trinket` deals a waiting cube offer after another offer is answered (`tp.cube_pending`) | Mystery Cube |
| Trinket that watches the Deity / makes it Golden | `golden_deity = true` (Mask of the Ancient Ones), a spell `effect = { target = "chosen", watch_stats = true }` (Makeshift Master) | `Act::GoldenDeity` sets `Deity.golden` and doubles its 1/1 base; `SpellTarget::WatchStats` marks the card (`Minion.watched`, a Tavern minion included: "a minion" reaches the shop) and `tp.deity_watch_stats` keeps the stats it had, and `Game::watch_deity_gain` (after every action) gives the Deity whatever the marked card gained, wherever it is: in the Tavern, in hand after a buy, on the board after it is played. The mark comes off at the end of the turn, and a card that leaves the game takes it | Mask of the Ancient Ones, Makeshift Master |
| Trinket on any card played from the hand | `on_card_play` (`Event::CardPlayed` in `Action::Play`, spells and minions both) | `Game::trinket_event` | Dragonwing Glider |
| Trinket on a Battlecry trigger | `on_battlecry`, once per trigger (Brann's double fires it twice) | `battlecry_or_choose_one`, after `fire_after_battlecry` | Dragon Skull |
| Trinket that adds Battlecry triggers | `war_drum = 2` (`tp.war_drum`, spent from the trinket's `uses_per_turn` by `Game::take_war_drum`), `dragon_battlecry = 1` (`tp.dragon_battlecry`, Dragons only) | `battlecry_or_choose_one` | War Drum, Dragon's Eye |
| Trinket that triggers a Battlecry outside a play | `trigger_battlecries_ends` (the two ends), `trigger_battlecries_of = "X"` (every copy of a card); `drakkari_end = true` runs the end-of-turn list once per Drakkari | `Game::run_battlecry_at(p, pos, random)`; `random` answers the target out of the board and the Tavern | Young Murk-Eye Sticker, Hackerfin Portrait |
| Trinket that triggers Rallies or Deathrattles at the end of the turn | `trigger_all_rallies`, `trigger_deathrattles_leftmost = 2` | the shop runs the Rally's own effect; `Game::trigger_deathrattles_in_shop` -> `fight::shop_trigger_deathrattles` (a one-sided `Side`, nothing dies) | Insurrectionist's Blade, Wolfhead Flail |
| Trinket that casts a named spell on a minion | `cast_on_leftmost = true` / `cast_on_random = true` with `card` and `tribe` | `Game::cast_spell_on`, a real cast (spell power, cast triggers, Balinda) | Emergency Gearblade, Spell-powered Wrench |
| Trinket that reads the card just bought | `satellite_of_bought = "Satellite"` (its stats), `teach_bought_spell = 1` (a Tavern spell) | `tp.bought_stats` and `tp.bought_spell`, written in `Action::Buy`/`BuySpell` before the Buy event | Reusable Batteries, Magicfin Sticker |
| Trinket on a destroy outside combat | `on_destroy`, `rule = { event = "destroy" }`, `destroy_card = "X"` to narrow it | `Game::destroy_in_shop`, one event per dead minion | Maw Caster Portrait, Plaguerunner Portrait |
| Trinket on a spell cast on a Tavern minion | `on_cast_spell_on_shop`; `tp.shop_hit` is the slot it hit | `apply_target_shop`'s spell arm | Felsteel Cleaver |
| Trinket on a Demon's consume | `consume_claw = [5, 5]` (`tp.consume_claw`): the Demon also takes the Bonus Keywords and the stats | `Game::consume` | Consuming Claw |
| Trinket that rewrites a card the player has | `card_of_mod = "X"` with `also_attack = n` / `also_neighbors = true` / `also_health = true` / `also_board = "tribe"` | `TrinketPowers.card_mods`, read by the card's own effect: `after_hero_damage_mods`, `trigger_on_cast_spell`, `consume`, the `Effect::TavernBuff` arm | the six portraits |
| Trinket that always combines a Choose One | `choose_one_all = true` | `tp.choose_one_all` in `cast_spell_inner` and `battlecry_or_choose_one` | Trailblazer Sticker |
| Trinket that becomes another one | `souvenir = true` (Souvenir Stand: copies the next Greater pick), `replace_with_random = ["greater", "dragon"]` (Dragonhead Staff, Ruby Tusk) | `pick_trinket`, `Game::replace_trinket_at` | Souvenir Stand |
| Trinket effect inside a fight that outlives the minions | `fight = [{ when = "soc", fermenter = 3 }]`, `gem_golem = "X"` (the dead minion's Blood Gem stats) | `Side.fermenter` and `Side.last_dead`/`dead_at`, run by `Side::run_fight_act`; the resummon is at the end of `Side::resolve` | Soul Fermenter, Blood Golem Sticker |
| Trinket that fires on a spell cast in a fight | `fight = [{ when = "cast", pick = "all", buff = [2, 4] }]` | `Side::trinket_hook` from `Effect::CastSpell` | Bewitched Ribbon |
| A Darkmoon Prize (block 5) | `prize = 1` to `4` on a `[[spell]]` or a `[[token]]`, `tier = 0`, `token = true`, `tavern_spell = false` | `CardDb.prizes` holds the four lists; `DiscoverRequest::Prize(tier)` deals three different ones (`CardDb::random_prize`), the pick lands with no pool copy. `SpellEffect::DiscoverPrize` / `DiscoverPrizesBelow` / `RandomPrize` and the trinket `Act`s of the same name open them | Banana Bunch |
| A Prize's standing rule | `Player.prize` (`PrizePowers`): `extra_shop`, `free_rolls_per_turn`, `spell_discount_turn`, `minion_price`, `battlecry_extra_turn`, `return_each_turn` | read by `Player::shop_size`, `fill_shop`, `start_turn`, `spell_price`/`spell_cost`, `Game::minion_price`, `battlecry_or_choose_one` and `run_end_of_turn`. `extra_shop_minion = [2, 2]` sets the flag and adds to `Player.refresh_buff`, the after-Refresh buff Waveling and Easterly Winds share | New Recruit |
| A minion that gains stats at the end of every turn | a spell `effect = { target = "chosen", grant_end_of_turn = [4, 4] }` writes `Minion.end_turn_gain` | `run_minion_end_of_turn` (a triple sums it, `magnetize` adds it) | Gruul Rules |
| Trinket that brings every Triple Reward back as another card | `on_pick = [{ triple_prize = "Triple Prize" }]` sets `tp.triple_prize` | `give_triple_reward`, the one place a Triple Reward is made (a Golden minion, Natural Balance, Heroic Inspiration) | Corrupted Tome |
| Trinket every N turns that Discovers a Prize | `start_every = 3` with `start_of_turn = [{ discover_prize = 3 }]` | `trinket_start_of_turn` | Tickatus Sticker |
| Trinket that saves the hero once | `on_pick = [{ ice_block = true }]` adds the Ice Block kind to `Player.secrets` (the Secret zone, docs/rules/heroes.md "Secrets") | the fight damage loop and `take_hero_damage` prevent the fatal hit and set `Player.immune` for the turn (cleared in `start_turn`); `SCALAR_ICE_BLOCK`/`SCALAR_IMMUNE` show both | Safety Patch |
| Trinket with a hidden random trigger | `on_pick = [{ surprise = 5 }]` sets `tp.surprise`, a chance out of 100 per buy, sell and Refresh, never twice in a row | `Game::surprise_roll`, called at the top of `trinket_event`; the cards it makes are generated | Surprise Portrait |
| Trinket with a random wheel | `wheel = [..]` (a list of acts) and `yogg_wheel = true` in a hook | `Act::YoggWheel` picks one entry of `Trinket.wheel` and runs it | Yogg-Tastic Pastry |
| Magnetic minions that merge onto more than a Mech | `magnetize_to_tribes = ["mech", "demon"]` on the card (`Card.magnetize_tribes`, Mech by default) | `can_magnetize`, `is_legal(Play)` and the bots' `play_pos` | Rusted Reggie |
| A portrait that gives a card more tribes | `card_of_mod = "X", also_tribes = ["murloc", "dragon"]` (`CardMod::AlsoTribes`) writes `Minion.extra_tribes` on the copies the player has (the pick) and on every later one (`Player::make_card`) | `CardDb::tribes_of` ORs it in, so every tribe check sees it; `most_common_card_type` reads the cards alone for the Dark Discovery's "your type" slot | Bronzebeard Portrait |
| Hero power that Discovers a Prize on a turn count | `heroes.toml` `effect = { prize_wall = 4 }` (`PowerEffect::PrizeWall`) | `power_start_of_turn`: turn 4, 8, 12, 16 and every four after, the Prize tier being turn / 4 (at most 4); the power's `progress` holds the turns to the next one and `obs::power_progress` reports it | Tickatus |
| A combat Secret (the Secret zone) | kind constants in `crates/bgcore/src/secrets.rs` (`VENOMSTRIKE_TRAP`, `AUTODEFENSE_MATRIX`, `PACK_TACTICS`, `REDEMPTION`, plus `ICE_BLOCK`) | `Player.secrets` -> `Counters.secrets` -> `Side.secrets`: `Side::attack_secrets` from `attack_swings_at` ("when a friendly minion is attacked") and `Side::redemption` from `Side::resolve` ("when a friendly minion dies"); `SideOutcome.secrets_fired` takes a fired one out of the zone | Venomstrike Trap |
| A hero power that offers Secrets | `heroes.toml` `effect = { discover_secret = true }` (`PowerEffect::DiscoverSecret`) | `press_power` fills `Player.secret_offer` (kinds, `ArrayVec<u8, CHOICES>`), answered by `Action::Choose` through `resolve_choose` -> `Game::pick_secret`; `power_usable` refuses it while all four are held | The Great Akazamzarak |

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
| Buff one minion of each type (one buff per minion) | The Last One Standing (`buff_each_type`), Misplaced Tea Set | `cards::pick_one_per_type`; an all-type minion is a type of its own ("all") and takes that one pick, never another tribe's (`15_25_26-g1` turn 10 step 24, Era 2026-11-02) |
| Buff scaled by a counter that improves | Tasty Lobster (`improve_counter`) | player counter `Counter`, kept through fights via `improve_gain` |
| Per-minion "N left" counter | Gunpowder Courier (`on_event`), Treasure Parrot (`damage_milestone`) | triple keeps the highest count |
| Stat per thing counted game-long | Falling Sky Golem (`buff_per_deathrattle`), Maritime Extortionist | wherever-it-is: update shop, hand, board (`add_golden_played`, `add_deathrattles_triggered`) |
| Scale by gold spent this turn | Lovesick Balladist (`per_gold_spent`, +2 per Gold: the log's own numbers, `10_28_02_28_32-g4` turn 11 step 19) | `Player.gold_spent_turn`, which the shop replay seats from the log's `gold_spent` counter (`seat_spell_bonus_turn`) |
| Get a generated card | Weapons Forge (`get`), Chromadrake effects | no pool copy, hand slot needed, `give_generated` |
| Get a random pool minion (generated) | `random_minion`, Highkeeper Ra (`random_tier_minion`) | `CardDb::random_pool_minion`, `random_tier_minion` |
| Get a pool minion that takes a copy | Discover (`DiscoverRequest`), Silent Deliverer (`draw_golden`) | pool copies come out of `Game.pool`, go back on sell |
| Discover | Rodeo Performer (spell), Patient Scout (minion by tier), `discover_tier7_minion` | queue + `maybe_start_discover`; unpicked minions go back |
| Tier 7 draw | Greedy Conniver, Hallowed Ritual (`discover = "tier_7_minion"`) | `give_tier7_minion`, `discover_tier7_minion` |
| Tier 7 spell | Sacred Gift, Sharing is Caring, Menagerie Tableware | in `spells` like any pool spell, but every spell draw (`random_spell`, `random_spell_up_to`, `random_tier_spell`, `random_spell_of_cost`) caps at `MAX_TIER`, so none is ever offered |
| Start of Combat spell | Brood of Nozdormu, Upper Hand, Sharing is Caring | a `Player` count passed in `Counters`, read in `run_start_of_combat`, cleared after the fight |
| Start of Combat from a hero power | Wingmen, Wax Warband, Fragrant Phylactery (`heroes.toml` power effects) | a `Player::counters` field read in `run_start_of_combat`: Wingmen first of all, the rest in the hero-power step; a passive tick runs `tp.power_triggers` times (`tick_power_effect`) |
| Summon when there is space in combat | Drek'Thar, Ozumat (Tentacular), Lock and Load | `Side::spawn_when_space`, called from `Side::resolve` after every death and at Start of Combat |
| A Deathrattle granted for the fight | Fragrant Phylactery (`GrantedDr::Phylactery`), Earth Invocation (`SocGrantDeathrattle`) | `Side.granted`; `fire_deathrattles` fires every grant a minion has, so two Start of Combats give two |
| A guess with two answers | Lord Barov (Friendly Wager), Murloc Holmes (Detective for Hire) | `Player.wager_offer`/`holmes_offer`, `is_legal` + `resolve_choose`, the observation, `answer_pending` in bots.rs |
| Golden: doubles numbers | any card, `mul` in `run_effect` | "twice" (fixed count, max over copies): Brann/Balinda/Proud Privateer. "An extra time" (stacks): Titus |
| Golden count differs from x2 | golden text says "twice" vs doubled stats | read the golden card; `golden_doubles_count` for summons, `golden_stats` for stats |
| Make a minion golden on the board | Elite Navigator (`make_golden_max_tier`) | no reward, not counted as played; `golden_up` (the golden card's printed stats and its own "for each ... this game" counters doubled, its buffs kept) |
| Golden played gives no reward | Aureate Laureate (`always_golden`, `no_triple`), `Minion.no_reward` | |
| Summon in combat | Cord Puller / Den Mother style (`summon`) | `golden_doubles_count` for Beetle style |
| Summon a copy / from hand | Flighty Scout, Expert Aviator | fights need the hand passed (`reads_hand`) |
| Deal damage in combat | Obsidian Ravager (`damage_target_and_neighbor`), Blade Collector (cleave) | `rally_damage`, `hit`; Divine Shield, Venomous and Poisonous honoured |
| Keep stats after the fight | Tarecgosa (`permanent_keep`), Eudora (`keep_golden`), Persistent Poet (adjacent), Timewarped Poet (all Dragons) | `SideOutcome.permanent_stats` by start slot |
| Keep a counter after the fight | Lurking Leviathan, Treasure Parrot | `permanent_improvements` |
| Cast a named spell | Gatekeeper Amalgam (`cast`), a Magicfin token (`cast_taught`) | `Effect::CastSpell` / `Effect::CastTaughtSpell` -> `cast_named_spell` with `by_minion` true: the spell runs and buffs as printed, but it is not your cast, so no cast trigger and no counter sees it (Era 2026-11-02); `PendingTarget.by_minion` carries that through the target prompt. It also gets no Choose One choice (Era 2026-11-04): the option is random, one per cast. A Discover it opens is a normal Discover: the player picks it and it counts for "after you Discover" cards (log Hearthstone_2026_10_28_02_28_32 at 02:41). A trinket's `cast` act is your cast (`cast_named_spell` with `by_minion` false). Cast `times` times by one Battlecry, a targeting spell asks for one target and casts again on it and a repeated Choose One reuses the option picked (`cast_spell_as`'s `repeats`, `QueuedRepeat.option`) |
| Trigger another card's Battlecry/Rally | Kelp Keeper, Sky-hatch Runaway | `TriggerBattlecry`, `TriggerRally` |
| Spells that share a pool slot | Volumizers (`volumizer_buff`), Bounties (`bounty = true`) | `CardDb.bounties`, only the first is in `spells` |
| Unplayable held card with a countdown | Lockbox (`lockbox = 5`) | counter in `improvements`, `tick_lockboxes` |
| Pay Health instead of Gold | Hasty Excavation (`pay_health`), Malchezaar | `can_pay_health`; Health payments never count as gold |
| Dual / all tribes | Treasure Parrot (`tribes = [..]`), `tribe = "all"` | `Tribes` bit set; every filter asks `has` |
| Effect only while in hand | Bream Counter, Choral Mrrrglr | |
| Stats that the Tavern can't change | Fishbait (`fixed_stats`) | |
| A minion that dies in a Tavern attack | Fishbait's Beast | `Game::tavern_attack`: the dead Beasts' pool copies go back to the pool, the survivors keep theirs. A Fishbait that dies there is reported as a one-card death of the attacker's side (`SideOutcome::death_cards`), never as a friendly death. The attack is a real swing, so the attacker's Rally fires and can summon a third copy: `tavern_attack` runs `check_board_triples` afterwards (the same check a shop destroy runs) | Lionfish's Fishbait, Flittering Bat's Rally |
| A pick of "a card in the Tavern" that may name a Tavern spell | `ShopTargetKind::takes_spell_slot` (`Fishbait`) | `valid_tavern_pick` (the spell slot is one past the last minion) and `put_fishbait`: the spell is dropped and the Fishbait takes a minion slot. A pick of a Tavern *minion* (a consume) still stops at the last minion | Lurking Lionfish (Era, 2026-11-03) |

## 2b. Adding a hero

1. `heroes.toml`: one `[[hero]]` entry (armor and `card_id` from
   `ref/bg-meta.json`, matched by card_id never by name; `extra_health` for
   Patchwerk; `tribes = [...]` when `lobby_subsets` ties the hero; the power
   as data). Powers that need a new effect shape get a `PowerEffect` variant
   in `crates/bgcore/src/heroes.rs` plus where it fires (the hook index
   above); reuse cards.toml machinery where one exists (Blood Gems, Discover,
   free Refreshes, Divine Shield, `extra_max_gold`, `gold_next_turn`,
   make Golden).
2. A button power with a target opens the pending flow in `press_power`
   (`offer_target`); check its needs in `Game::power_usable` (a target to
   land on, hand room for a steal or for the two linked cards, a card to
   discard, a Battlecry to trigger).
3. Tests in `crates/bgcore/tests/heroes.rs`: `hero_game(seed, "Name")` builds
   a game with the hero forced on player 0 (or `Game::new_heroes_with_lobby`
   to play the pick itself), `clear_board` keeps the pool exact.
4. Update `docs/engine.md` "Heroes" if the frame changed; the pick offers,
   the observation (hero indices are embedded, `N_HEROES` in bgpy) and the
   bots need nothing per hero. A power that counts something needs no new
   observation value either: `obs::power_progress` fills that slot's
   `SCALAR_POWER_PROGRESS`/`SCALAR_POWER_LEFT` from its own
   `PowerState.progress` and the shared counters, so add a match arm there
   when the new count does not fit one of the shapes it knows.

### A power a hero only reaches later

A power that is not a starting power (Sulfuras, an Invocation, a flightpath,
a King of X) is its own `[[power]]` entry, keyed by the game data's
`hero_powers` card_id, and the starting power points at it. Nothing is
matched by name: heroes.toml names powers by card_id and the parser turns
them into indexes at load.

```toml
[[hero]]
name = "Ragnaros the Firelord"
power = { name = "BUY, INSECT!", card_id = "TB_BaconShop_HP_087", passive = true,
          effect = { buy_milestone = 12, becomes = "TB_BaconShop_HP_087t" } }

[[power]]
name = "Sulfuras"
card_id = "TB_BaconShop_HP_087t"
passive = true
effect = { end_of_turn_ends = [8, 8] }
```

The keys that make a power change into another:

- `becomes = "<card_id>"` (with `buy_milestone = N`): after N bought cards the
  power is that one. `power_bought_card` counts them in both buy paths; a
  steal does not count.
- `choose = ["<card_id>", ...]` plus `effect = { choose = true }`: a press
  offers that fixed list (up to `CHOICES` = 4) in `power_offer`; the pick
  lands in the slot that was pressed (`pick_power`).
- `returns_to = "<card_id>"` on the chosen power: when it completes the
  chooser comes back, and the chooser remembers it (`PowerState.last_pick`) so
  the next offer can leave it out.
- `complete_turns = N`: the chosen power completes after N of your turns
  (0 means after the next fight's Start of Combat, which is how the
  Invocations work). Its own `effect` is the one-shot bonus it gives then
  (`gold`, `get_random_spell_cost`, `discover_my_tier`, or a Start of Combat
  effect through `Counters`).
- `family = "name"`: powers in one family rotate into each other at the start
  of each turn (`rotate_family_power`), among the ones whose tribe is in the
  lobby and never the tribe held now. The King of X powers are the example.

### A second hero power

`Player.power` is the main power and `Player.power2` the second; both are
ordinary `PowerState`s with their own uses, cost and effect, they both get
the turn-start reset in `power_start_of_turn`, and `Action::HeroPower2`
presses the second. Two things hand one out, and a new one should reuse them:

- `discover_hero_power(p, excludes, to_second, count)`: a Hero Power
  Discover, offered from `PowerDb::powers`' starting powers. `to_second`
  puts the pick in the second slot; `count = 2` deals two picks (the first
  into the second slot, the second into the main one: King of Duality).
  `excludes` is hero indexes; `HeroDb::hero_power_excludes` is the list from
  cards.toml's `exclude_heroes`, which every power-only Discover shares.
- A trinket act `second_power = true` (Reinvigorating Light) opens the same
  Discover into the second slot on the pick.

Only Unmasked Identity and Finley change the MAIN power: nothing changes or
disables the second one. Both are in the observation (`SCALAR_POWER2_*`) and
`Player::counters` reads both, so a Start of Combat power works in either
slot. A hero-power chooser belongs on either button: `press_power(p, second)`
passes the slot through.

### Skipped turns and cards locked in hand

- `skip_turns = N` on a hero's power: the player's first N turns allow only
  Freeze and End Turn (`Player.skipped`, `turn_skipped`), the fights still
  happen, and a Discover such a turn opened can still be answered (Choose is
  legal beside Freeze and End Turn, which resolves it).
  `effect = { turn_discovers = { turn = T, tiers = [..] } }` queues a
  Discover per tier at the start of turn T (`turn = 1` is the start of the
  game), and those picks land with `Minion.locked_tier` set: the card cannot
  be played, discarded or summoned from the hand until the Tavern reaches
  that tier, but it can still be buffed in hand.

## 2c. Adding a trinket

1. Find it in `ref/bg-meta.json` (`trinkets`): `card_id`, `slot` (LESSER or
   GREATER from tag 1635), `cost` (null means free), `lobby_subsets`. Add one
   `[[trinket]]` to `trinkets.toml`. The parser (`TrinketDb::from_toml`)
   resolves every `get`/`cast` name against the card set and refuses an
   unknown key, so a typo is a load error, not a card that does nothing.
2. Write the effects in the hook lists where the card's timing fits: `on_pick`,
   `start_of_turn`, `end_of_turn`, `on_buy` (+ `buy_tribe`), `on_buy_spell`,
   `on_play` (+ `play_tribe`), `on_sell`, `on_refresh`, `on_cast_spell`,
   `on_cast_spell_on_minion`, `on_discard` (+ `discard_spells_only` or
   `discard_minions_only`), `on_get` (+ `get_tribe`, "whenever you get a
   <tribe>"), and the block 4 events: `on_card_play`, `on_battlecry`,
   `on_gem`, `on_attack`, `on_destroy` (+ `destroy_card`), `on_magnetize`,
   `on_cast_spell_on_shop`, `on_consume`, `on_hero_damage`, and `on_any_spell`
   for "whenever you cast a spell" (`on_cast_spell` is the "Tavern spell"
   one); the `rule` events `play_battlecry`, `gem`, `attack`, `destroy`,
   `hero_damage` and `spell_cast` (with `cast_spell` for Tavern spells); and
   the flags `alternate` (the two buy lists take turns) and `drakkari_end`. The keys
   are the effect keys of cards.toml (`get`, `get_gems`, `gem_bonus`, `cast`,
   `tavern_buff`, `undead_bonus`, ...); `crates/bgcore/src/trinkets.rs` lists
   every key in `Act` with the card it was written for. "Every N events" or
   "every N turns" is one `rule` per trinket; "twice per turn" is
   `uses_per_turn`; "at the start of every N turns" is `start_every`, which
   gates the `start_of_turn` list (a `rule` on `turns` fires at the end of the
   turn it reaches N, which is wrong for Gold, so use `start_every` there).
3. Only add a key to `Act` when no `Effect` or `SpellEffect` arm does the job,
   and then add the arm to `Game::run_trinket_act` calling the same helper the
   card path calls, plus one arm to `Game::seat_trinket_act`: the shop replay
   seats a logged trinket without its pick, and that match says whether the new
   key is a rule a state cannot carry (it runs) or something the logged board,
   hand, numbers or Secret zone already holds (it is skipped). The compiler
   makes you choose. A trinket's in-fight effects go in `fight`, not into
   `run_trinket_act`: a fight cannot run a shop effect. Each `fight` table is
   one `FightAct` (`crates/bgcore/src/trinkets.rs`) and one hook in
   `Side::trinket_hook` / `Side::run_fight_act` (`crates/bgcore/src/fight.rs`);
   a reward that goes home travels through `SideOutcome` and
   `apply_side_outcome`. Three slots hold at most `FIGHT_ACTS` (6) acts, which
   the parser checks. A shop action that listens to fight events is a plain
   `rule` with `event = "death"`, `"summon"`, `"attack"` or a tribe of either.
   A hook that needs the card the event is about (the minion just bought, the
   spell just bought, the Tavern slot a spell hit) reads it from
   `TrinketPowers` (`bought_card`, `bought_stats`, `bought_spell`, `shop_hit`),
   which the card path writes just before firing the event; a
   `run_trinket_act` has no card argument and never asks the player anything.
4. Tests in `crates/bgcore/tests/trinkets.rs`: `equip` puts a trinket in a slot
   without its pick effects (`Game::set_trinket`, which also refreshes the
   player's event mask), `take` picks it through the real offer (so `on_pick`
   runs). The shared helpers there answer a prompt before passing the turn,
   because a trinket can open a Discover at the start of a turn. What the
   replay loader has to seat from a log has its own test in
   `crates/bgcore/tests/shop_replay.rs`: a one-turn state that names the
   trinket, loaded through `load`, then the step it changes.
5. A trinket that hands out a minion no card has yet needs a `[[token]]` in
   cards.toml (the "--- Trinket-only minions" block): stats, `golden_stats` and
   the text's mechanic as a card field, so a real card could reuse it. A token
   has no pool copy, and it must fit the lobby it is made in: if the trinket is
   neutral, the token has to be neutral too, and if the token has a tribe the
   trinket carries that tribe (otherwise
   `nothing_outside_the_lobby_ever_shows_up_in_whole_games` fails).
6. Update `docs/engine.md` "Trinkets" if the frame changed. The observation
   carries the slots by trinket index (an embedding, `N_TRINKETS` in bgpy), so
   a new trinket needs nothing there.

## 2d. Adding a Dark Gift

1. Find it in `ref/bg-meta.json` (`dark_gifts`): its `card_id`, the turn
   window (tags 4853 and 4854) and the weight (4856). Add one `[[gift]]` to
   `darkgifts.toml`: `name`, `id` (the game data's card_id, for the real-board
   files and the docs), `card` (the token that does the work, see step 2),
   `window = [first, last]` (one number for "no last turn" left out),
   `weight = 50` if it is one of the half-as-likely ones, `repeats = true` if
   it is a countdown gift that starts its count again after paying out
   (Replication, Affinity; Incubation has none), and `condition`
   (`any`, `battlecry`, `deathrattle`, `tribe:<name>`, `typeless`,
   `end_of_turn`, `murloc_not_venomous`, `activate`, `deity`, `spellcraft`,
   `not_dragon`, `not_typeless`, `not_taunt_or_typeless`).
2. Add its effect as a `[[token]]` in cards.toml's "--- Dark Gifts" section,
   named `Gift: <name>`, using the same keys a card uses. Two of its numbers
   mean something special: `stats` is what the gift adds when it lands, and
   the gift never doubles on a golden host. Reach for an existing field
   first (`keywords`, `deathrattle`, `start_of_combat`, `end_of_turn`,
   `rally`, `permanent_keep`, `buff_per_*`, `on_card_play`, `countdown`,
   `shield_hits`, `immune_attacking`, `all_types`, `reborn_full`,
   `makes_golden`, `acquire_copies`, `activate_twice`,
   `end_of_turn_at_start`). A gift that needs new timing earns a new
   `Effect`, `StartOfCombat`, `EndOfTurn` or `Deathrattle` arm plus its parse
   key, and `Minion::has_gift` (a card flag) is how a fight asks "does this
   minion have that gift". `Sunken Persistence` shows a gift with no arm yet:
   its token is plain and only the test that walks the table knows why.
3. A gift can only be offered from turn 1 or its window, and only on a host
   its condition fits (`GiftCond::fits`). A source with its own tiers calls
   `Game::open_dark_gift_trinket` (trinkets) or pushes a
   `DiscoverRequest::DarkGift(DarkGiftPlan)` itself (a hero power). A gift
   whose token sets `makes_golden` (Gilding) is never offered by a Golden
   source: the offer's minion is golden already, so `GiftDb::pick` leaves
   every such gift out when the plan is golden.
4. Tests in `crates/bgcore/tests/darkgifts.rs`: `board_with_gift` gives a
   board minion a gift through `Game::give_gift`, `gifted` builds one for a
   fight (`bgcore::game::darkgifts::land_gift`), and `every_gift_effect_card_
   runs_somewhere` fails a gift whose token has nothing the engine reads. The
   shared `act` helper returns the turn's cursor first: every action moves it.
5. Update `docs/engine.md` "Dark Gifts" if a mechanism changed; the
   observation carries the gifts as effect card ids in `SLOT_GIFTS`, so a new
   gift needs nothing there.

## 3. New-tribe checklist

1. `cards.rs`: add the variant to `Tribe`, add it to `Tribe::EVERY` (rules use this) and
   `tribe()` parse, and bump the length in `EVERY: [Tribe; N]`. The mask `Tribes`
   is a `u16` (ten tribes fit with room for more); `Tribes::ALL` ("all types")
   gets the new bit.
2. Tribe loops that must use `EVERY`: `Tribes::random_type`, `pick_one_per_type`,
   `bots::preferred_tribe`, sandbox `screen.rs` (`tribe_name` match, `parse_tribe`,
   `card_tribes`), Planar Telescope and Friendly Bounty counts in `game/spells.rs`.
   grep `Tribe::EVERY` to check nothing else needs it.
   The lobby needs nothing: `Tribes::random_lobby(5, ..)` rolls 5 of `EVERY` and
   every draw filters through `Card::lobby_ok` on its own.
3. Tests that use an unknown tribe as the bad value: `tests/beast_mech.rs`
   (`attacker_tribe = "..."`) and `tests/core_rules.rs`
   (`a_card_needs_exactly_one_of_tribe_and_tribes`) use a tribe that does not
   exist: "naga" (it left the game and will never be built, so these tests
   never need changing again).
   Also raise `MAX_TAVERN_BUFFS` in game/mod.rs (one Tavern buff per tribe filter and
   duration, so 2 x (tribes + 1), 22 with ten tribes); a push past it panics.
4. `tests/tier7.rs` Last One Standing test counts picks per type: an all-type minion is
   every tribe, so a new tribe changes its expected count. Re-check it. An all-type
   minion is also a member of the new tribe for every "your <tribe>" rule (Amalgam,
   Par-tea Guest and The Last One Standing count as Undead).
   `tests/deathrattles.rs` puts Titus next to the Deathrattle card under test: a card
   whose Start of Combat destroys its neighbours (Stitched Salvager) needs a filler
   between them (the test does that for any card with a Start of Combat).
5. Observation: tribes are ten-wide everywhere and the lobby is a multi-hot
   (the layout is in the `obs.rs` header; `TRIBES = 11` slots in
   `final_tribes` and `board_tribes`, the ten tribes then neutral). Still do
   not add features in a tribe run: the layout changes only in an observation
   run, and `N_CARDS` in bgpy is `cards.len()` and grows with every card.
   Either way old checkpoints stop loading, expected.
   New `TargetKind` variants need an arm in `obs::target_kind` (share a slot).
   New `Effect`/`SpellEffect` variants do not need obs changes unless a match is exhaustive.
6. `cards.toml`: pool minions go in the `[[minion]]` section (before tokens; ids
   `0..pool_cards` are the pool). Max 256 pool minions (`MAX_POOL_CARDS`; draws store
   ids as `u8`). Dual-tribe cards of older tribes get `tribes = [..]` edited in place.
   Check the export for pool spells and neutrals the new tribe ties
   (`lobby_subsets` in `ref/bg-cards.json`, matched by card_id) and give them
   `lobby = "<tribe>"`; a tribe's own spells usually have no tie.
7. `rules.toml`: pool copies per tier are by tier (`copies_per_tier`); nothing per
   tribe. Bonus keywords list is global. No change for a new tribe.
8. Pool slots: cards that share one pool slot set `pool_slot` in `from_toml`
   (Volumizers). A new group needs the same handling in `draw_filtered`.
9. `record.rs`: a new `Minion` field needs an `Option` in `FileMinion` plus the
   to/from conversions (done for `no_reward`).
9a. Quilboar run, done: Felboar is `tribes = ["demon", "quilboar"]`.
    Firescale Hoarder and Ominous Seer are also Naga in the game; irrelevant while
    Naga is out, edit them in place if Naga is ever built.
10. Sandbox: `--tribe A[,B,...]` fixes the lobby the sandbox plays with; the
    screen shows it (`Lobby:` line). New draw helpers must take the lobby mask
    (a `Tribes`, `Tribes::ALL` draws from everything) and filter through
    `Card::lobby_ok`; see `CardDb::random_tier_minion` for the shape. Update
    the usage string in `bgsandbox/src/main.rs`.
11. New `Effect` variants: add to `needs_choice` / `needs_any_minion` / `runs_in_fights`
    as fits, to the shop `Game::run_effect` (put fight-only ones in its last arm), to
    `Side::run_effect` if it runs in fights (it ends with `_ => {}`).
12. Update `docs/engine.md` where your work makes it wrong, and the tests list there.
13. If a card needs a rule the docs don't settle, say which guess you made in
    your change description; rules that are still guesses live in private
    notes (`docs/notes/UNSURE.md`).

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
from hand puts a Triple Reward spell in hand (cast it to open its Discover); an Activate
with `discard = true` opens `pending_discard`, answered with
`act(g, p, Action::Discard(i))`; Cord Puller has Divine Shield (do not use it as a
"no shield" minion); a card bought, played or given in a test can triple with the
copies already in hand.

Run one test file: `cargo test -p bgcore --test pirates`; one test: add its name;
`-- --nocapture` for prints. Whole suite: `cargo test --workspace`.
Parse-error tests build small TOML strings inline and hand them to
`CardDb::from_toml` (see `tests/core_rules.rs`); a full card set must come out of
`Data::bundled()` or `Data::from_toml`, which refuses a card set without the
Triple Reward spell.

## 5. Gotchas

- **Satellite.** The token is 4/4 (Glambot's); an attacher passes its own stats
  (`stats = [..]` on `on_play_mech` and `on_cast_spell_on_mech`, Spark Snapper 2/3).
- **Spell tiers.** A spell is tier 1 to 7, or 0 for a generated one (Lockbox). Tier 7
  spells are in the pool list but no draw reaches them (see the hook index).
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
  sums buffs, takes the highest `improvements`; a Living Prison's arms and a Drone
  Duplicator's held extras are the exceptions, they sum).
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
  docs/rules/design.md records the owner's answer.

## 6. Systems the Pirate run added

- **Pirates (tribe 6).** `Tribe::EVERY` is every tribe the rules know;
  `Tribes::ALL` includes Pirate.
- **Golden minions.** Played from hand, a Golden minion adds to
  `Player.golden_played` (Extortionist, Hooktusk) and gives a Triple Reward unless
  `Minion.no_reward`: Aureate Laureate is `always_golden`, Silent Deliverer's
  Battlecry goldens and the golden copies a Dark Gift makes (Sinister
  Invitation, Gilding) give none (Era, 2026-11-02). The replay reads that from
  the action file's `no_reward`, which the extractor writes for those sources
  (`collect_actions.rs`'s `no_triple_reward`: Silent Deliverer as the creator,
  the Dark Gift Discover `BG36_MidGameEffect_010` as the creator, the Gilding
  gift in `dark_gifts`, the Laureate's card id), and the loader also sets it
  from the card's `always_golden`, so a Laureate does not depend on the file; a
  golden carrying any other Dark Gift, a triple's golden, a Lockbox's and a
  Golden bought do give one. `TargetKind::MakeGolden` makes a board minion
  Golden with no reward and no count (Elite Navigator, Captain Sanders); the
  golden happens once however often the Battlecry triggers, so its `runs` is
  the card's own count only.
- **Golden draws from the pool.** `draw_golden` (game/shop.rs) takes up to three copies for
  one Golden minion (Lockbox, Silent Deliverer).
- **Gold spent.** Every payment goes through `fire_event(GoldSpent, n)`, which also
  adds to `Player.gold_spent_turn` (Lovesick Balladist). Health payments and free
  Refreshes never count. The shop replay seats the turn's own part of it from the
  log's `gold_spent` counter, the difference against the turn's start state
  (`shop_replay::seat_spell_bonus_turn`); the counter itself is the game's total.
  An Activate whose effect leaves a choice open hands its Gold to
  `Player.pending_gold_spent` instead, and `maybe_start_discover` fires it at
  the pick (see docs/engine.md "Activate").
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
  Elemental (`tribes = [..]`).
- **The Refresh amount.** One `Player.refresh_buff` that Waveling's
  Deathrattle (fight or shop), En-Djinn Blazer and Easterly Winds add into;
  every Refresh gives all of it to one random non-spell, non-`fixed_stats`
  Tavern minion, the turn-start roll included when the Tavern changed (see
  docs/rules/design.md "Two kinds of Refresh"). No random number is drawn while it is
  0, so seeded games without these cards are unchanged.
- **A second player counter.** `N_COUNTERS` is 2: `Counter::Baller` is the
  count of Ballers sold. Tests that spell out `improve` arrays need two entries.
- **Elemental plays.** In `Action::Play`, right after the board insert:
  `Player.elementals_played_turn` (Conflagration, reset in `start_turn`) and
  `give_play_shield`, then, before the Battlecry, `fire_event(ElementalsPlayed)`
  (Unbound Tempest). Every `after_play_tribe` card (Wrath Weaver, Insatiable
  Ur'zul, Ultraviolet Ascendant) waits in `Player.pending_after_play` instead:
  `resolve_after_play` fires it once the Battlecry, and whatever that asks (a
  Choose One, a target, a Discover), is done. None counts the minion being
  played (docs/rules/design.md): its own play marks it `just_played`, and
  `fire_after_play_tribe` skips that minion, not a board slot. Only minions
  played from hand count.
- **Triples with a stand-in.** `check_triple_tracking` builds a `TriplePlan`:
  three plain copies first, else two of a card plus one stand-in, else one
  plus two. Each source minion's buffs are measured against its own card,
  the stand-in keeps only keywords it was given, and its pool copy goes back
  to the pool. A stand-in (`triple_wildcard`, the Elemental of Surprise)
  covers a card when every copy the triple would use is of that tribe
  through `CardDb::tribes_of`, not just on the printed card: the printed
  tribe is the fast path, and a copy the Amalgamation Dark Gift made every
  type counts too (docs/rules/design.md "Elemental of Surprise").
- **Triples from a summon.** A shop-phase summon checks the board for a triple
  once it lands (`check_triples_on_board`, called by `Effect::Summon` when it
  summons more than one and by `trigger_one_deathrattle`): a Cadaver
  Caretaker's three Skeletons are a triple like any other three, and the Golden
  one goes to the hand.
- **Spells counted twice.** `apply_target` fires one extra `SpellsCast` event for
  every extra Balinda cast on a friendly minion (Proud Privateer's extra Bounty
  casts were already counted in `cast_spell`).
- **Triples wait for the shop phase.** Nothing triples during a fight
  (docs/rules/design.md "Nothing triples during combat"). `Game.defer_triples`
  is set while `apply_side_outcome` applies a fight's carry-over, so a
  minion card a fight gives the hand then (a Rally's Beast, a Deathrattle's
  card, a trinket's "after N deaths / summons" counter, MechGyver) is
  recorded on `Player.triples_next_turn` (`defer_triple`, from `add_to_hand`)
  instead of running `check_triple`; `dark_gift_start_of_turn` runs those
  checks as the next turn starts. A new fight-side source of cards gets this
  for free as long as it hands the card over through `add_to_hand`.
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
  Maw Caster (`TargetKind::DestroyDiscover`), Disguised Graverobber
  (`TargetKind::DestroyCopy`: destroy, then `give_generated` the plain card, golden
  gives two, lost when the hand is full), Dead Bellringer
  (`TargetKind::RebornDestroy`), Butchering (`SpellTarget::Butchering`) and Tomb
  Turning's death all use it. A Deathrattle a destroy fires can summon the third copy
  of a card (Cadaver Caretaker's three Skeletons, Eternal Summoner's Eternal Knight):
  `destroy_in_shop` and `trigger_one_deathrattle` run `check_board_triples`, so the
  Golden joins the hand in the same step (docs/rules/design.md "Triples", log
  `2026_10_30_19_45_15-g2` turn 5).
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
  cast by `trigger_on_cast_spell`, so a double cast counts twice; its card says
  "After you cast a Tavern spell", so it sits in `on_cast_spell_after` and fires with
  the "after" half, once the spell has resolved (a Discover it opened comes first).
  Casts inside a fight
  (Runic Arcanist's Shiny Ring) trigger it through `Side::on_spell_cast`, and the gain
  leaves the fight in `SideOutcome::undead_gain`. A second cast that does nothing
  (Balinda on Butchering or Channel the Devourer, `stop_after`) is not a spell cast.
- **Tests.** `tests/undead.rs`; a fight with a victim that has Taunt and 1 Health, or
  Taunt "fodder" minions that die one per attack, is the way to script who dies when.

## 9. Systems the Quilboar run added

- **Quilboars (tribe 9).** `Tribe::EVERY` has nine tribes and the mask
  `Tribes` is a `u16`; `Tribes::ALL` is `0b1_1111_1111`. Felboar is Demon and
  Quilboar.
- **Tavern spells versus plain spells.** `Card.tavern_spell` (from
  `tavern_spell = false` in the toml; pool spells default to true) gates the
  "Tavern spell" triggers (`trigger_on_cast_spell`), the `SpellsCast` event
  counter and spell power: `PendingTarget.tavern` and
  `PendingChooseOne.tavern` carry the flag to `apply_target` and
  `execute_spell_effect`. The Blood Gem, Gem Day, the Triple Reward and the
  Lockbox are plain spells.
- **Blood Gems.** A player value `Player.gem_attack/gem_health` (starts 1/1,
  `Effect::GemBonus` raises it, in a fight too through
  `SideOutcome::gem_gain`), the generated Blood Gem spell whose buff is that
  value, and per-minion Blood Gem stats `Minion.gem_attack/gem_health` from
  every source (hand casts, Barrage, Confiscation, Roogug, Hogrider, Rally
  gems). Fight gems are plain buffs unless `permanent` (Vineweaver), which
  leaves through `SideOutcome::permanent_gems`. A hand cast on a friendly
  minion plays `gem_casts` times: Balinda sets, each Hot-Air Surveyor adds
  ("Set versus add", `cards::how_many_times`). A hand cast on a Tavern
  minion gets no Balinda ("on a friendly minion") but each Hot-Air
  Surveyor's extra cast does reach it: the log
  `10_28_19_47_59-g1` turn 8 gives a Tavern minion +2/+2 for one Gem with a
  Surveyor up. A card that reads "Blood Gems
  played from your hand give an extra +X/+Y" (Surveyor Portrait) raises
  `Player.gem_hand_attack/gem_hand_health` instead, and only a cast from the
  hand adds it: a Gem a minion plays (Hogrider, Confiscation, Roogug) gets
  the player value alone. Every Gem played on a minion goes through
  `Player::add_gem_stats` and `Game::roogug_on_gem`: `Game::gem_on_board`
  does both for a Gem a minion's or a spell's effect plays, a cast that
  plays several at once calls `Game::roogug_reacts` per Gem as each one
  lands (Gem Confiscation's three, before its steal: log
  `2026_10_28_02_28_32` line 1715147 onward), and the Tavern's own Gems go
  through `Game::add_gem_stats_shop`. A Gem an Activate casts because the
  hand had no room (Victorious Geomant) is a real cast on the left-most
  minion through `Game::cast_gem_on`: the target's "whenever you cast a spell
  on this" fires, and the "after you cast a spell" counters see it, but it is
  not played from the hand, so no Hot-Air Surveyor's extra cast and no
  Surveyor Portrait's bonus reach it. Roogug (`on_gem_played_on_self`)
  reacts to
  each one on the board, from the hand, from an extra cast, and from a
  minion's or a spell's effect. A Roogug in the Tavern doesn't react (open,
  in the private notes). The Gems it plays carry the from-Roogug mark
  (`from_roogug`), the one kind no Roogug reacts to, so two Roogugs never
  loop. The fight reacts the same way: every Gem a fight plays goes through
  `Side::gem_at`, which adds the stats and then, for a Gem that is not from
  a Roogug, lets a Roogug there play its own (`Side::roogug_on_gem`) on a
  random different living friendly minion, one per Gem, through the fight's
  own RNG. Its Gems carry the same mark. The fight adds a Gem's stats
  straight to the minion and keeps no `Minion.gem_attack` (that field is the
  board's, from the shop).
  Blood Gem
  Barrage adds to `Player.barrage` per cast; every Refresh gives it to every
  Tavern minion (`apply_barrage`), the turn-start roll included when the
  Tavern changed (see docs/rules/design.md "Two kinds of Refresh").
- **Triple Reward as a card.** A golden minion played from hand puts the
  Triple Reward spell in hand (`Game::give_triple_reward`), its Discover tier
  in the copy's `improvements`, fixed when given; it comes before anything the
  Battlecry adds to the hand and is lost on a full hand. Card sets without
  the spell, `Data::from_toml` refuses the card set.
- **Set versus add, one helper.** `cards::how_many_times(parts, role)`:
  the highest setter (Brann, Drakkari, Balinda, Proud Privateer; 1 if none)
  plus each adder's extra (Titus, Hot-Air Surveyor). `brann_mult`,
  `drakkari_mult`, `balinda_mult`, `bounty_mult` and the fight's
  `deathrattle_fires` all ask it. A repeated Battlecry or spell that picks a
  target uses the same target every time (docs/rules/design.md "Set versus
  add", Era 2026-11-04): put the count in `PendingTarget.runs` (a Battlecry)
  or pass it to `cast_spell_as` as `repeats` (a named spell), never loop a
  second prompt. A repeat that only starts after the first one's answer comes
  back (a golden Kelp Keeper, Drakkari on Young Murk-Eye's neighbours, a
  Choose One cast twice) goes on `Player.repeats` instead:
  `Game::run_queued_repeats` runs it once the prompt is answered, on the slot
  the first run picked and with the Choose One option the player picked there
  (`QueuedRepeat.at` and `.option`, `RepeatWhat`); only a cast with no choice
  of its own takes a new random option each (a spell a minion casts). Making
  a minion Golden never repeats: the prompt carries only the card's own
  count.
- **Discard.** `Action::Discard(i)` answers `Player.pending_discard`, opened
  by an Activate with `discard = true` (Mangled Bandit): the reward effect
  runs after the pick. Any playable card is discardable (a Blood Gem
  included), never the Lockbox or a locked minion; a discarded pool minion's
  copy goes back. The "when you discard" hook is the `Discarded` event.
- **Both effects combined.** Thorned Trailblazer
  (`choose_one_cover`: each Trailblazer on the board holds its own combine
  in `Minion.uses_left`, spent left to right) and
  Fandral's Fortune (a Discover whose pick gets `Minion.both_effects`) run a
  Choose One's two options in order, through `Player.choose_queue` when the
  first leaves a target prompt open.
- **Turbo Hogrider.** `after_choose_one_play` fires after every Choose One
  card played, minion or spell, but only once the card is fully resolved
  (no pending choice, target or Discover; `Game::choice_open`), so the
  played Quilboar gets the gems. Same deferral pattern as Tomb Turning's
  `resolve_doomed`.
- **Jailbird Juggernaut.** `Effect::SummonStatCopyAttacker`, intercepted in
  `attack_triggers`: a Golem token with the Juggernaut's stats (double when
  golden) is summoned to its right and `attack_swings_at` makes it attack the
  target first; a kill fizzles the Juggernaut's own swing (Ravager rule).

## 10. Systems the Aberration run added

- **Aberrations (tribe 10).** `Tribe::EVERY` has ten tribes and `Tribes::ALL`
  is `0b11_1111_1111`; `MAX_TAVERN_BUFFS` is 22. The observation covers
  every tribe since the 2026-10-25 observation run.
- **The Deity.** One god per lobby (C'Thun or Y'Shaarj), drawn from the game's
  RNG in `Game::unstarted`, and only when Aberration is in the lobby (see the
  lobby section in `docs/engine.md`). Every player's `Player.deity` starts 1/1;
  `Effect::BuffDeity` (with `per_spell`), `BuffSelfAndDeity` and the
  `OnCastSpell` deity parts grow it in the shop, `Side::buff_deity` in a
  fight, and a fight's gains come back through `SideOutcome::deity_gain`.
  `Side::note_deity_death` counts friendly Aberration deaths (tokens and
  Reborn copies, never the god itself) after each death's Deathrattles: at
  the fourth, `Side::try_awaken` summons the god at the right end with the
  Deity's current stats as a normal summon, retrying after any later death
  while the board is full. Once per fight, the count restarts each fight.
  C'Thun then splits its Attack and Health (golden double) among the other
  living minions, equal shares, the remainder +1 to random different ones,
  and keeps its own stats. Y'Shaarj's Deathrattle
  (`Effect::SummonDeadAberrations`) summons the first 2 (golden 4) friendly
  Aberrations that died that fight where it stood, at their maximum stats
  (Health plus the damage `Fm.taken`).
- **One discard path.** `Game::discard_card(p, slot)`: `valid_discard`,
  `give_back`, the `Player.discarded` count, `fire_event(Discarded)`, then
  the card's own `Card.on_discard`: `OnDiscard::CastSpell(2)` casts it as
  real Tavern spells (spell power, cast triggers, spell counters),
  `OnDiscard::MaxGold(2)` only raises maximum Gold. Every discard goes
  through it: Activate rewards, linked cards, Mysterious K'Thir
  (`Effect::DiscardLeftmostSpells`, the N left-most Tavern spells only,
  skipping Blood Gems, Triple Rewards and a Lockbox). An Activate that
  discards waits for the pick (`Game::resolve_discard`), and the reward runs
  before the discarded card's own `on_discard`, so a discarded Sludge
  Corrosion casts with the improvement the Activate just gave
  (log 2026_10_28_22_11_50-g6 turn 9 step 35).
- **Every-1 event triggers keep `improvements`.** `Game::fire_event` stores
  an event trigger's remainder in `Minion.improvements`; an every-1 trigger
  has none, so it leaves the field alone and an effect that improves itself
  on every fire (Harbinger Aph'lass, `Effect::BuffDeityAndImprove`) stores
  its level there.
- **One hand-insert hook.** `Game::add_to_hand` is the only path into a
  player's hand (buys, generated cards, Discover picks, Triple Rewards,
  Blood Gems, the Lockbox, `Game::give`); a full hand drops the card first
  (except at the start of the turn, where it waits in `Player.hand_queue`:
  docs/engine.md "A full hand"), then `Card.on_card_added` runs (The Shadow
  of Doubt buffs the Deity).
- **Linked cards.** `Minion.linked`: the low 14 bits are a group id from
  `Player.next_link`, bit 15 (0x8000) marks a golden four-card group, bit 14
  (0x4000) that one of it was played. `Game::give_linked`
  (`Effect::GetLinkedSpells`, `GetLinkedMinions`) stamps the cards;
  `Action::Play` discards the rest of the group at once for a pair, after
  the second play for a golden four, through `discard_card` (a real
  discard). The link lasts across turns and is recorded in `record.rs`.
- **De-volition-ist.** `Effect::DamageHighestHealth`, intercepted in
  `attack_triggers` after the minion's own swing (even when it died
  attacking): damage equal to its Attack (golden double) to a random tied
  highest-Health enemy, honouring Divine Shield and Venomous.
- **Hidden numbers** (docs/rules/design.md): Mindbending Recruiter's Activate costs 0;
  Aph'lass +2/+1 improving +2/+1 per discard; Parasitic Fleshling +2/+2 per
  card discarded this game (`Player.discarded`); Faceless Converter +2/+1
  per Tavern spell cast this game (`Player.spells_cast`, raised by fights
  through `SideOutcome::spells_cast_gain`).
- **Tests.** `tests/aberration.rs`. A Boom-in-a-Box Start of Combat (a
  golden one deals two waves) kills known low-health Aberrations in board
  order, which scripts the four deaths an awakening needs without a random
  attacker; two different sleeper tokens make per-minion gains measurable.

## 11. Systems the trinket block 4 added (2026-10-31)

- **Spells versus Tavern spells.** `Event::SpellCast` (trinket list
  `on_any_spell`, rule event `spell_cast`) is every spell; `Event::CastSpell`
  (`on_cast_spell`, `cast_spell`) is a `tavern_spell = true` card only, and a
  Triple Reward is a plain spell, so the first list has it and the second does
  not. `EventKind::AllSpellsCast` is a minion's
  "after you cast N spells" counter; `EventKind::SpellsCast` and
  `Player.spells_cast` stay Tavern-only. Read each card's words in
  `ref/hsjson-cards.json` before picking one.
- **One player-level trigger list per event, not a field per card.** The
  frame's lists and its mask carry every block 4 hook; see "Block 4: the
  player-level hooks" in `docs/engine.md`. `Player.trinket_events` is a `u32`
  (the events take bits 0 to 22, `STARTS_TURN`/`ENDS_TURN`/`HAS_TRINKET` the
  top three).
- **Battlecry multipliers in one place.** `battlecry_or_choose_one` counts
  Brann, War Drum (`take_war_drum` spends the trinket's own use) and Dragon's
  Eye, and the `runs` it reaches is what the "for each Battlecry you've
  triggered" cards read (docs/rules/design.md "Counting Battlecries": Dark Paradox t6,
  Murky Sticker, Battle Scars), so a new multiplier needs no second change.
  A Battlecry that opens a choice owes that count until the pick is made
  (`Player.pending_battlecries`, paid by `maybe_start_discover`), whatever
  the choice is, so a new target or Discover arm needs nothing either.
  `Game::run_battlecry_at` triggers a Battlecry outside a play and can
  answer its target at random out of the board and the Tavern; that is the
  only path that reaches `apply_target_shop`'s `TargetKind::MinionBuff` arm.
- **A Deathrattle outside combat.** `fight::shop_trigger_deathrattles` builds
  a one-sided `Side` (`outside`, no Avenge), fires one minion's Deathrattles
  and resolves what they leave, and `Game::trigger_one_deathrattle` puts the
  board and the outcome back. Nothing dies, so the minion keeps its place.
- **A destroy is its own event.** `Game::destroy_in_shop` fires `Event::Destroy`
  per dead minion with its card; Plaguerunner Portrait narrows it with
  `destroy_card`, Maw Caster Portrait counts every destroy.
- **Portraits are data.** `card_mods` on `TrinketPowers` is an array of
  `(card, CardMod)`; the trinket says which card and which rewrite, the
  card's own effect reads it (`also_attack`, `also_neighbors`, `also_health`,
  `also_board`). No rules code names a card.
- **Blood Gems from Jewelry Box.** `Blood Gem (Taunt)`, `(Divine Shield)` and
  `(Reborn)` are `blood_gem = true` tokens whose `SpellTarget::GemKeyword`
  gives the player's Gem value and, on a Quilboar only, the keyword. The plain
  `Blood Gem` stays the canonical one `CardDb.blood_gem` points at.
- **Stats a fight's spell casts leave behind.** `permanent = true` on a fight
  `buff` act is `FightAct::PermanentStats`: `Side::gain_permanent_at` buffs the
  minion and records the gain by start slot, so it comes home after the fight
  (Bewitched Ribbon's "+2/+4 while in combat", which is permanent too). A
  minion summoned during the fight keeps nothing.
- **Two fight leftovers.** `FightAct::Fermenter` (Soul Fermenter) keeps the
  destroyed minions in `Side.fermenter` and `Side::resolve` brings them back
  when the side's last minion dies; `FightAct::GemGolem` summons a token with
  the dead minion's Blood Gem stats (`Side.last_dead`, `Side.dead_at`). The
  Golem token is Neutral (its own card has no race), so its death is no
  Quilboar death and no Golem summons another for ever.
- **Tests.** `tests/trinkets.rs`, the block 4 section at the end: one test per
  card, and `apply_side_outcome` is public so a `fight_with` outcome can be
  applied to a game the way a combat does.

## 12. Systems the hero batch 1 added (2026-11-01)

- **The counters are one block.** `Player.counts` (`HeroCounts`) is where a
  hero power counts what the player does; a new counter is a field there, not
  a field per hero. The growth points are `power_bought_card` (both buy paths
  and the spell slot, which is also where a power's own per-cycle count in
  `PowerState.progress` is bumped), `power_sold_minion` in `sell_minion`,
  `power_elemental_played` in the play path, `power_attacks` in
  `apply_side_outcome` (the fight's `friendly_attacks`), and
  `power_gold_spent` from `fire_event(EventKind::GoldSpent)`. A Health price
  is never Gold spent.
- **`power_bought_card(p, &Minion)`** takes the bought card itself: it needs
  its Tier (Natural Balance counts Tiers, a spell by its Tier) and whether
  it is a Battlecry minion (Battle Brand). It also runs the powers that watch
  a count, so a card bought outside these paths (a steal) counts for nothing.
- **A press price that moves.** `PowerState.cost_delta` plus
  `Game::power_cost(p, second)`; never read `HeroPower::cost` directly for a
  press, the price or the observation. `cheaper_each_turn = true` on a power
  takes 1 off at every end of turn.
- **Presses a game.** `HeroPower.uses_per_game` (0 no limit); the old
  `once_per_game = true` key still parses and sets it to 1. A passive checks
  it before it counts.
- **Ready-made Discover options.** `Player.discover_minions` holds the
  minions an open Discover offers as they stand, one per `discover` slot, and
  no pool copy moves for them (`resolve_choose` and `abandon_discover` both
  handle it). Kel'Thuzad's Kitty fills it from `Game.kitty_warbands`: every
  warband of a round is kept there with the round it died in
  (`Game::kitty_watch` from `knockouts` and `knock_out_mid_turn`), and each
  holder queues one `DeadWarband(i)` per warband at the start of its next
  turn, so several heroes dying in one fight give several Discovers back to
  back. Reclaimed Souls fills it with plain copies of
  `Player.dead_last_fight` (both sides' dead from `SideOutcome.dead_cards`,
  tokens included, which have no pool copy to take).
- **Cards that lock a card in hand** (Faelin's tier, Thorim's Gold) all use
  `Minion::locked_in_hand(tier, gold_spent)` over `locked_turns`,
  `locked_tier` and `locked_gold`; `discover_locked_tier` and
  `discover_locked_gold` mark the open Discover's pick. Nothing to hold
  outside the hand, nothing for `check_invariants` or `settle_dead`: the card
  is a hand slot, and the observation's `SLOT_LOCKED` and the action mask
  both read the lock.
- **Gone for good.** A minion that leaves the game without being sold (Stir
  the Pot's throw, The Perfect Crime's leftovers) has its `pool_copies` added
  to `Game.consumed` by `Game::consume_copies`; without that the pool
  invariant fails, and it is the one place that says "this copy is not
  coming back".
- **Three new target kinds.** `RemoveTierBelow` (remove a friendly minion,
  no sale, the pool copy back, then a Discover one Tier below it),
  `SwapWithShop` (the picked non-Golden minion and a random Tavern minion
  trade places, nothing triggers) and `ThrowInPot { count }` (board or
  Tavern, gone for good; a neutral minion is no target at all, and
  `Game::has_throwable` is the press's legality). The first two are board
  picks, so
  `TargetKind::takes_shop` does not list them.
- **A random cast.** `Game::cast_random_spell` is Puzzle Box: a random
  Tavern spell (`CardDb::random_castable_spell`, which skips the destructive
  ones by effect shape, see `destructive_at_random` in cards.rs), a random
  target (`answer_target_at_random`) and a random Choose One option. A spell
  with nothing to land on is skipped through `Game::spell_castable`, which
  `cast_spell_inner` shares.
- **Tests.** `tests/heroes_batch1.rs`, one test per hero. Players shop at the
  same time and the cursor moves on after every action, so a test walks the
  turn back to its player (`turn_of`) before pressing or answering; a power
  pressed twice in one turn needs its use given back (`again`). `duel` leaves
  two players paired with each other for the fight-driven powers.

## 13. Systems the hero batch 3 added (2026-11-01)

- **A power that fights.** Read the fight section here first: a hero power has
  no host, so anything it does inside a fight is a `Counters` field that
  `Player::counters` fills from the two power slots, and the fight reads it in
  `run_start_of_combat` (Wingmen before everything, the rest at the
  hero-power step) or at the hook the rules name (`Side::spawn_when_space` for
  "when you have space", `Side::count_damage` for damage dealt,
  `attack_swings_at` for a kill). A passive power's tick is multiplied by
  `tp.power_triggers` (`Player::tick_power_effect`), so "your Start of Combat
  powers trigger twice" (Valdrakken Wind Chimes, Ancient Wishbone) needs no
  per-power code: every Start of Combat count in `Player::counters` goes
  through it, while a keyword mask (`soc_leftmost`, `soc_rightmost`) keeps the
  single grant, because the same keywords twice are the same keywords. A
  press that fires in the
  next fight arms the slot (`PowerState.armed`, set in `press_power`, read
  through `Player::armed_power_effect`, cleared at the start of every turn).
- **A copy or a token.** `Side::exact_copy(i)` is the copy of a board minion
  as it stands right now (buffs, keywords and gifts kept, the damage it has
  taken carried over, `pool_copies = 0`), and `Side::summon_exact_copy` puts
  it on the board with the original's `taken`, so a 10/10 at 4 Health copies
  as a 10/4 whose maximum Health is still 10 (docs/rules/design.md "Exact copies").
  `Side::summon_saved_copy` summons a copy the fight saved earlier (Rapid
  Reanimation's queue, Stitched Salvager's), so all four cards copy the same
  way. A hero
  power's token is a cards.toml `[[token]]` and the power is
  `effect = { start_minion = "Name" }`: the board gets it when the hero is
  taken (`take_hero`) and when such a power is picked at turn 0
  (`set_power`). Menagerist's Amalgam, Avatar of N'Zoth's Fish and Pilot the
  Shredder's Shredder are the three.
- **Something stored for the fight.** `Player.locked_and_loaded` is Lock and
  Load's Tavern minion: it leaves the Tavern, its pool copy goes back
  (`give_back`) and the player keeps `pool_copies = 0`, because it fights that
  one combat and is dropped at the start of the next turn. The fight reads it
  from `Counters.locked_and_loaded`. Anything else a power holds for one
  fight belongs in that shape, not in a card on the board.
- **A press that opens a two-way choice.** Friendly Wager and Detective for
  Hire open `Player.wager_offer` / `Player.holmes_offer` in `press_power`; a
  choice like that needs four things or the bots and the training loop hang:
  the `is_legal` arm (and `Game::is_choice_open`), the `resolve_choose` arm,
  a scalar in `obs.rs` and `train/model.py` (`SCALAR_IDS` when the value is a
  card id), and an arm in `bots::answer_pending`. `Game::settle_wagers` shows
  how a guess's payout is settled after the round's fights.
- **A counter a fight grows.** `apply_side_outcome` is where a fight's outcome
  reaches the player: `power_damage_dealt` (Bloodfury, from
  `SideOutcome.damage_dealt`), `power_friendly_deaths` (MechGyver, shop
  destroys included because they go through the same call) and
  `power_broodmother_gain` (Onyxia). A power that becomes another power at a
  milestone is `becomes` in heroes.toml (`DamageMilestone`,
  `AttackMilestone`), like BUY, INSECT!.
- **Tests.** `tests/heroes_batch3.rs`, one test per hero: the game-side ones
  use the batch 2 helpers, the fight-side ones call `fight_with` /
  `fight_with_hands` with the counters set by hand and watch the order with a
  `Watch`. One trap: a `Game` is 128 KB and libtest gives a test 2 MiB, so a
  test with several games alive at once (or a fresh local per section) must
  reuse one binding through a `reset` helper, or it overflows the stack in a
  debug build before any assertion runs.

## 14. Systems the trinket block 5 added (2026-11-01)

- **The Darkmoon Prizes.** A Prize is a card with `prize = <tier 1 to 4>`
  (`Card.prize`), tier 0, a token and never a Tavern spell; the 31 spells and
  the one Prize minion (All-Seeing Elder) are in cards.toml's "Darkmoon
  Prizes" block. `CardDb.prizes` is the four lists a `DiscoverRequest::Prize`
  deals three different cards from, and a Prize offer holds no pool copy
  (`resolve_choose` and `abandon_discover` both read `prize`/`token` for
  that). Their effects are the ordinary `SpellEffect`/`SpellTarget` arms:
  `DiscoverPrize`, `DiscoverPrizesBelow` (Big Winner!), `RandomPrize`,
  `FillHand`, `FillHandSpells`, `ReturnEachTurn`, `EvolvingTavern`,
  `StealAllTavern`, `ExtraShopMinion`, `FreeRollsPerTurn { now, per_turn }`,
  `RandomSpellAtLeast`, `SpellDiscountThisTurn`, `MinionPrice`,
  `BattlecryExtraThisTurn`, `SpellBonus`, and the targets `BuffThenDouble`
  (a "then double it" buff), `ReturnToHand`, `MakeGoldenReturn` and
  `GrantEndOfTurn`. Two Discover kinds are theirs too
  (`HigherTierMinion` for Top Shelf, `LastOpponentMinion` for Time Thief,
  which fills `discover_minions` with plain copies of `Player.seen`).
- **What outlasts the cast.** `Player.prize` (`PrizePowers`) is a second small
  block beside `TrinketPowers` for the Prizes' standing rules; its per-turn
  fields are cleared in `start_turn`. A "gain at the end of your turn" a spell
  gives one minion is `Minion.end_turn_gain` (recorded in `record.rs`, summed
  by a triple, added by a Magnetization). A standing Tavern buff a Prize adds
  is no new mechanism: New Recruit's +2/+2 goes into `Player.refresh_buff`,
  which Waveling, Easterly Winds, `apply_refresh_buff` and the turn-start roll
  already give, so the two stack (Era's rule).
- **Balinda Stonehearth and the Prizes.** A Prize is a spell, so a Prize that
  targets a friendly minion gets `PendingTarget.runs = balinda_mult(p)` like
  any other: with Balinda the whole effect runs twice (three times golden),
  and a "then double it" buff doubles on each cast. Nothing on that path asks
  whether the spell is a Tavern spell. `evolve_tavern` replaces the Tavern
  spell with one Tier higher as well; `check_invariants` only asks the spell
  slot to hold a spell, the way it only asks a Tavern minion to be a minion
  for Robust Evolution's above-tier card.
- **The last trinkets.** Their acts are `TriplePrize`, `DiscoverPrize`,
  `RandomPrize`, `IceBlock`, `Surprise`, `YoggWheel` (with the trinket's
  `wheel` list), `CurseOfFlesh`, `DevouringHunger`, `RodOfRoasting`,
  `GoldenTavern`, `CastRandomSpells` and `MechaDemon`. Two card-level games
  came with them: `Card.magnetize_tribes` (which tribes a Magnetic card
  merges onto, Mech by default) and `Minion.extra_tribes` (the Bronzebeard
  Portrait's tribe change, read by `CardDb::tribes_of` and written by
  `Player::make_card` and by the pick's sweep over the board and hand).
  Mishmash's stat copy is `Game::note_amalgam_gains`, run after every action
  beside `watch_deity_gain`, with a per-slot snapshot on the player.
- **Two tags nothing reads yet.** `Card.buddy` (Mishmash, for when buddies
  come) and `Trinket.secret` (Safety Patch, for Akazamzarak later).
- **Tests.** `tests/prizes.rs`, one test per Prize plus the Discover, the
  Triple Prize and Balinda's extra casts (plain and golden); the block 5 section at the end of `tests/trinkets.rs`, one
  test per trinket and token; `tests/heroes_batch4.rs` for Tickatus. The
  Yogg wheel is random, so its test hunts seeds until each of the six results
  has been seen.

## 15. Systems the trinket block 6 added (2026-11-01)

- **A trinket that keeps a minion type.** `lock_type` (`Act::LockType`) reads
  the board's most common type once and stores it in `TrinketState.tribe`,
  which `set_trinket` clears with the rest of the slot (docs/rules/trinkets.md: "the
  type is fixed at the pick"). The read is
  `crate::darkgifts::most_common_type` on `CardDb::tribes_of` of every board
  minion, so an all-type minion counts for every tribe, a tie is broken at
  random and an empty board gives None; `Game::board_most_common_type` is the
  one wrapper. `random_most_common_type` (which already existed for Charisma)
  now hands out minions of the slot's stored type and only falls back to the
  live board when the slot holds none, so a card can use it for "of the type
  this trinket locked in". The observation carries the stored type per slot in
  `SCALAR_TRINKET_TRIBE`.
- **A trinket that makes a bought minion Golden.** `golden_bought` on `on_buy`
  (`Act::GoldenBoughtType`) makes the minion just bought Golden when its
  tribes include the slot's stored type: `golden_up` for the stats, then the
  type is cleared. It is not a triple, but it was bought and played from the
  hand, so it gives its Triple Reward like any other Golden bought
  (docs/rules/design.md "Triple rewards", and the log: a Golden Sanguine
  Refiner's play creates its Triple Reward in
  `Hearthstone_2026_10_28_22_11_50-g1` turn 9), so nothing sets
  `Minion.no_reward`. The act finds the minion through
  `TrinketPowers.bought_card`, which `Action::Buy` writes beside
  `bought_stats` just before the Buy event; the buy's own `check_triple` has
  already run by then, so a buy that made a triple keeps its Triple Reward and
  the trinket stays armed.
  A loaded state's pick never ran, so the type comes from the game's own
  trace of it: `seat_trinkets` reads the race code the game writes in the
  trinket's first number (43 a Quilboar, 20 a Beast: `race_code_tribe`,
  `tools/export_cards.py`'s `RACES`), and `shop_replay::compass_spent` empties
  it for a step the file shows the payout behind (a plain Tavern card bought
  Golden into the hand with no copies behind it for a triple).
- **The keep for every Dragon.** `permanent_keep = "all_dragons"`
  (`PermanentKeep::AllDragons`) is Persistent Poet's `adjacent_dragons` for
  every living friendly Dragon instead of the two neighbours, the keeper's own
  gains included; `Side::keep_mul` is the one place that reads it, so a golden
  keeper doubles the gains and a keyword gained in the fight is kept the usual
  way. Its token, Timewarped Poet, is in cards.toml's "Trinket-only minions"
  block.
- **Tests.** The block 6 section at the end of `tests/trinkets.rs` (the Poet
  Portrait's token, both compasses' fixed type, an all-type board, an empty
  board, and the Gold-plated Compass's Golden buy that yields no Triple
  Reward) and `timewarped_poet_keeps_every_dragons_gains_and_its_own` in
  `tests/dragons.rs`.

## 16. Systems the Secrets run added (2026-11-01)

docs/rules/heroes.md "Secrets (Era, 2026-11-01)" is the rule; this is where it
hooks in.

- **The zone is data, not cards.** `crates/bgcore/src/secrets.rs` holds
  `Secrets`: five `u8` kinds (the four combat Secrets, then the Ice Block)
  and a length, in the order they were got, which is the order they fire in.
  `Secrets::add`/`remove`/`has`/`iter`/`combat_len`/`all_combat`; `Copy`,
  `Eq`, `Hash` and `Default`, so it rides in `Player` and in `Counters`
  without allocating. `NAMES` and `CARD_IDS` name each kind (the export's
  `TB_Bacon_Secrets_01`, `_07`, `_15`, `_10` and `_12`). The other Secrets in
  the data are not in the game and must not be added.
- **Moving the Ice Block in.** It used to be `Player.ice_block`; it is now a
  kind, so the two fatal-damage paths ask `secrets.ice_block()` and remove
  `ICE_BLOCK`. It is never offered (Prestidigitation counts combat Secrets
  only) and never fires in a fight.
- **A fight gets the zone.** `Player::counters` copies it to
  `Counters.secrets`; `Side::new` keeps `Side.secrets` and fires each one in
  order with `Side::fire_secret`, which also sets its bit in
  `SideOutcome.secrets_fired`. `apply_side_outcome` removes those kinds from
  the player's zone after the fight, so a fired Secret is gone for good.
- **Where the two events fire.** "When a friendly minion is attacked" is
  `Side::attack_secrets`, called from `attack_swings_at` after the attack
  triggers and before the hit, so Autodefense Matrix's Divine Shield blocks
  it. "When a friendly minion dies" is `Side::redemption`, called at the end
  of the dead minion's turn in `Side::resolve` (after its Deathrattles and
  its Reborn). All three summoning paths go through `Side::summon`, so a copy
  or a Cobra gets the summoned-minion bonuses (Undead, Eternal Knights,
  Goldrinn, the trinket aura) with no card named anywhere. A Secret that
  would summon and finds the board full stays in the zone; Redemption's copy
  waits in `Side.secret_wait` and `spawn_when_space` puts it down when room
  opens.
- **The Cobra is a token.** cards.toml's "Cobra" (`[[token]]`, 2/3, Beast,
  Venomous, `secret_cobra = true`) is the only card flag: the parser stores
  it in `CardDb::cobra_token`, the way `beetle` gives `beetle_token`. The
  export has no card for it (the spell's script makes it).
- **Combat only.** `Side.outside` (a Tavern attack, a shop destroy) turns
  every Secret hook off.
- **Prestidigitation reuses the Choose flow, not a Discover.** Four Secrets do
  not fit `DISCOVER` (three), so the offer is `Player.secret_offer`
  (`ArrayVec<u8, CHOICES>`), read by `is_legal`, `legal_actions`,
  `resolve_choose` (`Game::pick_secret`), the bots' `answer_pending`, and
  `other_choice_open` (a trinket offer waits behind it). `power_usable` needs
  a combat Secret still missing.
- **The observation** writes the player's own zone only: `SCALAR_SECRET`
  (four flags) and `SCALAR_SECRET_OFFER` (kind + 1 per slot); an opponent's
  Secrets are written nowhere. `obs::power_progress` gives the holdings and
  the count still missing.
- **Tests.** `tests/secrets.rs`: one per Secret, two Secrets on one attack in
  both orders, a full board (Venomstrike waits, and Pack Tactics fills a six
  to make it wait), the Eternal Knight copy at 7/5, Goldrinn reaching the copy
  and the Cobra, Redemption on a board its Deathrattle filled, the zone and
  `apply_side_outcome`, the offer and the Ice Block, and the observation.
  `tests/trinkets.rs` and `tests/heroes_batch4.rs` read the Ice Block through
  the zone now.
