//! The shop replay (docs/real-boards.md "The shop replay"): a logged state
//! loaded into the engine field by field, a step that draws no random number
//! replayed exactly, and a Refresh, which does, judged by shape.
//!
//! The fixture is one real turn of one of Era's games, trimmed out of
//! `boards/actions/`: a Dark Discovery, the pick it offered, a minion played
//! from the hand, a Refresh, a Tavern spell bought and the end of the turn.

use std::sync::Arc;

use bgcore::shop_replay::{
    ReplayOptions, ShopFileSpec, ShopSnapshot, ShopStepSpec, StepResult, Verdict, load,
    parse_action_files, replay_step, replay_turn,
};
use bgcore::real_boards::CardIdMap;
use bgcore::{Data, Game};

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
            {"card_id": "BG36_733", "name": "Eredar Escapist"}
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
