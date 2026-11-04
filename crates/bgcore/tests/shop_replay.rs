//! The shop replay (docs/real-boards.md "The shop replay"): a logged state
//! loaded into the engine field by field, a step that draws no random number
//! replayed exactly, and a Refresh, which does, judged by shape.
//!
//! The fixture is one real turn of one of Era's games, trimmed out of
//! `boards/actions/`: a Dark Discovery, the pick it offered, a minion played
//! from the hand, a Refresh, a Tavern spell bought and the end of the turn.

use std::sync::Arc;

use bgcore::shop_replay::{
    LoadedShop, ReplayOptions, ShopFileSpec, ShopShape, ShopSnapshot, ShopStepSpec, SnapMinion,
    StepResult, Verdict, load, parse_action_files, replay_step, replay_turn,
    seat_spell_bonus_turn, seat_taught_spells,
};
use bgcore::real_boards::CardIdMap;
use bgcore::trinkets::CardMod;
use bgcore::{Action, CardId, Data, Game, Tribe};

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

    let loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 1, None);
    assert!(loaded.missing.is_empty(), "loaded with: {:?}", loaded.missing);
    let mut got = ShopSnapshot::of_game(&data, &loaded.game, bgcore::shop_replay::ME);
    // The Fodder queue is a key the fixture predates: the engine always has
    // one (empty when nothing is queued), and a file without the key says
    // nothing about it (`None`), so it is not part of this comparison.
    got.fodder_refreshes = want.fodder_refreshes.clone();
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
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {:?}", r.diffs, r.missing);
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
    let loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 7, None);
    let game: &Game = &loaded.game;
    assert_eq!(game.current_player(), Some(bgcore::shop_replay::ME));
    assert_eq!(game.turn, turn.turn);
}

/// The warband of the opponent I last fought (`last_opponent_board`): the
/// loader seats it as the last opponent's seen board, which Bob's Burgles and
/// Time Thief copy from, and as the next opponent's warband, which is what I
/// Spy's button reads (the offers a press opens are the file's own). A state
/// without the key leaves the game with no warband to copy, and the power
/// stays unpressed, as before the key existed.
#[test]
fn the_last_opponents_warband_is_seated_for_the_powers_that_copy_it() {
    // One minion per Tavern slot (Tier 1 offers three), so a press fills the
    // Tavern from the warband alone and no card is dealt twice.
    let seated = r#", "last_opponent_board": [
        {"position": 1, "card_id": "BG31_803", "name": "Buzzing Vermin",
         "attack": 3, "health": 4, "golden": false, "keywords": []},
        {"position": 2, "card_id": "BGS_004", "name": "Wrath Weaver",
         "attack": 2, "health": 3, "golden": false, "keywords": []},
        {"position": 3, "card_id": "BG31_149", "name": "Bubble Gunner",
         "attack": 5, "health": 6, "golden": true, "keywords": ["VENOMOUS"]}]"#;
    let file_of = |key: &str| {
        let text = format!(
            r#"{{"source": "burgles", "game": 1,
            "hero": {{"card_id": "TB_BaconShop_HERO_50", "name": "Tess Greymane"}},
            "turns": [
            {{"turn": 4,
              "start": {{"board": [], "hand": [], "tavern": [
                 {{"id": 21, "card_id": "BG31_803", "name": "Buzzing Vermin", "attack": 1, "health": 1, "golden": false, "kind": "minion", "slot": 1}},
                 {{"id": 22, "card_id": "BG31_803", "name": "Buzzing Vermin", "attack": 1, "health": 1, "golden": false, "kind": "minion", "slot": 2}},
                 {{"id": 23, "card_id": "BG31_803", "name": "Buzzing Vermin", "attack": 1, "health": 1, "golden": false, "kind": "minion", "slot": 3}}],
               "hand_hidden": 0, "gold": 5, "gold_max": 5, "tier": 1,
               "upgrade_cost": 5, "health": 30, "armor": 0, "free_refreshes": 0,
               "frozen": false,
               "secrets": [],
               "hero_powers": [{{"card_id": "TB_BaconShop_HP_077", "name": "Bob's Burgles", "cost": 1, "used_this_turn": 0}}],
               "trinkets": []{key}}},
             "actions": [], "end": null}}]}}"#
        );
        parse_action_files(&[("burgles.json".to_string(), text)]).unwrap().remove(0)
    };
    let data = data();
    let with = file_of(seated);
    let loaded = load(
        &data,
        &with.turns[0].start,
        with.hero(&data, &with.turns[0].start),
        with.lobby(),
        with.turns[0].turn,
        1,
        None,
    );
    assert!(loaded.missing.is_empty(), "loaded with: {:?}", loaded.missing);
    let card = |name: &str| data.cards.by_name(name).unwrap_or_else(|| panic!("{name}"));
    let warband = [card("Buzzing Vermin"), card("Wrath Weaver"), card("Bubble Gunner")];
    let mine = &loaded.game.players[bgcore::shop_replay::ME];
    let seen: Vec<CardId> = mine.seen[1].board.iter().map(|m| m.card).collect();
    assert_eq!(seen, warband, "the last opponent's warband is the seen board");
    assert_eq!(mine.last_opponent, Some(bgcore::Opponent::Player(1)));
    let next: Vec<CardId> = mine.next_board.iter().map(|m| m.card).collect();
    assert_eq!(next, warband, "and the next opponent's warband I Spy reads");
    assert_eq!(loaded.game.check_invariants(), Ok(()));

    // Bob's Burgles copies the warband into the Tavern, made fresh.
    let mut game = loaded.game.clone();
    game.step(Action::HeroPower).expect("the press is legal with a warband");
    let mut dealt: Vec<CardId> = game.players[bgcore::shop_replay::ME]
        .shop
        .iter()
        .map(|m| m.card)
        .collect();
    dealt.sort();
    let mut want = warband.to_vec();
    want.sort();
    assert_eq!(dealt, want);
    assert!(
        game.players[bgcore::shop_replay::ME]
            .shop
            .iter()
            .all(|m| m.pool_copies == 0),
        "plain copies, made fresh"
    );
    assert_eq!(game.check_invariants(), Ok(()));

    // A state without the key: no warband to copy, and the power cannot be
    // pressed (the game's own rule: "if that board was empty").
    let without = file_of("");
    let loaded = load(
        &data,
        &without.turns[0].start,
        without.hero(&data, &without.turns[0].start),
        without.lobby(),
        without.turns[0].turn,
        1,
        None,
    );
    let mine = &loaded.game.players[bgcore::shop_replay::ME];
    assert_eq!(mine.last_opponent, None);
    assert!(mine.seen[1].board.is_empty(), "no seen board");
    assert!(mine.next_board.is_empty(), "no next warband");
    let mut game = loaded.game.clone();
    assert!(!game.is_legal(Action::HeroPower), "the press has nothing to copy");
    assert!(game.step(Action::HeroPower).is_err());
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
    let loaded = load(&data, &turn.start, None, None, turn.turn, 1, None);
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
    let loaded = load(&data, &other, None, None, file.turns[0].turn, 1, None);
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
    let loaded = load(&data, &turn.start, None, None, turn.turn, 1, None);
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
    let mut loaded = load(&data, &turn.start, None, file.lobby(), turn.turn, 1, None);
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
    let mut loaded = load(&data, &turn.start, None, files[0].lobby(), turn.turn, 1, None);
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
    let loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 1, None);
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
    load(&data, &turn.start, None, None, turn.turn, 7, None)
}

/// One board minion's (attack, health) on the replay's player.
fn stats(g: &Game, at: usize) -> (i32, i32) {
    let m = g.players[bgcore::shop_replay::ME].board[at];
    (m.attack, m.health)
}

/// The two numbers the log keeps per Volumizer and per host, and neither on
/// the player: "your Volumizers have +A/+H this game (wherever they are)" (the
/// `Volumized` enchantment, `BG34_170e`) and how many Magnetizations a minion
/// carries (one `Magnetic` enchantment per merge). The collector writes them
/// as `counters.volumizer_attack`/`volumizer_health` and as a minion's
/// `magnetizations`, and the replay seats them; a file written before the keys
/// existed carries neither and the replay starts them at 0, as it always did.
#[test]
fn the_volumizer_bonus_and_the_magnetization_count_load_into_the_player() {
    let holster = r#"{"name": "Drone Duplicator", "attack": 15, "health": 17,
                      "magnetizations": 4}"#;
    let text = format!(
        r#"{{"source": "volumizer", "game": 1, "turns": [
        {{"turn": 8,
          "start": {{"board": [{holster}], "hand": [], "tavern": [],
                     "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": 5,
                     "health": 30, "armor": 0, "free_refreshes": 0,
                     "frozen": false, "hero_powers": [], "trinkets": [],
                     "counters": {{"volumizer_attack": 24, "volumizer_health": 39}}}},
          "actions": [], "end": null}}]}}"#
    );
    let files = parse_action_files(&[("volumizer.json".to_string(), text)]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let data = data();
    let loaded = load(&data, &turn.start, None, None, turn.turn, 7, None);
    let me = bgcore::shop_replay::ME;
    assert_eq!(loaded.game.players[me].volumizer_bonus, (24, 39));
    assert_eq!(loaded.game.players[me].board[0].magnetizations, 4);
    // A file from before the collector read them says nothing about either.
    let mut bare = turn.start.clone();
    bare.counters.clear();
    bare.board[0].magnetizations = u32::default();
    let loaded = load(&data, &bare, None, None, turn.turn, 7, None);
    assert_eq!(loaded.game.players[me].volumizer_bonus, (0, 0));
    assert_eq!(loaded.game.players[me].board[0].magnetizations, 0);
}

/// The Blood Gem stats a logged card carries (`gem_attack`/`gem_health`, the
/// log's `4270`/`4271` pair) are the engine's `Minion::gem_attack`/
/// `gem_health`, and Gem Confiscation takes exactly that pair from the
/// target's neighbours onto the target. A loader that seats a minion at 0
/// steals nothing: log `2026_10_26_00_48_15-g2` turn 7 step 24 (the Sly
/// Infiltrator beside the target at 4/4 Gem stats, which the step takes).
#[test]
fn a_seated_minions_blood_gem_stats_are_what_confiscation_takes() {
    let mut loaded = seat(
        r#"{"name": "Roadboar", "attack": 10, "health": 12, "gem_attack": 4, "gem_health": 4},
           {"name": "Roadboar", "attack": 6, "health": 6},
           {"name": "Roadboar", "attack": 8, "health": 8, "gem_attack": 2, "gem_health": 2}"#,
        r#"{"name": "Gem Confiscation", "kind": "spell"}"#,
        "",
        4,
    );
    let me = bgcore::shop_replay::ME;
    let gems = |g: &Game, at: usize| {
        let m = g.players[me].board[at];
        ((m.attack, m.health), (m.gem_attack, m.gem_health))
    };
    let g = &mut loaded.game;
    assert_eq!(gems(g, 0), ((10, 12), (4, 4)), "the file's pair is the engine's");
    assert_eq!(gems(g, 1), ((6, 6), (0, 0)), "a minion the file gives none starts at 0");
    assert_eq!(gems(g, 2), ((8, 8), (2, 2)));
    g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    g.step(Action::Target(1)).unwrap();
    // 3 Gems of the player's +1/+1 land on the target, then the two
    // neighbours' pairs move onto it and are cleared off them.
    assert_eq!(gems(g, 1), ((6 + 3 + 4 + 2, 6 + 3 + 4 + 2), (3 + 4 + 2, 3 + 4 + 2)));
    assert_eq!(gems(g, 0), ((10 - 4, 12 - 4), (0, 0)));
    assert_eq!(gems(g, 2), ((8 - 2, 8 - 2), (0, 0)));
    g.check_invariants().unwrap();
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
    let loaded = load(&data, &turn.start, None, file.lobby(), turn.turn, 1, None);
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
    let mut loaded = load(&data, &turn.start, None, file.lobby(), turn.turn, 1, None);
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

/// A power's button price is the log's, not the card's own: the standing
/// change (`PowerState.cost_delta`) the power carries (Pirate Parrrrty!'s -1
/// a Pirate bought, The Galaxy's Lens's -1 a turn waited) is seated from the
/// price the file shows (Hearthstone_2026_10_27_04_30_04-g1 turn 3: a 3 Gold
/// power at 2).
#[test]
fn a_hero_powers_button_price_is_seated_from_the_log() {
    let text = r#"{"source": "price", "game": 1,
        "hero": {"card_id": "TB_BaconShop_HERO_18", "name": "Patches the Pirate"},
        "turns": [
        {"turn": 3,
         "start": {"board": [], "hand": [], "tavern": [], "gold": 2, "tier": 2,
                   "upgrade_cost": 5, "health": 30, "armor": 0, "free_refreshes": 0,
                   "frozen": false, "secrets": [],
                   "hero_powers": [{"card_id": "TB_BaconShop_HP_072",
                                    "name": "Pirate Parrrrty!", "numbers": [0, 0, 0, 0],
                                    "cost": 2, "used_this_turn": 0}],
                   "trinkets": []},
         "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("price.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let data = data();
    let turn = &file.turns[0];
    let loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 1, None);
    assert!(loaded.missing.is_empty(), "{:?}", loaded.missing);
    let me = bgcore::shop_replay::ME;
    assert_eq!(loaded.game.power_cost(me, false), 2, "the log's price");
    assert!(loaded.game.power_usable(me), "2 Gold is enough for it");
}

/// Buried Treasure's number is the digs left, and the engine counts the digs
/// done, so "(1 left!)" is the fourth dig and hands out a Golden minion
/// (Hearthstone_2026_11_02_00_10_22-g4 turn 4: a Golden Tusked Camper).
#[test]
fn buried_treasure_seats_the_digs_it_has_left() {
    let text = r#"{"source": "digs", "game": 1,
        "hero": {"card_id": "TB_BaconShop_HERO_64", "name": "Captain Eudora"},
        "turns": [
        {"turn": 4,
         "start": {"board": [], "hand": [], "tavern": [], "gold": 5, "tier": 2,
                   "upgrade_cost": 5, "health": 30, "armor": 0, "free_refreshes": 0,
                   "frozen": false, "secrets": [],
                   "hero_powers": [{"card_id": "TB_BaconShop_HP_074",
                                    "name": "Buried Treasure", "numbers": [1, 0, 0, 0],
                                    "cost": 1, "used_this_turn": 0}],
                   "trinkets": []},
         "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("digs.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let data = data();
    let turn = &file.turns[0];
    let mut loaded = load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 1, None);
    assert!(loaded.missing.is_empty(), "{:?}", loaded.missing);
    let me = bgcore::shop_replay::ME;
    assert_eq!(loaded.game.players[me].power.unwrap().progress, 3, "three digs done");
    loaded.game.step(Action::HeroPower).expect("the fourth dig");
    let dug = loaded.game.players[me].hand.last().copied().expect("a card in hand");
    assert!(dug.golden, "the fourth dig is Golden");
    assert!(loaded.game.players[me].power.unwrap().progress == 0, "the digging starts over");
}

/// "The last Tavern spell you cast" (The Galaxy's Lens) is in no logged state,
/// so the loader reads it off the steps before the press: the last play of a
/// Tavern spell in the game, earlier turns included
/// (docs/rules/heroes.md, Era 2026-11-03).
#[test]
fn the_last_tavern_spell_comes_from_the_steps_before_the_press() {
    let text = r#"{"source": "lens", "game": 1,
        "hero": {"card_id": "BG31_HERO_003", "name": "Farseer Nobundo"},
        "turns": [
        {"turn": 5,
         "start": {"board": [], "hand": [
                       {"id": 11, "index": 1, "card_id": "BG28_810", "name": "Tavern Coin",
                        "attack": 0, "health": 0, "golden": false, "keywords": []}],
                   "tavern": [], "gold": 5, "tier": 2, "upgrade_cost": 5, "health": 30,
                   "armor": 0, "free_refreshes": 0, "frozen": false, "secrets": [],
                   "hero_powers": [{"card_id": "BG31_HERO_003p",
                                    "name": "The Galaxy's Lens", "numbers": [0, 0, 0, 0],
                                    "cost": 3, "used_this_turn": 0}],
                   "trinkets": []},
         "actions": [
            {"step": "play", "index": 1,
             "card": {"id": 11, "card_id": "BG28_810", "name": "Tavern Coin",
                      "attack": 0, "health": 0, "golden": false},
             "moved": {"id": 11, "from": {"where": "hand", "index": 1}, "to": null},
             "paid": {"gold": 0, "health": 0},
             "after": {"board": [], "hand": [], "tavern": [], "gold": 6, "tier": 2,
                       "upgrade_cost": 5, "health": 30, "armor": 0, "free_refreshes": 0,
                       "frozen": false, "secrets": [],
                       "hero_powers": [{"card_id": "BG31_HERO_003p",
                                        "name": "The Galaxy's Lens", "numbers": [0, 0, 0, 0],
                                        "cost": 3, "used_this_turn": 0}],
                       "trinkets": []}},
            {"step": "hero_power",
             "paid": {"gold": 3, "health": 0},
             "after": {"board": [], "hand": [
                           {"id": 12, "index": 1, "card_id": "BG28_810", "name": "Tavern Coin",
                            "attack": 0, "health": 0, "golden": false, "keywords": []}],
                       "tavern": [], "gold": 3, "tier": 2, "upgrade_cost": 5, "health": 30,
                       "armor": 0, "free_refreshes": 0, "frozen": false, "secrets": [],
                       "hero_powers": [{"card_id": "BG31_HERO_003p",
                                        "name": "The Galaxy's Lens", "numbers": [0, 0, 0, 0],
                                        "cost": 3, "used_this_turn": 1}],
                       "trinkets": []}}
         ], "end": null}]}"#;
    let files = parse_action_files(&[("lens.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(
        &data(),
        &ids(),
        file,
        turn,
        1,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {}", r.diffs, r.action);
    assert!(r.action.contains("HeroPower"), "{}", r.action);
}

/// A minion's Battlecry or Choose One picks its target before the minion
/// lands, so the log's board place for the target is the board as it stood;
/// the engine has already put the played minion in its slot, so a pick from
/// that slot on is one place further along
/// (Hearthstone_2026_10_24_00_56_06-g1 turn 11: a Drustfallen Butcher taken
/// by Disguised Graverobber).
#[test]
fn a_played_minions_target_counts_the_card_that_landed() {
    let text = r#"{"source": "landing", "game": 1,
        "hero": {"card_id": "TB_BaconShop_HERO_57", "name": "Nozdormu"},
        "turns": [
        {"turn": 6,
         "start": {"board": [
                       {"id": 21, "position": 1, "card_id": "BG25_011",
                        "name": "Nerubian Deathswarmer", "attack": 1, "health": 4,
                        "golden": false, "keywords": []},
                       {"id": 22, "position": 2, "card_id": "BG25_011",
                        "name": "Nerubian Deathswarmer", "attack": 1, "health": 4,
                        "golden": false, "keywords": []}],
                   "hand": [
                       {"id": 31, "index": 1, "card_id": "BG28_303",
                        "name": "Disguised Graverobber", "attack": 4, "health": 4,
                        "golden": false, "keywords": []}],
                   "tavern": [], "gold": 3, "tier": 3, "upgrade_cost": 6, "health": 30,
                   "armor": 0, "free_refreshes": 0, "frozen": false, "secrets": [],
                   "hero_powers": [], "trinkets": []},
         "actions": [
            {"step": "play", "index": 1,
             "card": {"id": 31, "card_id": "BG28_303", "name": "Disguised Graverobber",
                      "attack": 4, "health": 4, "golden": false},
             "moved": {"id": 31, "from": {"where": "hand", "index": 1},
                       "to": {"where": "board", "position": 2}},
             "target": {"id": 22, "card_id": "BG25_011", "name": "Nerubian Deathswarmer",
                        "kind": "board", "position": 2},
             "paid": {"gold": 0, "health": 0},
             "after": {"board": [
                           {"id": 21, "position": 1, "card_id": "BG25_011",
                            "name": "Nerubian Deathswarmer", "attack": 1, "health": 4,
                            "golden": false, "keywords": []},
                           {"id": 31, "position": 2, "card_id": "BG28_303",
                            "name": "Disguised Graverobber", "attack": 4, "health": 4,
                            "golden": false, "keywords": []}],
                       "hand": [
                           {"id": 41, "index": 1, "card_id": "BG25_011",
                            "name": "Nerubian Deathswarmer", "attack": 1, "health": 4,
                            "golden": false, "keywords": []}],
                       "tavern": [], "gold": 3, "tier": 3, "upgrade_cost": 6, "health": 30,
                       "armor": 0, "free_refreshes": 0, "frozen": false, "secrets": [],
                       "hero_powers": [], "trinkets": []}}
         ], "end": null}]}"#;
    let files = parse_action_files(&[("landing.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(
        &data(),
        &ids(),
        file,
        turn,
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert!(r.action.contains("Target(2)"), "the pick moved with the card: {}", r.action);
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {}", r.diffs, r.action);
}

/// A chooser's power completes at the start of the turn its count runs out,
/// and the turn-start state the log writes is taken before that trigger: the
/// file still names Eastern Plaguelands with one turn left, and the step it
/// opens is the Discover that comes with the chooser's return (Galewing,
/// Hearthstone_2026_10_27_04_30_04-g2 turn 4). The state before it says the
/// same number, which is what tells the loader the count is due.
#[test]
fn a_chosen_power_that_completes_at_the_start_of_the_turn_goes_back_to_its_chooser() {
    let power = |card_id: &str, name: &str, numbers: &str| {
        format!(
            r#"{{"card_id": "{card_id}", "name": "{name}", "numbers": {numbers},
                "cost": 0, "used_this_turn": 0}}"#
        )
    };
    let state = |hand: &str, power: &str| {
        format!(
            r#"{{"board": [], "hand": [{hand}], "tavern": [], "gold": 4, "tier": 2,
                "upgrade_cost": 5, "health": 30, "armor": 0, "free_refreshes": 0,
                "frozen": false, "secrets": [], "hero_powers": [{power}], "trinkets": []}}"#
        )
    };
    let minion = r#"{"id": 51, "index": 1, "card_id": "BG36_098", "name": "Zoatroid",
                     "attack": 2, "health": 2, "golden": false, "keywords": []}"#;
    let text = format!(
        r#"{{"source": "galewing", "game": 1,
        "hero": {{"card_id": "BG20_HERO_283", "name": "Galewing"}},
        "turns": [
        {{"turn": 4,
          "start": {},
          "actions": [{{"step": "end_turn", "paid": {{"gold": 0, "health": 0}}}}],
          "end": {}}},
        {{"turn": 5,
          "start": {},
          "actions": [
             {{"step": "discover",
               "options": [{{"id": 71, "card_id": "BG36_098", "name": "Zoatroid",
                             "attack": 2, "health": 2, "golden": false}}],
               "picked": {{"id": 71, "card_id": "BG36_098", "name": "Zoatroid"}},
               "option": 0, "paid": {{"gold": 0, "health": 0}},
               "after": {}}}],
          "end": null}}]}}"#,
        state("", &power("BG20_HERO_283p_t3", "Eastern Plaguelands", "[1, 0, 0, 0]")),
        state("", &power("BG20_HERO_283p_t3", "Eastern Plaguelands", "[1, 0, 0, 0]")),
        state("", &power("BG20_HERO_283p_t3", "Eastern Plaguelands", "[1, 0, 0, 0]")),
        state(minion, &power("BG20_HERO_283p", "Dungar's Gryphon", "[0, 0, 0, 0]")),
    );
    let files = parse_action_files(&[("galewing.json".to_string(), text)]).unwrap();
    let file = &files[0];
    let r = replay_step(
        &data(),
        &ids(),
        file,
        &file.turns[1],
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert!(
        !r.diffs.iter().any(|d| d.contains("hero power")),
        "the chooser is back: {:?}",
        r.diffs
    );
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {}", r.diffs, r.action);
}

/// A prompt with one answer is no prompt: Trash for Treasure on a board of one
/// minion takes it itself and opens the Discover from the Tier below, so the
/// step's own target click is one the engine has already answered and must not
/// be sent again (Hearthstone_2026_10_24_17_59_26-g1 turns 1 to 3).
#[test]
fn a_one_answer_target_is_not_sent_again_even_with_a_discover_open() {
    let state = |board: &str| {
        format!(
            r#"{{"board": [{board}], "hand": [], "tavern": [], "gold": 2, "tier": 2,
                "upgrade_cost": 5, "health": 30, "armor": 0, "free_refreshes": 0,
                "frozen": false, "secrets": [],
                "hero_powers": [{{"card_id": "TB_BaconShop_HP_075", "name": "Trash for Treasure",
                                  "numbers": [0, 0, 0, 0], "cost": 0, "used_this_turn": 0}}],
                "trinkets": []}}"#
        )
    };
    let undead = r#"{"id": 61, "position": 1, "card_id": "BG25_011",
                     "name": "Nerubian Deathswarmer", "attack": 1, "health": 4,
                     "golden": false, "keywords": []}"#;
    let text = format!(
        r#"{{"source": "trash", "game": 1,
        "hero": {{"card_id": "TB_BaconShop_HERO_67", "name": "Captain Hooktusk"}},
        "turns": [
        {{"turn": 3,
          "start": {},
          "actions": [
             {{"step": "hero_power",
               "target": {{"id": 61, "card_id": "BG25_011",
                           "name": "Nerubian Deathswarmer", "kind": "board", "position": 1}},
               "paid": {{"gold": 0, "health": 0}},
               "after": {}}}],
          "end": null}}]}}"#,
        state(undead),
        state(""),
    );
    let files = parse_action_files(&[("trash.json".to_string(), text)]).unwrap();
    let file = &files[0];
    let r = replay_step(
        &data(),
        &ids(),
        file,
        &file.turns[0],
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert!(r.action.contains("one answer"), "{}", r.action);
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {}", r.diffs, r.action);
}

/// A played minion goes into the slot it stands behind in the state the step
/// left, not the position the log showed when it landed: a Battlecry's summons
/// come after it and a minion it kills leaves a gap before it, so the file's
/// `moved.to` is past where it went (triage group D3; with a full board it is
/// even past the board, group I5).
#[test]
fn a_played_minion_goes_where_it_stands_in_the_state_the_step_left() {
    let text = r#"{"source": "slot", "game": 1, "turns": [
      {"turn": 8,
       "start": {"board": [
          {"id": 1, "card_id": "X1", "name": "Wolf Pup", "attack": 1, "health": 1, "position": 1},
          {"id": 2, "card_id": "X2", "name": "Alleycat", "attack": 1, "health": 1, "position": 2},
          {"id": 3, "card_id": "X3", "name": "Rockpool Hunter", "attack": 2, "health": 3, "position": 3}],
        "hand": [{"id": 4, "card_id": "BG36_367", "name": "Auto Reveille",
                  "index": 1, "kind": "minion"}],
        "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": []},
       "actions": [{"step": "play", "index": 1, "sub_option": 0,
          "card": {"id": 4, "card_id": "BG36_367", "name": "Auto Reveille"},
          "moved": {"id": 4, "to": {"where": "board", "position": 4}},
          "after": {"board": [
              {"id": 1, "card_id": "X1", "name": "Wolf Pup", "attack": 1, "health": 1, "position": 1},
              {"id": 2, "card_id": "X2", "name": "Alleycat", "attack": 1, "health": 1, "position": 2},
              {"id": 4, "card_id": "BG36_367", "name": "Auto Reveille", "attack": 4, "health": 8,
               "position": 3},
              {"id": 9, "card_id": "BG36_515", "name": "Snazzy Phantom", "attack": 1, "health": 7,
               "position": 4},
              {"id": 3, "card_id": "X3", "name": "Rockpool Hunter", "attack": 2, "health": 3,
               "position": 5}],
            "hand": [], "tavern": [],
            "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("slot.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert!(
        r.action.contains("Play { hand: 0, pos: 2 }"),
        "the slot is behind the minion it stands after, not the logged position: {:?} ({:?})",
        r.action,
        r.verdict
    );
}

/// A card the game destroyed and brought back to the slot it stood in (a
/// Reborn minion) still holds that slot when a minion is played past it: the
/// step's `left` says which card was there, and its new entity id is the same
/// card (triage group D3; the Reborn Dead Bellringer of
/// `2026_10_24_15_25_26-g6` turn 9 step 47).
#[test]
fn a_reborn_card_still_holds_the_slot_it_stood_in() {
    let text = r#"{"source": "reborn", "game": 1, "turns": [
      {"turn": 9,
       "start": {"board": [
          {"id": 1, "card_id": "X1", "name": "Wolf Pup", "attack": 1, "health": 1, "position": 1},
          {"id": 2, "card_id": "X2", "name": "Alleycat", "attack": 1, "health": 1, "position": 2},
          {"id": 3, "card_id": "BG36_511", "name": "Dead Bellringer", "attack": 4, "health": 4,
           "position": 3}],
        "hand": [{"id": 4, "card_id": "BG36_367", "name": "Auto Reveille",
                  "index": 1, "kind": "minion"}],
        "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": []},
       "actions": [{"step": "play", "index": 1, "sub_option": 0,
          "card": {"id": 4, "card_id": "BG36_367", "name": "Auto Reveille"},
          "moved": {"id": 4, "to": {"where": "board", "position": 3}},
          "left": [{"id": 3, "where": "board", "card_id": "BG36_511", "name": "Dead Bellringer",
                    "position": 3}],
          "entered": [{"id": 9, "where": "board", "card_id": "BG36_511", "name": "Dead Bellringer",
                       "position": 3}],
          "after": {"board": [
              {"id": 1, "card_id": "X1", "name": "Wolf Pup", "attack": 1, "health": 1, "position": 1},
              {"id": 2, "card_id": "X2", "name": "Alleycat", "attack": 1, "health": 1, "position": 2},
              {"id": 9, "card_id": "BG36_511", "name": "Dead Bellringer", "attack": 8, "health": 8,
               "position": 3},
              {"id": 4, "card_id": "BG36_367", "name": "Auto Reveille", "attack": 4, "health": 8,
               "position": 4}],
            "hand": [], "tavern": [],
            "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("reborn.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert!(
        r.action.contains("Play { hand: 0, pos: 3 }"),
        "the played minion lands after the Reborn card that still stands at 3: {:?}",
        r.action
    );
}

/// A Magnetized minion merges into the host the file's `changed` list shows,
/// not the first card it lists: a stat static the play set off comes first
/// (triage group M1; `2026_10_28_22_11_50-g5` turn 6 step 12 merges a Red
/// Volumizer into Relentless Deflector, and Timecap'n Hooktail's +3 is the
/// first entry).
#[test]
fn a_magnetized_minion_merges_into_the_host_the_step_changed() {
    let text = r#"{"source": "magnet", "game": 1, "turns": [
      {"turn": 6,
       "start": {"board": [
          {"id": 1, "card_id": "BG27_005", "name": "Timecap'n Hooktail", "attack": 43, "health": 4,
           "position": 1},
          {"id": 2, "card_id": "BG34_405", "name": "Relentless Deflector", "attack": 13, "health": 19,
           "position": 2}],
        "hand": [{"id": 4, "card_id": "BG34_170t", "name": "Red Volumizer", "attack": 4, "health": 8,
                  "index": 1, "kind": "minion"}],
        "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": []},
       "actions": [{"step": "play", "index": 1, "sub_option": 0,
          "card": {"id": 4, "card_id": "BG34_170t", "name": "Red Volumizer",
                   "attack": 4, "health": 8},
          "moved": {"id": 4, "to": null},
          "changed": [
            {"position": 1, "card_id": "BG27_005", "name": "Timecap'n Hooktail",
             "was": {"card_id": "BG27_005", "attack": 43, "health": 4},
             "now": {"card_id": "BG27_005", "attack": 46, "health": 4}},
            {"position": 2, "card_id": "BG34_405", "name": "Relentless Deflector",
             "was": {"card_id": "BG34_405", "attack": 13, "health": 19},
             "now": {"card_id": "BG34_405", "attack": 17, "health": 27}}],
          "after": {"board": [
              {"id": 1, "card_id": "BG27_005", "name": "Timecap'n Hooktail", "attack": 46, "health": 4,
               "position": 1},
              {"id": 2, "card_id": "BG34_405", "name": "Relentless Deflector", "attack": 17, "health": 27,
               "position": 2}],
            "hand": [], "tavern": [],
            "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("magnet.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert!(
        r.action.contains("Play { hand: 0, pos: 1 }"),
        "the Volumizer merges into the minion its own stats went into: {:?} ({:?})",
        r.action,
        r.verdict
    );
}

/// "Golden minions you played this game" (Hooktusk, Master Marauder) is not a
/// number the log tags: a file the collector wrote carries it in the counters,
/// and an older one still shows it on the card that improves with it (triage
/// group D1; Hooktusk's script numbers hold the pair it gives now, which is the
/// printed +1/+1 times one more than the count).
#[test]
fn golden_minions_played_come_from_the_counters_or_from_hooktusks_own_numbers() {
    let loaded = seat(
        r#"{"name": "Hooktusk, Master Marauder", "attack": 4, "health": 4, "numbers": [6, 6, 0, 0]}"#,
        "",
        "",
        6,
    );
    assert_eq!(
        loaded.game.players[bgcore::shop_replay::ME].golden_played,
        5,
        "a Hooktusk giving +6/+6 says five Golden minions were played"
    );
    let mut with_counter = seat(
        r#"{"name": "Cord Puller", "attack": 1, "health": 1}"#,
        "",
        "",
        6,
    );
    with_counter.game.players[bgcore::shop_replay::ME].golden_played = 0;
    let text = r#"{"source": "counter", "game": 1, "turns": [
        {"turn": 8,
          "start": {"board": [{"name": "Cord Puller", "attack": 1, "health": 1}],
                     "hand": [], "tavern": [],
                     "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": 5,
                     "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                     "hero_powers": [], "trinkets": [],
                     "counters": {"golden_played": 7}},
          "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("counter.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let loaded = load(&data(), &turn.start, None, None, turn.turn, 7, None);
    assert_eq!(
        loaded.game.players[bgcore::shop_replay::ME].golden_played,
        7,
        "the collector's own count is what the loader reads first"
    );
}

/// The Fodders queued for the coming Refreshes are the file's numbers, one per
/// Refresh (`Player.fodder_refreshes`): the queue the collector reads off the
/// log's "Fodder Player Enchant". A Refresh takes the first number and leaves
/// the rest, which is what makes a Trapped Clapper's three land in three
/// Refreshes and what the replay has to carry from the state.
#[test]
fn the_queued_fodders_load_into_the_player() {
    let text = r#"{"source": "fodder", "game": 1, "turns": [
      {"turn": 8,
       "start": {"board": [], "hand": [], "tavern": [], "hand_hidden": 0,
                 "gold": 10, "tier": 5, "upgrade_cost": null, "health": 30,
                 "armor": 0, "free_refreshes": 0, "fodder_refreshes": [2, 1],
                 "frozen": false, "hero_powers": [], "trinkets": [], "secrets": []},
       "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("fodder.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let data = data();
    let loaded = load(&data, &turn.start, None, None, turn.turn, 1, None);
    let queue: Vec<u8> = loaded.game.players[bgcore::shop_replay::ME]
        .fodder_refreshes
        .iter()
        .copied()
        .collect();
    assert_eq!(queue, vec![2, 1], "the file's queue is the player's");
    // A file written before the collector read the queue says nothing about
    // it, and the player starts with none.
    let mut bare = turn.start.clone();
    bare.fodder_refreshes = None;
    let loaded = load(&data, &bare, None, None, turn.turn, 1, None);
    assert!(loaded.game.players[bgcore::shop_replay::ME].fodder_refreshes.is_empty());
}

/// A queued Fodder lands in the Refresh's Tavern and feeds itself to the one
/// Demon on the board, and its slot refills: the Demon gains the Fodder's
/// 2/2 and the Tavern's card count does not change. The queue is the state's
/// (the log's own numbers), and the Refresh takes it.
#[test]
fn a_queued_fodder_feeds_itself_on_the_refresh() {
    let demon = r#"{"id": 5, "position": 1, "card_id": "BG26_174", "name": "Soul Rewinder",
                    "attack": 4, "health": 10, "golden": false, "keywords": []}"#;
    let text = format!(
        r#"{{"source": "fodder", "game": 1, "turns": [
        {{"turn": 8,
          "start": {{"board": [{demon}], "hand": [], "tavern": [], "hand_hidden": 0,
                    "gold": 10, "tier": 5, "upgrade_cost": null, "health": 30,
                    "armor": 0, "free_refreshes": 0, "fodder_refreshes": [1],
                    "frozen": false, "hero_powers": [], "trinkets": [], "secrets": []}},
          "actions": [
             {{"step": "roll", "paid": {{"gold": 1, "health": 0}}, "after": {{}}}}],
          "end": null}}]}}"#,
    );
    let files = parse_action_files(&[("fodder-feed.json".to_string(), text)]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    // The state's snapshot is what a step is compared against; the engine's
    // own result is what this test reads.
    let r = replay_step(
        &data(),
        &ids(),
        file,
        turn,
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    let demon = &r.ours.board[0];
    assert_eq!(
        (demon.attack, demon.health),
        (6, 12),
        "the Fodder fed the one Demon: {:?}",
        r.diffs
    );
    // Tier 5 deals five minions and a spell, and the Fodder took a minion
    // slot for a moment and refilled it: the Tavern's shape is the tier's.
    assert_eq!(r.ours.tavern.len(), 6, "five minions and the spell: {:?}", r.ours.tavern);
    assert_eq!(r.ours.fodder_refreshes, Some(Vec::new()), "the Refresh took the queue");
    assert_eq!(r.ours.gold, 9, "the Refresh cost its Gold");
}

/// A Malchezaar's Health Refreshes left this turn are the log's first number:
/// the card carries "(2 left!)", "(1 left!)" and no number at all, and a fresh
/// copy would give the state both back, which makes a Refresh paid in Gold
/// look free.
#[test]
fn malchezaars_health_refreshes_left_are_the_logged_number() {
    let malchezaar = |numbers: &str| {
        format!(
            r#"{{"id": 7, "position": 1, "card_id": "BG26_524",
                 "name": "Malchezaar, Prince of Dance", "attack": 4, "health": 3,
                 "golden": false, "keywords": []{numbers}}}"#,
            numbers = if numbers.is_empty() {
                String::new()
            } else {
                format!(r#", "numbers": {numbers}"#)
            }
        )
    };
    let text = |numbers: &str| {
        format!(
            r#"{{"source": "malchezaar", "game": 1, "turns": [
            {{"turn": 8,
              "start": {{"board": [{}], "hand": [], "tavern": [], "hand_hidden": 0,
                        "gold": 3, "tier": 5, "upgrade_cost": null, "health": 30,
                        "armor": 0, "free_refreshes": 0, "frozen": false,
                        "hero_powers": [], "trinkets": []}},
              "actions": [], "end": null}}]}}"#,
            malchezaar(numbers)
        )
    };
    let data = data();
    for (numbers, left) in [("[2, 0, 0, 0]", 2), ("[1, 0, 0, 0]", 1), ("", 0)] {
        let files =
            parse_action_files(&[("malchezaar.json".to_string(), text(numbers))]).unwrap();
        let file = &files[0];
        let turn = &file.turns[0];
        let loaded = load(&data, &turn.start, None, None, turn.turn, 1, None);
        let pl = &loaded.game.players[bgcore::shop_replay::ME];
        assert_eq!(
            pl.health_refreshes_left(&data.cards),
            left,
            "the log says {left} Health Refreshes left ({numbers})"
        );
    }
}

/// A Tavern spell the log marks as paid in Health (`costs_health`, Bazaar
/// Sticker's one spell a turn) is bought for Health, not Gold: the state's own
/// card says the trinket's charge is still there, and the buy is legal with no
/// Gold at all (Hearthstone_2026_10_24_15_25_26-g4 turn 6 step 4).
#[test]
fn a_tavern_spell_priced_in_health_is_bought_for_health() {
    let text = r#"{"source": "bazaar", "game": 1, "turns": [
      {"turn": 6,
       "start": {"board": [], "hand": [], "hand_hidden": 0,
                 "tavern": [{"id": 9, "slot": 1, "card_id": "BG28_521", "name": "Hired Headhunter",
                             "kind": "spell", "cost": 3, "costs_health": true, "frozen": false}],
                 "gold": 0, "tier": 4, "upgrade_cost": null, "health": 30,
                 "armor": 0, "free_refreshes": 0, "frozen": false,
                 "hero_powers": [], "trinkets": []},
       "actions": [{"step": "buy", "slot": 1, "spell": true,
                    "target": {"id": 9, "card_id": "BG28_521", "name": "Hired Headhunter",
                               "kind": "slot", "slot": 1},
                    "paid": {"gold": 0, "health": 3},
                    "after": {"board": [], "tavern": [], "hand_hidden": 0, "gold": 0,
                              "tier": 4, "upgrade_cost": null, "health": 27,
                              "armor": 0, "free_refreshes": 0, "frozen": false,
                              "hero_powers": [], "trinkets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("bazaar.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(
        &data(),
        &ids(),
        file,
        turn,
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    assert_ne!(r.verdict, Verdict::Illegal, "{} {:?}", r.action, r.diffs);
}

/// A Tavern price the log shows on the cards themselves: Chillmere Mosaic's
/// Spellcraft deals Battlecry minions that cost (1) (docs/rules/trinkets.md),
/// which the game writes as the card's own override, and a free buy (Aranna's
/// Spectral Sight) takes the whole buy cost off every Tavern minion until it
/// is used. The state's cards say which, and the buy is one the engine can
/// pay for.
#[test]
fn a_tavern_price_the_log_shows_is_the_engines_price() {
    let tavern = |price: &str| {
        format!(
            r#"[{{"id": 21, "slot": 1, "card_id": "BG31_816", "name": "Fire Baller",
                 "attack": 4, "health": 3, "kind": "minion"{price}}},
                {{"id": 22, "slot": 2, "card_id": "BG31_815", "name": "Dune Dweller",
                 "attack": 3, "health": 3, "kind": "minion"{price}}}]"#
        )
    };
    for (price, gold, paid, left) in [
        // Chillmere Mosaic: the whole Tavern costs (1).
        (r#", "cost_override": 1"#, 1, 1, 0),
        // Aranna's free first minion of the turn.
        (r#", "cost_reduce": 3"#, 0, 0, 0),
    ] {
        let text = format!(
            r#"{{"source": "price", "game": 1, "turns": [
            {{"turn": 8,
              "start": {{"board": [], "hand": [], "tavern": {}, "hand_hidden": 0,
                        "gold": {gold}, "tier": 5, "upgrade_cost": null, "health": 30,
                        "armor": 0, "free_refreshes": 0, "frozen": false,
                        "hero_powers": [], "trinkets": [], "secrets": []}},
              "actions": [
                 {{"step": "buy", "slot": 1,
                   "target": {{"id": 21, "card_id": "BG31_816", "name": "Fire Baller",
                              "kind": "slot", "slot": 1}},
                   "paid": {{"gold": {paid}, "health": 0}},
                   "after": {{"board": [], "hand": [], "tavern": [], "hand_hidden": 0,
                             "gold": {left}, "tier": 5, "upgrade_cost": null,
                             "health": 30, "armor": 0, "free_refreshes": 0,
                             "frozen": false, "hero_powers": [], "trinkets": []}}}}],
              "end": null}}]}}"#,
            tavern(price)
        );
        let files = parse_action_files(&[("price.json".to_string(), text)]).unwrap();
        let file = &files[0];
        let turn = &file.turns[0];
        let r = replay_step(
            &data(),
            &ids(),
            file,
            turn,
            0,
            &ReplayOptions { seeds: 1, seed: 1 },
        );
        assert_ne!(r.verdict, Verdict::Illegal, "{} {:?}", r.action, r.diffs);
        assert_eq!(r.ours.gold, left, "the buy cost {paid} Gold: {:?}", r.diffs);
    }
}

/// The log's Tavern spell bonus is the total the game gives every spell, so a
/// minion's own aura is already inside it: the loader takes the engine's part
/// back out and the total stays the log's number. Evidence
/// `Hearthstone_2026_10_27_17_42_08-g1` turn 8: the counter reads 4/5 with a
/// Humon'gozz on the board, the Tavern Dish Banana gives +6/+7 (2/2 plus 4/5,
/// no +1/+2 on top), and selling the Humon'gozz drops the counter by exactly
/// 1/2 (turn 9).
#[test]
fn the_logged_tavern_spell_bonus_is_the_total_an_aura_is_already_in() {
    let text = r#"{"source": "aura", "game": 1, "turns": [
        {"turn": 8,
          "start": {"board": [{"name": "Humon'gozz", "attack": 5, "health": 5, "position": 1}],
                     "hand": [], "tavern": [],
                     "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": 5,
                     "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                     "hero_powers": [], "trinkets": [],
                     "counters": {"spell_attack": 4, "spell_health": 5}},
          "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("aura.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let loaded = load(&data(), &turn.start, None, None, turn.turn, 7, None);
    let pl = &loaded.game.players[bgcore::shop_replay::ME];
    assert_eq!(
        pl.total_spell_bonus(&loaded.game.data.cards),
        (4, 5),
        "the log's number is the whole bonus, Humon'gozz's +1/+2 included"
    );
}

/// A seated trinket's from-hand Gem bonus is not in the log's `gem_attack`
/// counter (Surveyor Portrait writes it on the Gem cards: 3/5 in the counter
/// against 9/11 given, `Hearthstone_2026_11_03_00_29_06-g2` turn 9), so the
/// loader runs that `on_pick` act while the player value stays the log's.
#[test]
fn a_seated_surveyor_portraits_gem_bonus_is_for_the_hand() {
    let text = r#"{"source": "gem", "game": 1, "turns": [
        {"turn": 9,
          "start": {"board": [{"name": "Lullabot", "attack": 1, "health": 1, "position": 1}],
                     "hand": [], "tavern": [],
                     "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": 5,
                     "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                     "hero_powers": [],
                     "trinkets": [{"card_id": "BG30_MagicItem_943", "name": "Surveyor Portrait",
                                   "numbers": [6, 6, 0, 43]}],
                     "counters": {"gem_attack": 3, "gem_health": 5}},
          "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("gem.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let loaded = load(&data(), &turn.start, None, None, turn.turn, 7, None);
    let pl = &loaded.game.players[bgcore::shop_replay::ME];
    assert_eq!(
        (pl.gem_attack, pl.gem_health),
        (3, 5),
        "the counter is the player's own Gem value"
    );
    assert_eq!(
        (pl.gem_hand_attack, pl.gem_hand_health),
        (6, 6),
        "and the Portrait's +6/+6 is read off the trinket"
    );
}

/// A minion as the comparison sees one, for the shape tests below.
fn snap(name: &str, attack: i32, health: i32) -> SnapMinion {
    SnapMinion {
        name: name.to_string(),
        golden: false,
        attack,
        health,
        keywords: Vec::new(),
        gifts: Vec::new(),
        frozen: false,
        spell: false,
    }
}

/// The change a step made, as a shape: what the sum of its buffs was, and
/// which cards they landed on, are told apart.
#[test]
fn a_buff_on_another_copy_of_the_same_card_keeps_the_shape() {
    // Before: a Wolf Pup and two Bristleback Brutes. The step sells the Wolf
    // Pup and buffs one Brute, and the two sides pick different Brutes: the
    // same set of changes, only another copy of the card.
    let before = ShopSnapshot {
        board: vec![
            snap("Wolf Pup", 1, 1),
            snap("Bristleback Brute", 10, 10),
            snap("Bristleback Brute", 5, 5),
        ],
        ..Default::default()
    };
    let ours = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 12, 12), snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    let log = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 10, 10), snap("Bristleback Brute", 7, 7)],
        ..Default::default()
    };
    let diffs = ShopShape::of(&ours, &before).diffs(&ShopShape::of(&log, &before));
    assert_eq!(diffs, Vec::<String>::new(), "one +2/+2 on a copy either way");

    // The same with a random receiver of another name: a Tea Set spread that
    // landed on another Quilboar.
    let before = ShopSnapshot {
        board: vec![snap("Bramble Tunneler", 10, 10), snap("Victorious Geomant", 3, 3)],
        ..Default::default()
    };
    let ours = ShopSnapshot {
        board: vec![snap("Bramble Tunneler", 14, 14), snap("Victorious Geomant", 3, 3)],
        ..Default::default()
    };
    let log = ShopSnapshot {
        board: vec![snap("Bramble Tunneler", 10, 10), snap("Victorious Geomant", 7, 7)],
        ..Default::default()
    };
    let diffs = ShopShape::of(&ours, &before).diffs(&ShopShape::of(&log, &before));
    assert_eq!(diffs, Vec::<String>::new(), "the same +4/+4 on another Quilboar");
}

/// The other way: a change of another size, a change one side does not have,
/// a keyword one side gave and the other did not, and a card made golden are
/// all differences, so a step whose changes only look alike is still flagged.
#[test]
fn a_buff_that_is_not_the_same_change_is_a_difference() {
    let before = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 10, 10), snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    let shape = |after: &ShopSnapshot| ShopShape::of(after, &before);
    let ours = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 12, 12), snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    // Another size.
    let log = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 14, 14), snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    assert!(!shape(&ours).diffs(&shape(&log)).is_empty(), "a +2/+2 is not a +4/+4");
    // A change that is missing: the log buffed both copies.
    let log = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 12, 12), snap("Bristleback Brute", 7, 7)],
        ..Default::default()
    };
    assert!(
        !shape(&ours).diffs(&shape(&log)).is_empty(),
        "the second copy's buff is missing"
    );
    // An extra change on our side.
    let ours = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 12, 12), snap("Bristleback Brute", 7, 7)],
        ..Default::default()
    };
    let log = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 12, 12), snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    assert!(!shape(&ours).diffs(&shape(&log)).is_empty(), "one buff too many");
    // A keyword only one side gave, at the same size.
    let ours = ShopSnapshot {
        board: vec![snap("Bristleback Brute", 12, 12), snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    let mut taunt = snap("Bristleback Brute", 12, 12);
    taunt.keywords.push("taunt".to_string());
    let log = ShopSnapshot {
        board: vec![taunt, snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    assert!(!shape(&ours).diffs(&shape(&log)).is_empty(), "the Taunt is missing");
    // A card made golden that the other side left alone.
    let mut golden = snap("Bristleback Brute", 12, 12);
    golden.golden = true;
    let log = ShopSnapshot {
        board: vec![golden, snap("Bristleback Brute", 5, 5)],
        ..Default::default()
    };
    assert!(!shape(&ours).diffs(&shape(&log)).is_empty(), "the golden mark differs");
}

/// A Tavern spell the log buys for less than its printed cost: the price on
/// the card (a reduction off the rule's) or, when the card says nothing, what
/// the step itself paid. Without the seat the engine charges the printed cost
/// and the buy is illegal when that is all the Gold there is.
#[test]
fn a_tavern_spells_price_is_the_logs_own() {
    // Enchanted Lasso costs 2; a reduction takes 1 off it.
    let reduce = r#"{"source": "spell-price", "game": 1, "turns": [
      {"turn": 6,
       "start": {"board": [], "hand": [], "hand_hidden": 0,
                 "tavern": [{"id": 9, "slot": 1, "card_id": "BG28_512", "name": "Enchanted Lasso",
                             "kind": "spell", "cost_reduce": 1, "frozen": false}],
                 "gold": 1, "tier": 4, "upgrade_cost": null, "health": 30,
                 "armor": 0, "free_refreshes": 0, "frozen": false,
                 "hero_powers": [], "trinkets": []},
       "actions": [{"step": "buy", "slot": 1, "spell": true,
                    "target": {"id": 9, "card_id": "BG28_512", "name": "Enchanted Lasso",
                               "kind": "slot", "slot": 1},
                    "paid": {"gold": 1, "health": 0},
                    "after": {"board": [], "tavern": [], "hand_hidden": 0, "gold": 0,
                              "tier": 4, "upgrade_cost": null, "health": 30,
                              "armor": 0, "free_refreshes": 0, "frozen": false,
                              "hero_powers": [], "trinkets": []}}],
       "end": null}]}"#;
    // Tavern Coin costs 1 and the log bought it for 0 with no Gold at all, and
    // nothing on the card says why (the frozen games' 8 illegal buys).
    let paid = r#"{"source": "spell-paid", "game": 1, "turns": [
      {"turn": 1,
       "start": {"board": [], "hand": [], "hand_hidden": 0,
                 "tavern": [{"id": 3, "slot": 1, "card_id": "BG28_810", "name": "Tavern Coin",
                             "kind": "spell", "frozen": false}],
                 "gold": 0, "tier": 1, "upgrade_cost": null, "health": 30,
                 "armor": 0, "free_refreshes": 0, "frozen": false,
                 "hero_powers": [], "trinkets": []},
       "actions": [{"step": "buy", "slot": 1, "spell": true,
                    "target": {"id": 3, "card_id": "BG28_810", "name": "Tavern Coin",
                               "kind": "slot", "slot": 1},
                    "paid": {"gold": 0, "health": 0},
                    "after": {"board": [], "tavern": [], "hand_hidden": 0, "gold": 0,
                              "tier": 1, "upgrade_cost": null, "health": 30,
                              "armor": 0, "free_refreshes": 0, "frozen": false,
                              "hero_powers": [], "trinkets": []}}],
       "end": null}]}"#;
    for (name, text, left) in [("reduce", reduce, 0), ("paid", paid, 0)] {
        let files = parse_action_files(&[(format!("{name}.json"), text.to_string())]).unwrap();
        let file = &files[0];
        let turn = &file.turns[0];
        let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
        assert_ne!(r.verdict, Verdict::Illegal, "{name}: {} {:?}", r.action, r.diffs);
        assert_eq!(r.ours.gold, left, "{name}: the buy left {left} Gold: {:?}", r.diffs);
    }
}

/// The log's Tavern spell bonus is the whole bonus, and the part this turn's
/// spells on a minion added (Honeycomb Ring) belongs to the bonus that is this
/// turn's only: the loader seats the standing part and moves the rest, so the
/// total stays the log's (Hearthstone_2026_11_03_15_22_36-g4 turn 10: 2/3 at
/// the turn's start, 8/9 by its 27th step).
#[test]
fn the_logged_tavern_spell_bonus_this_turn_is_seated_as_this_turns() {
    let text = r#"{"source": "honeycomb", "game": 1, "turns": [
        {"turn": 9,
          "start": {"board": [], "hand": [], "tavern": [],
                     "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": 5,
                     "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                     "hero_powers": [], "trinkets": [],
                     "counters": {"spell_attack": 2, "spell_health": 3}},
          "actions": [], "end": null},
        {"turn": 10,
          "start": {"board": [], "hand": [], "tavern": [],
                     "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": 5,
                     "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                     "hero_powers": [],
                     "trinkets": [{"card_id": "BG36_MagicItem_371", "name": "Honeycomb Ring",
                                   "greater": true, "numbers": [1, 1, 0, 43]}],
                     "counters": {"spell_attack": 8, "spell_health": 9}},
          "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("honeycomb.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[1];
    let mut loaded = load(&data(), &turn.start, None, None, turn.turn, 7, None);
    let pl = &mut loaded.game.players[bgcore::shop_replay::ME];
    let total = pl.total_spell_bonus(&loaded.game.data.cards);
    assert_eq!(total, (8, 9), "the log's whole number");
    assert_eq!(pl.tp.spell_bonus_turn, (0, 0), "nothing seated before the step");
    seat_spell_bonus_turn(pl, &turn.start, &file.turns[0].start);
    assert_eq!(pl.tp.spell_bonus_turn, (6, 6), "what this turn's spells added");
    assert_eq!(
        pl.total_spell_bonus(&loaded.game.data.cards),
        (8, 9),
        "the total is still the log's number"
    );
}

/// The Gold-plated Compass in a loaded state. Its pick never ran, but the game
/// writes the type it locked into the trinket's own first number as its race
/// code (43 is a Quilboar, 20 a Beast: `tools/export_cards.py` names the
/// codes), so the next minion of that type the player buys comes to the hand
/// Golden where it stands in the Tavern. It does that once: no logged state
/// says whether it has, so the steps before the one being replayed do. A
/// second turn's buy of the same type stays plain.
#[test]
fn a_loaded_compass_makes_the_next_buy_of_its_type_golden_once() {
    let compass = r#"{"card_id": "BG32_MagicItem_901", "name": "Gold-plated Compass",
                      "greater": true, "numbers": [43, 0, 0, 43]}"#;
    let text = format!(
        r#"{{"source": "compass", "game": 1, "turns": [
        {{"turn": 9,
          "start": {{"board": [], "hand": [],
            "tavern": [
              {{"id": 700, "slot": 1, "card_id": "BG20_101", "name": "Roadboar", "attack": 2, "health": 4, "kind": "minion", "frozen": false}},
              {{"id": 702, "slot": 2, "card_id": "BG31_816", "name": "Fire Baller", "attack": 4, "health": 3, "kind": "minion", "frozen": false}}],
            "hand_hidden": 0, "gold": 10, "gold_max": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": [{compass}], "counters": {{}},
            "secrets": [], "won_last_combat": null}},
          "actions": [
            {{"step": "buy", "slot": 1,
              "target": {{"id": 700, "card_id": "BG20_101", "name": "Roadboar", "kind": "slot", "slot": 1}},
              "card": {{"id": 700, "card_id": "BG20_101", "name": "Roadboar"}},
              "paid": {{"gold": 3, "health": 0}},
              "after": {{"board": [],
                "hand": [{{"id": 700, "index": 1, "card_id": "BG20_101_G", "name": "Roadboar",
                           "attack": 4, "health": 8, "golden": true, "kind": "minion"}}],
                "tavern": [{{"id": 702, "slot": 1, "card_id": "BG31_816", "name": "Fire Baller", "attack": 4, "health": 3, "kind": "minion", "frozen": false}}],
                "hand_hidden": 0, "gold": 7, "gold_max": 10, "tier": 6, "upgrade_cost": null,
                "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                "hero_powers": [], "trinkets": [{compass}], "counters": {{}},
                "secrets": [], "won_last_combat": null}}}}]}},
        {{"turn": 10,
          "start": {{"board": [],
            "hand": [{{"id": 700, "index": 1, "card_id": "BG20_101_G", "name": "Roadboar",
                       "attack": 4, "health": 8, "golden": true, "kind": "minion"}}],
            "tavern": [
              {{"id": 800, "slot": 1, "card_id": "BG20_101", "name": "Roadboar", "attack": 2, "health": 4, "kind": "minion", "frozen": false}},
              {{"id": 802, "slot": 2, "card_id": "BG31_816", "name": "Fire Baller", "attack": 4, "health": 3, "kind": "minion", "frozen": false}}],
            "hand_hidden": 0, "gold": 10, "gold_max": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": [{compass}], "counters": {{}},
            "secrets": [], "won_last_combat": null}},
          "actions": [
            {{"step": "buy", "slot": 1,
              "target": {{"id": 800, "card_id": "BG20_101", "name": "Roadboar", "kind": "slot", "slot": 1}},
              "card": {{"id": 800, "card_id": "BG20_101", "name": "Roadboar"}},
              "paid": {{"gold": 3, "health": 0}},
              "after": {{"board": [],
                "hand": [
                  {{"id": 700, "index": 1, "card_id": "BG20_101_G", "name": "Roadboar",
                    "attack": 4, "health": 8, "golden": true, "kind": "minion"}},
                  {{"id": 800, "index": 2, "card_id": "BG20_101", "name": "Roadboar",
                    "attack": 2, "health": 4, "golden": false, "kind": "minion"}}],
                "tavern": [{{"id": 802, "slot": 1, "card_id": "BG31_816", "name": "Fire Baller", "attack": 4, "health": 3, "kind": "minion", "frozen": false}}],
                "hand_hidden": 0, "gold": 7, "gold_max": 10, "tier": 6, "upgrade_cost": null,
                "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                "hero_powers": [], "trinkets": [{compass}], "counters": {{}},
                "secrets": [], "won_last_combat": null}}}}]}}]}}"#
    );
    let files = parse_action_files(&[("compass.json".to_string(), text)]).unwrap();
    let file = &files[0];
    let opts = ReplayOptions { seeds: 1, seed: 1 };
    // The first buy: the type the Compass locked in the log (a Quilboar) is
    // still armed, so the bought Roadboar is Golden.
    let first = replay_step(&data(), &ids(), file, &file.turns[0], 0, &opts);
    assert_eq!(first.verdict, Verdict::Ok, "{:?} {:?}", first.diffs, first.missing);
    assert!(
        first.ours.hand.iter().any(|m| m.name == "Roadboar" && m.golden),
        "the bought Roadboar is Golden: {:?}",
        first.ours.hand
    );
    // The second turn: the file shows the payout behind it, so the trinket's
    // type is spent and the next Roadboar stays plain.
    let second = replay_step(&data(), &ids(), file, &file.turns[1], 0, &opts);
    assert_eq!(second.verdict, Verdict::Ok, "{:?} {:?}", second.diffs, second.missing);
    assert!(
        second.ours.hand.iter().any(|m| m.name == "Roadboar" && !m.golden),
        "the type is spent, so the Roadboar stays plain: {:?}",
        second.ours.hand
    );
}

/// Gone Fishing's button counts the sells left ("5" is none since the last
/// Murloc, then 4, 3, 2, 1 and a Murloc at 0), and the engine counts the sells
/// done: the loader turns one into the other. Without it every step would
/// start the count at 0 again and the power could never fire inside a step,
/// which is what `Hearthstone_2026_10_28_22_11_50-g4` turn 12 shows (four
/// Murlocs the log got and the engine did not).
#[test]
fn gone_fishing_counts_the_sells_that_are_left_as_sells_done() {
    let file_of = |numbers: &str| {
        let text = format!(
            r#"{{"source": "flurgl", "game": 1,
            "hero": {{"card_id": "TB_BaconShop_HERO_55", "name": "Fungalmancer Flurgl"}},
            "turns": [
            {{"turn": 8,
              "start": {{"board": [
                 {{"id": 11, "position": 1, "card_id": "BGS_004", "name": "Wrath Weaver",
                   "attack": 2, "health": 3, "golden": false, "keywords": []}}],
               "hand": [], "tavern": [], "hand_hidden": 0, "gold": 10, "tier": 3,
               "upgrade_cost": 5, "health": 30, "armor": 0, "free_refreshes": 0,
               "frozen": false,
               "hero_powers": [{{"card_id": "TB_BaconShop_HP_056", "name": "Gone Fishing",
                                 "cost": 0, "numbers": {numbers}}}],
               "trinkets": []}},
             "actions": [], "end": null}}]}}"#
        );
        parse_action_files(&[("flurgl.json".to_string(), text)]).unwrap().remove(0)
    };
    let data = data();
    let me = bgcore::shop_replay::ME;
    let load_of = |file: &ShopFileSpec| {
        let turn = &file.turns[0];
        load(&data, &turn.start, file.hero(&data, &turn.start), file.lobby(), turn.turn, 1, None)
    };
    // Three of five sells left is two done.
    let loaded = load_of(&file_of("[3, 5, 0, 0]"));
    let pl = &loaded.game.players[me];
    assert_eq!(pl.power.map(|p| p.progress), Some(2), "two of the five sells are done");
    // One left: this sell is the fifth, and the Murloc comes.
    let file = file_of("[1, 5, 0, 0]");
    let loaded = load_of(&file);
    let mut game = loaded.game.clone();
    let before = game.players[me].hand.len();
    game.step(Action::Sell(0)).expect("the sell is legal");
    let pl = &game.players[me];
    assert_eq!(pl.board.len(), 0, "the minion is sold");
    assert_eq!(pl.hand.len(), before + 1, "the fifth sell gets a random Murloc");
    assert!(data.cards.get(pl.hand[before].card).tribes.has(Tribe::Murloc), "a Murloc");
    assert_eq!(game.check_invariants(), Ok(()));
    // Five left (a fresh count) is a sell with no Murloc.
    let file = file_of("[5, 5, 0, 0]");
    let loaded = load_of(&file);
    let mut game = loaded.game.clone();
    let before = game.players[me].hand.len();
    game.step(Action::Sell(0)).expect("the sell is legal");
    assert_eq!(game.players[me].hand.len(), before, "the first sell of five gets nothing");
}

/// The pair a source hands out is a linked pair: playing one discards the
/// other (docs/rules/design.md "Linked cards"). A sold Faceless Operative gives
/// two random Aberrations, and the state says only which card each came from,
/// so the loader links two hand cards with one `from`
/// (`Hearthstone_2026_10_24_15_25_26-g2` turn 9: the Abyssal Envoy played and
/// the Unwilling Slacker discarded with it, which the log shows and the replay
/// did not).
#[test]
fn the_two_cards_one_source_hands_out_are_linked() {
    let text = |hand: &str| {
        format!(
            r#"{{"source": "linked", "game": 1, "turns": [
            {{"turn": 5,
              "start": {{"board": [], "hand": [{hand}], "tavern": [],
                "hand_hidden": 0, "gold": 10, "tier": 4, "upgrade_cost": 5,
                "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
                "hero_powers": [], "trinkets": []}},
              "actions": [], "end": null}}]}}"#
        )
    };
    let pair = r#"
        {"id": 11, "index": 1, "card_id": "BG36_311", "name": "Abyssal Envoy",
         "attack": 2, "health": 2, "golden": false, "keywords": [], "kind": "minion",
         "from": {"card_id": "BG36_308", "name": "Faceless Operative"}},
        {"id": 12, "index": 2, "card_id": "BG36_101", "name": "Unwilling Slacker",
         "attack": 3, "health": 2, "golden": false, "keywords": [], "kind": "minion",
         "from": {"card_id": "BG36_308", "name": "Faceless Operative"}}"#;
    let files = parse_action_files(&[("linked.json".to_string(), text(pair))]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let data = data();
    let loaded = load(&data, &turn.start, None, None, turn.turn, 1, None);
    let me = bgcore::shop_replay::ME;
    let mut game = loaded.game.clone();
    assert_eq!(game.players[me].hand.len(), 2);
    game.step(Action::Play { hand: 0, pos: 0 }).expect("the Envoy is played");
    assert_eq!(game.players[me].board.len(), 1, "the Envoy is on the board");
    assert!(game.players[me].hand.is_empty(), "playing one discards the other");

    // One card from that source on its own is no pair, and playing it leaves
    // the rest of the hand alone.
    let lone = r#"
        {"id": 11, "index": 1, "card_id": "BG36_311", "name": "Abyssal Envoy",
         "attack": 2, "health": 2, "golden": false, "keywords": [], "kind": "minion",
         "from": {"card_id": "BG36_308", "name": "Faceless Operative"}},
        {"id": 12, "index": 2, "card_id": "BG36_101", "name": "Unwilling Slacker",
         "attack": 3, "health": 2, "golden": false, "keywords": [], "kind": "minion"}"#;
    let files = parse_action_files(&[("linked.json".to_string(), text(lone))]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let loaded = load(&data, &turn.start, None, None, turn.turn, 1, None);
    let mut game = loaded.game.clone();
    game.step(Action::Play { hand: 0, pos: 0 }).expect("the Envoy is played");
    assert_eq!(game.players[me].hand.len(), 1, "an unlinked card stays");
}

/// A card a trinket hands out at the start of the turn that a full hand
/// refused waits for room and joins at the first step that frees a slot
/// (docs/rules/design.md "A full hand"). The state cannot show a waiting card,
/// so the step that frees the slot would never see it: the loader queues it
/// again from the trinket the state does show. The Lockbox Portrait's Lockbox
/// in `Hearthstone_2026_10_28_22_11_50-g6` turn 8 and the Token of the Old
/// Gods in `Hearthstone_2026_10_29_22_20_04-g3` turns 10 and 11 arrived like
/// that.
#[test]
fn a_start_of_turn_card_a_full_hand_refused_joins_when_a_slot_frees() {
    let gem = |i: usize| {
        format!(
            r#"{{"id": {i}, "index": {i}, "card_id": "BG20_GEM", "name": "Blood Gem",
             "attack": 0, "health": 0, "golden": false, "keywords": [],
             "numbers": [1, 1, 0, 0], "kind": "spell"}}"#
        )
    };
    let gems: Vec<String> = (1..=9).map(gem).collect();
    let hand = gems.join(",");
    let lockbox = r#"{"id": 100, "index": 10, "card_id": "BG36_520t", "name": "Lockbox",
                      "attack": 0, "health": 0, "golden": false, "keywords": [], "kind": "spell"}"#;
    let module = r#"{"id": 10, "index": 10, "card_id": "BG_BOT_911",
                     "name": "Annoy-o-Module", "attack": 2, "health": 4,
                     "golden": false, "keywords": ["TAUNT", "DIVINE_SHIELD"],
                     "kind": "minion"}"#;
    let on_board = r#"{"id": 10, "position": 1, "card_id": "BG_BOT_911",
                       "name": "Annoy-o-Module", "attack": 2, "health": 4,
                       "golden": false, "keywords": ["TAUNT", "DIVINE_SHIELD"],
                       "kind": "minion"}"#;
    let trinket = r#"{"card_id": "BG36_MagicItem_301", "name": "Lockbox Portrait",
                      "greater": false, "numbers": [2, 0, 0, 126]}"#;
    let state = |hand: &str, board: &str| {
        format!(
            r#"{{"board": [{board}], "hand": [{hand}], "tavern": [],
             "hand_hidden": 0, "gold": 10, "tier": 4, "upgrade_cost": 5,
             "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
             "hero_powers": [], "trinkets": [{trinket}], "secrets": []}}"#
        )
    };
    let text = format!(
        r#"{{"source": "lockbox", "game": 1, "turns": [
        {{"turn": 8,
          "start": {},
          "actions": [{{"step": "play", "paid": {{"gold": 0, "health": 0}},
            "index": 10,
            "card": {{"id": 10, "index": 10, "card_id": "BG_BOT_911", "name": "Annoy-o-Module"}},
            "moved": {{"id": 10, "from": {{"where": "hand", "index": 10}},
                      "to": {{"where": "board", "position": 1}}}},
            "after": {}}}],
          "end": null}}]}}"#,
        state(&format!("{hand},{module}"), ""),
        state(&format!("{hand},{lockbox}"), on_board),
    );
    let files = parse_action_files(&[("lockbox.json".to_string(), text.clone())])
        .unwrap_or_else(|e| panic!("{e}\n{text}"));
    let file = &files[0];
    let r = replay_step(
        &data(),
        &ids(),
        file,
        &file.turns[0],
        0,
        &ReplayOptions { seeds: 1, seed: 1 },
    );
    let names: Vec<&str> = r.ours.hand.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"Lockbox"), "the queued Lockbox joins: {names:?}");
}

/// "Sacrificial Wrathguard: Deathrattle: Give minions in the Tavern +2/+2 this
/// game. Activate ({4}): Improve this." The log's numbers are the buff as it
/// stands and the part the Activates added, so the second pair is the count
/// the Deathrattle scales with (log 2026_11_01_04_27_54-g2 turn 11 step 21:
/// +72/+72, where ours used to give +8/+8).
#[test]
fn a_wrathguards_activate_count_is_seated() {
    let text = r#"{"source": "seat", "game": 1, "turns": [
      {"turn": 11,
       "start": {"board": [
          {"id": 1, "card_id": "BG36_362", "name": "Sacrificial Wrathguard",
           "attack": 5, "health": 3, "position": 1, "numbers": [4, 4, 2, 2]}],
        "hand": [], "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": []},
       "actions": [], "end": null}]}"#;
    let files = parse_action_files(&[("wrathguard.json".to_string(), text.to_string())]).unwrap();
    let turn = &files[0].turns[0];
    let loaded = load(&data(), &turn.start, None, None, turn.turn, 1, None);
    let m = loaded.game.players[bgcore::shop_replay::ME].board[0];
    assert_eq!(m.improvements, 1, "one Activate behind a +4/+4 Tavern buff");
}

/// A play whose minion the game rebuilds under a new entity id still lands in
/// its slot: the file's hand id and its board id differ (log
/// 2026_10_24_15_25_26-g6 turn 12 step 29, Scarlet Skull: 18629 in hand,
/// 18667 on the board).
#[test]
fn a_played_card_the_game_rebuilds_keeps_its_slot() {
    let text = r#"{"source": "seat", "game": 1, "turns": [
      {"turn": 12,
       "start": {"board": [
          {"id": 1, "card_id": "BG25_010", "name": "Handless Forsaken", "attack": 2, "health": 1, "position": 1},
          {"id": 2, "card_id": "BG25_001", "name": "Risen Rider", "attack": 2, "health": 1, "position": 2}],
        "hand": [{"id": 3, "card_id": "BG25_001", "name": "Risen Rider", "index": 1,
                  "kind": "minion", "attack": 2, "health": 1, "keywords": ["REBORN"]}],
        "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": [], "secrets": []},
       "actions": [{"step": "play", "index": 1,
          "card": {"id": 3, "card_id": "BG25_001", "name": "Risen Rider", "attack": 2, "health": 1},
          "moved": {"id": 3, "from": {"where": "hand", "index": 1}, "to": null},
          "after": {"board": [
              {"id": 1, "card_id": "BG25_010", "name": "Handless Forsaken", "attack": 2, "health": 1, "position": 1},
              {"id": 4, "card_id": "BG25_001", "name": "Risen Rider", "attack": 2, "health": 1,
               "keywords": ["REBORN"], "position": 2},
              {"id": 2, "card_id": "BG25_001", "name": "Risen Rider", "attack": 2, "health": 1, "position": 3}],
            "hand": [], "tavern": [],
            "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": [], "secrets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("rebuilt.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert!(
        r.action.contains("Play { hand: 0, pos: 1 }"),
        "the new entity id still names the played card: {:?} (verdict {:?}, {:?})",
        r.action,
        r.verdict,
        r.diffs
    );
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {:?}", r.diffs, r.missing);
}

/// A state can hold three plain copies a summon left on the board while the
/// game's own triple check waits for a Discover to be answered: the step that
/// answers it is where the Golden card joins the hand (log
/// 2026_10_24_15_25_26-g6 turn 8 steps 14 and 16).
#[test]
fn a_step_whose_states_show_the_triple_check_runs_it() {
    let text = r#"{"source": "seat", "game": 1, "turns": [
      {"turn": 8,
       "start": {"board": [
          {"id": 2, "card_id": "BG_ICC_026t", "name": "Skeleton", "attack": 1, "health": 1, "position": 1},
          {"id": 3, "card_id": "BG_ICC_026t", "name": "Skeleton", "attack": 1, "health": 1, "position": 2},
          {"id": 4, "card_id": "BG_ICC_026t", "name": "Skeleton", "attack": 1, "health": 1, "position": 3}],
        "hand": [], "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": [], "secrets": []},
       "actions": [{"step": "discover",
          "options": [
            {"id": 7, "card_id": "BG25_001", "name": "Risen Rider", "attack": 2, "health": 1},
            {"id": 8, "card_id": "BG25_010", "name": "Handless Forsaken", "attack": 2, "health": 1}],
          "picked": {"id": 7, "card_id": "BG25_001", "name": "Risen Rider", "attack": 2, "health": 1,
                     "keywords": ["REBORN", "TAUNT"]},
          "after": {"board": [],
            "hand": [
              {"id": 7, "card_id": "BG25_001", "name": "Risen Rider", "index": 1, "attack": 2,
               "health": 1, "keywords": ["REBORN", "TAUNT"]},
              {"id": 9, "card_id": "BG_ICC_026t_G", "name": "Skeleton", "index": 2, "golden": true,
               "attack": 2, "health": 2}],
            "tavern": [],
            "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": [], "secrets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("triple-due.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {:?}", r.diffs, r.missing);
}

/// A pick that ends a spell the player cast is logged as its own step, so the
/// game's own action is split in two: the pick step loads a state where
/// nothing of the cast is owed any more, and the loader seats it again. Its
/// "after you cast a spell" triggers land at the pick, inside the cast (log
/// 2026_10_24_17_59_26 turn 10 step 43 to 44: Fandral's Fortune's pick and
/// the Felboar's consume, and 2026_10_27_04_30_04 turn 7 step 1 to 2).
#[test]
fn a_pick_that_ends_a_spell_carries_what_the_cast_still_owed() {
    let text = r#"{"source": "seat", "game": 1, "turns": [
      {"turn": 8,
       "start": {"board": [
          {"id": 1, "card_id": "BG34_692", "name": "Forsaken Weaver", "attack": 3, "health": 8, "position": 1},
          {"id": 2, "card_id": "BG31_813", "name": "Cadaver Caretaker", "attack": 2, "health": 4, "position": 2}],
        "hand": [], "tavern": [],
        "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
        "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
        "hero_powers": [], "trinkets": [], "secrets": []},
       "actions": [{"step": "discover", "kind": "discover",
          "source": {"id": 9, "card_id": "BG33_101", "name": "A New Sprout"},
          "options": [
            {"id": 7, "card_id": "BG36_207", "name": "Wolf Pup", "attack": 1, "health": 1},
            {"id": 8, "card_id": "BG_ICC_026t", "name": "Skeleton", "attack": 1, "health": 1}],
          "picked": {"id": 7, "card_id": "BG36_207", "name": "Wolf Pup", "attack": 1, "health": 1},
          "after": {"board": [
              {"id": 1, "card_id": "BG34_692", "name": "Forsaken Weaver", "attack": 6, "health": 8, "position": 1},
              {"id": 2, "card_id": "BG31_813", "name": "Cadaver Caretaker", "attack": 5, "health": 4, "position": 2}],
            "hand": [
              {"id": 7, "card_id": "BG36_207", "name": "Wolf Pup", "index": 1, "attack": 1, "health": 1}],
            "tavern": [],
            "hand_hidden": 0, "gold": 10, "tier": 6, "upgrade_cost": null,
            "health": 30, "armor": 0, "free_refreshes": 0, "frozen": false,
            "hero_powers": [], "trinkets": [], "secrets": []}}],
       "end": null}]}"#;
    let files = parse_action_files(&[("pick-owed.json".to_string(), text.to_string())]).unwrap();
    let file = &files[0];
    let turn = &file.turns[0];
    let r = replay_step(&data(), &ids(), file, turn, 0, &ReplayOptions { seeds: 1, seed: 1 });
    assert_eq!(r.verdict, Verdict::Ok, "{:?} {:?}", r.diffs, r.missing);
}
