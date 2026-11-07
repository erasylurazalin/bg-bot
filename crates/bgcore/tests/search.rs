//! The search's engine support (`crate::search`, train/search.py): the
//! dominance comparison on the two cycles Era names (the Tortollan Blue Shell
//! and Brann with Kalecgos and a Draconic Warden), the full pool a playout's
//! Refresh draws from, the coming fight played against real boards, and a
//! copy changing nothing in the game it came from.

use std::sync::Arc;

use bgcore::cards::Tribes;
use bgcore::real_boards::{CardIdMap, RealBoards, parse_board_files};
use bgcore::search::{
    Position, fight_boards, fight_health_change, fight_outcome, full_pool, refresh_from_full_pool,
};
use bgcore::{Action, CardDb, Data, Game, Minion, Tribe, PLAYERS, Rng};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn db() -> CardDb {
    Data::bundled().cards
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

/// The real lobby: every tribe in it, so any card can be dealt.
fn every_tribe() -> Tribes {
    Tribes::ALL
}

/// Put a pool card in a zone, taking its copy from the pool so the
/// accounting stays exact (the tests' `put_on_board`, `set_shop`).
fn place(g: &mut Game, p: usize, zone: usize, name: &str, golden: bool) {
    let c = g.data.cards.by_name(name).unwrap_or_else(|| panic!("{name}"));
    let slot = g.data.cards.get(c).pool_slot as usize;
    // Tokens hold no pool copy (Microbot and the like).
    let copies = if slot < g.pool.len() && !g.data.cards.get(c).token {
        g.pool[slot] -= 1;
        1
    } else {
        0
    };
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    match zone {
        0 => g.players[p].board.push(m),
        1 => g.players[p].hand.push(m),
        _ => g.players[p].shop.push(m),
    }
}

/// Give the pool every copy back and empty the zones the test owns, so a
/// hand-made position is legal again.
fn clean(g: &mut Game) {
    for m in g.players[0].shop.drain(..).collect::<Vec<_>>() {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for m in g.players[0].board.drain(..).collect::<Vec<_>>() {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for m in g.players[0].hand.drain(..).collect::<Vec<_>>() {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
}

/// A one-player sandbox position, so nothing else moves during the test.
fn sandbox(seed: u64) -> Game {
    Game::new_sandbox(data(), seed, Some(every_tribe()), 0)
}

/// A fresh sandbox position with empty zones and 10 Gold.
fn ready(g: &mut Game) {
    clean(g);
    let pl = &mut g.players[0];
    pl.gold = 10;
    pl.done = false;
}

/// Whose action is next: clear whatever stands in the way, so a test plays
/// player 0 straight away (`turn_of` in the other tests).
fn mine(g: &mut Game) {
    while g.current_player() != Some(0) {
        let a = if g.is_legal(Action::Freeze) { Action::Freeze } else { Action::EndTurn };
        g.step(a).unwrap();
    }
}

#[test]
fn the_blue_shell_cycle_dominates() {
    let mut g = sandbox(7);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    place(&mut g, 0, 0, "Microbot", false);
    mine(&mut g);
    let before = Position::of(&g, 0);
    let gold = g.players[0].gold;
    // Buy it, play it, sell it: the cycle Era plays every time it lost.
    g.step(Action::Buy(0)).unwrap();
    assert_eq!(g.players[0].gold, gold - 3, "the Shell costs 3");
    g.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    g.step(Action::Sell(1)).unwrap();
    assert_eq!(g.players[0].gold, gold + 2, "it sells for 5 after a loss");
    let d = before.judge(&g);
    assert!(d.dominates, "blocked by {:?}", d.blocked);
    assert!(d.better);
    assert!(g.players[0].board.len() == 1 && g.players[0].hand.is_empty());
}

#[test]
fn one_gold_less_or_one_health_less_does_not_dominate() {
    let mut g = sandbox(11);
    ready(&mut g);
    place(&mut g, 0, 0, "Microbot", false);
    mine(&mut g);
    let before = Position::of(&g, 0);
    // Nothing changes: no field is better, so it does not dominate.
    assert!(!before.judge(&g).dominates);
    // One Gold gone.
    let mut less = g.clone();
    less.players[0].gold -= 1;
    let d = before.judge(&less);
    assert!(!d.dominates);
    assert!(d.blocked.iter().any(|b| b.starts_with("gold")), "{:?}", d.blocked);
    // One Gold more and one minion 1 Health lower.
    let mut hurt = g.clone();
    hurt.players[0].gold += 1;
    hurt.players[0].board[0].health -= 1;
    let d = before.judge(&hurt);
    assert!(!d.dominates);
    assert!(d.blocked.iter().any(|b| b.starts_with("board[0]")), "{:?}", d.blocked);
    // A board minion one Health up, nothing else touched: it dominates.
    let mut healed = g.clone();
    healed.players[0].board[0].health += 1;
    let d = before.judge(&healed);
    assert!(d.dominates, "blocked by {:?}", d.blocked);
}

/// Era's free Dragon cycle: Brann, Kalecgos and four Dragons on the board,
/// a Draconic Warden in the Tavern for 3 Gold.
fn warden_position(seed: u64) -> Game {
    let mut g = sandbox(seed);
    ready(&mut g);
    for name in ["Brann Bronzebeard", "Kalecgos, Arcane Aspect"] {
        place(&mut g, 0, 0, name, false);
    }
    for name in ["Bronze Warden", "Blue Whelp", "Tarecgosa", "Persistent Poet"] {
        place(&mut g, 0, 0, name, false);
    }
    place(&mut g, 0, 2, "Draconic Warden", false);
    mine(&mut g);
    g
}

fn board_text(g: &Game) -> Vec<String> {
    let cards = db();
    g.players[0]
        .board
        .iter()
        .map(|m| {
            format!(
                "{} {}/{} {}",
                cards.get(m.card).name,
                m.attack,
                m.health,
                bgcore::search::keyword_names(m).join(",")
            )
        })
        .collect()
}

#[test]
fn the_warden_cycle_dominates_and_the_chromadrakes_are_played() {
    let mut g = warden_position(13);
    let before = Position::of(&g, 0);
    let gold = g.players[0].gold;
    let board = board_text(&g);
    // Buy the Warden and play it in the free slot.
    g.step(Action::Buy(0)).unwrap();
    g.step(Action::Play { hand: 0, pos: 6 }).unwrap();
    let drakes: Vec<u8> = g.players[0].hand.iter().enumerate().map(|(i, _)| i as u8).collect();
    assert_eq!(drakes.len(), 2, "Brann gives two Chromadrakes: {:?}", board_text(&g));
    let chromadrakes = &db().chromadrakes;
    assert!(
        g.players[0].hand.iter().all(|m| chromadrakes.contains(&m.card)),
        "both are Chromadrakes"
    );
    // Sell the Warden, then play and sell each Chromadrake.
    g.step(Action::Sell(6)).unwrap();
    for _ in 0..2 {
        g.step(Action::Play { hand: 0, pos: 6 }).unwrap();
        g.step(Action::Sell(6)).unwrap();
    }
    assert_eq!(g.players[0].gold, gold, "the cycle is free: {gold}");
    // The Blue Chromadrake's Battlecry hands Tavern spells over, so the hand
    // may come back with more than it had; that is a gain, not a loss.
    assert!(
        g.players[0].hand.iter().all(|m| db().get(m.card).is_spell()),
        "only the spells the Chromadrakes gave are left: {:?}",
        g.players[0].hand.iter().map(|m| db().get(m.card).name.clone()).collect::<Vec<_>>()
    );
    assert_eq!(g.players[0].board.len(), 6, "the board is as wide as it was");
    let after = board_text(&g);
    assert_ne!(after, board, "Kalecgos buffed the Dragons");
    let d = before.judge(&g);
    assert!(d.dominates, "blocked by {:?}", d.blocked);
    assert!(d.better);
}

#[test]
fn a_trinket_counter_that_rose_is_not_a_loss() {
    let mut g = sandbox(29);
    ready(&mut g);
    let before = Position::of(&g, 0);
    // A buy can move a held trinket's own counter (its "every N" count, the
    // number it improves): those only go up, so they cannot block a cycle.
    let mut after = g.clone();
    after.players[0].trinkets[0].count = 3;
    after.players[0].trinkets[0].value = 1;
    after.players[0].gold += 2;
    let d = before.judge(&after);
    assert!(d.dominates, "blocked by {:?}", d.blocked);
    // But a counter going down is a loss, and a different trinket is worse.
    let mut less = g.clone();
    less.players[0].gold += 2;
    less.players[0].trinkets[1].count = 1;
    let mut base = sandbox(29);
    ready(&mut base);
    base.players[0].trinkets[1].count = 2;
    let d = Position::of(&base, 0).judge(&less);
    assert!(!d.dominates);
    assert!(d.blocked.iter().any(|b| b.starts_with("trinket count")), "{:?}", d.blocked);
}

#[test]
fn a_copy_changes_nothing_in_the_game_it_came_from() {
    let g = sandbox(17);
    let snapshot = format!("{g:?}");
    let mut copy = g.clone();
    // Everything the search does to a copy: a cycle, a Refresh from the full
    // pool, the end of the turn and the fights.
    copy.players[0].lost_last_combat = true;
    copy.players[0].gold = 10;
    place(&mut copy, 0, 2, "Tortollan Blue Shell", false);
    copy.step(Action::Buy(0)).unwrap();
    let _ = refresh_from_full_pool(&mut copy, 0);
    copy.end_shop_turn(0);
    assert_eq!(format!("{g:?}"), snapshot, "the real game is untouched");
    assert!(!g.pool_untracked && copy.pool_untracked);
}

#[test]
fn a_playouts_refresh_draws_from_the_full_pool() {
    let cards = db();
    let mut g = sandbox(19);
    ready(&mut g);
    g.players[0].tier = 4;
    mine(&mut g);
    // One card of tiers 1 to 4 left in the pool: a Refresh from the real pool
    // can deal that card and nothing else, and only while its copy is left.
    let full = full_pool(&g);
    let keep = (0..g.pool.len())
        .find(|&c| {
            let card = &cards.cards[c];
            card.pool_slot as usize == c && full[c] > 0 && (1..=4).contains(&card.tier)
        })
        .expect("a pool card of tier 4 or less");
    let kept_name = cards.get(keep as bgcore::CardId).name.clone();
    let mut drained = g.clone();
    for (c, p) in drained.pool.iter_mut().enumerate() {
        *p = if c == keep { 1 } else { 0 };
    }
    // A pool that is not the game's own accounting (a search copy's) is what
    // the drained Refresh needs, exactly like the full one.
    drained.pool_untracked = true;
    // The game's own Refresh, from the pool as it stands.
    let mut thin = drained.clone();
    thin.players[0].shop.clear();
    thin.step(Action::Roll).unwrap();
    let thin_names: Vec<String> =
        thin.players[0].shop.iter().map(|m| cards.get(m.card).name.clone()).collect();
    assert!(
        thin_names.iter().all(|n| *n == kept_name),
        "the drained pool deals only what is left: {thin_names:?}"
    );
    // The search's Refresh puts every copy back first.
    let mut full_again = drained.clone();
    full_again.players[0].shop.clear();
    refresh_from_full_pool(&mut full_again, 0).unwrap();
    let names: Vec<String> =
        full_again.players[0].shop.iter().map(|m| cards.get(m.card).name.clone()).collect();
    assert_eq!(thin_names.len(), 1, "one copy left deals one card: {thin_names:?}");
    assert!(
        names.len() >= 4,
        "a full-pool Refresh fills the Tavern, not one card: {names:?}"
    );
    assert!(
        names.iter().filter(|n| **n != kept_name).count() >= names.len() - 1,
        "the draw saw every copy, not the one left: {names:?}"
    );
    // It draws from the tier's whole range: every card of the Tavern is of a
    // tier the player can see.
    for m in &full_again.players[0].shop {
        assert!(cards.get(m.card).tier <= 4);
    }
    assert!(full_again.pool_untracked && !g.pool_untracked);
    // And nothing of the real game moved.
    assert_eq!(full_pool(&g), full);
}

#[test]
fn the_fights_use_real_boards_only() {
    let dir = std::path::Path::new("tests/data/real-boards");
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_str().unwrap();
            n.ends_with(".json")
        })
        .map(|e| (e.file_name().to_str().unwrap().to_string(), std::fs::read_to_string(e.path()).unwrap()))
        .collect();
    files.sort();
    let ids = CardIdMap::parse(&std::fs::read_to_string("../../ref/bg-cards.json").unwrap()).unwrap();
    let (specs, _) = parse_board_files(&files, &ids).unwrap();
    let (boards, _) = RealBoards::build(&specs, &db(), &Data::bundled().gifts);
    let data = data();
    let mut g = sandbox(23);
    ready(&mut g);
    g.turn = 6;
    g.players[0].tier = 4;
    g.players[0].board.push(mk(&db(), "Microbot", false));
    // Every fight the search plays is a real board: the health change is a
    // loss's damage, never anything else.
    let picked = fight_boards(&data, &boards, 6, Some(4), None, 8);
    assert!(!picked.is_empty());
    let mut changes = Vec::new();
    for (k, &i) in picked.iter().enumerate() {
        let b = &boards.boards_at(6)[i];
        let mut rng = Rng::new(0x5eed + k as u64);
        let h = fight_health_change(&data, &g, 0, b, &mut rng);
        assert!(h <= 0, "a fight never gains Health");
        changes.push(h);
    }
    assert!(changes.iter().any(|&h| h < 0), "a 1/1 Microbot loses to a turn 6 board: {changes:?}");
    // The same fight gives the judge's own three-way outcome and the Health
    // change the search uses (train/era_turns.py reads both from one fight).
    for (k, &i) in picked.iter().enumerate() {
        let b = &boards.boards_at(6)[i];
        let mut rng = Rng::new(0x5eed + k as u64);
        let out = fight_outcome(&data, &g, 0, b, &mut rng);
        assert!(matches!(out.outcome, -1..=1));
        let mut rng = Rng::new(0x5eed + k as u64);
        assert_eq!(out.health_change, fight_health_change(&data, &g, 0, b, &mut rng));
        assert_eq!(out.outcome == -1, out.health_change < 0);
    }
    // The same board and seed give the same result for every candidate.
    let b = &boards.boards_at(6)[picked[0]];
    let one = fight_health_change(&data, &g, 0, b, &mut Rng::new(99));
    let two = fight_health_change(&data, &g, 0, b, &mut Rng::new(99));
    assert_eq!(one, two);
    // And a bigger board fights better than an empty one.
    let mut strong = g.clone();
    let cards = db();
    for _ in 0..6 {
        let mut m = mk(&cards, "Microbot", true);
        m.attack = 40;
        m.health = 40;
        strong.players[0].board.push(m);
    }
    let weak: i32 = picked
        .iter()
        .map(|&i| fight_health_change(&data, &g, 0, &boards.boards_at(6)[i], &mut Rng::new(1)))
        .sum();
    let strong_h: i32 = picked
        .iter()
        .map(|&i| fight_health_change(&data, &strong, 0, &boards.boards_at(6)[i], &mut Rng::new(1)))
        .sum();
    assert!(strong_h > weak, "a 40/40 board loses less than a Microbot");
}

#[test]
fn the_fight_boards_prefer_the_opponents_tier_and_tribe() {
    let dir = std::path::Path::new("tests/data/real-boards");
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().unwrap().ends_with(".json"))
        .map(|e| (e.file_name().to_str().unwrap().to_string(), std::fs::read_to_string(e.path()).unwrap()))
        .collect();
    files.sort();
    let ids = CardIdMap::parse(&std::fs::read_to_string("../../ref/bg-cards.json").unwrap()).unwrap();
    let (specs, _) = parse_board_files(&files, &ids).unwrap();
    let (boards, _) = RealBoards::build(&specs, &db(), &Data::bundled().gifts);
    let data = data();
    let all = boards.boards_at(8);
    // An opponent on tier 2: with fewer than MIN_MATCH boards at that tier
    // the whole bucket comes back, exactly as the brief says.
    let tier2 = all.iter().filter(|b| b.tier == 2).count();
    let picked = fight_boards(&data, &boards, 8, Some(2), Some(Tribe::Beast), 10);
    assert_eq!(picked.len(), 10);
    if tier2 < bgcore::search::MIN_MATCH {
        assert!(picked.iter().any(|&i| all[i].tier != 2), "the fallback is the whole bucket");
    } else {
        assert!(picked.iter().all(|&i| all[i].tier == 2));
    }
    let _: [u8; PLAYERS] = [0; PLAYERS];
}
