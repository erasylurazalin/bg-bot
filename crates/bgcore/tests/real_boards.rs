//! Real-board seats (docs/real-boards.md): building the board set from
//! specs (the loaders hand these in), the draw rules (the late-turn pool,
//! the nearest-earlier fallback), a seat getting its board each turn instead
//! of shopping, no hero, and exact pool accounting with real seats.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::cards::Counter;
use bgcore::fight::{Counters, Watch, fight_with_hands};
use bgcore::real_boards::{
    CardIdMap, MIN_FILTERED_BOARDS, RealBoardSpec, RealBoards, RealCounters, RealDeitySpec,
    RealMinionSpec, parse_board_files, LATE_TURN,
};
use bgcore::{Data, Game, Keywords, Minion, PLAYERS, Rng, Tribe, Tribes};

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
                numbers: Vec::new(),
                dark_gifts: Vec::new(),
                reanimate: false,
                no_reward: false,
                dark_gift_states: Vec::new(),
                vanguard_aura: [0, 0],
            })
            .collect(),
        ..RealBoardSpec::default()
    }
}

/// The first minion of a spec, with the numbers the log showed for it.
fn with_numbers(mut b: RealBoardSpec, index: usize, numbers: &[i32]) -> RealBoardSpec {
    b.minions[index].numbers = numbers.to_vec();
    b
}

#[derive(Default)]
struct Log {
    summons: Vec<Minion>,
}

impl Watch for Log {
    fn summon(&mut self, _side: usize, _pos: usize, m: &Minion) {
        self.summons.push(*m);
    }
}

fn name(db: &bgcore::CardDb, m: &Minion) -> String {
    db.get(m.card).name.clone()
}

fn build(specs: &[RealBoardSpec]) -> Arc<RealBoards> {
    let (boards, unmatched) = RealBoards::build(specs, &data().cards, &data().gifts);
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
    let (boards, unmatched) = RealBoards::build(&specs, &d.cards, &d.gifts);
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
    // POISONOUS stays Poisonous, unknown keywords are dropped.
    let g = &boards.boards_at(14)[0].minions[0];
    assert_eq!((g.attack, g.health), (9, 10));
    assert!(g.golden);
    assert!(g.keywords.has(Keywords::POISONOUS));
    assert!(!g.keywords.has(Keywords::VENOMOUS), "Poisonous is not Venomous");
    assert!(g.keywords.has(Keywords::STEALTH));
    assert!(!g.keywords.has(Keywords::TAUNT));
    assert_eq!(g.keywords.0 & Keywords::MAGNETIC, 0);
}

#[test]
fn the_loader_keeps_venomous_and_poisonous_apart() {
    // Poisonous and Venomous are two keywords, and a minion can wear both
    // (docs/rules/design.md "Keywords"; log 11_02_00_10_22-g7 turn 14, where
    // Tide Oracle Morgl gets Venomous on top of its printed Poisonous).
    let d = data();
    let specs = [
        spec(14, 3, &[
            ("Tide Oracle Morgl", 141, 109, false, &["POISONOUS"]),
            ("Tide Oracle Morgl", 141, 109, false, &["POISONOUS", "VENOMOUS"]),
            ("Tide Oracle Morgl", 141, 109, false, &["VENOMOUS"]),
        ]),
    ];
    let (boards, unmatched) = RealBoards::build(&specs, &d.cards, &d.gifts);
    assert!(unmatched.is_empty(), "unexpected unmatched: {unmatched:?}");
    let board = &boards.boards_at(14)[0];
    let (poisonous, both, venomous) = (&board.minions[0], &board.minions[1], &board.minions[2]);
    assert!(poisonous.keywords.has(Keywords::POISONOUS));
    assert!(!poisonous.keywords.has(Keywords::VENOMOUS));
    assert!(both.keywords.has(Keywords::POISONOUS), "both, still Poisonous");
    assert!(both.keywords.has(Keywords::VENOMOUS), "both, still Venomous");
    assert!(!venomous.keywords.has(Keywords::POISONOUS));
    assert!(venomous.keywords.has(Keywords::VENOMOUS));
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
    let (boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
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
    let (boards, unmatched) = RealBoards::build(&specs, &d.cards, &d.gifts);
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
    // The numbers, the hand and the Deity the collector writes (the older
    // boards in the same files have none and keep the base values).
    let d = data();
    let t2me = specs.iter().find(|s| s.turn == 2 && !s.hand.is_empty()).unwrap();
    assert_eq!(t2me.counters.gem_attack, 3);
    assert_eq!(t2me.counters.gem_health, 2);
    assert_eq!(t2me.counters.beetle_attack, 2);
    assert_eq!(t2me.counters.spell_attack, 1);
    assert_eq!(t2me.counters.spells_cast, 4);
    assert_eq!(t2me.hand.len(), 1);
    assert_eq!(t2me.hand[0].name, "Tasty Lobster");
    assert_eq!(t2me.hand[0].numbers, [10, 5, 0, 0]);
    assert_eq!(t2me.deity.as_ref().unwrap().name, "Y'Shaarj");
    let plain = specs.iter().find(|s| s.turn == 1).unwrap();
    assert_eq!(plain.counters, RealCounters::default());
    assert!(plain.hand.is_empty() && plain.deity.is_none());
    let t4opp = specs
        .iter()
        .find(|s| s.turn == 4 && s.counters.undead_attack == 6)
        .unwrap();
    assert_eq!(t4opp.counters.knight_deaths, 2);
    assert_eq!(t4opp.counters.improve[Counter::Baller.index()], 1);
    assert!(t4opp.deity.is_none());
    let (boards, unmatched) = RealBoards::build(&specs, &d.cards, &d.gifts);
    // Dark Paradox's file names it plainly and its id says which version it
    // is, so the board builds it (and nothing else in the folder is missing).
    assert!(unmatched.is_empty(), "{unmatched:?}");
    let paradox = boards
        .boards_at(3)
        .iter()
        .flat_map(|b| b.minions.iter())
        .find(|m| d.cards.get(m.card).name.starts_with("Dark Paradox"))
        .expect("the fixture's Dark Paradox is built");
    assert!(d.cards.get(paradox.card).tier > 1, "the version's own tier");
    // The hand's Lobster is the player's improve count (10 / 2 - 1), and the
    // Leviathan carries its growth (15 / 3).
    let t2 = boards.boards_at(2).iter().find(|b| !b.hand.is_empty()).unwrap();
    assert_eq!(t2.counters.improve[Counter::TastyLobster.index()], 4);
    assert_eq!(t2.hand[0].improvements, 0);
    assert_eq!(t2.minions[2].improvements, 5, "the Leviathan's growth: 15 / 3");
    let t4 = boards.boards_at(4).iter().find(|b| b.counters.knight_deaths == 2).unwrap();
    assert_eq!(t4.counters.undead_attack, 6);
    assert!(t4.hand.is_empty());
    let counts = boards.counts_by_turn();
    assert_eq!(counts.len(), LATE_TURN as usize);
    assert_eq!(&counts[..5], &[2, 4, 2, 2, 0]);
    assert!(counts[5..12].iter().all(|&n| n == 0));
    assert_eq!(counts[12], 2, "turn 14 lands in the late pool");
    assert_eq!(boards.len(), 12);
}

#[test]
fn the_loader_records_the_vanguard_aura_beside_the_logged_stats() {
    // The logged ATK and HEALTH carry Holy Vanguard's own +30/+30 while its
    // owner has 15 or less Health, in every zone: the file writes the pair on
    // the `Vanguard's Valor` enchantment beside them and the loader records it
    // on `Minion::vanguard_aura`, so a later Health crossing can take it off
    // exactly once (docs/rules/design.md "Triples").
    let text = r#"{"source": "vanguard", "game": 1, "boards": [
        {"who": "me", "turn": 8, "result": "win", "tier": 4, "health": 12, "armor": 0,
         "minions": [{"card_id": "BG36_372", "name": "Holy Vanguard", "attack": 40,
                      "health": 40, "golden": false, "keywords": ["DIVINE_SHIELD"],
                      "vanguard_aura": [30, 30]}],
         "hand": [], "counters": {}}]}"#;
    let ids = CardIdMap::parse(
        &std::fs::read_to_string("../../ref/bg-cards.json").unwrap(),
    )
    .unwrap();
    let (specs, unknown) =
        parse_board_files(&[("vanguard.json".to_string(), text.to_string())], &ids).unwrap();
    assert!(unknown.is_empty(), "{unknown:?}");
    assert_eq!(specs[0].health, 12);
    assert_eq!(specs[0].minions[0].vanguard_aura, [30, 30]);
    let boards = build(&specs);
    let m = &boards.boards_at(8)[0].minions[0];
    assert_eq!((m.attack, m.health, m.vanguard_aura), (40, 40, [30, 30]));
    // Above 15 Health the pair is not in the stats, even when the log shows a
    // `Vanguard's Valor` enchantment on the card: a copy can carry one from a
    // player whose Health was low (log 2026_11_04_15_53_32 game 2, an opponent
    // at 17 with a 13/13 Vanguard wearing a 30/30 one). The loader then
    // records no pair, since the stats hold none.
    let high = text.replace(r#""health": 12"#, r#""health": 17"#).replace(
        r#""attack": 40,
                      "health": 40"#,
        r#""attack": 13,
                      "health": 13"#,
    );
    let (specs, _) =
        parse_board_files(&[("vanguard.json".to_string(), high)], &ids).unwrap();
    assert_eq!(specs[0].minions[0].vanguard_aura, [30, 30], "the file still says so");
    let boards = build(&specs);
    let m = &boards.boards_at(8)[0].minions[0];
    assert_eq!((m.attack, m.health, m.vanguard_aura), (13, 13, [0, 0]));
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
    let (boards, _) = RealBoards::build(&folder_like(), &d.cards, &d.gifts);
    let real: Vec<usize> = (2..PLAYERS).collect();
    let g = Game::new_random_heroes_real(d, 5, &real, Arc::new(boards));
    assert_eq!(g.players[0].hero.unwrap(), 0);
    assert_eq!(g.players[1].hero.unwrap(), 1);
    for p in 2..PLAYERS {
        assert_eq!(g.players[p].hero, None);
    }
}

// ---- the numbers a board's player carried (2026-10-26) ----

/// The counters a fight takes, from a board's numbers: what `Player::counters`
/// builds out of the seat's fields.
fn fight_counters(c: &RealCounters, tier: u8, deity: Option<bgcore::fight::Deity>) -> Counters {
    Counters {
        beetle_attack: c.beetle_attack,
        beetle_health: c.beetle_health,
        improve: c.improve,
        undead_attack: c.undead_attack,
        undead_health: c.undead_health,
        spell_attack: c.spell_attack,
        spell_health: c.spell_health,
        knight_deaths: c.knight_deaths,
        gem_attack: c.gem_attack,
        gem_health: c.gem_health,
        deity,
        spells_cast: c.spells_cast,
        tier,
        lobby: Tribes::ALL,
        ..Counters::default()
    }
}

/// A turn-1 folder: one board (so the seat's draw is fixed), plus a board for
/// every turn after it so the game can run on.
fn single_board_folder(board: &RealBoardSpec) -> Arc<RealBoards> {
    let mut specs = vec![board.clone()];
    for turn in 2..=bgcore::LATE_TURN {
        specs.push(spec(turn, 4, &[("Sewer Rat", 3, 2, false, &[])]));
    }
    build(&specs)
}

/// Play the hero pick out so turn 1 (and the real seat's board) has started.
fn first_turn(g: &mut Game) {
    let mut rng = Rng::new(17);
    while g.turn == 0 && !g.over {
        let a = random_action(g, &mut rng);
        g.step(a).unwrap();
    }
}

#[test]
fn a_real_seat_takes_the_boards_numbers() {
    let d = data();
    let mut b = with_numbers(
        spec(1, 5, &[("Lurking Leviathan", 3, 9, false, &[])]),
        0,
        &[15, 18, 0, 0],
    );
    b.hand = vec![RealMinionSpec {
        name: "Tasty Lobster".to_string(),
        attack: 6,
        health: 3,
        golden: false,
        keywords: Vec::new(),
        numbers: vec![14, 7, 0, 0],
        dark_gifts: Vec::new(),
        reanimate: false,
        no_reward: false,
        dark_gift_states: Vec::new(),
        vanguard_aura: [0, 0],
    }];
    b.counters.set("gem_attack", 5);
    b.counters.set("gem_health", 4);
    b.counters.set("beetle_attack", 4);
    b.counters.set("beetle_health", 2);
    b.counters.set("undead_attack", 6);
    b.counters.set("undead_health", 1);
    b.counters.set("spell_attack", 2);
    b.counters.set("spell_health", 1);
    b.counters.set("spells_cast", 3);
    b.counters.set("knight_deaths", 2);
    b.counters.set("baller", 1);
    b.deity = Some(RealDeitySpec {
        name: "C'Thun".to_string(),
        attack: 7,
        health: 8,
        golden: false,
    });
    let boards = single_board_folder(&b);
    let mut g = Game::new_random_heroes_real(d.clone(), 42, &[0], boards);
    first_turn(&mut g);
    let pl = &g.players[0];
    // Everything the log showed is on the seat; the flat 30 Health and 10
    // armor stay as they are.
    assert_eq!((pl.health, pl.armor), (30, 10));
    assert_eq!((pl.gem_attack, pl.gem_health), (5, 4));
    assert_eq!(pl.beetle_bonus, (4, 2));
    assert_eq!(pl.undead_bonus, (6, 1));
    assert_eq!(pl.spell_bonus, (2, 1));
    assert_eq!(pl.spells_cast, 3);
    assert_eq!(pl.knight_deaths, 2);
    assert_eq!(pl.improve[Counter::Baller.index()], 1);
    assert_eq!(pl.improve[Counter::TastyLobster.index()], 6, "14 attack / 2 - 1");
    let god = pl.deity.unwrap();
    assert_eq!((god.attack, god.health, god.golden), (7, 8, false));
    assert_eq!(d.cards.get(god.card).name, "C'Thun");
    // The Leviathan's growth is its scaling state: 15 / 3 = 5 improvements.
    assert_eq!(pl.board[0].improvements, 5);
    assert_eq!(pl.hand.len(), 1);
    assert_eq!(d.cards.get(pl.hand[0].card).name, "Tasty Lobster");
    // The fight reads exactly those numbers (`Player::counters` is what a
    // fight is handed); only the seat's Health and the game's lobby differ.
    let c = pl.counters(&d, g.lobby);
    let mut want = fight_counters(&b.counters, pl.tier, pl.deity);
    // The Lobster's count comes from its numbers, not from the counters.
    want.improve = [6, 1];
    want.health = c.health;
    want.lobby = c.lobby;
    assert_eq!(c, want);
}

#[test]
fn the_numbers_reach_the_fight() {
    // A Buzzing Vermin (taunt) dies first, so its Beetle is summoned while the
    // Leviathan is still there: 2 + 4 Beetle + 18 from the improved Leviathan.
    let d = data();
    let mut b = with_numbers(
        spec(
            1,
            5,
            &[
                ("Buzzing Vermin", 1, 1, false, &["TAUNT"]),
                ("Lurking Leviathan", 3, 9, false, &[]),
            ],
        ),
        1,
        &[15, 18, 0, 0],
    );
    b.counters.set("beetle_attack", 4);
    b.counters.set("beetle_health", 2);
    let boards = single_board_folder(&b);
    let mut g = Game::new_random_heroes_real(d.clone(), 7, &[0], boards);
    first_turn(&mut g);
    let pl = &g.players[0];
    let mut wall = d.cards.make(d.cards.by_name("Microbot").unwrap(), false);
    wall.attack = 10;
    wall.health = 500;
    let mut log = Log::default();
    let counters = pl.counters(&d, g.lobby);
    fight_with_hands(
        &d.cards,
        &pl.board,
        &[wall],
        [&pl.hand, &[]],
        [counters, Counters::default()],
        500,
        &mut Rng::new(1),
        &mut log,
    );
    let beetle = log
        .summons
        .iter()
        .find(|m| name(&d.cards, m) == "Beetle")
        .expect("the Vermin's Deathrattle summons a Beetle");
    assert_eq!((beetle.attack, beetle.health), (2 + 4 + 18, 2 + 2));
}

#[test]
fn a_real_seat_hand_reaches_the_fight() {
    // Diremuck Forager's Start of Combat summons a Murloc out of the hand.
    let d = data();
    let mut b = spec(1, 4, &[("Diremuck Forager", 4, 5, false, &[])]);
    b.hand = vec![RealMinionSpec {
        name: "Bream Counter".to_string(),
        attack: 6,
        health: 6,
        golden: false,
        keywords: Vec::new(),
        numbers: Vec::new(),
        dark_gifts: Vec::new(),
        reanimate: false,
        no_reward: false,
        dark_gift_states: Vec::new(),
        vanguard_aura: [0, 0],
    }];
    let boards = single_board_folder(&b);
    let mut g = Game::new_random_heroes_real(d.clone(), 11, &[0], boards);
    first_turn(&mut g);
    let pl = &g.players[0];
    assert_eq!(pl.hand.len(), 1, "the seat got the board's hand");
    let mut wall = d.cards.make(d.cards.by_name("Microbot").unwrap(), false);
    wall.attack = 1;
    wall.health = 100;
    let mut log = Log::default();
    let counters = pl.counters(&d, g.lobby);
    fight_with_hands(
        &d.cards,
        &pl.board,
        &[wall],
        [&pl.hand, &[]],
        [counters, Counters::default()],
        500,
        &mut Rng::new(4),
        &mut log,
    );
    assert!(
        log.summons.iter().any(|m| name(&d.cards, m) == "Bream Counter"),
        "the Forager summons the minion the seat holds"
    );
}

#[test]
fn the_next_turns_board_replaces_the_numbers() {
    let d = data();
    let mut first = spec(1, 4, &[("Sewer Rat", 3, 2, false, &["TAUNT"])]);
    first.counters.set("gem_attack", 9);
    first.counters.set("beetle_attack", 7);
    first.counters.set("spells_cast", 5);
    first.counters.set("knight_deaths", 3);
    first.deity = Some(RealDeitySpec {
        name: "Y'Shaarj".to_string(),
        attack: 4,
        health: 6,
        golden: false,
    });
    let mut second = spec(2, 5, &[("Roadboar", 2, 4, false, &[])]);
    second.counters.set("gem_attack", 2);
    let mut specs = vec![first, second];
    for turn in 3..=bgcore::LATE_TURN {
        specs.push(spec(turn, 5, &[("Sewer Rat", 3, 2, false, &[])]));
    }
    let boards = build(&specs);
    let mut g = Game::new_random_heroes_real(d.clone(), 5, &[0], boards);
    first_turn(&mut g);
    assert_eq!(g.players[0].gem_attack, 9);
    // Play the first turn out (a real seat does nothing, so EndTurn until the
    // second turn starts).
    let mut rng = Rng::new(3);
    let turn = g.turn;
    while g.turn == turn {
        let a = random_action(&g, &mut rng);
        g.step(a).unwrap();
    }
    let pl = &g.players[0];
    // Turn 2's board brings its own numbers and nothing of turn 1's is left.
    assert_eq!((pl.gem_attack, pl.gem_health), (2, 1));
    assert_eq!(pl.beetle_bonus, (0, 0));
    assert_eq!(pl.spells_cast, 0);
    assert_eq!(pl.knight_deaths, 0);
    assert_eq!(pl.improve, [0; 2]);
    assert_eq!(pl.deity, None);
    assert_eq!(pl.tier, 5);
}

#[test]
fn an_old_file_loads_with_the_base_values() {
    // A board file from before the counters, the hands and the numbers were
    // collected: every number is the base value and there is no hand.
    let old = r#"{"boards": [
      {"who": "me", "turn": 4, "tier": 4, "minions": [
        {"card_id": "BG36_202", "name": "Tasty Lobster", "attack": 2, "health": 1,
         "golden": false, "keywords": ["DEATHRATTLE"], "dark_gifts": []}]},
      {"who": "opp", "turn": 4, "tier": 4, "minions": [
        {"card_id": "BG35_602", "name": "Lurking Leviathan", "attack": 3, "health": 9,
         "golden": false, "keywords": [], "dark_gifts": []}]}]}"#;
    let ids = CardIdMap::parse(&std::fs::read_to_string("../../ref/bg-cards.json").unwrap())
        .unwrap();
    let (specs, unknown) =
        parse_board_files(&[("old.json".to_string(), old.to_string())], &ids).unwrap();
    assert!(unknown.is_empty());
    assert_eq!(specs.len(), 2);
    for s in &specs {
        assert_eq!(s.counters, RealCounters::default());
        assert_eq!(s.hand.len(), 0);
        assert_eq!(s.deity, None);
        assert!(s.minions.iter().all(|m| m.numbers.is_empty()));
    }
    let (boards, _) = RealBoards::build(&specs, &data().cards, &data().gifts);
    let b = &boards.boards_at(4)[0];
    assert_eq!((b.counters.gem_attack, b.counters.gem_health), (1, 1));
    assert_eq!(b.counters.improve, [0; 2]);
    assert!(b.hand.is_empty());
    assert_eq!(b.deity, None);
    // A base Lobster and Leviathan scale from 1, as a fresh card does.
    assert_eq!(b.minions[0].improvements, 0);
    assert_eq!(b.counters.improve[Counter::TastyLobster.index()], 0);
}

#[test]
fn the_loaded_cards_carry_their_scaling_state() {
    let d = data();
    let specs = [
        with_numbers(spec(5, 5, &[("Lurking Leviathan", 13, 9, false, &[])]), 0, &[3, 6, 0, 0]),
        with_numbers(spec(5, 5, &[("Lurking Leviathan", 6, 18, true, &[])]), 0, &[12, 0, 0, 0]),
        with_numbers(spec(5, 4, &[("Spark Snapper", 6, 8, false, &[])]), 0, &[8, 10, 15, 12]),
        with_numbers(spec(5, 5, &[("Ultraviolet Ascendant", 6, 6, false, &[])]), 0, &[3, 3, 4, 0]),
        with_numbers(spec(5, 2, &[("Patient Scout", 1, 1, false, &[])]), 0, &[4, 0, 0, 0]),
        // A card whose numbers are not a scaling count of ours.
        with_numbers(spec(5, 1, &[("Scarlet Survivor", 6, 6, false, &[])]), 0, &[6, 1, 0, 0]),
        // A Tasty Lobster's numbers are the player's count, not its own.
        with_numbers(spec(5, 3, &[("Tasty Lobster", 6, 3, false, &[])]), 0, &[10, 5, 0, 0]),
        with_numbers(spec(5, 3, &[("Tasty Lobster", 12, 6, true, &[])]), 0, &[20, 10, 0, 0]),
    ];
    let (boards, unmatched) = RealBoards::build(&specs, &d.cards, &d.gifts);
    assert!(unmatched.is_empty());
    let imp = |turn: &[bgcore::real_boards::RealBoard]| turn[0].minions[0].improvements;
    let at5 = boards.boards_at(5);
    assert_eq!(at5.len(), 8);
    assert_eq!(imp(&at5[0..1]), 1, "Leviathan: 3 attack of growth / 3");
    assert_eq!(imp(&at5[1..2]), 2, "golden Leviathan: 12 / 6");
    assert_eq!(imp(&at5[2..3]), 4, "Snapper: 8 of growth / 2, a 10/15 Satellite");
    assert_eq!(imp(&at5[3..4]), 4, "Ultraviolet Ascendant: the third number");
    assert_eq!(imp(&at5[4..5]), 3, "Patient Scout: tier 4 is 3 improvements");
    assert_eq!(imp(&at5[5..6]), 0, "Scarlet Survivor's number is not a count");
    assert_eq!(imp(&at5[6..7]), 0, "the Lobster's count is the player's");
    // Every Lobster shares one count; the better of the two is kept.
    assert_eq!(at5[6].counters.improve[Counter::TastyLobster.index()], 4, "10 / 2 - 1");
    assert_eq!(at5[7].counters.improve[Counter::TastyLobster.index()], 4, "20 / 4 - 1");
}

/// How much the numbers change real boards: every board of a turn against a
/// sample of the same turn's boards, with the old loading (no numbers, no
/// hand) and the new. Ignored by default; the report number comes from
/// `cargo test --release -p bgcore --test real_boards -- --ignored --nocapture`.
#[test]
#[ignore = "measurement over boards/real, run with --ignored"]
fn the_numbers_make_real_boards_stronger() {
    let read = |dir: &str| -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .map(|e| e.unwrap())
            .filter(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                n.ends_with(".json") && !n.starts_with('.')
            })
            .map(|e| {
                let p = e.path();
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    };
    let ids =
        CardIdMap::parse(&std::fs::read_to_string("../../ref/bg-cards.json").unwrap()).unwrap();
    let d = data();
    let (new_specs, _) = parse_board_files(&read("../../boards/real"), &ids).unwrap();
    let (old_specs, _) =
        parse_board_files(&read("../../boards/real-before-counters"), &ids).unwrap();
    let new = RealBoards::build(&new_specs, &d.cards, &d.gifts).0;
    let old = RealBoards::build(&old_specs, &d.cards, &d.gifts).0;
    // Every board of a turn plays the next eight boards of that turn, loaded
    // the old way (no numbers, no hand) both times: the opponents are the
    // same, only the board in front of them changes.
    println!("turn   boards  fights   win rate old -> new");
    let mut total = (0u32, 0u32, 0u32);
    for turn in [5, 8, 11, 14] {
        let (nb, ob) = (new.boards_at(turn), old.boards_at(turn));
        assert_eq!(nb.len(), ob.len(), "the backup holds the same boards");
        let n = nb.len();
        let (mut w_old, mut w_new, mut fights) = (0u32, 0u32, 0u32);
        for i in 0..n {
            for k in 1..=8 {
                let j = (i + k) % n;
                let them = &ob[j];
                for (mine, wins) in [(&ob[i], &mut w_old), (&nb[i], &mut w_new)] {
                    let out = fight_with_hands(
                        &d.cards,
                        &mine.minions,
                        &them.minions,
                        [&mine.hand, &them.hand],
                        [
                            fight_counters(&mine.counters, mine.tier, mine.deity),
                            fight_counters(&them.counters, them.tier, them.deity),
                        ],
                        200,
                        &mut Rng::new(1_000 * i as u64 + k as u64),
                        &mut (),
                    );
                    if out.winner == Some(0) {
                        *wins += 1;
                    }
                }
                fights += 1;
            }
        }
        println!(
            "t{turn:<3}  {n:>4}  {fights:>6}   {:.1}% -> {:.1}%",
            100.0 * w_old as f64 / fights as f64,
            100.0 * w_new as f64 / fights as f64
        );
        total = (total.0 + fights, total.1 + w_old, total.2 + w_new);
    }
    println!(
        "all           {:>6}   {:.1}% -> {:.1}%",
        total.0,
        100.0 * total.1 as f64 / total.0 as f64,
        100.0 * total.2 as f64 / total.0 as f64
    );
}


#[test]
fn a_board_file_s_dark_gifts_load_onto_its_minions() {
    // The log carries the gifts each minion had (`dark_gifts`, the gift's own
    // card_id). Their stats are already in the file's attack and health; the
    // effect comes along.
    let text = r#"{"boards": [
      {"who": "me", "turn": 6, "tier": 4, "minions": [
        {"card_id": "BG36_202", "name": "Tasty Lobster", "attack": 2, "health": 1,
         "golden": false, "keywords": [],
         "dark_gifts": ["BG36_MidGameEffect_000t73", "BG36_MidGameEffect_000t51"]}]}]}"#;
    // 000t73 is Fortitude, 000t51 Steady Growth.
    let ids = CardIdMap::parse(&std::fs::read_to_string("../../ref/bg-cards.json").unwrap())
        .unwrap();
    let (specs, _) =
        parse_board_files(&[("gifted.json".to_string(), text.to_string())], &ids).unwrap();
    assert_eq!(specs[0].minions[0].dark_gifts.len(), 2);
    let d = data();
    let (boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
    let m = &boards.boards_at(6)[0].minions[0];
    let names: Vec<String> =
        m.gift_slice().iter().map(|g| d.cards.get(g.card()).name.clone()).collect();
    assert_eq!(names, ["Gift: Fortitude", "Gift: Steady Growth"]);
}

/// A "this turn" Tavern buff that the log keeps apart (Ashen Corruptor's
/// +2/+2 per Health payment, in its own player enchantment) rides along on
/// the loaded Tavern minions and comes off at the end of the turn, while the
/// standing "this game" buff stays.
#[test]
fn a_this_turn_tavern_buff_comes_off_at_the_end_of_the_turn() {
    let data = Data::bundled();
    let cards = &data.cards;
    let mut g = Game::new_with_lobby(
        Arc::new(Data::bundled()),
        3,
        bgcore::Tribe::EVERY.into_iter().fold(bgcore::Tribes::NONE, |m: bgcore::Tribes, t| m.with(t)),
    );
    let c = cards.by_name("Cord Puller").unwrap();
    let mut m = g.players[0].make_card(cards, c, false, 0);
    let (base_a, base_h) = (m.attack, m.health);
    // The log's Tavern minion shows both buffs: +10/+10 this game and +8/+8
    // this turn (the 28/30 Hyena that goes back to 20/22 at the end of turn
    // 10 of 15_25_26-g4).
    m.attack += 18;
    m.health += 18;
    g.players[0].shop.clear();
    g.players[0].shop.push(m);
    let at = 0;
    let counters = RealCounters {
        tavern_attack: 10,
        tavern_health: 10,
        tavern_turn_attack: 8,
        tavern_turn_health: 8,
        ..RealCounters::default()
    };
    g.players[0].take_real_counters_public(&counters);
    assert_eq!(
        (g.players[0].shop[at].attack, g.players[0].shop[at].health),
        (base_a + 18, base_h + 18),
        "the loaded minion already carries both buffs once"
    );
    g.players[0].end_turn_tavern_buffs(cards);
    assert_eq!(
        (g.players[0].shop[at].attack, g.players[0].shop[at].health),
        (base_a + 10, base_h + 10),
        "the this turn half comes off, the this game half stays"
    );
}

/// bg-overlay sends a side's pending Brood of Nozdormu casts as
/// `brood_of_nozdormu` (its `BG34_889` copies in the SECRET zone,
/// `2026_11_17_00_28_04` game 1 turn 10): the seat takes the count and the
/// fight doubles the leftmost minion's Attack that many times.
#[test]
fn a_board_carries_its_brood_of_nozdormu_casts() {
    let data = Data::bundled();
    let mut c = RealCounters::default();
    c.set("brood_of_nozdormu", 2);
    assert_eq!(c.brood_of_nozdormu, 2);
    let mut g = Game::new_with_lobby(
        Arc::new(Data::bundled()),
        3,
        bgcore::Tribe::EVERY.into_iter().fold(bgcore::Tribes::NONE, |m: bgcore::Tribes, t| m.with(t)),
    );
    g.players[0].take_real_counters_public(&c);
    assert_eq!(g.players[0].counters(&data, Tribes::ALL).brood_of_nozdormu, 2);
    // The next board without the key puts it back to none.
    g.players[0].take_real_counters_public(&RealCounters::default());
    assert_eq!(g.players[0].counters(&data, Tribes::ALL).brood_of_nozdormu, 0);
}

/// Dark Paradox: all five versions are named "Dark Paradox" in the ref data,
/// golden included, and only the card id says which one; cards.toml builds
/// them as "Dark Paradox t3" to "t9", so the id map names them that way and
/// the loader builds the version the file had rather than a Stand-in
/// (docs/rules/darkgifts.md "Dark Paradox", docs/real-boards.md).
#[test]
fn dark_paradox_is_matched_by_its_card_id() {
    let ids = CardIdMap::parse(
        r#"{"cards": [
            {"card_id": "BG36_360", "name": "Dark Paradox", "is_golden": false},
            {"card_id": "BG36_360_G", "name": "Dark Paradox", "is_golden": true,
             "normal_card_id": "BG36_360"},
            {"card_id": "BG36_360t3", "name": "Dark Paradox", "is_golden": false},
            {"card_id": "BG36_360_Gt3", "name": "Dark Paradox", "is_golden": true,
             "normal_card_id": "BG36_360t3"},
            {"card_id": "BG36_360t9", "name": "Dark Paradox", "is_golden": false},
            {"card_id": "BG36_360_Gt9", "name": "Dark Paradox", "is_golden": true,
             "normal_card_id": "BG36_360t9"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(ids.get("BG36_360t3"), Some(("Dark Paradox t3", false)));
    assert_eq!(ids.get("BG36_360_Gt3"), Some(("Dark Paradox t3", true)));
    assert_eq!(ids.get("BG36_360t9"), Some(("Dark Paradox t9", false)));
    assert_eq!(ids.get("BG36_360_Gt9"), Some(("Dark Paradox t9", true)));
    // The Tier 1 placeholder is no game's card and stays unnamed by version.
    assert_eq!(ids.get("BG36_360"), Some(("Dark Paradox", false)));
    // Every version cards.toml builds is a card the engine knows, and the
    // stats are the version's own (t3 is the Tier 2, 6/2 one).
    let d = data();
    for t in ["t3", "t4", "t5", "t6", "t9"] {
        let name = format!("Dark Paradox {t}");
        let id = d.cards.by_name(&name).unwrap_or_else(|| panic!("cards.toml has {name}"));
        let m = d.cards.make(id, false);
        let golden = d.cards.make(id, true);
        assert!(golden.attack == 2 * m.attack, "{name}'s golden doubles its attack");
    }
    let t3 = d.cards.make(d.cards.by_name("Dark Paradox t3").unwrap(), false);
    assert_eq!((t3.attack, t3.health), (6, 2));
    assert_eq!(d.cards.get(d.cards.by_name("Dark Paradox t3").unwrap()).tier, 2);
}

/// Floating Candle Set: the trinket's Spellcraft spell is logged under the
/// trinket's name ("Floating Candle Set", `BG36_MagicItem_208t`), and
/// cards.toml builds the spell as the "Floating Candle" token. The id maps to
/// the name we build, so a state that holds the spell builds it instead of a
/// Stand-in (docs/rules/trinkets.md "Floating Candle Set").
#[test]
fn floating_candle_sets_spell_is_matched_by_its_card_id() {
    let ids = CardIdMap::parse(
        r#"{"cards": [
            {"card_id": "BG36_MagicItem_208t", "name": "Floating Candle Set"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(ids.get("BG36_MagicItem_208t"), Some(("Floating Candle", false)));
    // The board loader builds it: a hand holding the spell comes through with
    // the token's card, not a Stand-in.
    let json = r#"{"boards": [{"game": "g", "turn": 6, "tier": 3,
        "minions": [], "hand": [
            {"card_id": "BG36_MagicItem_208t", "name": "Floating Candle Set",
             "attack": 0, "health": 0, "golden": false}]}]}"#;
    let (specs, unknown) = parse_board_files(&[("f".to_string(), json.to_string())], &ids).unwrap();
    assert!(unknown.is_empty(), "nothing is unmatched: {unknown:?}");
    assert_eq!(specs[0].hand[0].name, "Floating Candle");
}

/// Both Jailer Sticker spells print the same name in the log ("Jailer
/// Sticker"), and the lesser (one random Undead) and the greater trinket's
/// (two) are different cards.toml tokens. Only the card id tells them apart:
/// the greater's `BG35_MagicItem_733t` maps to the "(Greater)" token, and the
/// lesser's `BG35_MagicItem_306t` keeps the plain name
/// (`Hearthstone_2026_11_03_15_22_36-g2`, the five Jailer Sticker plays).
#[test]
fn the_two_jailer_sticker_spells_are_matched_by_their_card_ids() {
    let ids = CardIdMap::parse(
        r#"{"cards": [
            {"card_id": "BG35_MagicItem_306t", "name": "Jailer Sticker"},
            {"card_id": "BG35_MagicItem_733t", "name": "Jailer Sticker"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(ids.get("BG35_MagicItem_306t"), Some(("Jailer Sticker", false)));
    assert_eq!(ids.get("BG35_MagicItem_733t"), Some(("Jailer Sticker (Greater)", false)));
    // Both are built, and the greater one is the one that gets two Undead.
    let d = data();
    for (name, count) in [("Jailer Sticker", 1), ("Jailer Sticker (Greater)", 2)] {
        let card = d.cards.by_name(name).unwrap_or_else(|| panic!("{name}"));
        match d.cards.get(card).spell_target {
            Some(bgcore::cards::SpellTarget::DestroyGetRandom { count: n, .. }) => {
                assert_eq!(n, count, "{name}")
            }
            other => panic!("{name} is not a destroy-and-get: {other:?}"),
        }
    }
}

// ---- the lobby filter and the Era seat (2026-11-09) ----

fn game_spec(
    turn: u32,
    source: &str,
    game: u32,
    side: bgcore::RealSide,
    minions: &[(&str, i32, i32, bool, &[&str])],
) -> RealBoardSpec {
    RealBoardSpec {
        source: source.to_string(),
        game,
        side,
        ..spec(turn, 1, minions)
    }
}

fn board_name(d: &Data, b: &bgcore::RealBoard) -> String {
    d.cards.get(b.minions[0].card).name.clone()
}

#[test]
fn lobby_filter_never_draws_an_off_lobby_board() {
    let d = data();
    let mut specs = vec![
        // A Mech board: off the lobby below.
        game_spec(3, "g", 1, bgcore::RealSide::Me, &[("Cord Puller", 1, 1, false, &[])]),
        // A neutral board: always passes.
        game_spec(3, "g", 1, bgcore::RealSide::Me, &[("Sewer Rat", 3, 2, false, &[])]),
        // A Demon/Quilboar board: passes only with either type in the lobby.
        game_spec(3, "g", 1, bgcore::RealSide::Me, &[("Felboar", 4, 4, false, &[])]),
    ];
    for _ in 0..8 {
        specs.push(game_spec(
            3,
            "g",
            1,
            bgcore::RealSide::Me,
            &[("Flittering Bat", 1, 4, false, &[])],
        ));
    }
    let (mut boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
    boards.set_lobby_filter(true);
    let beasts = Tribes::NONE.with(Tribe::Beast);
    let mut rng = Rng::new(7);
    let mut saw_neutral = false;
    for _ in 0..500 {
        let b = boards.draw_filtered(3, beasts, None, &mut rng);
        let name = board_name(&d, b);
        assert_ne!(name, "Cord Puller", "an off-lobby Mech board was drawn");
        assert_ne!(name, "Felboar", "a Demon/Quilboar board was drawn in a Beast lobby");
        saw_neutral |= name == "Sewer Rat";
    }
    assert!(saw_neutral, "the neutral board never passed");
    let stats = boards.filter_stats();
    assert_eq!(stats.widened, 0, "the turn's own pool was large enough");
    assert_eq!(stats.unfiltered, 0);
}

#[test]
fn lobby_filter_passes_a_dual_type_minion_on_either_type() {
    let d = data();
    let specs: Vec<RealBoardSpec> = (0..8)
        .map(|_| {
            game_spec(2, "g", 1, bgcore::RealSide::Me, &[("Felboar", 5, 5, false, &[])])
        })
        .collect();
    let (mut boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
    boards.set_lobby_filter(true);
    let mut rng = Rng::new(3);
    // Quilboar is one of Felboar's types.
    for _ in 0..50 {
        assert_eq!(board_name(&d, boards.draw_filtered(2, Tribes::NONE.with(Tribe::Quilboar), None, &mut rng)), "Felboar");
    }
    // Demon is its other type.
    for _ in 0..50 {
        assert_eq!(board_name(&d, boards.draw_filtered(2, Tribes::NONE.with(Tribe::Demon), None, &mut rng)), "Felboar");
    }
}

#[test]
fn a_thin_filtered_pool_widens_to_the_nearest_earlier_turn() {
    let d = data();
    let mut specs = Vec::new();
    for _ in 0..MIN_FILTERED_BOARDS {
        specs.push(game_spec(3, "g", 1, bgcore::RealSide::Me, &[("Flittering Bat", 1, 4, false, &[])]));
    }
    // Turn 4 has only two Beast boards: below the threshold.
    for _ in 0..2 {
        specs.push(game_spec(4, "g", 1, bgcore::RealSide::Me, &[("Humming Bird", 1, 2, false, &[])]));
    }
    let (mut boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
    boards.set_lobby_filter(true);
    boards.reset_filter_stats();
    let mut rng = Rng::new(11);
    for _ in 0..20 {
        let b = boards.draw_filtered(4, Tribes::NONE.with(Tribe::Beast), None, &mut rng);
        assert_eq!(board_name(&d, b), "Flittering Bat", "turn 4 should widen to turn 3");
    }
    let stats = boards.filter_stats();
    assert_eq!(stats.draws, 20);
    assert_eq!(stats.widened, 20);
    assert_eq!(stats.current, 0);
    assert_eq!(stats.unfiltered, 0);
}

#[test]
fn the_era_seat_repeats_its_last_board_and_cannot_draw_its_own_game() {
    let d = data();
    let specs = vec![
        game_spec(1, "era", 1, bgcore::RealSide::Me, &[("Razorfen Geomancer", 2, 1, false, &[])]),
        game_spec(3, "era", 1, bgcore::RealSide::Me, &[("Roadboar", 2, 4, false, &[])]),
        game_spec(5, "era", 1, bgcore::RealSide::Me, &[("Bonker", 9, 9, false, &[])]),
        game_spec(1, "other", 2, bgcore::RealSide::Me, &[("Sewer Rat", 3, 2, false, &[])]),
        game_spec(3, "other", 2, bgcore::RealSide::Me, &[("Cord Puller", 1, 1, false, &[])]),
        game_spec(5, "other", 2, bgcore::RealSide::Me, &[("Amber Guardian", 3, 2, false, &[])]),
    ];
    let (mut boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
    assert_eq!(boards.era_last_turn("era", 1), Some(5));
    let mut rng = Rng::new(5);
    // Turn 2 has no board of its own: the nearest earlier one.
    assert_eq!(board_name(&d, boards.draw_era("era", 1, 2, &mut rng).unwrap()), "Razorfen Geomancer");
    // Past the last turn Era really played: the last board again.
    for turn in [6, 9, 20] {
        assert_eq!(board_name(&d, boards.draw_era("era", 1, turn, &mut rng).unwrap()), "Bonker");
    }
    // The opponents' pool leaves the Era game out, filter on or off.
    for _ in 0..200 {
        let b = boards.draw_filtered(1, Tribes::NONE.with(Tribe::Beast), Some(("era", 1)), &mut rng);
        assert_eq!(board_name(&d, b), "Sewer Rat");
    }
    boards.set_lobby_filter(true);
    // With every board of the era game excluded, one of the other game's
    // boards still comes up (the filter passes a neutral one).
    for _ in 0..100 {
        let b = boards.draw_filtered(1, Tribes::NONE.with(Tribe::Beast), Some(("era", 1)), &mut rng);
        assert_eq!(board_name(&d, b), "Sewer Rat");
    }
}

#[test]
fn the_era_seat_plays_its_own_game_and_outlives_it() {
    let d = data();
    let mut specs = Vec::new();
    for turn in 1..=3 {
        specs.push(game_spec(turn, "era", 1, bgcore::RealSide::Me, &[("Flittering Bat", 400, 400, false, &[]) ]));
    }
    // Seven real opponents with tiny boards, so the Era seat survives.
    for _ in 0..7 {
        for turn in 1..=6 {
            specs.push(game_spec(turn, "other", 2, bgcore::RealSide::Me, &[("Sewer Rat", 1, 1, false, &[])]));
        }
    }
    let (boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
    let boards = Arc::new(boards);
    let real: Vec<usize> = (0..PLAYERS).collect();
    let g = Game::new_random_heroes_era(
        d.clone(),
        9,
        &real,
        boards,
        0,
        "era".to_string(),
        1,
    );
    // An all-real game plays itself as the turns start: there is no shopper
    // to step, and the game is over when the constructor returns.
    assert!(g.over, "an all-real game should have finished");
    assert!(g.turn > 3, "the era seat died too early to test the repeat");
    assert!(g.era_outlived, "the era seat did not outlive its three-turn game");
    assert_eq!(g.era_turns_added, g.turn.saturating_sub(3));
    // Its own board is the one it repeated, not a random opponent's.
    let repeated = g
        .players
        .iter()
        .flat_map(|p| p.board.iter())
        .filter(|m| d.cards.get(m.card).name == "Flittering Bat")
        .count();
    assert!(repeated >= 1, "the Era seat lost its repeated board");
}

#[test]
fn the_flag_off_draw_is_exactly_the_old_draw() {
    let d = data();
    let (boards, _) = RealBoards::build(&folder_like(), &d.cards, &d.gifts);
    let beasts = Tribes::NONE.with(Tribe::Beast);
    let mut old = Rng::new(17);
    let mut new = Rng::new(17);
    for turn in 1..=20 {
        let a = boards.draw(turn, &mut old);
        let b = boards.draw_filtered(turn, beasts, None, &mut new);
        assert_eq!(a, b, "turn {turn}");
    }
    assert_eq!(boards.filter_stats(), Default::default(), "the filter never ran");
}

// ---- the lag on a real seat's draw (2026-11-10) ----

#[test]
fn lag_zero_is_exactly_the_old_game() {
    let d = data();
    let plain = build(&folder_like());
    let mut lagged = (*plain).clone();
    lagged.set_lag(0);
    assert_eq!(lagged.lag(), 0);
    let lagged = Arc::new(lagged);
    for seed in 0..3u64 {
        let mut a = Game::new_random_heroes_real(d.clone(), seed, &[0], plain.clone());
        let mut b = Game::new_random_heroes_real(d.clone(), seed, &[0], lagged.clone());
        let mut ra = Rng::new(seed ^ 0x1a6);
        let mut rb = Rng::new(seed ^ 0x1a6);
        let mut step = 0;
        while !a.over || !b.over {
            assert_eq!(a.over, b.over, "seed {seed}: games ended on different steps");
            let aa = random_action(&a, &mut ra);
            let ab = random_action(&b, &mut rb);
            assert_eq!(aa, ab, "seed {seed} step {step}: actions differ");
            a.step(aa).unwrap();
            b.step(ab).unwrap();
            a.check_invariants().unwrap();
            b.check_invariants().unwrap();
            assert_eq!(format!("{a:?}"), format!("{b:?}"),
                       "seed {seed} step {step}: lag 0 changed the game");
            step += 1;
        }
    }
}

#[test]
fn lag_three_makes_turn_eight_play_the_turn_five_board() {
    let d = data();
    // One board per turn, each with its turn in its Attack, and enough Health
    // that the real seat survives to turn 8 against the random shoppers.
    let mut specs = Vec::new();
    for turn in 1..=8u32 {
        specs.push(spec(turn, 4, &[("Bonker", 500 + turn as i32, 5000, false, &[])]));
    }
    let (mut boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
    assert_eq!(boards.lag(), 0, "the default lag is 0");
    boards.set_lag(3);
    assert_eq!(boards.lag(), 3);
    let boards = Arc::new(boards);
    let mut g = Game::new_random_heroes_real(d, 21, &[0], boards.clone());
    let mut rng = Rng::new(31);
    while g.turn < 8 && !g.over {
        let a = random_action(&g, &mut rng);
        g.step(a).unwrap();
        g.check_invariants().unwrap();
    }
    assert!(!g.over, "the real seat died before turn 8");
    assert_eq!(g.turn, 8);
    let pl = &g.players[0];
    // Lag 3 on turn 8 reads the turn 5 bucket, not turn 8's.
    let want = &boards.boards_at(5)[0];
    assert_eq!(pl.board.as_slice(), want.minions.as_slice());
    assert_eq!(pl.tier, want.tier);
    assert_eq!(pl.board[0].attack, 505);
    assert_ne!(pl.board[0].attack, 508, "turn 8 drew its own board");
}

// ---- the per-game lag mix (2026-11-10) ----

#[test]
fn the_lag_mix_draws_each_lag_at_its_share_and_follows_the_seed() {
    let mut boards = (*build(&folder_like())).clone();
    boards.set_lag_mix(&[(0, 1), (1, 2), (2, 1), (9, 0)]);
    assert_eq!(boards.lag_mix(), &[(0, 1), (1, 2), (2, 1)],
               "a zero weight is dropped");
    let n = 20_000u64;
    let mut counts = [0u32; 3];
    for seed in 0..n {
        let lag = boards.lag_for_game(seed);
        assert_eq!(lag, boards.lag_for_game(seed), "seed {seed} drew twice");
        counts[lag as usize] += 1;
    }
    // 1/4, 1/2, 1/4: generous bounds for a 20,000 draw.
    for (lag, want) in [(0, 0.25), (1, 0.5), (2, 0.25)] {
        let share = counts[lag as usize] as f64 / n as f64;
        assert!((share - want).abs() < 0.02,
                "lag {lag}: share {share:.3}, want about {want}");
    }
    // A fixed lag still wins when there is no mix.
    boards.set_lag(4);
    assert!(boards.lag_mix().is_empty());
    assert_eq!(boards.lag_for_game(1234), 4);
    // An empty or all-zero list goes back to the fixed lag.
    boards.set_lag_mix(&[(0, 0)]);
    assert_eq!(boards.lag_for_game(1234), 4);
}

#[test]
fn a_game_that_drew_lag_zero_is_the_old_game() {
    let d = data();
    let plain = build(&folder_like());
    let mut mixed = (*plain).clone();
    mixed.set_lag_mix(&[(0, 1), (1, 1)]);
    let mixed = Arc::new(mixed);
    let seed = (0..10_000u64)
        .find(|&seed| mixed.lag_for_game(seed) == 0)
        .expect("the mix never drew lag 0");
    let mut a = Game::new_random_heroes_real(d.clone(), seed, &[0], plain.clone());
    let mut b = Game::new_random_heroes_real(d, seed, &[0], mixed.clone());
    assert_eq!(a.lag, 0);
    assert_eq!(b.lag, 0);
    let mut ra = Rng::new(seed ^ 0x1a6);
    let mut rb = Rng::new(seed ^ 0x1a6);
    let mut step = 0;
    while !a.over || !b.over {
        assert_eq!(a.over, b.over, "seed {seed}: games ended on different steps");
        let aa = random_action(&a, &mut ra);
        let ab = random_action(&b, &mut rb);
        assert_eq!(aa, ab, "seed {seed} step {step}: actions differ");
        a.step(aa).unwrap();
        b.step(ab).unwrap();
        a.check_invariants().unwrap();
        b.check_invariants().unwrap();
        // The two board sets differ in their mix (the plain one has none), so
        // take them out of the debug view: everything the game did is still
        // compared.
        let aboards = a.real_boards.take();
        let bboards = b.real_boards.take();
        let da = format!("{a:?}");
        let db = format!("{b:?}");
        a.real_boards = aboards;
        b.real_boards = bboards;
        assert_eq!(da, db, "seed {seed} step {step}: a lag 0 game changed");
        step += 1;
    }
}

#[test]
fn the_same_seed_draws_the_same_game_lag() {
    let d = data();
    let mut boards = (*build(&folder_like())).clone();
    boards.set_lag_mix(&[(0, 1), (1, 1), (2, 1)]);
    let boards = Arc::new(boards);
    for seed in [0u64, 7, 14606, 999_999] {
        let a = Game::new_random_heroes_real(d.clone(), seed, &[0], boards.clone());
        let b = Game::new_random_heroes_real(d.clone(), seed, &[0], boards.clone());
        assert_eq!(a.lag, b.lag);
        assert_eq!(a.lag, boards.lag_for_game(seed));
    }
}

// ---- the first lagged turn (2026-11-10) ----

#[test]
fn lag_from_defaults_to_one_and_one_is_exactly_todays_lag() {
    let d = data();
    let plain = build(&folder_like());
    assert_eq!(plain.lag(), 0, "the default fixed lag is still 0");
    assert_eq!(plain.lag_from(), 1, "the default first lagged turn is 1");
    assert!(plain.lag_mix().is_empty());

    let mut today = (*plain).clone();
    today.set_lag(2);
    let mut explicit = (*plain).clone();
    explicit.set_lag(2);
    explicit.set_lag_from(1);
    let today = Arc::new(today);
    let explicit = Arc::new(explicit);
    for seed in 0..2u64 {
        let mut a = Game::new_random_heroes_real(d.clone(), seed, &[0], today.clone());
        let mut b = Game::new_random_heroes_real(d.clone(), seed, &[0], explicit.clone());
        let mut ra = Rng::new(seed ^ 0x1a6);
        let mut rb = Rng::new(seed ^ 0x1a6);
        let mut step = 0;
        while !a.over || !b.over {
            assert_eq!(a.over, b.over, "seed {seed}: games ended on different steps");
            let aa = random_action(&a, &mut ra);
            let ab = random_action(&b, &mut rb);
            assert_eq!(aa, ab, "seed {seed} step {step}: actions differ");
            a.step(aa).unwrap();
            b.step(ab).unwrap();
            a.check_invariants().unwrap();
            b.check_invariants().unwrap();
            assert_eq!(format!("{a:?}"), format!("{b:?}"),
                       "seed {seed} step {step}: lag_from 1 changed the game");
            step += 1;
        }
    }
}

#[test]
fn lag_from_seven_keeps_turns_one_to_six_exactly_the_plain_game() {
    let d = data();
    let plain = build(&folder_like());
    let mut late = (*plain).clone();
    late.set_lag(2);
    late.set_lag_from(7);
    let late = Arc::new(late);
    for seed in 0..2u64 {
        let mut a = Game::new_random_heroes_real(d.clone(), seed, &[0], plain.clone());
        let mut b = Game::new_random_heroes_real(d.clone(), seed, &[0], late.clone());
        let mut ra = Rng::new(seed ^ 0x1a6);
        let mut rb = Rng::new(seed ^ 0x1a6);
        let mut step = 0;
        // Every state the game shows before the turn 7 draw must match.
        while !a.over && !b.over && a.turn < 7 {
            let aa = random_action(&a, &mut ra);
            let ab = random_action(&b, &mut rb);
            assert_eq!(aa, ab, "seed {seed} step {step}: actions differ");
            a.step(aa).unwrap();
            b.step(ab).unwrap();
            a.check_invariants().unwrap();
            b.check_invariants().unwrap();
            if a.turn < 7 {
                // The two board sets differ only in their lag settings, which
                // the turn 7 draw is allowed to feel: take them, and the
                // game's drawn lag, out of the debug view. Everything the game
                // did is still compared.
                let aboards = a.real_boards.take();
                let bboards = b.real_boards.take();
                let blag = b.lag;
                b.lag = a.lag;
                let da = format!("{a:?}");
                let db = format!("{b:?}");
                a.real_boards = aboards;
                b.real_boards = bboards;
                b.lag = blag;
                assert_eq!(da, db, "seed {seed} step {step}: lag_from 7 changed an early turn");
            }
            step += 1;
        }
        assert!(a.turn >= 7, "seed {seed}: the game ended before turn 7");
        assert!(b.turn >= 7, "seed {seed}: the game ended before turn 7");
    }
}

#[test]
fn lag_from_seven_draws_the_turn_five_board_on_turn_seven() {
    let d = data();
    // One board per turn, each with its turn in its Attack, and enough Health
    // that the real seat survives past turn 7.
    let mut specs = Vec::new();
    for turn in 1..=9u32 {
        specs.push(spec(turn, 4, &[("Bonker", 500 + turn as i32, 5000, false, &[])]));
    }
    for mixed in [false, true] {
        let (mut boards, _) = RealBoards::build(&specs, &d.cards, &d.gifts);
        if mixed {
            boards.set_lag_mix(&[(2, 1)]);
        } else {
            boards.set_lag(2);
        }
        boards.set_lag_from(7);
        let boards = Arc::new(boards);
        let mut g = Game::new_random_heroes_real(d.clone(), 21, &[0], boards.clone());
        let mut rng = Rng::new(31);
        while g.turn < 6 && !g.over {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
        assert!(!g.over, "the real seat died before turn 6");
        assert_eq!(g.turn, 6);
        assert_eq!(g.players[0].board[0].attack, 506, "turn 6 was lagged");
        while g.turn < 7 && !g.over {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
        assert!(!g.over, "the real seat died before turn 7");
        assert_eq!(g.turn, 7);
        let pl = &g.players[0];
        let want = &boards.boards_at(5)[0];
        assert_eq!(pl.board.as_slice(), want.minions.as_slice());
        assert_eq!(pl.tier, want.tier);
        assert_eq!(pl.board[0].attack, 505, "turn 7 did not draw the turn 5 board");
        assert_ne!(pl.board[0].attack, 507, "turn 7 drew its own board");
    }
}

#[test]
fn lag_in_obs_off_is_the_old_observation_and_on_inserts_one_scalar() {
    use bgcore::obs::{
        self, OBS_SIZE, OBS_SIZE_WITH_LAG, OWN_END, OWN_END_WITH_LAG, SCALARS, SCALARS_WITH_LAG,
        SCALAR_LAG,
    };
    use bgcore::obs::OldBoards;

    let d = data();
    let mut g = Game::new_random_heroes(d, 17);
    g.lag = 5;
    let mut off = vec![0.0f32; OBS_SIZE];
    obs::write_with(&g, 0, &mut off, OldBoards::Keep);
    let mut plain = vec![0.0f32; OBS_SIZE];
    obs::write(&g, 0, &mut plain);
    assert_eq!(off, plain);
    // The option off never reads the lag.
    g.lag = 0;
    let mut zero = vec![0.0f32; OBS_SIZE];
    obs::write_with(&g, 0, &mut zero, OldBoards::Keep);
    assert_eq!(off, zero, "the old observation does not carry the lag");
    g.lag = 5;
    let mut on = vec![0.0f32; OBS_SIZE_WITH_LAG];
    obs::write_with_lag(&g, 0, &mut on, OldBoards::Keep);
    assert_eq!(on.len(), off.len() + 1);
    assert_eq!(&on[..SCALARS], &off[..SCALARS]);
    assert_eq!(on[SCALAR_LAG], 5.0);
    assert_eq!(&on[SCALARS_WITH_LAG..OWN_END_WITH_LAG], &off[SCALARS..OWN_END]);
    assert_eq!(&on[OWN_END_WITH_LAG..], &off[OWN_END..]);
}

