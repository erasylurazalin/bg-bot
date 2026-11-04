//! The Battlegrounds views on snippets cut from Era's logs.
//!
//! The fixtures are lines from real games, with the player names and account
//! ids replaced, and they carry nothing personal:
//!
//! | fixture | `Power.log` folder |
//! |---|---|
//! | `gifts.log` | `2026_10_24_00_56_06`: 2-3, 88858-88931: a minion, one Sharpened Sword carrier and the buff it left |
//! | `gift_twice.log` | `2026_11_02_00_10_22` game 5: the golden Clever Castaway with two Toughened Shield carriers and their two buffs |
//! | `gift_buffs.log` | `2026_11_02_00_10_22` game 1: an Incubation carrier with its two buffs |
//! | `gift_copies.log` | `2026_10_26_00_48_15`: a golden Cadaver Caretaker and the two Replication carriers the game copied onto it for the triple |
//! | `fight.log` | the same game, in pieces: the setup, one fight and its first attack (see the file's own lines) |
//! | `views.log` | the same game: a leaderboard hero, a hero power, both trinket slots, a hand minion, the spell count |
//! | `deity.log` | `2026_10_24_15_25_26`: the `BG_OldGod` entity |
//! | `waiting.log` | `2026_10_26_00_48_15`: a Diremuck Forager enchantment in PLAY on its player |

use std::fs;

use bglog::{
    Cards, Game, PowerLog, TagValue, Tracker, board, counts, deity, gem, gifts_by_minion,
    hand, hero_power, hidden_in_hand, one_minion, rewards, trinkets, turn_of, waiting,
};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn read(name: &str) -> PowerLog {
    let text = fixture(name);
    let mut lines = text.as_bytes();
    let mut log = PowerLog::new();
    log.feed_reader(&mut lines, &mut ())
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    log
}

fn tracker(name: &str) -> Tracker {
    let text = fixture(name);
    let mut tracker = Tracker::new(Cards::default());
    for line in text.lines() {
        tracker.feed(line);
    }
    tracker
}

#[test]
fn a_gift_the_minion_carries_twice_counts_twice() {
    // Entity 10380, the golden Clever Castaway of game 5: two Toughened Shield
    // carriers (10381 and 10384, one from each of the two gift cards that made
    // the golden) and the two "+0/+0" buffs they left (10382, 10385). The
    // Python kept one id per gift; we count the carriers.
    let log = read("gift_twice.log");
    let game = log.game().unwrap();
    assert_eq!(
        gifts_by_minion(game).get(&10380),
        Some(&vec![
            "BG36_MidGameEffect_000t75".to_string(),
            "BG36_MidGameEffect_000t75".to_string(),
        ])
    );
    let boards = board(game, &Cards::default(), Some(6));
    assert_eq!(boards.len(), 1);
    let minion = &boards[0];
    assert_eq!(minion.card_id, "BG36_342_G");
    assert!(minion.golden);
    assert_eq!(minion.dark_gifts.len(), 2);
    // Both buffs are attached to the minion all the same; they are not gifts.
    for buff in [10382, 10385] {
        let buff = &game.entities[&buff];
        assert_eq!(buff.text("ZONE"), "PLAY");
        assert_eq!(buff.int("ATTACHED"), 10380);
        assert_eq!(buff.int("IS_NIGHTMARE_BONUS"), 0);
    }
}

#[test]
fn two_carriers_the_game_copied_from_different_gifts_count_twice() {
    // Entity 4024, the golden Cadaver Caretaker of `2026_10_26_00_48_15`, as
    // the triple is made (01:21:51): two Replication carriers (4025 and 4026),
    // both naming the gift card 1720 as their CREATOR, copied from 3770 and
    // 3771. Counting creators alone makes them one gift, which is what the
    // collected state said; the golden carries two, and at the end of its turn
    // it pays out two plain copies (entities 5585 and 5587, 01:23:57).
    let log = read("gift_copies.log");
    let game = log.game().unwrap();
    assert_eq!(
        gifts_by_minion(game).get(&4024),
        Some(&vec![
            "BG36_MidGameEffect_000t18".to_string(),
            "BG36_MidGameEffect_000t18".to_string(),
        ])
    );
    for (carrier, copied_from) in [(4025, 3770), (4026, 3771)] {
        let x = &game.entities[&carrier];
        assert_eq!(x.text("ZONE"), "PLAY");
        assert_eq!(x.int("ATTACHED"), 4024);
        assert_eq!(x.int("CREATOR"), 1720, "one gift card made both");
        assert_eq!(x.int("COPIED_FROM_ENTITY_ID"), copied_from);
        assert_eq!(x.int("IS_NIGHTMARE_BONUS"), 1);
    }
}

#[test]
fn one_gift_and_two_of_its_buffs_count_once() {
    // Entity 11308: one Incubation carrier (11309, which carries the gift tag)
    // and two buffs it left, 11310 (the "+0/+0") and 11312 ("Stats set to
    // 0/0", the doubling half way through).
    let log = read("gift_buffs.log");
    let game = log.game().unwrap();
    // The minion itself is still in SETASIDE in this snippet; its three
    // enchantments are in PLAY, which is what the gifts are read from.
    assert_eq!(
        gifts_by_minion(game).get(&11308),
        Some(&vec!["BG36_MidGameEffect_000t4".to_string()])
    );
    let gifts = gifts_by_minion(game);
    let carried = one_minion(&game.entities[&11308], game, &Cards::default());
    assert_eq!(carried.dark_gifts.len(), 1);
    assert_eq!(gifts.get(&11308).map(Vec::len), Some(1));
    // The carrier is the one with the gift tag; the two buffs have no tag. The
    // fight copies point their CREATOR at ids this game never made, and the
    // "+0/+0" buff shares its carrier's creator, so counting creators alone
    // would keep them apart wrongly; the tag is what settles it.
    let carrier = &game.entities[&11309];
    assert_eq!(carrier.int("IS_NIGHTMARE_BONUS"), 1);
    for buff in [11310, 11312] {
        let buff = &game.entities[&buff];
        assert_eq!(buff.text("ZONE"), "PLAY");
        assert_eq!(buff.int("ATTACHED"), 11308);
        assert_eq!(buff.int("IS_NIGHTMARE_BONUS"), 0);
    }
}

#[test]
fn a_gift_and_the_one_buff_it_left_count_once() {
    // Entity 5422: one Sharpened Sword carrier (5423) and its "+0/+0" buff
    // (5424).
    let log = read("gifts.log");
    let game = log.game().unwrap();
    assert_eq!(
        gifts_by_minion(game).get(&5422),
        Some(&vec!["BG36_MidGameEffect_000t74".to_string()])
    );
    let boards = board(game, &Cards::default(), Some(9));
    assert_eq!(boards.len(), 1);
    let minion = &boards[0];
    assert_eq!(minion.card_id, "BG28_309");
    // No name data in the test: the card id stands in for the name.
    assert_eq!(minion.name, "BG28_309");
    assert_eq!((minion.attack, minion.health, minion.tier), (5, 2, 3));
    assert!(!minion.golden);
    assert_eq!(
        minion.dark_gifts,
        vec!["BG36_MidGameEffect_000t74".to_string()]
    );
    // DEATHRATTLE is a tag but not one of the keywords the views keep.
    assert!(minion.keywords.is_empty());
    assert_eq!(minion.tags.get("DEATHRATTLE"), Some(&TagValue::Int(1)));
}

#[test]
fn turn_of_counts_recruit_and_combat_as_one_round() {
    let log = read("fight.log");
    let game = log.game().unwrap();
    assert_eq!(game.game_tag("TURN"), 1);
    assert_eq!(turn_of(game), 1);
}

#[test]
fn a_fight_is_snapped_when_it_is_staged_and_again_at_the_first_attack() {
    let tracker = tracker("fight.log");
    assert_eq!(tracker.games().len(), 1);
    let fights = tracker.fights();
    assert_eq!(fights.len(), 1);
    let fight = &fights[0];
    assert_eq!(fight.turn, 1);
    // The stand-in player's BACON_CURRENT_COMBAT_PLAYER_ID says who we fight.
    assert_eq!(fight.opponent, 4);
    // Both boards as the window shows them...
    assert_eq!(fight.mine.len(), 1);
    assert_eq!(fight.mine[0].card_id, "BG36_345");
    assert_eq!(fight.theirs.len(), 1);
    assert_eq!(fight.theirs[0].card_id, "BG28_300");
    // ...and as they stood at the first attack, with who swung first.
    let (mine, theirs) = fight.at_attack.as_ref().expect("a snapshot at the attack");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].card_id, "BG36_345");
    assert_eq!(theirs.len(), 1);
    assert_eq!(fight.first, "me");
    // The first attack was a minion's, not a hero's, so nothing was won.
    assert_eq!(fight.result, "tie");
    assert_eq!(fight.gems, ((1, 1), (1, 1)));
    assert_eq!(fight.hidden, (0, 0));
    assert_eq!(fight.opening, None);
    // The fight is also the latest one.
    assert_eq!(tracker.fight().map(|f| f.turn), Some(1));
    assert!(tracker.version() > 0);
}

#[test]
fn a_staged_fight_carries_what_only_exists_before_start_of_combat() {
    // The board file reads these as the fight is staged and at the first
    // attack; `fight.log` has neither a Secret, a reanimation mark, an Upper
    // Hand nor a held minion, so they are the empty readings.
    let tracker = tracker("fight.log");
    let fight = &tracker.fights()[0];
    assert_eq!(fight.reanimates.0.len(), fight.mine.len());
    assert_eq!(fight.reanimates.1.len(), fight.theirs.len());
    assert!(fight.reanimates.0.iter().all(|marked| !marked));
    assert!(fight.secrets.0.is_empty() && fight.secrets.1.is_empty());
    assert_eq!(fight.upper_hands, (0, 0));
    assert_eq!(fight.waiting, (None, None));
    // The first attack captured both sides' player-level numbers.
    assert!(fight.at_attack.is_some());
    assert!(fight.side_numbers.0.is_empty() && fight.side_numbers.1.is_empty());
}

#[test]
fn a_seat_comes_from_the_leaderboard_hero() {
    let tracker = tracker("views.log");
    let seats = tracker.seats();
    assert_eq!(seats.len(), 1);
    let seat = &seats[&1];
    assert_eq!(seat.player_id, 1);
    assert_eq!(seat.hero_card, "BG33_HERO_001");
    assert_eq!(seat.hero, "BG33_HERO_001");
    assert_eq!(seat.health, 30);
    assert_eq!(seat.armor, 15);
    assert_eq!(seat.tier, 6);
    assert_eq!(seat.place, 1);
    assert!(!seat.dead());
    assert_eq!(seat.last_fight, None);
}

#[test]
fn my_hero_power_and_both_trinket_slots() {
    let log = read("views.log");
    let game = log.game().unwrap();
    let cards = Cards::default();

    let power = hero_power(game, &cards, Some(1), 5).expect("a hero power in PLAY");
    assert_eq!(power.card_id, "BG22_HERO_001p_t4");
    assert_eq!(power.name, "BG22_HERO_001p_t4");
    assert_eq!(power.turn, 5);
    // Its text's numbers: this power was never given any.
    assert_eq!(power.numbers, [0, 0, 0, 0]);
    assert_eq!(power.waiting, None);
    // The opponent's side has no hero power in this snippet.
    assert_eq!(hero_power(game, &cards, Some(9), 5), None);

    // Lesser slot first: it is the one whose TAG_SCRIPT_DATA_NUM_6 is 1.
    let held = trinkets(game, &cards, Some(1));
    assert_eq!(held.len(), 2);
    assert_eq!(held[0].card_id, "BG36_MagicItem_220");
    assert!(!held[0].greater);
    // The {0} {1} {2} {3} of its text: only the fourth is set here.
    assert_eq!(held[0].numbers, [0, 0, 0, 11]);
    assert_eq!(held[1].card_id, "BG36_MagicItem_217");
    assert!(held[1].greater);
    assert_eq!(held[1].numbers, [0, 3, 0, 11]);

    // No quest rewards in this snippet, and nothing under the stand-in.
    assert!(rewards(game, &cards, Some(1)).is_empty());
    assert!(trinkets(game, &cards, Some(9)).is_empty());
}

#[test]
fn the_hand_the_hidden_cards_and_the_game_long_counts() {
    let log = read("views.log");
    let game = log.game().unwrap();
    let cards = Cards::default();

    let mine = hand(game, &cards, Some(1));
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].card_id, "BG32_324");
    assert_eq!(mine[0].health, 7);
    // Every hand card here shows its card id.
    assert_eq!(hidden_in_hand(game, Some(1)), 0);
    // The stand-in player holds no hand of its own outside a fight.
    assert!(hand(game, &cards, Some(9)).is_empty());

    // Spells cast is a tag on the player.
    let me = game.player_entity(1).expect("my player entity");
    assert_eq!(counts(game, Some(me)).get("spells"), Some(&41));
    assert!(!counts(game, Some(me)).contains_key("knights"));

    // No Blood Gem tags on this player: a plain +1/+1.
    assert_eq!(gem(Some(me)), (1, 1));
    assert_eq!(gem(None), (1, 1));
}

#[test]
fn the_deity_names_the_minion_it_awakens() {
    let log = read("deity.log");
    let game = log.game().unwrap();
    let thun = deity(game, &Cards::default(), Some(6)).expect("a Deity in zone SECRET");
    // TAG_SCRIPT_DATA_NUM_6 130610 is C'Thun's dbf id.
    assert_eq!(thun.card_id, "BGFYM_000");
    assert_eq!(thun.name, "BGFYM_000");
    assert_eq!((thun.attack, thun.health), (7, 4));
    // QUEST_PROGRESS has not reached 4, so all four Aberration deaths are due.
    assert_eq!(thun.left, 4);
    // Nobody else has one.
    assert_eq!(deity(game, &Cards::default(), Some(1)), None);
}

#[test]
fn a_diremuck_summon_still_waiting_for_a_slot() {
    let log = read("waiting.log");
    let game = log.game().unwrap();
    let player = game.entities.get(&17).expect("the player it hangs off");
    assert_eq!(waiting(game, Some(player)), 1);
    // The golden one summons two, and its enchantment id is a guess.
    assert_eq!(waiting(game, None), 0);
    assert_eq!(waiting(game, game.entities.get(&18)), 0);
}

#[test]
fn a_game_with_no_lines_has_no_views() {
    // Nothing read: every view is empty rather than a panic.
    let game = Game::new();
    let cards = Cards::default();
    assert_eq!(turn_of(&game), 0);
    assert!(board(&game, &cards, Some(1)).is_empty());
    assert_eq!(hero_power(&game, &cards, Some(1), 1), None);
    assert!(trinkets(&game, &cards, None).is_empty());
    assert_eq!(deity(&game, &cards, None), None);
    assert!(gifts_by_minion(&game).is_empty());
}
