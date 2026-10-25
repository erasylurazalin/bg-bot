//! Real-board seats (docs/real-boards.md): building the board set from
//! specs (the loaders hand these in), the draw rules (the late-turn pool,
//! the nearest-earlier fallback), a seat getting its board each turn instead
//! of shopping, no hero, and exact pool accounting with real seats.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::real_boards::{
    CardIdMap, RealBoardSpec, RealBoards, RealMinionSpec, parse_board_files, LATE_TURN,
};
use bgcore::{Data, Game, Keywords, PLAYERS, Rng};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

/// (name, attack, health, golden, keywords), the shape the loaders hand in.
fn spec(turn: u32, tier: u8, minions: &[(&str, i32, i32, bool, &[&str])]) -> RealBoardSpec {
    RealBoardSpec {
        turn,
        tier,
        minions: minions
            .iter()
            .map(|&(name, attack, health, golden, keywords)| RealMinionSpec {
                name: name.to_string(),
                attack,
                health,
                golden,
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
            })
            .collect(),
    }
}

fn build(specs: &[RealBoardSpec]) -> Arc<RealBoards> {
    let (boards, unmatched) = RealBoards::build(specs, &data().cards);
    assert!(unmatched.is_empty(), "unexpected unmatched: {unmatched:?}");
    Arc::new(boards)
}

// A spread over the turns, like the real folder (thick early, thin late).
fn folder_like() -> Vec<RealBoardSpec> {
    vec![
        spec(1, 1, &[("Razorfen Geomancer", 2, 1, false, &[])]),
        spec(1, 1, &[("Risen Rider", 2, 1, false, &[])]),
        spec(2, 2, &[("Roadboar", 2, 4, false, &[])]),
        spec(2, 1, &[("Sewer Rat", 3, 2, false, &["TAUNT"])]),
        spec(3, 3, &[("Amber Guardian", 3, 2, false, &["DIVINE_SHIELD"])]),
        spec(4, 4, &[("Tortollan Blue Shell", 3, 6, false, &["TAUNT"])]),
        spec(14, 6, &[("Bonker", 2, 7, false, &["WINDFURY"])]),
        spec(15, 6, &[("Leeroy the Reckless", 6, 2, false, &["POISONOUS"])]),
    ]
}

#[test]
fn build_resolves_minions_and_stands_in_for_the_unknown() {
    let d = data();
    let specs = [
        spec(3, 2, &[
            ("Amber Guardian", 3, 2, false, &[]),
            // A card the game has but we don't build, and one no source knows.
            ("Dark Paradox", 6, 6, false, &[]),
            ("No Such Card", 1, 1, false, &[]),
        ]),
        spec(3, 1, &[("Sewer Rat", 3, 2, false, &["TAUNT"])]),
        spec(14, 6, &[("Tarecgosa", 9, 10, true, &["POISONOUS", "CLEARE", "STEALTH"])]),
    ];
    let (boards, unmatched) = RealBoards::build(&specs, &d.cards);
    assert_eq!(unmatched, ["Dark Paradox", "No Such Card"]);
    let t3 = boards.boards_at(3);
    assert_eq!(t3.len(), 2);
    // The unknown minions become Stand-ins with their stats and no effect.
    assert_eq!(t3[0].minions.len(), 3);
    for (m, stats) in t3[0].minions[1..].iter().zip([(6, 6), (1, 1)]) {
        assert_eq!(d.cards.get(m.card).name, "Stand-in");
        assert_eq!((m.attack, m.health), stats);
        assert_eq!(m.pool_copies, 0);
    }
    let m = &t3[0].minions[0];
    assert_eq!(d.cards.get(m.card).name, "Amber Guardian");
    assert_eq!((m.attack, m.health), (3, 2));
    assert!(!m.golden);
    assert_eq!(m.pool_copies, 0, "a real minion holds no pool copies");
    assert!(t3[1].minions[0].keywords.has(Keywords::TAUNT));
    // The file's stats stand, golden or not (9/10, not the card's golden 8/8),
    // POISONOUS becomes Venomous, unknown keywords are dropped.
    let g = &boards.boards_at(14)[0].minions[0];
    assert_eq!((g.attack, g.health), (9, 10));
    assert!(g.golden);
    assert!(g.keywords.has(Keywords::VENOMOUS));
    assert!(g.keywords.has(Keywords::STEALTH));
    assert!(!g.keywords.has(Keywords::TAUNT));
    assert_eq!(g.keywords.0 & Keywords::MAGNETIC, 0);
}

#[test]
fn turns_13_and_up_share_a_pool_and_empty_turns_fall_back() {
    let d = data();
    let specs = [
        spec(1, 1, &[("Risen Rider", 2, 1, false, &[])]),
        spec(4, 3, &[("Roadboar", 2, 4, false, &[])]),
        spec(4, 3, &[("Sewer Rat", 3, 2, false, &[])]),
        spec(14, 6, &[("Bonker", 2, 7, false, &[])]),
        spec(LATE_TURN + 6, 6, &[("Eternal Summoner", 8, 1, false, &["STEALTH"])]),
    ];
    let (boards, _) = RealBoards::build(&specs, &d.cards);
    assert_eq!(boards.len(), 5);
    assert_eq!(boards.boards_at(1).len(), 1);
    assert_eq!(boards.boards_at(4).len(), 2);
    // Turns 5 to 12 have no boards of their own: nearest earlier with some.
    for t in 5..LATE_TURN {
        assert_eq!(boards.boards_at(t).len(), 2, "turn {t}");
    }
    // Turn 13 and later draw from every board of turn 13 or later alike.
    for t in [LATE_TURN, LATE_TURN + 1, 40] {
        assert_eq!(boards.boards_at(t).len(), 2, "turn {t}");
    }
    // A fresh uniform draw from the bucket: both boards come up.
    let mut rng = Rng::new(7);
    let bucket = boards.boards_at(4);
    let name_of = |b: &bgcore::real_boards::RealBoard| {
        d.cards.get(b.minions[0].card).name.clone()
    };
    let first = name_of(&bucket[0]);
    let mut seen = [false; 2];
    for _ in 0..200 {
        let b = boards.draw(4, &mut rng);
        seen[usize::from(name_of(b) != first)] = true;
    }
    assert!(seen[0] && seen[1]);
    // The bucket itself never leaks turn-13 boards into earlier turns.
    assert_eq!(boards.boards_at(4).len(), 2);
}

#[test]
fn a_real_seat_gets_its_board_each_turn_and_never_shops() {
    let d = data();
    let specs = [
        spec(1, 1, &[("Razorfen Geomancer", 2, 1, false, &[])]),
        spec(2, 2, &[("Roadboar", 2, 4, false, &[]), ("Tarecgosa", 8, 8, true, &["TAUNT"])]),
        spec(2, 1, &[("Sewer Rat", 3, 2, false, &["VENOMOUS"])]),
        spec(3, 3, &[("Amber Guardian", 3, 2, false, &["DIVINE_SHIELD"])]),
        spec(4, 4, &[("Bonker", 2, 7, false, &["WINDFURY"])]),
    ];
    let (boards, unmatched) = RealBoards::build(&specs, &d.cards);
    assert!(unmatched.is_empty());
    let boards = Arc::new(boards);
    let mut g = Game::new_random_heroes_real(d.clone(), 42, &[0], boards.clone());
    // The game starts in the hero pick (turn 0), which the real seat waits
    // out with no hero, no power and the flat 30 Health and 10 armor whatever
    // the boards say (Era's rule).
    let pl = &g.players[0];
    assert!(pl.real);
    assert_eq!((pl.health, pl.armor), (30, 10));
    assert_eq!(pl.hero, None);
    assert_eq!(pl.power, None);
    assert!(pl.done, "the pick phase is over for the seat at once");
    assert!(pl.shop.is_empty() && pl.shop_spell.is_none() && pl.hand.is_empty());
    assert_eq!(pl.gold, 0);
    for p in 1..PLAYERS {
        assert!(g.players[p].hero.is_some(), "player {p} got no hero");
        assert!(!g.players[p].real);
    }
    // Every turn from 1 on, the board is one of that turn's, tier and all.
    let matches = |g: &Game| {
        let pl = &g.players[0];
        boards.boards_at(g.turn).iter().any(|b| {
            b.tier == pl.tier
                && b.minions.as_slice() == &pl.board[..]
                && pl.board.iter().all(|m| m.pool_copies == 0)
        })
    };
    let mut rng = Rng::new(99);
    let mut last_turn = g.turn;
    while !g.over {
        let a = random_action(&g, &mut rng);
        g.step(a).unwrap();
        g.check_invariants().unwrap();
        if g.turn != last_turn {
            last_turn = g.turn;
            if g.players[0].alive {
                assert!(g.players[0].done, "the seat's turn ends without shopping");
                assert!(matches(&g), "turn {}: board not from this turn's", g.turn);
            }
        }
    }
    assert!(g.players[0].placement > 0);
}

#[test]
fn pool_accounting_holds_with_real_seats() {
    let d = data();
    let boards = build(&folder_like());
    for seed in 0..5u64 {
        let mut g = Game::new_random_heroes_real(d.clone(), seed, &[0, 1], boards.clone());
        let mut rng = Rng::new(seed ^ 0x5eed);
        while !g.over {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
        // A knocked-out real seat put nothing back: every copy is still
        // pool + held + consumed, which check_invariants just proved.
        assert!(g.players[0].placement > 0 || g.players[1].placement > 0);
    }
}

#[test]
fn a_game_of_only_real_seats_finishes() {
    let d = data();
    let boards = build(&folder_like());
    let real: Vec<usize> = (0..PLAYERS).collect();
    // Every seat done at every turn start: the game runs on fights alone and
    // still ends with everyone placed.
    let g = Game::new_random_heroes_real(d, 3, &real, boards);
    assert!(g.over);
    assert!(g.players.iter().all(|pl| pl.placement > 0));
}

/// The fixture folder (tests/data/real-boards, a small stand-in for
/// boards/real) through the same parser `bgbench --real-boards` uses.
#[test]
fn the_fixture_folder_loads_through_the_shared_parser() {
    let mut files: Vec<(String, String)> = std::fs::read_dir("tests/data/real-boards")
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_str().unwrap();
            n.ends_with(".json") && !n.starts_with('.')
        })
        .map(|e| {
            let p = e.path();
            (p.file_name().unwrap().to_str().unwrap().to_string(),
             std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    files.sort();
    assert_eq!(files.len(), 3, "the fixture holds three games");
    let ids = CardIdMap::parse(
        &std::fs::read_to_string("../../ref/bg-cards.json").unwrap(),
    )
    .unwrap();
    let (specs, unknown) = parse_board_files(&files, &ids).unwrap();
    // The buddy id no source knows, reported by the name its file shows.
    assert_eq!(unknown, ["BG20_100t"]);
    // Golden both ways: the game's own _G id, and the base id plus the
    // file's golden flag.
    let t2 = specs.iter().find(|s| s.turn == 2 && s.minions.iter().any(|m| m.name == "Tarecgosa")).unwrap();
    assert!(t2.minions.iter().find(|m| m.name == "Tarecgosa").unwrap().golden);
    let wd = specs.iter().flat_map(|s| &s.minions).find(|m| m.name == "Water Droplet").unwrap();
    assert!(wd.golden);
    let (boards, unmatched) = RealBoards::build(&specs, &data().cards);
    // Dark Paradox: the id is known, the card is not built here.
    assert_eq!(unmatched, ["Dark Paradox"]);
    let counts = boards.counts_by_turn();
    assert_eq!(counts.len(), LATE_TURN as usize);
    assert_eq!(&counts[..5], &[2, 4, 2, 2, 0]);
    assert!(counts[5..12].iter().all(|&n| n == 0));
    assert_eq!(counts[12], 2, "turn 14 lands in the late pool");
    assert_eq!(boards.len(), 12);
}

#[test]
fn real_seats_take_no_hero_from_the_deck() {
    // Two heroes, two hero seats, six real seats: both heroes reach the hero
    // seats only because the real seats are dealt none.
    let heroes = "
[[hero]]
name = \"Nozdormu\"
card_id = \"TB_BaconShop_HERO_57\"
armor = 13
power = { name = \"Clairvoyance\", card_id = \"TB_BaconShop_HP_063\", passive = true, effect = { free_refresh_per_turn = 1 } }

[[hero]]
name = \"Forest Warden Omu\"
card_id = \"TB_BaconShop_HERO_74\"
armor = 6
power = { name = \"Everbloom\", card_id = \"TB_BaconShop_HP_082\", passive = true, effect = { gold_after_upgrade = 2 } }
";
    let d = Arc::new(Data::from_toml_with_heroes(
        include_str!("../../../cards.toml"),
        include_str!("../../../rules.toml"),
        heroes,
    )
    .unwrap());
    let (boards, _) = RealBoards::build(&folder_like(), &d.cards);
    let real: Vec<usize> = (2..PLAYERS).collect();
    let g = Game::new_random_heroes_real(d, 5, &real, Arc::new(boards));
    assert_eq!(g.players[0].hero.unwrap(), 0);
    assert_eq!(g.players[1].hero.unwrap(), 1);
    for p in 2..PLAYERS {
        assert_eq!(g.players[p].hero, None);
    }
}
