//! Era starts (`docs/briefs/era-starts.md`): a game begins on Era's logged
//! turn-start position, with the exact board, hand, Gold, tier, Health, hero,
//! trinkets and counters, exact pool accounting, a source-game split, and a
//! deterministic choice.

use std::sync::Arc;

use bgcore::action::Action;
use bgcore::era_start::{EraSplit, EraStartConfig, EraStartPlan, EraStarts};
use bgcore::real_boards::{CardIdMap, RealBoards, parse_board_files};
use bgcore::shop_replay::{self, ShopFileSpec};
use bgcore::{Data, Game, PLAYERS, Rng};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn ids() -> CardIdMap {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let text = std::fs::read_to_string(format!("{root}ref/bg-cards.json"))
        .expect("the card id mapping");
    CardIdMap::parse(&text).expect("the card id mapping parses")
}

fn files_of(dir: &str) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("the folder")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".json")))
        .map(|e| {
            (
                e.file_name().to_str().unwrap().to_string(),
                std::fs::read_to_string(e.path()).expect("the file"),
            )
        })
        .collect();
    files.sort();
    files
}

fn specs(files: &[(String, String)], ids: &CardIdMap) -> Vec<ShopFileSpec> {
    let mut specs = shop_replay::parse_action_files(files).expect("the action files parse");
    shop_replay::seat_taught_spells(&mut specs, ids);
    specs
}

fn fixture_specs() -> (Vec<ShopFileSpec>, CardIdMap) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let files = files_of(&format!("{root}crates/bgcore/tests/data/actions"));
    let ids = ids();
    (specs(&files, &ids), ids)
}

fn fixture_boards() -> Arc<RealBoards> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let dir = format!("{root}crates/bgcore/tests/data/real-boards");
    let files = files_of(&dir);
    let ids = ids();
    let (specs, unknown) = parse_board_files(&files, &ids).expect("the fixture boards");
    let (boards, unmatched) = RealBoards::build(&specs, &data().cards, &data().gifts);
    assert!(unmatched.is_empty(), "unmatched: {unmatched:?}");
    let _ = unknown;
    Arc::new(boards)
}

/// The fixture specs for one turn, with Era in seat 0 and every other seat
/// real, so the Era seat is the only one left to act.
fn one_turn_specs(turn: u32) -> (Vec<ShopFileSpec>, CardIdMap, Arc<RealBoards>) {
    let (specs, ids) = fixture_specs();
    let specs: Vec<_> = specs
        .into_iter()
        .filter(|f| f.turns.iter().any(|t| t.turn == turn))
        .collect();
    assert!(!specs.is_empty(), "a fixture game has turn {turn}");
    (specs, ids, fixture_boards())
}

fn era_game(
    specs: &[ShopFileSpec],
    ids: &CardIdMap,
    boards: Arc<RealBoards>,
    turn_min: u32,
    turn_max: u32,
    turn: u32,
) -> (Game, EraStarts, usize) {
    let d = data();
    let (starts, missing) = EraStarts::build(specs, &d, ids, turn_min, turn_max);
    assert!(missing.is_empty(), "seat misses: {missing:?}");
    let position = (0..starts.len())
        .filter(|&i| starts.position(i).turn == turn)
        .max_by_key(|&i| starts.position(i).player.board.len())
        .expect("a position on that turn");
    let real: Vec<usize> = (1..PLAYERS).collect();
    let plan = EraStartPlan { position, seat: 0 };
    (Game::new_era_start(d, 12345, &real, boards, &starts, &plan, None), starts, position)
}

#[test]
fn era_start_equals_the_logged_turn_start() {
    let (specs, ids, boards) = one_turn_specs(3);
    let (game, starts, position) = era_game(&specs, &ids, boards, 3, 7, 3);
    let pos = starts.position(position);
    let file = specs
        .iter()
        .find(|f| f.source == pos.source && f.game == pos.game)
        .expect("the position's file");
    let turn = file.turns.iter().find(|t| t.turn == pos.turn).expect("the turn");
    let (source, missing) = shop_replay::turn_start_game(&data(), &ids, file, turn, 1)
        .expect("the logged turn seats");
    assert!(missing.is_empty(), "the fixture has no misses: {missing:?}");
    let mut source = source;
    for m in source.players[0].board.iter_mut().chain(source.players[0].hand.iter_mut()) {
        m.pool_copies = 0;
    }

    assert_eq!(game.turn, pos.turn, "the game starts on the position's turn");
    assert_eq!(game.lobby, pos.lobby.expect("the file's lobby"));
    let (era, src) = (&game.players[0], &source.players[0]);
    assert_eq!(era.board, src.board, "board");
    assert_eq!(era.hand, src.hand, "hand");
    assert_eq!(era.gold, src.gold, "Gold");
    assert_eq!(era.tier, src.tier, "tier");
    assert_eq!(era.upgrade_cost, src.upgrade_cost, "upgrade cost");
    assert_eq!(era.health, src.health, "Health");
    assert_eq!(era.armor, src.armor, "armor");
    assert_eq!(era.hero, src.hero, "hero");
    assert_eq!(era.power, src.power, "hero power");
    assert_eq!(era.power2, src.power2, "second hero power");
    assert_eq!(era.trinkets, src.trinkets, "trinkets");
    assert_eq!(
        era.counters(&game.data, game.lobby),
        src.counters(&game.data, game.lobby),
        "player counters"
    );
    assert_eq!(era.counts, src.counts, "hero counters");
    assert_eq!(era.golden_played, src.golden_played, "Golden minions played");
    assert_eq!(era.spell_bonus, src.spell_bonus, "Tavern spell bonus");
    game.check_invariants().expect("exact pool accounting");
}

#[test]
fn era_start_pool_survives_a_sale_and_a_triple() {
    let (turn_specs, turn_ids, turn_boards) = one_turn_specs(3);
    let (mut game, _, _) = era_game(&turn_specs, &turn_ids, turn_boards, 3, 3, 3);
    assert_eq!(game.current_player(), Some(0), "the Era seat acts first");
    if !game.is_legal(Action::Sell(0)) {
        let pl = &game.players[0];
        eprintln!(
            "board {} hand {} skipped {} trinket_offer {} discover {} power_offer {} secret_offer {} pending {:?} {:?} {:?} {:?}",
            pl.board.len(), pl.hand.len(), pl.skipped, pl.trinket_offer.len(),
            pl.discover.len(), pl.power_offer.len(), pl.secret_offer.len(),
            pl.pending_target, pl.pending_shop_target, pl.pending_choose_one, pl.pending_discard
        );
    }
    assert!(game.is_legal(Action::Sell(0)), "a starting minion can be sold");
    game.step(Action::Sell(0)).expect("the sale is legal");
    game.check_invariants().expect("pool accounting after selling a starting minion");

    // A real logged position with two Forest Rovers on the board and one in
    // hand: playing the hand copy makes the third and merges the triple.
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let name = "Hearthstone_2026_11_02_00_10_22-g1.json";
    let text = std::fs::read_to_string(format!("{root}boards/actions/{name}")).expect("the log");
    let log_ids = ids();
    let log_specs = specs(&[(name.to_string(), text)], &log_ids);
    let boards = fixture_boards();
    let (mut game, starts, position) = era_game(&log_specs, &log_ids, boards, 7, 11, 10);
    let forest = data().cards.by_name("Forest Rover").expect("Forest Rover");
    let p = &starts.position(position).player;
    assert_eq!(p.board.iter().filter(|m| m.card == forest).count(), 2);
    assert!(p.hand.iter().any(|m| m.card == forest));
    let hand = p.hand.iter().position(|m| m.card == forest).unwrap() as u8;
    assert_eq!(game.current_player(), Some(0));
    // The logged board is full: sell one card to make room, then play the
    // third Forest Rover from the hand.
    game.step(Action::Sell(0)).expect("a starting minion can be sold");
    game.check_invariants().expect("pool accounting after selling a starting minion");
    game.step(Action::Play { hand, pos: 0 }).expect("the third copy is played");
    game.check_invariants().expect("pool accounting after a starting triple");
    let golden = game.players[0]
        .board
        .iter()
        .chain(game.players[0].hand.iter())
        .any(|m| m.card == forest && m.golden);
    assert!(golden, "the three Forest Rovers made a Golden one");
}

#[test]
fn the_split_is_by_source_game_and_does_not_move() {
    let (specs, ids) = fixture_specs();
    let d = data();
    let (some, _) = EraStarts::build(&specs[..1], &d, &ids, 1, 17);
    let (all, _) = EraStarts::build(&specs, &d, &ids, 1, 17);
    for i in 0..some.len() {
        let a = some.position(i);
        let b = (0..all.len())
            .map(|j| all.position(j))
            .find(|p| p.source == a.source && p.game == a.game && p.turn == a.turn)
            .expect("the same position in the larger set");
        assert_eq!(a.holdout, b.holdout, "adding files does not move a game's split");
    }
    let train: Vec<_> = all.indices(EraSplit::Train).iter().map(|&i| all.position(i)).collect();
    let holdout: Vec<_> = all
        .indices(EraSplit::Holdout)
        .iter()
        .map(|&i| all.position(i))
        .collect();
    for a in &train {
        for b in &holdout {
            assert!(
                a.source != b.source || a.game != b.game,
                "{} g{} is on both sides",
                a.source,
                a.game
            );
        }
    }
    assert_eq!(train.len() + holdout.len(), all.len());
}

#[test]
fn the_same_seed_starts_the_same_era_game() {
    let (specs, ids, boards) = one_turn_specs(3);
    let d = data();
    let (starts, _) = EraStarts::build(&specs, &d, &ids, 3, 7);
    let starts = Arc::new(starts);
    let config = EraStartConfig::new(starts.clone(), 1.0, EraSplit::All, None).expect("a config");
    let seats = [0usize, 1, 2];
    let a = config.plan(99, &seats).expect("an Era game");
    let b = config.plan(99, &seats).expect("an Era game");
    assert_eq!(a.position, b.position);
    assert_eq!(a.seat, b.seat);

    let mut ga = Game::new_era_start(d.clone(), 99, &[1, 2, 3, 4, 5, 6, 7], boards.clone(), &starts, &a, None);
    let mut gb = Game::new_era_start(d, 99, &[1, 2, 3, 4, 5, 6, 7], boards, &starts, &b, None);
    let mut ra = Rng::new(7);
    let mut rb = Rng::new(7);
    for _ in 0..100 {
        assert_eq!(ga.turn, gb.turn);
        assert_eq!(ga.current_player(), gb.current_player());
        assert_eq!(ga.pool, gb.pool);
        let aa = bgcore::bots::random_action(&ga, &mut ra);
        let ba = bgcore::bots::random_action(&gb, &mut rb);
        assert_eq!(aa, ba);
        ga.step(aa).expect("the first copy's action is legal");
        gb.step(ba).expect("the second copy's action is legal");
        ga.check_invariants().expect("first copy invariants");
        gb.check_invariants().expect("second copy invariants");
        if ga.over {
            break;
        }
    }
    assert_eq!(ga.over, gb.over);
    assert_eq!(ga.placements(), gb.placements());
}

#[test]
fn era_start_lag_overrides_the_late_start_mix() {
    let (specs, ids, boards) = one_turn_specs(3);
    let d = data();
    let (starts, _) = EraStarts::build(&specs, &d, &ids, 3, 7);
    let mut boards = (*boards).clone();
    boards.set_lag_mix(&[(0, 1), (1, 1), (2, 1)]);
    let boards = Arc::new(boards);
    let config = EraStartConfig::new(Arc::new(starts), 1.0, EraSplit::All, Some(2))
        .expect("a config");
    let real: Vec<usize> = (1..PLAYERS).collect();
    let seats = [0usize];
    for seed in 0..60 {
        let plan = config.plan(seed, &seats).expect("an Era game");
        let g = Game::new_era_start(
            d.clone(), seed, &real, boards.clone(), &config.starts, &plan,
            config.era_start_lag,
        );
        assert_eq!(g.lag, 2, "seed {seed}: the late start took the mix");
    }
    // Normal games of the same batch still draw the mix, at its shares.
    let mut counts = [0usize; 3];
    let n = 3000;
    for seed in 0..n {
        counts[boards.lag_for_game(seed) as usize] += 1;
    }
    for (lag, count) in counts.iter().enumerate() {
        let share = *count as f64 / n as f64;
        assert!(
            (share - 1.0 / 3.0).abs() < 0.03,
            "lag {lag}: normal games draw it {share:.3}, want about 0.333"
        );
    }
    for seed in 0..30 {
        let g = Game::new_random_heroes_real(d.clone(), seed, &real, boards.clone());
        assert_eq!(
            g.lag,
            boards.lag_for_game(seed),
            "seed {seed}: a normal game left the mix"
        );
    }
}

#[test]
fn unset_era_start_lag_keeps_todays_draw_and_play() {
    let (specs, ids, boards) = one_turn_specs(3);
    let d = data();
    let (starts, _) = EraStarts::build(&specs, &d, &ids, 3, 7);
    let starts = Arc::new(starts);
    let mut boards = (*boards).clone();
    boards.set_lag_mix(&[(0, 1), (1, 1), (2, 1)]);
    let boards = Arc::new(boards);
    let config = EraStartConfig::new(starts.clone(), 1.0, EraSplit::All, None)
        .expect("a config");
    let real: Vec<usize> = (1..PLAYERS).collect();
    let seats = [0usize];
    let mut seen = [false; 3];
    for seed in 0..60 {
        let drawn = boards.lag_for_game(seed);
        let plan = config.plan(seed, &seats).expect("an Era game");
        let mut a = Game::new_era_start(
            d.clone(), seed, &real, boards.clone(), &starts, &plan, None,
        );
        assert_eq!(a.lag, drawn, "seed {seed}: unset lag is not today's draw");
        let mut b = Game::new_era_start(
            d.clone(), seed, &real, boards.clone(), &starts, &plan, Some(drawn),
        );
        let mut ra = Rng::new(11);
        let mut rb = Rng::new(11);
        for _ in 0..80 {
            assert_eq!(a.turn, b.turn);
            assert_eq!(a.current_player(), b.current_player());
            assert_eq!(a.pool, b.pool);
            let aa = bgcore::bots::random_action(&a, &mut ra);
            let ba = bgcore::bots::random_action(&b, &mut rb);
            assert_eq!(aa, ba);
            a.step(aa).expect("the first copy's action is legal");
            b.step(ba).expect("the second copy's action is legal");
            a.check_invariants().expect("first copy invariants");
            b.check_invariants().expect("second copy invariants");
            if a.over {
                break;
            }
        }
        assert_eq!(a.over, b.over);
        assert_eq!(a.placements(), b.placements());
        if drawn < 3 {
            seen[drawn as usize] = true;
        }
        if seen.iter().all(|&s| s) {
            break;
        }
    }
    assert!(seen.iter().all(|&s| s), "the mix never drew all three lags");
}

