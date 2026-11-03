//! The shop replay (docs/real-boards.md "The shop replay"): a logged state
//! loaded into the engine field by field, a step that draws no random number
//! replayed exactly, and a Refresh, which does, judged by shape.
//!
//! The fixture is one real turn of one of Era's games, trimmed out of
//! `boards/actions/`: a Dark Discovery, the pick it offered, a minion played
//! from the hand, a Refresh, a Tavern spell bought and the end of the turn.

use std::sync::Arc;

use bgcore::shop_replay::{
    LoadedShop, ReplayOptions, ShopFileSpec, ShopSnapshot, ShopStepSpec, StepResult, Verdict,
    load, parse_action_files, replay_step, replay_turn, seat_taught_spells,
};
use bgcore::real_boards::CardIdMap;
use bgcore::trinkets::CardMod;
use bgcore::{Action, Data, Game, Tribe};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

/// The card ids the fixture uses, mapped to their names the way
/// `ref/bg-cards.json` maps the game's ids (the real file is not needed: the
/// loader falls back to the name in the file).
fn ids() -> CardIdMap {
    CardIdMap::parse(
        r#"{"cards": [
            {"card_id": "TB_BaconShop_HERO_57", "name": "Nozdormu"},
            {"card_id": "TB_BaconShop_HP_063", "name": "Clairvoyance"},
            {"card_id": "BG26_135", "name": "Southsea Busker"},
            {"card_id": "BG35_951", "name": "Might of Stormwind"},
            {"card_id": "BG36_733", "name": "Eredar Escapist"},
            {"card_id": "BG28_520", "name": "Tricky Trousers", "dbf_id": 105667}
        ]}"#,
    )
    .unwrap()
}

/// The fixture folder: one real turn with a Dark Discovery, a minion played, a
/// Refresh, a Buy and the end of the turn, and one with a Sell and a level up.
fn fixtures() -> Vec<ShopFileSpec> {
    let mut files: Vec<(String, String)> = std::fs::read_dir("tests/data/actions")
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".json")))
        .map(|e| {
            (
                e.file_name().to_str().unwrap().to_string(),
                std::fs::read_to_string(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    parse_action_files(&files).unwrap()
}

/// The fixture file with a Sell in it (the second game's turn).
fn sell_file() -> ShopFileSpec {
    fixtures()
        .into_iter()
        .find(|f| f.turns.iter().any(|t| t.actions.iter().any(|s| s.step == "sell")))
        .expect("the fixture folder holds a turn with a Sell")
}

fn fixture() -> ShopFileSpec {
    fixtures()
        .into_iter()
        .find(|f| f.turns[0].actions.iter().any(|s| s.step == "buy"))
        .expect("the fixture folder holds a turn with a Buy")
}

fn step_named(turn: &bgcore::shop_replay::ShopTurnSpec, kind: &str) -> usize {
    turn.actions
        .iter()
        .position(|s| s.step == kind)
        .unwrap_or_else(|| panic!("the fixture has no {kind} step"))
}

fn replay(file: &ShopFileSpec, kind: &str) -> StepResult {
    let turn = &file.turns[0];
    let i = step_named(turn, kind);
    replay_step(&data(), &ids(), file, turn, i, &ReplayOptions { seeds: 4, seed: 1 })
}

/// A play that lands a minion in front of its own target shifts the target:
/// the file's board position is the board before the play, and the engine
/// asks after the minion has landed (log 2026_10_30_19_45_15-g2 turn 6 step
/// 18: a Sprightly Scarab played at 3 buffs the Beast that the play pushed
/// from 5 to 6).
#[test]
fn a_played_minions_board_target_moves_past_where_it_lands() {
    let text = r#"{"source": "seat", "game": 1, "turns": [
      {"turn": 8,
       "start": {"board": [
          {"id": 1, "card_id": "X1", "name": "Wolf Pup", "attack": 1, "health": 1, "position": 1},
          {"id": 2, "card_id": "X2", "name": "Alleycat", "attack": 1, "health": 1, "position": 2},
          {"id": 3, "card_id": "X3", "name": "Rockpool Hunter", "attack": 2, "health": 3, "position": 3},
          {"id": 4, "card_id": "X4", "name": "Murloc Tidehunter", "attack": 2, "health": 1, "position": 4},
          {"id": 5, "card_id": "BG36_209", "name": "Ravaging Scorpid", "attack": 6, "health": 7, "position": 5}],
        "hand": [{"id": 6, "card_id": "BG27_084", "name": "Sprightly Scarab", "index": 1, "kind": "minion"}],
        "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": []},
       "actions": [{"step": "play", "index": 1, "sub_option": 0,
          "card": {"id": 6, "card_id": "BG27_084", "name": "Sprightly Scarab"},
          "moved": {"id": 6, "to": {"where": "board", "position": 3}},
          "target": {"id": 5, "card_id": "BG36_209", "name": "Ravaging Scorpid",
                     "kind": "board", "position": 5},
          "after": {"board": [
              {"id": 1, "card_id": "X1", "name": "Wolf Pup", "attack": 1, "health": 1, "position": 1},
              {"id": 2, "card_id": "X2", "name": "Alleycat", "attack": 1, "health": 1, "position": 2},
              {"id": 6, "card_id": "BG27_084", "name": "Sprightly Scarab", "attack": 3, "health": 1, "position": 3},
              {"id": 3, "card_id": "X3", "name": "Rockpool Hunter", "attack": 2, "health": 3, "position": 4},
              {"id": 4, "card_id": "X4", "name": "Murloc Tidehunter", "attack": 2, "health": 1, "position": 5},
              {"id": 5, "card_id": "BG36_209", "name": "Ravaging Scorpid", "attack": 7, "health": 8,
               "keywords": ["reborn"], "position": 6}],
            "hand": [], "tavern": [],
            "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("shift.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert!(
        r.action.contains("Target(5)"),
        "the target followed the Beast past the landing: {:?} (verdict {:?}, {:?})",
        r.action,
        r.verdict,
        r.diffs
    );
}

/// A logged state loads into the engine and matches the file field by field:
/// the client's numbers, the board, the hand and the Tavern, the hero power,
/// the trinkets and the Secret zone.
#[test]
fn a_logged_state_loads_field_by_field() {
    let file = fixture();
    let turn = &file.turns[0];
    let data = data();
    let mut missing = Vec::new();
    let want = ShopSnapshot::of_state(&data, &turn.start, turn.turn, &mut missing);
    assert!(missing.is_empty(), "the fixture's state is all built: {missing:?}");

    let loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 1);
    assert!(loaded.missing.is_empty(), "loaded with: {:?}", loaded.missing);
    let got = ShopSnapshot::of_game(&data, &loaded.game, bgcore::shop_replay::ME);
    assert_eq!(got, want);
    assert_eq!(got.gold, turn.start.gold);
    assert_eq!(got.tier, turn.start.tier);
    assert_eq!(got.board.len(), turn.start.board.len());
    assert_eq!(got.hand.len(), turn.start.hand.len());
    assert_eq!(got.tavern.len(), turn.start.tavern.len());
    assert_eq!(got.powers.len(), turn.start.hero_powers.len());
    // The Tavern keeps the file's slot order, the spell slot included.
    let names: Vec<&str> = got.tavern.iter().map(|m| m.name.as_str()).collect();
    let want: Vec<&str> = turn
        .start
        .tavern
        .iter()
        .map(|m| m.name.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(names, want);
    // The pool holds the lobby's cards minus the ones this player has: every
    // card in the Tavern took one copy.
    let pools: usize = loaded.game.pool.iter().map(|&n| n as usize).sum();
    let held: usize = loaded
        .game
        .players[bgcore::shop_replay::ME]
        .shop
        .iter()
        .chain(loaded.game.players[bgcore::shop_replay::ME].hand.iter())
        .map(|m| m.pool_copies as usize)
        .sum();
    assert!(pools > 0);
    assert!(held <= turn.start.tavern.len(), "{held} copies for {} cards", turn.start.tavern.len());
    // The engine's own invariants (pool accounting, hero and power, the
    // Tavern's shape) hold for a loaded state.
    assert_eq!(loaded.game.check_invariants(), Ok(()));
}

/// A deterministic step replays exactly: the state the engine produces is the
/// one the log holds, for a Buy, a Sell, a minion played from the hand, the
/// pick of a Dark Discovery and the end of a turn.
#[test]
fn deterministic_steps_replay_exactly() {
    for file in fixtures() {
        for kind in ["buy", "play", "discover", "sell", "level", "end_turn"] {
            if !file.turns[0].actions.iter().any(|s| s.step == kind) {
                continue;
            }
            let r = replay(&file, kind);
            assert!(
                r.verdict == Verdict::Ok || r.verdict == Verdict::OkRandom,
                "{kind} step in {}: {:?} {:?}",
                file.file(),
                r.verdict,
                r.diffs
            );
        }
    }
    // The fixture folder really does hold the Sell the brief asks for.
    assert!(sell_file().turns[0].actions.iter().any(|s| s.step == "sell"));
}

/// The steps replay through the whole turn, as the run does: one Row per step
/// for a turn, each judged on its own.
#[test]
fn a_turn_replays_step_by_step() {
    let data = data();
    let ids = ids();
    for file in fixtures() {
        let turn = &file.turns[0];
        let out =
            replay_turn(&data, &ids, &file, turn, &ReplayOptions { seeds: 4, seed: 1 }, None);
        assert_eq!(out.len(), turn.actions.len());
        for r in &out {
            assert!(
                !matches!(r.verdict, Verdict::Flagged | Verdict::Illegal),
                "{} step {}: {:?} {:?}",
                r.kind,
                r.index,
                r.verdict,
                r.diffs
            );
            assert!(
                !r.deterministic || r.verdict == Verdict::Ok || r.verdict == Verdict::Incomplete
            );
        }
    }
}

/// A Refresh draws random cards, so it is judged by shape: the same Gold, the
/// same number of cards in each zone, and the same stat changes.
#[test]
fn a_refresh_passes_by_shape() {
    let file = fixture();
    let r = replay(&file, "roll");
    assert!(!r.deterministic, "a Refresh draws random cards");
    assert!(
        matches!(r.verdict, Verdict::OkShape | Verdict::OkRandom),
        "{:?} {:?}",
        r.verdict,
        r.diffs
    );
}

/// Galakrond's Greed's Discover lands in the Tavern slot the power emptied,
/// not in the hand: the file says where the pick went (`moved.to`, a Tavern
/// slot) and the replay seats the engine's `discover_into_shop` with it.
#[test]
fn a_pick_that_takes_a_tavern_slot_lands_there() {
    let file = fixtures()
        .into_iter()
        .find(|f| {
            f.turns[0]
                .actions
                .iter()
                .any(|s| s.moved.as_ref().and_then(|m| m.to.as_ref()).and_then(|t| t.zone()) == Some("tavern"))
        })
        .expect("a fixture whose Discover takes a Tavern slot");
    let turn = &file.turns[0];
    let i = step_named(turn, "discover");
    let step = &turn.actions[i];
    let to = step
        .moved
        .as_ref()
        .and_then(|m| m.to.as_ref())
        .expect("this Discover's pick takes a slot");
    assert_eq!(to.zone(), Some("tavern"), "the file says the Tavern slot it took");
    let picked = step.picked.as_ref().and_then(|p| p.name.clone()).unwrap_or_default();
    let r = replay(&file, "discover");
    assert_eq!(r.verdict, Verdict::Ok, "{:?}", r.diffs);
    assert!(
        r.ours.tavern.iter().any(|m| m.name == picked),
        "{picked} is in the Tavern the engine left"
    );
    assert!(
        !r.ours.hand.iter().any(|m| m.name == picked),
        "{picked} is not in the hand"
    );
    assert_eq!(r.ours.tavern.len(), r.log_after.tavern.len(), "the Tavern keeps its size");
}

/// A step that involves something the engine does not build is incomplete, not
/// flagged: the state it starts from says what was missing.
#[test]
fn an_unbuilt_card_makes_the_step_incomplete() {
    let file = fixture();
    let mut turn = file.turns[0].clone();
    // A card id no card set has: the loader seats a Stand-in and says so.
    turn.start.tavern[0].card_id = Some("ZZ_NoSuchCard".to_string());
    turn.start.tavern[0].name = Some("No Such Card".to_string());
    // The state before the first step is the turn's start, so every step of
    // the turn starts from the unbuilt card.
    for step in turn.actions.iter_mut().skip(1) {
        if let Some(after) = step.after.as_mut() {
            after.tavern[0].card_id = Some("ZZ_NoSuchCard".to_string());
            after.tavern[0].name = Some("No Such Card".to_string());
        }
    }
    let r = replay_step(
        &data(),
        &ids(),
        &file,
        &turn,
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert_eq!(r.verdict, Verdict::Incomplete, "{:?}", r.diffs);
    assert!(
        r.missing.iter().any(|m| m.contains("No Such Card")),
        "{:?}",
        r.missing
    );
}

/// The per-card tally and the three files: every step names the card it acted
/// on, and a flagged step names it in `shop-cards.tsv`.
#[test]
fn the_report_names_the_cards_of_every_step() {
    let file = fixture();
    let data = data();
    let ids = ids();
    let mut report = bgcore::shop_replay::ShopReplayReport {
        files: 1,
        turns: 1,
        ..Default::default()
    };
    report.entries = replay_turn(&data, &ids, &file, &file.turns[0], &ReplayOptions { seeds: 2, seed: 1 }, None);
    assert!(report.report_md(&data).contains("The shop replay"));
    assert_eq!(report.steps_tsv().lines().count(), report.steps() + 1);
    let cards = report.cards_tsv(&data);
    assert!(cards.starts_with("card\tname"));
    // A Buy step names the card it bought.
    let buy: Vec<&StepResult> = report.entries.iter().filter(|e| e.kind == "buy").collect();
    assert!(!buy.is_empty());
    assert!(buy[0].card.is_some());
    // `--card NAME` limits a run to the steps that involve it.
    let name = "Might of Stormwind";
    let only = replay_turn(&data, &ids, &file, &file.turns[0], &ReplayOptions { seeds: 1, seed: 1 }, Some(name));
    assert_eq!(only.len(), 1, "--card picks the one step that involves it");
    assert_eq!(only[0].kind, "buy");
    assert_eq!(only[0].card.as_deref(), Some("BG35_951"));
}

/// A game built straight from a state is a sandbox with one player, and the
/// step API works on it: this is what makes one step independent of the one
/// before it.
#[test]
fn the_loader_gives_a_one_player_game() {
    let file = fixture();
    let data = data();
    let turn = &file.turns[0];
    let loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 7);
    let game: &Game = &loaded.game;
    assert_eq!(game.current_player(), Some(bgcore::shop_replay::ME));
    assert_eq!(game.turn, turn.turn);
}

/// A file written before the new keys existed still loads: no counters, no
/// Secret zone, no `after` on a step (the step is then incomplete, never
/// flagged).
#[test]
fn an_old_file_loads_with_what_it_has() {
    let text = r#"{"source": "old", "game": 1, "turns": [
        {"turn": 1,
         "start": {"board": [], "hand": [], "tavern": [], "hand_hidden": 0,
                   "gold": 3, "tier": 1, "upgrade_cost": 5, "health": 30,
                   "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": []},
         "actions": [{"step": "roll", "paid": {"gold": 1, "health": 0}}],
         "end": null}]}"#;
    let files = parse_action_files(&[("old.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let r = replay_step(
        &data(),
        &ids(),
        file,
        &file.turns[0],
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(!r.diffs.is_empty() || !r.missing.is_empty());
}

/// A step the engine has no action for is incomplete, with the reason.
#[test]
fn a_step_the_engine_cannot_play_is_incomplete() {
    let file = fixture();
    let mut turn = file.turns[0].clone();
    let mut step = ShopStepSpec { step: "other".to_string(), ..Default::default() };
    step.after = turn.actions[0].after.clone();
    turn.actions = vec![step];
    let r = replay_step(
        &data(),
        &ids(),
        &file,
        &turn,
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.diffs[0].contains("no action for"), "{:?}", r.diffs);
}

/// A Golden minion the file marks `no_reward` is seated that way: the sources
/// docs/rules/design.md names (Silent Deliverer's Battlecry goldens, Sinister
/// Invitation's golden, the Gilding Dark Gift) give no Triple Reward when
/// played, and the extractor writes the mark for them. The loader seats it;
/// playing it here gives no reward.
#[test]
fn a_golden_the_file_marks_with_no_reward_gives_no_triple_reward() {
    let data = data();
    let mut file = fixture();
    let turn = &mut file.turns[0];
    let minion = turn
        .start
        .hand
        .iter_mut()
        .find(|m| !m.keywords.iter().any(|k| k == "undefined") && m.name.is_some())
        .expect("the fixture's hand has a minion");
    minion.golden = true;
    minion.no_reward = true;
    let name = minion.name.clone().unwrap();
    let loaded = load(&data, &turn.start, None, None, turn.turn, 1);
    let at = loaded
        .game
        .players[0]
        .hand
        .iter()
        .position(|m| m.golden && data.cards.get(m.card).name == name)
        .unwrap_or_else(|| panic!("{name} (golden) is in the loaded hand"));
    assert!(loaded.game.players[0].hand[at].no_reward, "the mark came along");
    // Without the mark the same Golden would give one.
    let mut other = file.turns[0].start.clone();
    for m in &mut other.hand {
        m.no_reward = false;
    }
    let loaded = load(&data, &other, None, None, file.turns[0].turn, 1);
    let at = loaded
        .game
        .players[0]
        .hand
        .iter()
        .position(|m| m.golden && data.cards.get(m.card).name == name)
        .expect("the same Golden, unmarked");
    assert!(!loaded.game.players[0].hand[at].no_reward);
}

/// Aureate Laureate is `always_golden`, so `CardDb::make` marks it as giving no
/// Triple Reward and the game shows it Golden in the shop. A logged state is
/// built from the file, which the collector of an older run may leave unmarked,
/// so the loader keeps the card's own mark: the replay gives no reward for it
/// either way (`Hearthstone_2026_11_02_00_10_22-g5` turn 10 step 58).
#[test]
fn a_logged_aureate_laureate_gives_no_triple_reward_whatever_the_file_says() {
    let data = data();
    let mut file = fixture();
    let turn = &mut file.turns[0];
    let slot = turn
        .start
        .hand
        .iter()
        .position(|m| m.kind.as_deref() != Some("spell"))
        .expect("the fixture's hand has a minion");
    let logged = &mut turn.start.hand[slot];
    logged.card_id = Some("BG32_236".to_string());
    logged.name = Some("Aureate Laureate".to_string());
    logged.attack = 2;
    logged.health = 2;
    logged.golden = true;
    logged.no_reward = false;
    let loaded = load(&data, &turn.start, None, None, turn.turn, 1);
    let at = loaded
        .game
        .players[0]
        .hand
        .iter()
        .position(|m| data.cards.get(m.card).name == "Aureate Laureate")
        .expect("the Aureate Laureate is in the loaded hand");
    assert!(loaded.game.players[0].hand[at].golden, "it is Golden");
    assert!(
        loaded.game.players[0].hand[at].no_reward,
        "its own card says it gives no Triple Reward"
    );
}

/// Steady Growth: "At the end of your turn, gain +X/+Y", where the pair is
/// fixed by the turn the gift landed (darkgifts.toml's table). The loader
/// used to seat the gift at the board's own turn, so every turn the replay
/// loaded grew it a step further; the turn it landed is the first turn the
/// file shows the gift on that card.
#[test]
fn steady_growth_grows_by_the_pair_of_the_turn_it_landed() {
    let text = r#"{"source": "steady", "game": 1, "turns": [
        {"turn": 3,
         "start": {"board": [{"id": 7, "card_id": "BG28_300", "name": "Harmless Bonehead",
                              "attack": 3, "health": 4, "golden": false, "keywords": [],
                              "dark_gifts": [{"card_id": "BG36_MidGameEffect_000t51",
                                              "name": "Steady Growth"}],
                              "position": 1}],
                   "hand": [], "tavern": [], "gold": 5, "tier": 3, "upgrade_cost": 6,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": [], "secrets": []},
         "actions": [], "end": null},
        {"turn": 4,
         "start": {"board": [{"id": 70, "card_id": "BG28_300", "name": "Harmless Bonehead",
                              "attack": 5, "health": 6, "golden": false, "keywords": [],
                              "dark_gifts": [{"card_id": "BG36_MidGameEffect_000t51",
                                              "name": "Steady Growth"}],
                              "position": 1}],
                   "hand": [], "tavern": [], "gold": 6, "tier": 3, "upgrade_cost": 6,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": [], "secrets": []},
         "actions": [{"step": "end_turn", "paid": {"gold": 0, "health": 0}}], "end": null}]}"#;
    let files = parse_action_files(&[("steady.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let data = data();
    let turn = &file.turns[1];
    let mut loaded = load(&data, &turn.start, None, file.lobby(), turn.turn, 1);
    assert!(loaded.missing.is_empty(), "{:?}", loaded.missing);
    let gift = data.gifts.gifts.iter().find(|g| g.id == "BG36_MidGameEffect_000t51").unwrap();
    assert_eq!(loaded.game.players[0].board[0].gift_state(gift.card), Some(3),
        "the gift landed on turn 3, the first turn the file shows it");
    // The growth is +2/+2, the pair for turn 3, not the +2/+3 of turn 4.
    loaded.game.shop_end_of_turn(0);
    let m = loaded.game.players[0].board[0];
    assert_eq!((m.attack, m.health), (7, 8), "5/6 plus turn 3's +2/+2");
}

/// Steady Growth runs as many times as the player's end of turn effects:
/// Drakkari Enchanter makes the end of your turn happen twice
/// (docs/rules/design.md "'Twice' versus 'an extra time'"), so the gift's pair lands
/// twice.
#[test]
fn steady_growth_grows_once_per_end_of_turn() {
    let text = r#"{"source": "steady", "game": 1, "turns": [
        {"turn": 4,
         "start": {"board": [{"id": 70, "card_id": "BG28_300", "name": "Harmless Bonehead",
                              "attack": 5, "health": 6, "golden": false, "keywords": [],
                              "dark_gifts": [{"card_id": "BG36_MidGameEffect_000t51",
                                              "name": "Steady Growth"}],
                              "position": 1},
                             {"id": 8, "card_id": "BG26_520", "name": "Drakkari Enchanter",
                              "attack": 1, "health": 5, "golden": false, "keywords": [],
                              "position": 2}],
                   "hand": [], "tavern": [], "gold": 6, "tier": 4, "upgrade_cost": 7,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": [], "secrets": []},
         "actions": [{"step": "end_turn", "paid": {"gold": 0, "health": 0}}], "end": null}]}"#;
    let files = parse_action_files(&[("steady.json".to_string(), text.to_string())]).unwrap();
    let data = data();
    let turn = &files[0].turns[0];
    let mut loaded = load(&data, &turn.start, None, files[0].lobby(), turn.turn, 1);
    assert!(loaded.missing.is_empty(), "{:?}", loaded.missing);
    loaded.game.shop_end_of_turn(0);
    let m = loaded.game.players[0].board[0];
    // The file's first turn is this one, so the gift landed here: +2/+3,
    // the turn 4 pair, twice.
    assert_eq!((m.attack, m.health), (9, 12), "5/6 plus +2/+3, twice");
}

/// The two power slots: a state marks its second power with `second`, and the
/// list can put the second one first (Rafaam holding Patches' power in the
/// second slot and a trinket's power in the main one: a step that loads that
/// state used to seat the second power as the main one and then fall back to
/// the hero's starting power, which made every step after it incomplete).
#[test]
fn a_second_power_is_seated_by_its_mark_not_its_place_in_the_list() {
    let text = r#"{"source": "powers", "game": 1,
        "hero": {"card_id": "TB_BaconShop_HERO_18", "name": "Patches the Pirate"},
        "turns": [
        {"turn": 9,
         "start": {"board": [], "hand": [], "tavern": [], "gold": 5, "tier": 4,
                   "upgrade_cost": 6, "health": 30, "armor": 0, "free_refreshes": 0,
                   "frozen": false, "secrets": [],
                   "hero_powers": [
                      {"card_id": "TB_BaconShop_HP_072", "name": "Pirate Parrrrty!",
                       "second": true},
                      {"card_id": "TB_BaconShop_HP_014", "name": "Stay Frosty"}],
                   "trinkets": []},
         "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("powers.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let data = data();
    let turn = &file.turns[0];
    let loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 1);
    assert!(loaded.missing.is_empty(), "{:?}", loaded.missing);
    let power = loaded.game.players[0].power.map(|p| p.of);
    let power2 = loaded.game.players[0].power2.map(|p| p.of);
    assert_eq!(
        power.map(|i| data.heroes.power(i).card_id.clone()),
        Some("TB_BaconShop_HP_014".to_string()),
        "the main slot holds the power the file leaves unmarked"
    );
    assert_eq!(
        power2.map(|i| data.heroes.power(i).card_id.clone()),
        Some("TB_BaconShop_HP_072".to_string()),
        "the marked one is the second"
    );
}

/// A logged state loaded the way the replay loads one, from a one-turn file
/// with no step in it: the board, the hand, the tier and the trinkets are the
/// state's own. The state names no hero power, so the loader seats the player
/// without a hero, which none of these tests reads.
fn seat(board: &str, hand: &str, trinkets: &str, tier: u8) -> LoadedShop {
    let text = format!(
        r#"{{"source": "seat", "game": 1, "turns": [
        {{"turn": 8,
          "start": {{"board": [{board}], "hand": [{hand}], "tavern": [],
                     "hand_hidden": 0, "gold": 10, "tier": {tier},
                     "upgrade_cost": 5, "health": 30, "armor": 0,
                     "free_refreshes": 0, "frozen": false,
                     "hero_powers": [], "trinkets": [{trinkets}]}},
          "actions": [], "end": null}}]}}"#
    );
    let files = parse_action_files(&[("seat.json".to_string(), text)]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let data = data();
    load(&data, &turn.start, None, None, turn.turn, 7)
}

/// One board minion's (attack, health) on the replay's player.
fn stats(g: &Game, at: usize) -> (i32, i32) {
    let m = g.players[bgcore::shop_replay::ME].board[at];
    (m.attack, m.health)
}

/// Lovely Locket: a trinket the loader seats still casts the same spell on
/// another friendly minion after a spell is cast on one. The replay dropped
/// its `locket` rule, so the second minion never took the spell (triage group
/// 1: every `board stats` step of the game that holds it).
#[test]
fn a_seated_lovely_locket_casts_the_spell_on_another_minion() {
    let mut loaded = seat(
        r#"{"name": "Sellemental", "attack": 2, "health": 2},
           {"name": "Sellemental", "attack": 2, "health": 2}"#,
        r#"{"name": "Repair Job", "kind": "spell"}"#,
        r#"{"card_id": "BG36_MagicItem_211", "name": "Lovely Locket"}"#,
        4,
    );
    let g = &mut loaded.game;
    let me = bgcore::shop_replay::ME;
    assert!(g.players[me].tp.locket, "the seated trinket's rule is set");
    let before = [stats(g, 0), stats(g, 1)];
    g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    g.step(Action::Target(0)).unwrap();
    let after = [stats(g, 0), stats(g, 1)];
    let first = (after[0].0 - before[0].0, after[0].1 - before[0].1);
    let second = (after[1].0 - before[1].0, after[1].1 - before[1].1);
    assert_eq!(first, (4, 8), "Repair Job's own +4/+8 lands on the minion it was cast on");
    assert_eq!(second, first, "the Locket casts the same spell again, on the other minion");
    g.check_invariants().unwrap();
}

/// Azerite Portrait: "your Living Azerites also give stats to friendly
/// Elementals" (`CardMod::AlsoBoard`). A seated portrait used to leave the
/// board's Elementals out of the Tavern buff (triage group 1: 63 steps in one
/// game, 50 of them on Wildfire Elemental).
#[test]
fn a_seated_azerite_portrait_buffs_the_board_elementals() {
    let mut loaded = seat(
        r#"{"name": "Living Azerite", "attack": 8, "health": 6},
           {"name": "Sellemental", "attack": 2, "health": 2}"#,
        r#"{"name": "Repair Job", "kind": "spell"}"#,
        r#"{"card_id": "BG30_MagicItem_431", "name": "Azerite Portrait", "greater": true}"#,
        5,
    );
    let g = &mut loaded.game;
    let me = bgcore::shop_replay::ME;
    let azerite = g.data.cards.by_name("Living Azerite").unwrap();
    assert_eq!(
        g.player_card_mod(me, azerite),
        Some(CardMod::AlsoBoard(Tribe::Elemental)),
        "the portrait's rewrite of the card is seated"
    );
    assert_eq!(g.players[me].hand.len(), 1, "only the logged spell is in hand");
    let before = stats(g, 1);
    g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    g.step(Action::Target(0)).unwrap();
    let after = stats(g, 1);
    assert_eq!(
        (after.0 - before.0, after.1 - before.1),
        (4, 3),
        "the Living Azerite's Tavern buff reaches the board's Elemental too"
    );
    g.check_invariants().unwrap();
}

/// Innkeeper's Stein: "the Tavern offers an extra minion of a higher Tier
/// whenever it is Refreshed". Without its rule a Refresh dealt one card too
/// few, which is the `Tavern count` signature of triage group 1.
#[test]
fn a_seated_innkeepers_stein_deals_one_more_card() {
    let mut loaded = seat(
        "",
        "",
        r#"{"card_id": "BG30_MagicItem_423", "name": "Innkeeper's Stein"}"#,
        4,
    );
    let g = &mut loaded.game;
    let me = bgcore::shop_replay::ME;
    assert!(g.players[me].tp.extra_high_tier, "the seated trinket's rule is set");
    let size = g.players[me].shop_size(&g.data.rules);
    g.step(Action::Roll).unwrap();
    let shop = &g.players[me].shop;
    assert_eq!(shop.len(), size, "the Tavern holds the size the rule gives it");
    assert_eq!(
        shop.len(),
        g.data.rules.shop_size(g.players[me].tier) + 1,
        "one card more than a Tavern without the Stein"
    );
    let high = shop
        .iter()
        .filter(|m| g.data.cards.get(m.card).tier == g.players[me].tier + 1)
        .count();
    assert_eq!(high, 1, "the extra card is one Tier above yours");
    g.check_invariants().unwrap();
}

/// Mindslasher Portrait: "your Vicious Mindslashers also give stats to the
/// minions beside them" (`CardMod::AlsoNeighbors`). Triage group 1: the
/// neighbours of the Mindslasher stayed a buff behind on every step.
#[test]
fn a_seated_mindslasher_portrait_buffs_the_neighbours() {
    let mut loaded = seat(
        r#"{"name": "Sellemental", "attack": 2, "health": 2},
           {"name": "Vicious Mindslasher", "attack": 1, "health": 2},
           {"name": "Sellemental", "attack": 2, "health": 2}"#,
        r#"{"name": "Repair Job", "kind": "spell"}"#,
        r#"{"card_id": "BG36_MagicItem_402", "name": "Mindslasher Portrait"}"#,
        5,
    );
    let g = &mut loaded.game;
    let me = bgcore::shop_replay::ME;
    let mindslasher = g.data.cards.by_name("Vicious Mindslasher").unwrap();
    assert_eq!(
        g.player_card_mod(me, mindslasher),
        Some(CardMod::AlsoNeighbors),
        "the portrait's rewrite of the card is seated"
    );
    let before = [stats(g, 0), stats(g, 2)];
    // The spell's target is the Mindslasher, whose own "after you cast a spell"
    // buff is the one the portrait hands to its neighbours.
    g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    g.step(Action::Target(1)).unwrap();
    let after = [stats(g, 0), stats(g, 2)];
    assert_eq!(after[0], (before[0].0 + 1, before[0].1 + 2), "the left neighbour takes it");
    assert_eq!(after[1], (before[1].0 + 1, before[1].1 + 2), "and so does the right one");
    g.check_invariants().unwrap();
}

/// A trinket that hands out a card gives none when it is seated: the pick's
/// `get` is already in the logged hand, and dealing it again would be a card
/// the real game never had.
#[test]
fn seating_a_card_granting_trinket_gives_no_card() {
    let loaded = seat(
        "",
        "",
        r#"{"card_id": "BG32_MagicItem_831", "name": "Sellemental Portrait"}"#,
        4,
    );
    let game = &loaded.game;
    let me = bgcore::shop_replay::ME;
    let i = game.data.trinkets.by_card_id("BG32_MagicItem_831").unwrap();
    assert_eq!(game.players[me].trinkets[0].of, i + 1, "the trinket is in its slot");
    assert!(
        game.players[me].hand.is_empty(),
        "the pick's Sellemental is the file's business: {:?}",
        game.players[me].hand.len()
    );
    game.check_invariants().unwrap();
}

/// A Magicfin token's taught spell is a dbf id in its first number in the
/// log, and only `ref/bg-cards.json` can turn that into a card: the loader
/// seats it before the replay runs, so the token's Battlecry casts the spell
/// the game taught it instead of nothing.
#[test]
fn a_magicfin_tokens_taught_spell_is_seated_from_its_number() {
    let text = r#"{"source": "taught", "game": 1, "turns": [
        {"turn": 9,
         "start": {"board": [{"id": 5, "card_id": "BG33_890t", "name": "Magicfin Apprentice",
                              "attack": 1, "health": 1, "golden": false, "keywords": [],
                              "numbers": [105667, 0, 0, 0], "position": 1}],
                   "hand": [], "tavern": [], "gold": 5, "tier": 4, "upgrade_cost": 6,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": [], "secrets": []},
         "actions": [], "end": null}]}"#;
    let mut files = parse_action_files(&[("taught.json".to_string(), text.to_string())]).unwrap();
    seat_taught_spells(&mut files, &ids());
    let file = &files[0];
    let data = data();
    let turn = &file.turns[0];
    let loaded = load(&data, &turn.start, None, file.lobby(), turn.turn, 1);
    assert!(loaded.missing.is_empty(), "{:?}", loaded.missing);
    let m = loaded.game.players[0].board[0];
    assert_eq!(m.taught_spell, data.cards.by_name("Tricky Trousers"));
}

/// Two of the loader's numbers are a count the engine would otherwise start
/// full: Goldenizer Supply's first number is the turns left of its "every 3
/// turns" rule (1 left is the turn it hands the Goldenizer out, the log of
/// Hearthstone_2026_10_28_22_11_50-g2 turn 8), and a Magicfin Mycologist's
/// is the teachings it has left this turn (no number at all means none).
#[test]
fn the_turns_trinket_and_the_mycologists_teachings_come_from_the_log_numbers() {
    let text = r#"{"source": "numbers", "game": 1, "turns": [
        {"turn": 8,
         "start": {"board": [{"id": 5, "card_id": "BG33_891", "name": "Magicfin Mycologist",
                              "attack": 4, "health": 8, "golden": false, "keywords": [],
                              "position": 1},
                             {"id": 6, "card_id": "BG33_891", "name": "Magicfin Mycologist",
                              "attack": 4, "health": 8, "golden": false, "keywords": [],
                              "numbers": [1, 0, 0, 0], "position": 2}],
                   "hand": [], "tavern": [], "gold": 5, "tier": 6, "upgrade_cost": 10,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "secrets": [],
                   "trinkets": [{"card_id": "BG30_MagicItem_435", "name": "Goldenizer Supply",
                                 "numbers": [1, 3, 0, 11]}]},
         "actions": [{"step": "end_turn", "paid": {"gold": 0, "health": 0}}], "end": null}]}"#;
    let files = parse_action_files(&[("numbers.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let data = data();
    let turn = &file.turns[0];
    let mut loaded = load(&data, &turn.start, None, file.lobby(), turn.turn, 1);
    assert!(loaded.missing.is_empty(), "{:?}", loaded.missing);
    assert_eq!(loaded.game.players[0].board[0].uses_left, 0, "the log shows no number: spent");
    assert_eq!(loaded.game.players[0].board[1].uses_left, 1, "one teaching left");
    // One turn left of the rule: the end of this turn hands out the Goldenizer.
    loaded.game.shop_end_of_turn(0);
    let goldenizer = data.cards.by_name("The Goldenizer").unwrap();
    let n = loaded.game.players[0].hand.iter().filter(|m| m.card == goldenizer).count();
    assert_eq!(n, 1, "the end of the turn the rule names");
}

/// A minion played into a board slot pushes the minions after it down one,
/// and the target a play step names is one of the minions that stood there
/// before the play: the engine's index must follow it, or a Battlecry hits
/// the card just played (Hearthstone_2026_10_28_02_28_32-g1 turn 10 step 2,
/// a Magicfin token's Tricky Trousers on the Drakkari Enchanter).
#[test]
fn a_played_minions_target_moves_with_the_insert() {
    let text = r#"{"source": "insert", "game": 1, "turns": [
        {"turn": 10,
         "start": {"board": [{"id": 4, "card_id": "BG26_ICC_901", "name": "Drakkari Enchanter",
                              "attack": 1, "health": 5, "golden": false, "keywords": [],
                              "position": 1}],
                   "hand": [{"id": 5, "card_id": "BG33_890t", "name": "Magicfin Apprentice",
                             "attack": 1, "health": 1, "golden": false, "keywords": [],
                             "numbers": [105667, 0, 0, 0], "index": 1, "kind": "minion"}],
                   "tavern": [], "gold": 5, "tier": 5, "upgrade_cost": 10,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": [], "secrets": []},
         "actions": [
           {"step": "play", "paid": {"gold": 0, "health": 0},
            "card": {"id": 5, "card_id": "BG33_890t", "name": "Magicfin Apprentice"},
            "moved": {"id": 5, "from": {"where": "hand", "index": 1},
                      "to": {"where": "board", "position": 1}},
            "target": {"id": 4, "card_id": "BG26_ICC_901", "name": "Drakkari Enchanter",
                       "kind": "board", "position": 1},
            "after": {"board": [{"id": 5, "card_id": "BG33_890t", "name": "Magicfin Apprentice",
                                 "attack": 1, "health": 1, "golden": false, "keywords": [],
                                 "position": 1},
                                {"id": 4, "card_id": "BG26_ICC_901", "name": "Drakkari Enchanter",
                                 "attack": 2, "health": 7, "golden": false,
                                 "keywords": ["taunt"], "position": 2}],
                      "hand": [], "tavern": [], "gold": 5, "tier": 5, "upgrade_cost": 10,
                      "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                      "hero_powers": [], "trinkets": [], "secrets": []}}],
         "end": null}]}"#;
    let mut files = parse_action_files(&[("insert.json".to_string(), text.to_string())]).unwrap();
    seat_taught_spells(&mut files, &ids());
    let file = &files[0];
    let turn = &file.turns[0];
    let result = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert_eq!(
        result.verdict,
        Verdict::Ok,
        "the Tricky Trousers land on the Drakkari: {:?} {:?}",
        result.diffs,
        result.missing
    );
    let target = &result.ours.board[1];
    assert_eq!((target.attack, target.health), (2, 7), "the Drakkari has the buff");
}

/// "The last Tavern spell you cast" (Cataclysmic Harbinger): no logged state
/// carries it, so the replay reads it from the file's own history. A Blood
/// Gem played after the Tavern spell is not one.
#[test]
fn the_last_tavern_spell_comes_from_the_files_history() {
    let text = r#"{"source": "last", "game": 1, "turns": [
        {"turn": 6,
         "start": {"board": [{"id": 3, "card_id": "BG28_300", "name": "Harmless Bonehead",
                              "attack": 3, "health": 4, "golden": false, "keywords": [],
                              "position": 1}],
                   "hand": [{"id": 4, "card_id": "BG28_520", "name": "Tricky Trousers",
                             "index": 1, "kind": "spell"}],
                   "tavern": [], "gold": 5, "tier": 5, "upgrade_cost": 10,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": [], "secrets": []},
         "actions": [
           {"step": "play", "paid": {"gold": 0, "health": 0},
            "card": {"id": 4, "card_id": "BG28_520", "name": "Tricky Trousers"},
            "moved": {"id": 4, "from": {"where": "hand", "index": 1}, "to": null},
            "target": {"id": 3, "card_id": "BG28_300", "name": "Harmless Bonehead",
                       "kind": "board", "position": 1},
            "after": {"board": [{"id": 3, "card_id": "BG28_300", "name": "Harmless Bonehead",
                                 "attack": 4, "health": 6, "golden": false,
                                 "keywords": ["taunt"], "position": 1}],
                      "hand": [], "tavern": [], "gold": 5, "tier": 5, "upgrade_cost": 10,
                      "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                      "hero_powers": [], "trinkets": [], "secrets": []}}],
         "end": null},
        {"turn": 7,
         "start": {"board": [{"id": 5, "card_id": "BG35_123", "name": "Cataclysmic Harbinger",
                              "attack": 6, "health": 10, "golden": false, "keywords": [],
                              "position": 1}],
                   "hand": [], "tavern": [], "gold": 5, "tier": 5, "upgrade_cost": 10,
                   "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                   "hero_powers": [], "trinkets": [], "secrets": []},
         "actions": [{"step": "end_turn", "paid": {"gold": 0, "health": 0},
            "after": {"board": [{"id": 5, "card_id": "BG35_123", "name": "Cataclysmic Harbinger",
                                 "attack": 6, "health": 10, "golden": false, "keywords": [],
                                 "position": 1}],
                      "hand": [{"id": 9, "card_id": "BG28_520", "name": "Tricky Trousers",
                                "index": 1, "kind": "spell"}],
                      "tavern": [], "gold": 5, "tier": 5, "upgrade_cost": 10,
                      "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                      "hero_powers": [], "trinkets": [], "secrets": []}}],
         "end": null}]}"#;
    let files = parse_action_files(&[("last.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let t6 = &file.turns[0];
    let cast = replay_step(&data(), &ids(), file, t6, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert_eq!(cast.verdict, Verdict::Ok, "the play itself: {:?}", cast.diffs);
    let t7 = &file.turns[1];
    let result = replay_step(&data(), &ids(), file, t7, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert_eq!(
        result.verdict,
        Verdict::Ok,
        "the Harbinger copies the Tricky Trousers: {:?} {:?}",
        result.diffs,
        result.missing
    );
    assert_eq!(result.ours.hand.len(), 1, "one copy in hand");
    assert_eq!(result.ours.hand[0].name, "Tricky Trousers");
}
