# Adding cards and tribes

A working reference for whoever adds the next tribe. Rules are in
`DESIGN.md` Part 2 (they win over everything here), the code map and
principles in `docs/ENGINE.md`. Function names, no line numbers: grep them.

Files: `cards.toml` (data), `crates/bgcore/src/cards.rs` (parsing, `Effect` and
trigger enums), `game/` (shop phase, one file per system, see `docs/ENGINE.md`),
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

The shop-phase functions named below live in `crates/bgcore/src/game/`, one file
per system (see `docs/ENGINE.md` for the map): `run_effect` and event triggers in
effects.rs, spells in spells.rs, hero powers in heroes.rs, the Tavern and pool in
shop.rs, Discover and Choose One in discover.rs, pending targets in targets.rs,
triples in triples.rs, turn flow and combat in turn.rs, and `step`/`apply` and
legality in mod.rs.

| Kind | cards.toml field | Fires in | Example |
|---|---|---|---|
| Battlecry | `battlecry = {..}` | `Game::battlecry_or_choose_one` (via Play), Brann's `brann_mult` runs, `fire_after_battlecry` after | Southsea Busker |
| Choose One | `choose_one = [a, b]` | `battlecry_or_choose_one` -> `PendingChooseOne` -> `resolve_choose`; with no option the state can answer, the minion is played and the choice is skipped (as for a Choose One spell) | Intrepid Botanist |
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
| Gold spent / hero damage / sold / bought / Elementals played / discarded | `on_event = { event = "gold_spent" / "hero_damage" / "minions_sold" / "cards_bought" / "elementals_played" / "discarded", every, effect }` | `Game::fire_event` (count in `improvements`, remainder kept, can fire several times; an every-1 trigger leaves `improvements` alone so a self-improving effect can use it) | Gunpowder Courier |
| Sell | `on_sell = {..}` | `Game::sell_minion` (`OnSell`) | Greedy Conniver |
| Buy | `Action::Buy`/`BuySpell` in `apply`; triple via `check_triple` | `apply` | none yet |
| Play a minion of a tribe | `after_play_tribe = { tribe, effects }` (never counts the minion being played, no flag; runs right after it lands, before its Battlecry) | `fire_after_play_tribe` | Wrath Weaver |
| While in hand, after a tribe is played | `while_in_hand` | `trigger_in_hand_on_play` | Bream Counter |
| After you trigger a Battlecry | `after_battlecry` | `fire_after_battlecry` | Kalecgos |
| Discover (the effect) | `discover_spell`, `discover_tribe_if_another`, `Effect::Discover*`, spell `discover = ..` | `discover_queue` of `DiscoverRequest`, `maybe_start_discover`, `resolve_choose` | Rodeo Performer |
| After you Discover | `after_discover = {..}` | `fire_after_discover` (end of `resolve_choose`) | Hooktusk |
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
| A Blood Gem cast from hand lands on this | `on_gem_played_on_self = 1` | `Game::roogug_on_gem` (from-hand casts only, never a chain) | Geomagus Roogug |
| A Choose One card each turn gets both effects | `choose_one_cover = 1` | one combine in `Minion.uses_left` per Trailblazer on the board, spent by `take_trailblazer_cover`, `resolve_choose_options` | Thorned Trailblazer |
| After a Choose One card is played | `after_choose_one_play = 2` | `Player.hogrider_pending`, fired by `Game::resolve_hogrider` once no choice is open (from `step` and `finish_shop`) | Turbo Hogrider |
| Rally: summon a stat copy that attacks first | `rally = { summon_stat_copy = "Golem" }` | `attack_triggers` + `attack_swings_at` (fight); shop copy summons only | Jailbird Juggernaut |
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
| A card is discarded, any source | `on_event = { event = "discarded" }`; on the discarded spell itself `on_discard = "cast_twice"` / `"max_gold_2"` | `Game::discard_card` (Activates, linked cards, Mysterious K'Thir): pool copy back, the `Discarded` event, then `OnDiscard` (two real Tavern spell casts, or maximum Gold) | Cutthroat K'Thir |
| A card is added to your hand | `on_card_added = { buff }` | `Game::add_to_hand`, the one path into a hand (buys, Gems, Triple Rewards, Discover picks, generated, Lockbox, `Game::give`) | The Shadow of Doubt |
| You get a card of a tribe | `on_event = { event = "got_pirate", every = 4, effect }` (the event is `got_<tribe>`); a trinket listens with `get_tribe` + `on_get` | `Game::add_to_hand` fires `EventKind::CardGot(tribe)` and `trinkets::Event::GetTribe(tribe)` for every tribe the added card has | Doubloon Grifter, Trusty Crowbar |
| After this attacks and kills | `on_kill = "max_stats_to_hand"` (`KillReward`) | `attack_swings` in fight.rs, at the hit that takes the defender below 1 Health; "maximum stats" are its Attack and its Health before the hit plus what it had already taken, so a Venomous kill reads right | Tide Oracle Morgl |
| Stats for every Elemental played / time Magnetized this game | `buff_per_elemental_played`, `buff_per_magnetize` (`[a, h]`, doubled when golden) | `Player::make_card` reads `Player.elementals_played` (the play path) and `Player.magnetizations` (`Game::note_magnetized`, every `Minion::magnetize` the shop does) | Flourishing Frostling, Dr. Boom's Monster |
| Selling a minion of a tribe gives its stats to the ones that stay | `on_sell = { gain_sold_stats = "<tribe>" }` (empty string for any) | `Game::sell_minion` after the sell: the minion with the text is still on the board, so it is scanned there, not on the sold card | Arcane Behemoth |
| Gain the Deathrattle of a friendly Deathrattle minion that dies | `gains_friendly_deathrattles = true` | `Side::gain_deathrattles` (from `Side::resolve`) fills `Side.gained_dr` by uid; `Side::fire_deathrattles` fires them with the minion's own, once each | Fish of N'Zoth |
| Choose a minion and become a copy of it | `battlecry = { become_copy = true }` | `Effect::BecomeCopyChosen` -> `TargetKind::BecomeCopy` (board or Tavern) -> `Game::become_copy_of`: an exact copy, no pool copy, no Triple Reward | Faceless Manipulator |
| Get a plain copy of a minion | `Effect::PlainCopyRandomFriendly` (`plain_copy_random_friendly`), a spell `effect = { target = "chosen", plain_copy = 3 }` (`SpellTarget::PlainCopy`) | a plain copy is `give_generated` (the card at its printed stats, no pool copy): the same shape Disguised Graverobber's copies use | Reflective Pendant, Duplicating Lens |
| Give your Deity stats | `buff_deity` (+`per_spell`), `buff_self_and_deity`, `buff_deity_and_improve`, spell `buff_deity`, `on_cast_spell = { buff_self_and_deity / buff_all_and_deity }` | `Game::run_effect` / `execute_spell_effect` / `trigger_on_cast_spell`; in a fight `Side::buff_deity`, the gain leaves through `SideOutcome::deity_gain` | Joyous |
| Linked cards | `linked_spells = 2`, `linked_minions = "aberration"` (golden links four) | `Game::give_linked`, `Minion.linked`; `Action::Play` discards the rest of the group (a golden four after two plays) | Wandering Willbreaker |
| The god tokens | token `deity = "cthun"` / `"yshaarj"` | `Game::unstarted` picks one god per lobby onto `Player.deity`; fights: `Side::note_deity_death`, `try_awaken`, C'Thun's split, `Effect::SummonDeadAberrations` | C'Thun |
| A neutral or spell tied to one tribe | `lobby = "<tribe>"` (the export's `lobby_subsets`) | `Card.lobby_ok` filters it into the pool and every draw; it only exists in lobbies with that tribe | Butchering |
| Set your Armor to N | `effect = { set_armor = N }` | `cast_spell_inner` -> `SpellEffect::SetArmor` | Armor Stash |
| Discover a new Hero Power | `effect = { discover_hero_power = true, exclude_heroes = [..] }` | `cast_spell_inner` -> `Game::discover_hero_power` (offers in `Player.power_offer`, answered with Choose) | Unmasked Identity |
| Hero power (button) | `heroes.toml` `power = { cost, uses_per_turn, once_per_game, unlock_tier, unlock_turn, effect }` | `Action::HeroPower` -> `Game::press_power` (`power_usable` is the legality); a targeted power opens the pending-target flow | Lich Baz'hial |
| Hero power (passive) | `passive = true` + the effect | the trigger it hooks: `FreeRefreshPerTurn` in `start_turn`, `GoldAfterUpgrade` in `Action::Level`, `GoldAfterBuyTribe` in `Action::Buy`, `GoldNextTurnAfterSell` in `sell_minion`, `SocLeftmost` through `Counters.soc_leftmost` (before any minion's Start of Combat) | Nozdormu |
| Hero power that asks for a card to discard | `effect = { discard_get_random_minion = "<tribe>" }` (`PowerEffect::DiscardGetRandomMinion`) | `press_power` opens `Player.pending_discard` with `source: None` (a power has no board slot), answered by `Action::Discard`; the reward runs in `resolve_discard`, the same path as an Activate with `discard = true` | Incubate |
| Hero power that triggers a Battlecry | `effect = { trigger_battlecry = true }` (`PowerEffect::TriggerBattlecry`) | `press_power` offers `TargetKind::TriggerBattlecry` (Kelp Keeper's path); `power_usable` needs a target, `has_battlecry_target` | Snicker-snack |
| Hero power that gives linked cards | `effect = { get_linked_minions = 2 }` (`PowerEffect::GetLinkedMinions`) | `press_power` draws with `CardDb::random_any_minion` and calls `give_linked`, so playing one discards the rest | Dark Ritual |
| Hero power that is a card effect | `spell_discount = 1`, `get_random_spell = true`, `refresh_with_spells = true` (`PowerEffect::SpellDiscount`, `GetRandomSpell`, `RefreshWithSpells`) | `press_power` writes the same player field, draws through `CardDb::random_spell` + `give_generated`, or calls `refresh_with_spells` (Saloon's Finest) | Arcane Knowledge, Blessing of the Nine Frogs, Mana Per Minute |
| Hero power that is a trinket offer | `effect = { trinket_offer = "lesser" }` (`PowerEffect::TrinketOffer(slot)`) | `trinket_start_of_turn` deals the offer at the power's `unlock_turn` into the power slot; the pick turns the power into the trinket and `power_usable` goes false | Fantastic Treasure, Growing Collection |
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
| Tier 7 draw | Greedy Conniver, Hallowed Ritual (`discover = "tier_7_minion"`) | `give_tier7_minion`, `discover_tier7_minion` |
| Tier 7 spell | Sacred Gift, Sharing is Caring, Menagerie Tableware | in `spells` like any pool spell, but every spell draw (`random_spell`, `random_spell_up_to`, `random_tier_spell`, `random_spell_of_cost`) caps at `MAX_TIER`, so none is ever offered |
| Start of Combat spell | Brood of Nozdormu, Upper Hand, Sharing is Caring | a `Player` count passed in `Counters`, read in `run_start_of_combat`, cleared after the fight |
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
4. Update `docs/ENGINE.md` "Heroes" if the frame changed; the pick offers,
   the observation (hero indices are embedded, `N_HEROES` in bgpy) and the
   bots need nothing per hero.

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
   <tribe>"). The keys
   are the effect keys of cards.toml (`get`, `get_gems`, `gem_bonus`, `cast`,
   `tavern_buff`, `undead_bonus`, ...); `crates/bgcore/src/trinkets.rs` lists
   every key in `Act` with the card it was written for. "Every N events" or
   "every N turns" is one `rule` per trinket; "twice per turn" is
   `uses_per_turn`; "at the start of every N turns" is `start_every`, which
   gates the `start_of_turn` list (a `rule` on `turns` fires at the end of the
   turn it reaches N, which is wrong for Gold, so use `start_every` there).
3. Only add a key to `Act` when no `Effect` or `SpellEffect` arm does the job,
   and then add the arm to `Game::run_trinket_act` calling the same helper the
   card path calls. A trinket's in-fight effects go in `fight`, not into
   `run_trinket_act`: a fight cannot run a shop effect. Each `fight` table is
   one `FightAct` (`crates/bgcore/src/trinkets.rs`) and one hook in
   `Side::trinket_hook` / `Side::run_fight_act` (`crates/bgcore/src/fight.rs`);
   a reward that goes home travels through `SideOutcome` and
   `apply_side_outcome`. Three slots hold at most `FIGHT_ACTS` (6) acts, which
   the parser checks. A shop action that listens to fight events is a plain
   `rule` with `event = "death"`, `"summon"` or a tribe of either.
4. Tests in `crates/bgcore/tests/trinkets.rs`: `equip` puts a trinket in a slot
   without its pick effects (`Game::set_trinket`, which also refreshes the
   player's event mask), `take` picks it through the real offer (so `on_pick`
   runs). The shared helpers there answer a prompt before passing the turn,
   because a trinket can open a Discover at the start of a turn.
5. A trinket that hands out a minion no card has yet needs a `[[token]]` in
   cards.toml (the "--- Trinket-only minions" block): stats, `golden_stats` and
   the text's mechanic as a card field, so a real card could reuse it. A token
   has no pool copy, and it must fit the lobby it is made in: if the trinket is
   neutral, the token has to be neutral too, and if the token has a tribe the
   trinket carries that tribe (otherwise
   `nothing_outside_the_lobby_ever_shows_up_in_whole_games` fails).
6. Update `docs/ENGINE.md` "Trinkets" if the frame changed. The observation
   carries the slots by trinket index (an embedding, `N_TRINKETS` in bgpy), so
   a new trinket needs nothing there.

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
  DESIGN.md records the owner's answer.

## 6. Systems the Pirate run added

- **Pirates (tribe 6).** `Tribe::EVERY` is every tribe the rules know;
  `Tribes::ALL` includes Pirate.
- **Golden minions.** Played from hand, a Golden minion adds to
  `Player.golden_played` (Extortionist, Hooktusk) and gives a Triple Reward unless
  `Minion.no_reward` (Aureate Laureate is `always_golden`, Silent Deliverer's
  minions). `TargetKind::MakeGolden` makes a board minion Golden with no reward and
  no count (Elite Navigator, Captain Sanders).
- **Golden draws from the pool.** `draw_golden` (game/shop.rs) takes up to three copies for
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
  Elemental (`tribes = [..]`).
- **The Refresh amount.** One `Player.refresh_buff` that Waveling's
  Deathrattle (fight or shop), En-Djinn Blazer and Easterly Winds add into;
  every Refresh gives all of it to one random non-spell, non-`fixed_stats`
  Tavern minion, the turn-start roll included when the Tavern changed (see
  DESIGN.md "Two kinds of Refresh"). No random number is drawn while it is
  0, so seeded games without these cards are unchanged.
- **A second player counter.** `N_COUNTERS` is 2: `Counter::Baller` is the
  count of Ballers sold. Tests that spell out `improve` arrays need two entries.
- **Elemental plays.** In `Action::Play`, right after the board insert:
  `Player.elementals_played_turn` (Conflagration, reset in `start_turn`) and
  `give_play_shield`, then, before the Battlecry, `fire_event(ElementalsPlayed)`
  (Unbound Tempest) and every `after_play_tribe` card (Wrath Weaver, Insatiable
  Ur'zul, Ultraviolet Ascendant). None counts the minion being played (DESIGN.md);
  `fire_after_play_tribe` skips its board slot, and they all run before the
  Battlecry. Only minions played from hand count.
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
  Maw Caster (`TargetKind::DestroyDiscover`), Disguised Graverobber
  (`TargetKind::DestroyCopy`: destroy, then `give_generated` the plain card, golden
  gives two, lost when the hand is full), Dead Bellringer
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
  ("Set versus add", `cards::how_many_times`). Roogug
  (`on_gem_played_on_self`) reacts only to gems cast from hand. Blood Gem
  Barrage adds to `Player.barrage` per cast; every Refresh gives it to every
  Tavern minion (`apply_barrage`), the turn-start roll included when the
  Tavern changed (see DESIGN.md "Two kinds of Refresh").
- **Triple Reward as a card.** A golden minion played from hand puts the
  Triple Reward spell in hand (`Game::give_triple_reward`), its Discover tier
  in the copy's `improvements`, fixed when given; it comes before anything the
  Battlecry adds to the hand and is lost on a full hand. Card sets without
  the spell, `Data::from_toml` refuses the card set.
- **Set versus add, one helper.** `cards::how_many_times(parts, role)`:
  the highest setter (Brann, Drakkari, Balinda, Proud Privateer; 1 if none)
  plus each adder's extra (Titus, Hot-Air Surveyor). `brann_mult`,
  `drakkari_mult`, `balinda_mult`, `bounty_mult` and the fight's
  `deathrattle_fires` all ask it.
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
  lobby section in `docs/ENGINE.md`). Every player's `Player.deity` starts 1/1;
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
  skipping Blood Gems and Triple Rewards).
- **Every-1 event triggers keep `improvements`.** `Game::fire_event` stores
  an event trigger's remainder in `Minion.improvements`; an every-1 trigger
  has none, so it leaves the field alone and an effect that improves itself
  on every fire (Harbinger Aph'lass, `Effect::BuffDeityAndImprove`) stores
  its level there.
- **One hand-insert hook.** `Game::add_to_hand` is the only path into a
  player's hand (buys, generated cards, Discover picks, Triple Rewards,
  Blood Gems, the Lockbox, `Game::give`); a full hand drops the card first,
  then `Card.on_card_added` runs (The Shadow of Doubt buffs the Deity).
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
- **Hidden numbers** (DESIGN.md): Mindbending Recruiter's Activate costs 0;
  Aph'lass +2/+1 improving +2/+1 per discard; Parasitic Fleshling +2/+2 per
  card discarded this game (`Player.discarded`); Faceless Converter +2/+1
  per Tavern spell cast this game (`Player.spells_cast`, raised by fights
  through `SideOutcome::spells_cast_gain`).
- **Tests.** `tests/aberration.rs`. A Boom-in-a-Box Start of Combat (a
  golden one deals two waves) kills known low-health Aberrations in board
  order, which scripts the four deaths an awakening needs without a random
  attacker; two different sleeper tokens make per-minion gains measurable.
