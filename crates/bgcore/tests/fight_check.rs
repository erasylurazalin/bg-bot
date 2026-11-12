//! The fight check (docs/real-boards.md "The fight check"): a real fight
//! seated in the engine exactly as the file describes it, and the three files
//! the checker writes from the fixture folder.

use std::sync::Arc;

use bgcore::fight_check::{
    CheckOptions, PreparedFight as FightPrepared, check_all, missing_kind, prepare,
};
use bgcore::real_boards::{CardIdMap, pair_fights, parse_board_files};
use bgcore::{Data, Keywords};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn fixture_files() -> Vec<(String, String)> {
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
            (p.file_name().unwrap().to_str().unwrap().to_string(), std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    files.sort();
    files
}

fn ids() -> CardIdMap {
    CardIdMap::parse(&std::fs::read_to_string("../../ref/bg-cards.json").unwrap()).unwrap()
}

/// One game with both sides of two fights, in the file's format: a hero power,
/// trinkets, a Dark Gift, hand cards, counters, Health and armor.
const TWO_FIGHTS: &str = r#"
{"source": "fight-check-test", "game": 1, "place": 2, "boards": [
 {"who": "me", "turn": 7, "result": "win", "hero": "Al'Akir",
  "hero_card": "TB_BaconShop_HERO_76", "hero_base": "TB_BaconShop_HERO_76",
  "tier": 4, "health": 26, "armor": 5,
  "hero_power": {"card_id": "TB_BaconShop_HP_086", "name": "Swatting Insects",
                 "numbers": [0, 0, 0, 0]},
  "trinkets": [],
  "minions": [{"card_id": "BG26_147", "name": "Accord-o-Tron", "attack": 7,
               "health": 6, "golden": true, "keywords": ["TAUNT", "DIVINE_SHIELD"],
               "dark_gifts": ["BG36_MidGameEffect_000t"], "numbers": []}],
  "counters": {"gem_attack": 4, "gem_health": 3, "spells_cast": 6},
  "deity": null,
  "hand": [{"card_id": "BGS_115t", "name": "Water Droplet", "attack": 2,
            "health": 2, "golden": false, "keywords": [], "dark_gifts": [],
            "numbers": []}],
  "hand_hidden": 0},
 {"who": "opp", "turn": 7, "result": "lose", "hero": "The Great Akazamzarak",
  "hero_card": "TB_BaconShop_HERO_21", "hero_base": "TB_BaconShop_HERO_21",
  "tier": 4, "health": 30, "armor": 12,
  "hero_power": null,
  "trinkets": [{"card_id": "BG35_MagicItem_711", "name": "Protective Ring",
                "greater": true, "numbers": [4, 0, 0, 0]}],
  "minions": [{"card_id": "BGS_115t", "name": "Water Droplet", "attack": 3,
               "health": 3, "golden": false, "keywords": [], "dark_gifts": [],
               "numbers": []}],
  "counters": {}, "deity": null, "hand": [], "hand_hidden": 0},
 {"who": "me", "turn": 8, "result": "lose", "hero": "Al'Akir",
  "hero_base": "TB_BaconShop_HERO_76", "tier": 4, "health": 24, "armor": 5,
  "hero_power": null, "trinkets": [], "minions": [], "counters": {},
  "deity": null, "hand": [], "hand_hidden": 0},
 {"who": "opp", "turn": 8, "result": "win", "hero": "The Great Akazamzarak",
  "hero_base": "TB_BaconShop_HERO_21", "tier": 4, "health": 30, "armor": 12,
  "hero_power": null, "trinkets": [], "minions": [], "counters": {},
  "deity": null, "hand": [], "hand_hidden": 0}]}
"#;

/// The two boards of the fight above, parsed and paired.
fn prepared_fight() -> (Arc<Data>, Vec<bgcore::RealFightSpec>) {
    let files = vec![("fight-check-test-g1.json".to_string(), TWO_FIGHTS.to_string())];
    let (specs, unknown) = parse_board_files(&files, &ids()).unwrap();
    assert!(unknown.is_empty(), "unexpected unknown ids: {unknown:?}");
    let (fights, unpaired) = pair_fights(specs);
    assert!(unpaired.is_empty(), "unexpected unpaired boards: {unpaired:?}");
    assert_eq!(fights.len(), 2);
    (data(), fights)
}

/// A fight set up from a board file matches the board: stats, keywords, the
/// Dark Gift, the hand, the side-wide numbers, the hero power and the
/// trinkets, and the damage the fight did can be read off the next board.
#[test]
fn a_board_file_seats_the_fight_it_describes() {
    let (d, fights) = prepared_fight();
    let f = &fights[0];
    assert_eq!(f.file(), "fight-check-test-g1.json");
    assert_eq!(f.turn(), 7);

    let p = prepare(&d, f);
    assert!(p.is_complete(), "nothing should be missing: {:?}", p.missing_lines());

    // Era's side: the minion's own numbers, its keywords, its gift, the hand
    // and the Tavern tier.
    let me = &p.sides[0];
    assert_eq!(me.board.len(), 1);
    let m = &me.board[0];
    assert_eq!(d.cards.get(m.card).name, "Accord-o-Tron");
    assert_eq!((m.attack, m.health), (7, 6));
    assert!(m.golden);
    assert!(m.keywords.has(Keywords::TAUNT));
    assert!(m.keywords.has(Keywords::DIVINE_SHIELD));
    assert_eq!(m.gift_slice().len(), 1, "the Dark Gift is attached");
    assert_eq!(
        d.cards.get(m.gift_slice()[0].card()).name,
        "Gift: Offensive Sacrifice"
    );
    assert_eq!(me.hand.len(), 1);
    assert_eq!(d.cards.get(me.hand[0].card).name, "Water Droplet");
    // The numbers a fight reads off the player.
    assert_eq!(me.counters.tier, 4);
    assert_eq!(me.counters.health, Some(26));
    assert_eq!((me.counters.gem_attack, me.counters.gem_health), (4, 3));
    assert_eq!(me.counters.spells_cast, 6);
    // Al'Akir's Swatting Insects: the left-most minion's keyword mask.
    assert_ne!(me.counters.soc_leftmost, 0, "the Start of Combat power runs");

    // The opponent: no power, one greater trinket with a fight effect.
    let opp = &p.sides[1];
    assert_eq!(opp.counters.health, Some(30));
    assert_eq!(opp.counters.trinkets.len(), 1, "Protective Ring fights");
    assert_eq!(opp.counters.tier, 4);

    // The fight after this one wrote the Health and the armor Era lost, so the
    // damage the file shows is the drop between the two boards.
    assert_eq!(f.next, Some((8, 24, 5)));
    assert_eq!(f.real_damage(), Some(2), "26 + 5 down to 24 + 5");
}

/// Something the engine does not build leaves the fight incomplete, with what
/// was missing named, and a Secret-granting power counts as one (the file
/// carries no Secret zone).
#[test]
fn what_the_engine_does_not_build_is_named() {
    let files = vec![(
        "fight-check-gaps-g1.json".to_string(),
        r#"{"source": "fight-check-gaps", "game": 1, "boards": [
 {"who": "me", "turn": 5, "result": "win", "tier": 3, "health": 30, "armor": 9,
  "hero_power": {"card_id": "TB_BaconShop_HP_020", "name": "Prestidigitation",
                 "numbers": [0, 0, 0, 0]},
  "trinkets": [{"card_id": "BG35_MagicItem_9999", "name": "Unbuilt Trinket",
                "greater": false, "numbers": []}],
  "rewards": [{"card_id": "BG24_Quest_000", "name": "A Quest Reward",
               "numbers": [1, 0, 0, 0]}],
  "minions": [{"card_id": "BGS_115t", "name": "Water Droplet", "attack": 2,
               "health": 2, "golden": false, "keywords": [],
               "dark_gifts": [], "numbers": []}],
  "counters": {}, "deity": null, "hand": [], "hand_hidden": 2},
 {"who": "opp", "turn": 5, "result": "lose", "tier": 3, "health": 30, "armor": 9,
  "minions": [], "counters": {}, "deity": null, "hand": [], "hand_hidden": 0}]}"#
            .to_string(),
    )];
    let (specs, _) = parse_board_files(&files, &ids()).unwrap();
    let (fights, unpaired) = pair_fights(specs);
    assert!(unpaired.is_empty());
    let d = data();
    let p = prepare(&d, &fights[0]);
    assert!(!p.is_complete());
    let lines = p.missing_lines();
    let kinds: Vec<&str> = lines.iter().map(|l| missing_kind(l)).collect();
    assert!(kinds.contains(&"trinket not built"), "{lines:?}");
    assert!(kinds.contains(&"quest reward not modelled"), "{lines:?}");
    assert!(kinds.contains(&"Secrets not in the file"), "{lines:?}");
    assert_eq!(
        lines.iter().filter(|l| l.contains("card(s) in hand")).count(),
        1,
        "{lines:?}"
    );
}

/// The checker runs on the fixture folder and writes the three files with the
/// shape the brief asks for.
#[test]
fn the_checker_runs_on_the_fixture_folder() {
    let d = data();
    let (specs, unknown) = parse_board_files(&fixture_files(), &ids()).unwrap();
    assert_eq!(unknown, ["BG20_100t"]);
    let (fights, unpaired) = pair_fights(specs);
    assert!(unpaired.is_empty(), "{unpaired:?}");
    assert_eq!(fights.len(), 6, "three games, six fights");
    let opts = CheckOptions { runs: 20, ..CheckOptions::default() };
    let mut report = check_all(&d, &fights, &opts);
    report.unknown_minions = unknown;
    assert_eq!(report.entries.len(), 6);
    // Every entry ran the asked number of times and nothing else.
    for e in &report.entries {
        assert_eq!(e.check.wins + e.check.ties + e.check.losses, 20);
        assert_eq!(e.check.complete, e.check.missing.is_empty());
    }
    assert!(report.complete() > 0, "the fixture has fights the engine can play");
    assert!(report.complete() < 6, "and fights it cannot (a Stand-in, a hidden hand card)");

    // fights.tsv: a header and one row per fight, the same number of columns.
    let tsv = report.fights_tsv();
    let mut lines = tsv.lines();
    let header = lines.next().unwrap();
    assert_eq!(header.split('\t').count(), 16, "{header}");
    assert!(header.starts_with("file\tgame\tturn\treal\t"));
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 6);
    for row in &rows {
        assert_eq!(row.split('\t').count(), 16, "{row}");
        let f: Vec<&str> = row.split('\t').collect();
        assert!(f[0].ends_with(".json"));
        assert!(matches!(f[3], "win" | "lose" | "tie"));
        assert!(matches!(f[8], "yes" | "no"));
        // A percentage for the real result, or a dash when it never happened.
        assert!(f[7] == "-" || f[7].parse::<f64>().is_ok(), "{row}");
    }

    // cards.tsv: a header and rows of (kind, name, counts, ratio).
    let cards = report.cards_tsv();
    let mut lines = cards.lines();
    assert_eq!(
        lines.next().unwrap(),
        "kind\tname\tfights\tcomplete\tflagged\tflag_pct"
    );
    let mut card_rows = 0;
    for row in lines {
        let f: Vec<&str> = row.split('\t').collect();
        assert_eq!(f.len(), 6, "{row}");
        assert!(
            ["minion", "hero power", "trinket", "dark gift", "quest reward"].contains(&f[0]),
            "{row}"
        );
        assert!(!f[1].is_empty());
        assert!(f[5].parse::<f64>().is_ok(), "{row}");
        card_rows += 1;
    }
    assert!(card_rows > 10, "the check tallies the cards: {card_rows}");
    let resort: Vec<f64> =
        report.pieces.iter().map(|t| t.ratio()).collect();
    assert!(resort.windows(2).all(|w| w[0] >= w[1]), "most suspicious first");

    // report.md: the summary, the calibration table and the surprising fights.
    let md = report.report_md(&d);
    assert!(md.starts_with("# The fight check"));
    assert!(md.contains("- fights: **6**"));
    assert!(md.contains("## Calibration"));
    assert_eq!(md.matches("| 0 to 10% |").count(), 1);
    assert!(md.contains("## The 30 most surprising complete fights"));
    assert!(md.contains("### 1. `"), "{md}");
    // The known-gaps list names the minion whose id no source knows.
    assert!(md.contains("minion not built"));
}

/// Rapid Reanimation's mark (the file's `reanimate`) reaches the minion the
/// power destroys: the board's own minion and not its neighbour, with the
/// armed power next to it.
#[test]
fn a_board_file_marks_the_minion_rapid_reanimation_chose() {
    let files = vec![(
        "fight-check-reanimate-g1.json".to_string(),
        r#"{"source": "fight-check-reanimate", "game": 1, "boards": [
 {"who": "me", "turn": 5, "result": "win", "tier": 3, "health": 30, "armor": 9,
  "minions": [{"card_id": "BGS_115t", "name": "Water Droplet", "attack": 2,
               "health": 2, "golden": false, "keywords": [],
               "dark_gifts": [], "numbers": []}],
  "counters": {}, "deity": null, "hand": [], "hand_hidden": 0},
 {"who": "opp", "turn": 5, "result": "lose", "tier": 3, "health": 30, "armor": 9,
  "hero_power": {"card_id": "BG25_HERO_103p", "name": "Rapid Reanimation",
                 "numbers": [0, 0, 0, 0]},
  "minions": [{"card_id": "BGS_115t", "name": "Water Droplet", "attack": 2,
               "health": 2, "golden": false, "keywords": [],
               "dark_gifts": [], "numbers": []},
              {"card_id": "BG25_001", "name": "Risen Rider", "attack": 2,
               "health": 1, "golden": false, "keywords": ["TAUNT", "REBORN"],
               "dark_gifts": [], "numbers": [], "reanimate": true}],
  "counters": {}, "deity": null, "hand": [], "hand_hidden": 0}]}"#
            .to_string(),
    )];
    let (specs, _) = parse_board_files(&files, &ids()).unwrap();
    assert!(!specs[0].minions[0].reanimate, "the key is read, not assumed");
    assert!(specs[1].minions[1].reanimate, "the file's reanimate is read");
    let (fights, unpaired) = pair_fights(specs);
    assert!(unpaired.is_empty());
    let d = data();
    let p = prepare(&d, &fights[0]);
    assert!(p.is_complete(), "{:?}", p.missing_lines());
    let opp = &p.sides[1];
    assert_eq!(opp.board.len(), 2);
    assert!(!opp.board[0].reanimate);
    assert!(opp.board[1].reanimate, "the marked minion is the one the power chose");
    assert!(opp.board[1].keywords.has(Keywords::REBORN), "the mark keeps its keywords");
    assert_eq!(
        opp.counters.rapid_reanimation, 1,
        "the power is armed, so the fight has its target"
    );
    // Era's side carries neither.
    assert!(p.sides[0].board.iter().all(|m| !m.reanimate));
    assert_eq!(p.sides[0].counters.rapid_reanimation, 0);
}

/// One fight of Era's side against an empty board, from an inline file, so a
/// power's effect can be read off the seated counters.
fn side_of(power: &str, extra: &str) -> (Arc<Data>, FightPrepared) {
    let text = format!(
        r#"{{"source": "fight-check-one", "game": 1, "boards": [
 {{"who": "me", "turn": 5, "result": "win", "tier": 3, "health": 30, "armor": 9,
  "hero_power": {power},
  "minions": [{{"card_id": "BGS_115t", "name": "Water Droplet", "attack": 2,
               "health": 2, "golden": false, "keywords": [],
               "dark_gifts": [], "numbers": []}}],
  "counters": {{}}, "deity": null, "hand": [], "hand_hidden": 0{extra}}},
 {{"who": "opp", "turn": 5, "result": "lose", "tier": 3, "health": 30, "armor": 9,
  "minions": [], "counters": {{}}, "deity": null, "hand": [], "hand_hidden": 0}}]}}"#
    );
    let files = vec![("fight-check-one-g1.json".to_string(), text)];
    let (specs, _) = parse_board_files(&files, &ids()).unwrap();
    let (fights, unpaired) = pair_fights(specs);
    assert!(unpaired.is_empty());
    let d = data();
    let p = prepare(&d, &fights[0]);
    (d, p)
}

/// Ozumat's power number is the Tentacle's Attack now, so the fight gets the
/// sales its Tentacle grows by.
#[test]
fn ozumats_power_number_seats_the_tentacle_growth() {
    let (d, p) = side_of(
        r#"{"card_id": "BG23_HERO_201p", "name": "Tentacular", "numbers": [5, 0, 0, 0], "waiting": null}"#,
        "",
    );
    assert!(p.is_complete(), "{:?}", p.missing_lines());
    let me = &p.sides[0];
    assert_eq!(me.counters.minions_sold, 3, "a 5/5 Tentacle means three sales");
    assert_eq!(
        me.counters.tentacular,
        Some((d.cards.by_name("Ozumat's Tentacle").unwrap(), 1))
    );

    // A file with no number for the power cannot say, and says so.
    let (_, p) = side_of(
        r#"{"card_id": "BG23_HERO_201p", "name": "Tentacular", "numbers": [], "waiting": null}"#,
        "",
    );
    assert!(!p.is_complete());
    assert!(
        p.missing_lines().iter().any(|l| l.contains("Ozumat")),
        "{:?}",
        p.missing_lines()
    );
}

/// Lock and Load's stored minion comes into the fight as the engine's
/// `locked_and_loaded`: the side with one is complete, an empty `waiting` is
/// complete too (the power owes nothing), and a file that cannot say is not.
#[test]
fn lock_and_load_seats_the_minion_it_holds() {
    let waiting = r#"{"card_id": "BGS_115t", "name": "Water Droplet", "attack": 4,
                      "health": 5, "golden": false, "keywords": ["TAUNT"],
                      "dark_gifts": [], "numbers": []}"#;
    let (d, p) = side_of(
        &format!(
            r#"{{"card_id": "BG22_HERO_000p_Alt", "name": "Lock and Load",
                 "numbers": [1, 0, 0, 0], "waiting": {waiting}}}"#
        ),
        "",
    );
    assert!(p.is_complete(), "{:?}", p.missing_lines());
    let loaded = p.sides[0].counters.locked_and_loaded.expect("the shot is seated");
    assert_eq!(loaded.0, d.cards.by_name("Water Droplet").unwrap());
    assert_eq!((loaded.1, loaded.2, loaded.3), (false, 4, 5), "its stats, not the card's");
    assert!(loaded.4.has(Keywords::TAUNT));
    assert_eq!(loaded.5, 1, "one shot");

    // Known and empty: the power owes nothing this fight, so it adds nothing.
    let (_, p) = side_of(
        r#"{"card_id": "BG22_HERO_000p_Alt", "name": "Lock and Load", "numbers": [1, 0, 0, 0], "waiting": null}"#,
        "",
    );
    assert!(p.is_complete(), "{:?}", p.missing_lines());
    assert!(p.sides[0].counters.locked_and_loaded.is_none());

    // An older file has no key at all, so the fight cannot be trusted.
    let (_, p) = side_of(
        r#"{"card_id": "BG22_HERO_000p_Alt", "name": "Lock and Load", "numbers": [1, 0, 0, 0]}"#,
        "",
    );
    assert!(!p.is_complete());
    assert!(
        p.missing_lines().iter().any(|l| l.contains("locked in hand")),
        "{:?}",
        p.missing_lines()
    );
}

/// A side's Secrets reach the player's zone in the order the file writes them,
/// and a Prestidigitation board with a zone is complete.
#[test]
fn the_secret_zone_reaches_the_fight_in_order() {
    let (_, p) = side_of(
        r#"{"card_id": "TB_BaconShop_HP_020", "name": "Prestidigitation", "numbers": [0, 0, 0, 0], "waiting": null}"#,
        r#", "secrets": ["TB_Bacon_Secrets_07", "TB_Bacon_Secrets_10", "TB_Bacon_Secrets_12"]"#,
    );
    assert!(p.is_complete(), "{:?}", p.missing_lines());
    // Autodefense Matrix (1), Redemption (3), and the Safety Patch's Ice Block
    // (4), in that order.
    let kinds: Vec<u8> = p.sides[0].counters.secrets.iter().collect();
    assert_eq!(kinds, [1, 3, 4], "same order, and the Ice Block is kept");
    assert_eq!(p.sides[1].counters.secrets.len(), 0);

    // An empty zone is a zone too: the power was simply not pressed.
    let (_, p) = side_of(
        r#"{"card_id": "TB_BaconShop_HP_020", "name": "Prestidigitation", "numbers": [0, 0, 0, 0], "waiting": null}"#,
        r#", "secrets": []"#,
    );
    assert!(p.is_complete(), "{:?}", p.missing_lines());
    assert_eq!(p.sides[0].counters.secrets.len(), 0);

    // A Secret card the engine does not model is named.
    let (_, p) = side_of(
        r#"{"card_id": "TB_BaconShop_HP_020", "name": "Prestidigitation", "numbers": [0, 0, 0, 0], "waiting": null}"#,
        r#", "secrets": ["TB_Bacon_Secrets_99"]"#,
    );
    assert!(!p.is_complete());
    assert!(
        p.missing_lines().iter().any(|l| l.contains("Secret not modelled")),
        "{:?}",
        p.missing_lines()
    );
}
