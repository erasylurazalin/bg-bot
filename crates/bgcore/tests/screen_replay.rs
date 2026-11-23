use std::sync::Arc;

use bgcore::Data;
use bgcore::real_boards::CardIdMap;
use bgcore::screen_replay::{self, ScreenPick, ScreenTurnOutcome, ScreenVerdict};
use bgcore::shop_replay;

fn run(json: &str, seeds: u64) -> ScreenTurnOutcome {
    let data = Arc::new(Data::bundled());
    let ids = CardIdMap::default();
    let files = vec![("screen.json".to_string(), json.to_string())];
    let specs = screen_replay::parse_screen_files(&files).expect("the screen file parses");
    let file = &specs[0];
    screen_replay::replay_screen_turn_seeds(&data, &ids, file, &file.turns[0], seeds)
}

fn run_chain(json: &str, seeds: u64) -> Vec<ScreenTurnOutcome> {
    let data = Arc::new(Data::bundled());
    let ids = CardIdMap::default();
    let files = vec![("screen.json".to_string(), json.to_string())];
    let specs = screen_replay::parse_screen_files(&files).expect("the screen file parses");
    screen_replay::replay_screen_file_chain(&data, &ids, &specs[0], seeds)
}

fn run_turn(json: &str, index: usize, seeds: u64) -> ScreenTurnOutcome {
    let data = Arc::new(Data::bundled());
    let ids = CardIdMap::default();
    let files = vec![("screen.json".to_string(), json.to_string())];
    let specs = screen_replay::parse_screen_files(&files).expect("the screen file parses");
    let file = &specs[0];
    screen_replay::replay_screen_turn_seeds(&data, &ids, file, &file.turns[index], seeds)
}

const BUY_PLAY_SELL: &str = r#"{
  "source": "hand-written",
  "game": 1,
  "turns": [{
    "turn": 3,
    "start": {
      "gold": 10, "tier": 1, "health": 30,
      "board": [], "hand": [],
      "tavern": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                  "golden": false, "slot": 1, "kind": "minion"}]
    },
    "steps": [
      {"step": "buy", "slot": 1},
      {"step": "play", "index": 1},
      {"step": "sell", "position": 1}
    ],
    "end": {"gold": 8, "tier": 1, "health": 30, "board": [], "hand": []}
  }]
}"#;

#[test]
fn buy_play_sell_is_ok() {
    let outcome = run(BUY_PLAY_SELL, 3);
    assert_eq!(
        outcome.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        outcome.differences
    );
    assert_eq!(outcome.seeds[0].played, 3);
    assert!(outcome.stop.is_none());
}

#[test]
fn wrong_end_gold_is_flagged_with_the_gold_line() {
    let json = BUY_PLAY_SELL.replace("\"gold\": 8, \"tier\"", "\"gold\": 5, \"tier\"");
    let outcome = run(&json, 1);
    assert_eq!(outcome.verdict, ScreenVerdict::Flagged);
    assert!(
        outcome
            .differences
            .iter()
            .any(|d| d == "Gold: engine 8, screen 5"),
        "{:?}",
        outcome.differences
    );
}

#[test]
fn unseen_hand_is_not_compared() {
    let json = r#"{
      "source": "hand-written",
      "game": 2,
      "turns": [{
        "turn": 3,
        "start": {
          "gold": 10, "tier": 1, "health": 30,
          "board": [],
          "tavern": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                      "golden": false, "slot": 1, "kind": "minion"}]
        },
        "steps": [{"step": "buy", "slot": 1}],
        "end": {"gold": 7, "tier": 1, "health": 30, "board": []}
      }]
    }"#;
    let outcome = run(json, 2);
    assert_eq!(
        outcome.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        outcome.differences
    );
    assert!(
        !outcome.differences.iter().any(|d| d.starts_with("hand")),
        "{:?}",
        outcome.differences
    );
}

#[test]
fn a_roll_seats_the_seen_tavern_and_the_next_buy_takes_that_card() {
    let json = r#"{
      "source": "hand-written",
      "game": 3,
      "turns": [{
        "turn": 3,
        "start": {
          "gold": 10, "tier": 1, "health": 30,
          "board": [], "hand": [], "tavern": []
        },
        "steps": [
          {"step": "roll",
           "seen": {"tavern": [{"name": "Southsea Busker", "attack": 3,
                                "health": 1, "golden": false, "slot": 1,
                                "kind": "minion"}]}},
          {"step": "buy", "slot": 1}
        ],
        "end": {
          "gold": 6, "tier": 1, "health": 30, "board": [],
          "hand": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                    "golden": false}]
        }
      }]
    }"#;
    let outcome = run(json, 4);
    assert_eq!(
        outcome.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        outcome.differences
    );
    assert_eq!(outcome.seeds[0].played, 2);
}

#[test]
fn a_step_the_engine_refuses_stops_the_turn() {
    let json = r#"{
      "source": "hand-written",
      "game": 4,
      "turns": [{
        "turn": 3,
        "start": {
          "gold": 10, "tier": 1, "health": 30,
          "board": [], "hand": [],
          "tavern": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                      "golden": false, "slot": 1, "kind": "minion"}]
        },
        "steps": [{"step": "sell", "position": 1}],
        "end": {"gold": 10, "tier": 1, "health": 30, "board": []}
      }]
    }"#;
    let outcome = run(json, 1);
    assert_eq!(outcome.verdict, ScreenVerdict::Stopped);
    let stop = outcome.stop.as_ref().expect("the turn stops");
    assert_eq!(stop.step, 0);
    assert_eq!(stop.kind, "sell");
    assert!(!stop.reason.is_empty());
}

#[test]
fn a_seen_gold_after_a_step_flags_and_the_turn_carries_on() {
    let json = r#"{
      "source": "hand-written",
      "game": 5,
      "turns": [{
        "turn": 3,
        "start": {
          "gold": 10, "tier": 1, "health": 30,
          "board": [], "hand": [],
          "tavern": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                      "golden": false, "slot": 1, "kind": "minion"}]
        },
        "steps": [{"step": "buy", "slot": 1,
                   "seen": {"gold": 5}}],
        "end": {"gold": 7, "tier": 1, "health": 30, "board": []}
      }]
    }"#;
    let outcome = run(json, 1);
    assert_eq!(outcome.verdict, ScreenVerdict::Flagged);
    assert!(
        outcome
            .differences
            .iter()
            .any(|d| d == "after step 1 (buy): engine Gold 7, screen 5"),
        "{:?}",
        outcome.differences
    );
    assert_eq!(outcome.seeds[0].played, 1);
    assert!(outcome.stop.is_none());
}

#[test]
fn a_seen_board_that_differs_after_a_play_flags_the_step_and_the_next_play_runs() {
    let json = r#"{
      "source": "hand-written",
      "game": 6,
      "turns": [{
        "turn": 3,
        "start": {
          "gold": 10, "tier": 1, "health": 30,
          "board": [],
          "hand": [
            {"name": "Southsea Busker", "attack": 3, "health": 1, "golden": false},
            {"name": "Southsea Busker", "attack": 3, "health": 1, "golden": false}
          ],
          "tavern": []
        },
        "steps": [
          {"step": "play", "index": 1,
           "seen": {
             "board": [{"name": "Southsea Busker", "attack": 4, "health": 1,
                        "golden": false}],
             "hand": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                       "golden": false}]
           }},
          {"step": "play", "index": 1,
           "seen": {
             "board": [
               {"name": "Southsea Busker", "attack": 4, "health": 1, "golden": false},
               {"name": "Southsea Busker", "attack": 3, "health": 1, "golden": false}
             ],
             "hand": []
           }}
        ]
      }]
    }"#;
    let outcome = run(json, 1);
    assert_eq!(outcome.verdict, ScreenVerdict::Flagged);
    assert_eq!(
        outcome.differences.first().map(String::as_str),
        Some("after step 1 (play): board 1 engine Southsea Busker 3/1, screen Southsea Busker 4/1")
    );
    assert_eq!(outcome.seeds[0].played, 2);
    assert!(outcome.stop.is_none());
}

#[test]
fn a_picked_trinket_pays_the_price_the_screen_showed() {
    let json = r#"{
      "source": "hand-written",
      "game": 7,
      "turns": [{
        "turn": 3,
        "start": {
          "gold": 10, "tier": 2, "health": 30,
          "board": [], "hand": [], "tavern": [], "trinkets": []
        },
        "steps": [{
          "step": "trinket_pick", "slot_kind": "lesser",
          "options": [{"card_id": "BG35_MagicItem_801", "name": "Sous Chef Sticker",
                       "cost": 2}],
          "picked": {"card_id": "BG35_MagicItem_801", "name": "Sous Chef Sticker",
                     "cost": 2},
          "seen": {
            "gold": 8,
            "trinkets": [{"card_id": "BG35_MagicItem_801",
                          "name": "Sous Chef Sticker", "greater": false}]
          }
        }],
        "end": {"gold": 8, "tier": 2, "health": 30, "board": [], "hand": [],
                "trinkets": [{"card_id": "BG35_MagicItem_801",
                              "name": "Sous Chef Sticker", "greater": false}]}
      }]
    }"#;
    let outcome = run(json, 1);
    assert_eq!(
        outcome.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        outcome.differences
    );
    assert!(
        !outcome.differences.iter().any(|d| d.contains("Gold")),
        "{:?}",
        outcome.differences
    );
}

/// A Dark Gift handed out by a Discover in turn 1 is carried in the hand. Turn
/// 2 plays both cards; the second play triggers the gifted card's Dexterity,
/// which gives it +2/+2. A turn seated fresh from the screen cannot know the
/// gift, so only chained mode gets those stats.
const DARK_GIFT_TWO_TURNS: &str = r#"{
  "source": "hand-written", "game": 20,
  "turns": [
    {"turn": 1,
     "start": {"gold": 10, "tier": 1, "health": 30, "board": [], "hand": [],
               "tavern": []},
     "steps": [
       {"step": "discover", "kind": "dark_gift",
        "options": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                     "golden": false, "dark_gift": "Dexterity"}],
        "picked": 0},
       {"step": "end_turn"}
     ],
     "end": {"gold": 10, "tier": 1, "health": 30, "board": [],
             "hand": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                       "golden": false}]}},
    {"turn": 2,
     "start": {"gold": 10, "tier": 1, "health": 30, "board": [],
               "hand": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                         "golden": false},
                        {"name": "Southsea Busker", "attack": 3, "health": 1,
                         "golden": false}],
               "tavern": []},
     "steps": [
       {"step": "play", "index": 1},
       {"step": "play", "index": 1}
     ],
     "end": {"gold": 10, "tier": 1, "health": 30,
             "board": [{"name": "Southsea Busker", "attack": 5, "health": 3,
                        "golden": false},
                       {"name": "Southsea Busker", "attack": 3, "health": 1,
                        "golden": false}],
             "hand": []}}
  ]
}"#;

#[test]
fn a_dark_gift_from_a_discover_reaches_the_next_turn_only_when_chained() {
    let chained = run_chain(DARK_GIFT_TWO_TURNS, 1);
    assert_eq!(chained.len(), 2);
    assert_eq!(
        chained[1].verdict,
        ScreenVerdict::Ok,
        "{:?} between {:?}",
        chained[1].differences,
        chained[1].between_turns
    );
    let engine = chained[1].seeds[0]
        .engine
        .as_ref()
        .expect("the turn finished");
    assert_eq!(
        engine
            .board
            .iter()
            .map(|m| (m.name.as_str(), m.attack, m.health))
            .collect::<Vec<_>>(),
        vec![("Southsea Busker", 5, 3), ("Southsea Busker", 3, 1)],
        "the gift's +2/+2 landed when the second card was played"
    );

    let fresh = run_turn(DARK_GIFT_TWO_TURNS, 1, 1);
    assert_eq!(
        fresh.verdict,
        ScreenVerdict::Flagged,
        "without the gift the card plays as 3/1: {:?}",
        fresh.differences
    );
    assert!(
        fresh
            .differences
            .iter()
            .any(|d| d.contains("Southsea Busker 3/1") && d.contains("5/3")),
        "{:?}",
        fresh.differences
    );
}

/// Fearless Foodie's Choose One raises the Blood Gem value by +1/+1 in turn 1.
/// Turn 2's Blood Gem reads that value: chained mode gives +2/+2, a fresh seat
/// gives +1/+1.
const GEM_COUNTER_TWO_TURNS: &str = r#"{
  "source": "hand-written", "game": 21,
  "turns": [
    {"turn": 1,
     "start": {"gold": 10, "tier": 1, "health": 30, "board": [],
               "hand": [{"name": "Fearless Foodie", "attack": 2, "health": 4,
                         "golden": false}],
               "tavern": []},
     "steps": [
       {"step": "play", "index": 1, "sub_option": 0},
       {"step": "end_turn"}
     ],
     "end": {"gold": 10, "tier": 1, "health": 30,
             "board": [{"name": "Fearless Foodie", "attack": 2, "health": 4,
                        "golden": false}],
             "hand": []}},
    {"turn": 2,
     "start": {"gold": 10, "tier": 1, "health": 30,
               "board": [{"name": "Fearless Foodie", "attack": 2, "health": 4,
                          "golden": false}],
               "hand": [{"name": "Blood Gem", "kind": "spell",
                         "attack": 0, "health": 0, "golden": false}],
               "tavern": []},
     "steps": [
       {"step": "play", "index": 1,
        "target": {"kind": "board", "position": 1}}
     ],
     "end": {"gold": 10, "tier": 1, "health": 30,
             "board": [{"name": "Fearless Foodie", "attack": 4, "health": 6,
                        "golden": false}],
             "hand": []}}
  ]
}"#;

#[test]
fn a_counter_built_in_turn_one_is_read_in_turn_two_only_when_chained() {
    let chained = run_chain(GEM_COUNTER_TWO_TURNS, 1);
    assert_eq!(chained.len(), 2);
    assert_eq!(
        chained[1].verdict,
        ScreenVerdict::Ok,
        "{:?} between {:?}",
        chained[1].differences,
        chained[1].between_turns
    );
    let engine = chained[1].seeds[0]
        .engine
        .as_ref()
        .expect("the turn finished");
    assert_eq!(
        engine
            .board
            .iter()
            .map(|m| (m.name.as_str(), m.attack, m.health))
            .collect::<Vec<_>>(),
        vec![("Fearless Foodie", 4, 6)],
        "the turn 1 gem bonus is +2/+2"
    );

    let fresh = run_turn(GEM_COUNTER_TWO_TURNS, 1, 1);
    assert_eq!(
        fresh.verdict,
        ScreenVerdict::Flagged,
        "{:?}",
        fresh.differences
    );
    assert!(
        fresh
            .differences
            .iter()
            .any(|d| d.contains("Fearless Foodie 3/5")),
        "{:?}",
        fresh.differences
    );
}

#[test]
fn a_between_turn_difference_is_written_and_does_not_flag_the_turn() {
    let json = r#"{
      "source": "hand-written", "game": 22,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 4, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": 2, "health": 2,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 4, "tier": 1, "health": 30,
                 "board": [{"name": "Deadly Spore", "attack": 2, "health": 2,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
    assert!(second.differences.is_empty(), "{:?}", second.differences);
    assert_eq!(
        second.between_turns,
        vec!["before turn 2: board 1 engine Deadly Spore 1/1, screen Deadly Spore 2/2".to_string()]
    );
    assert_eq!(second.uncertain, vec!["Deadly Spore"]);
}

#[test]
fn a_fight_only_health_change_marks_nothing() {
    let json = r#"{
      "source": "hand-written", "game": 24,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 10, "tier": 1, "health": 25,
                   "board": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 10, "tier": 1, "health": 25,
                 "board": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert!(
        second
            .between_turns
            .iter()
            .any(|l| l.contains("Health engine 30, screen 25")),
        "{:?}",
        second.between_turns
    );
    assert!(second.uncertain_lines.is_empty(), "{:?}", second.uncertain_lines);
    assert!(second.uncertain.is_empty(), "{:?}", second.uncertain);
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
}

#[test]
fn a_board_minion_with_a_combat_counter_effect_marks_its_counter() {
    let json = r#"{
      "source": "hand-written", "game": 25,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Turquoise Skitterer", "attack": 5, "health": 5,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Turquoise Skitterer", "attack": 5, "health": 5,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 10, "tier": 1, "health": 30,
                 "board": [{"name": "Turquoise Skitterer", "attack": 5, "health": 5,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert!(
        second.uncertain.iter().any(|c| c == "Beetle bonus"),
        "{:?}",
        second.uncertain
    );
    assert!(second.uncertain_lines.is_empty(), "{:?}", second.uncertain_lines);
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
}

#[test]
fn a_stat_line_on_a_card_with_no_hidden_dependence_is_certain() {
    let json = r#"{
      "source": "hand-written", "game": 26,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [{"step": "roll",
                    "seen": {"board": [{"name": "Southsea Busker", "attack": 7,
                                        "health": 7, "golden": false}]}}],
         "end": {"gold": 9, "tier": 1, "health": 30,
                 "board": [{"name": "Southsea Busker", "attack": 7, "health": 7,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert!(
        second
            .differences
            .iter()
            .any(|d| d.contains("Southsea Busker 3/1")),
        "{:?}",
        second.differences
    );
    assert!(second.uncertain_lines.is_empty(), "{:?}", second.uncertain_lines);
    assert_eq!(
        second.verdict,
        ScreenVerdict::Flagged,
        "diffs {:?} uncertain {:?}",
        second.differences,
        second.uncertain_lines
    );
}

#[test]
fn a_skipped_turn_reseats_from_its_start() {
    let json = r#"{
      "source": "hand-written", "game": 23,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 3,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": 2, "health": 2,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 10, "tier": 1, "health": 30,
                 "board": [{"name": "Deadly Spore", "attack": 2, "health": 2,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let third = &chained[1];
    assert!(third.reseated, "a skipped turn is reseated");
    assert!(
        third.between_turns.is_empty(),
        "a reseated turn compares nothing with a carry: {:?}",
        third.between_turns
    );
    assert_eq!(third.verdict, ScreenVerdict::Ok, "{:?}", third.differences);
}

#[test]
fn a_random_step_line_does_not_flag_the_turn() {
    let json = r#"{
      "source": "hand-written", "game": 30,
      "turns": [{
        "turn": 3,
        "start": {"gold": 10, "tier": 1, "health": 30,
                  "board": [], "hand": [], "tavern": []},
        "steps": [
          {"step": "roll"},
          {"step": "buy", "slot": 1,
           "seen": {"tavern": [{"name": "Southsea Busker", "attack": 3,
                                "health": 1, "golden": false, "slot": 1,
                                "kind": "minion"}]}}
        ]
      }]
    }"#;
    let outcome = run(json, 8);
    assert_eq!(
        outcome.verdict,
        ScreenVerdict::OkRandom,
        "diffs {:?} random {:?}",
        outcome.differences,
        outcome.random_lines
    );
    assert!(outcome.differences.is_empty(), "{:?}", outcome.differences);
    assert!(!outcome.random_lines.is_empty(), "a random line is kept");
    assert!(
        outcome
            .lines
            .iter()
            .any(|l| l.class == screen_replay::ScreenLineClass::Random),
        "{:?}",
        outcome.lines
    );
}

#[test]
fn a_range_agrees_or_takes_the_nearest_bound() {
    let json = r#"{
      "source": "hand-written", "game": 31,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": 100, "health": 100,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 4, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore",
                              "attack": {"min": 95, "max": 105},
                              "health": {"min": 110, "max": 120},
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 4, "tier": 1, "health": 30,
                 "board": [{"name": "Deadly Spore", "attack": 100, "health": 110,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
    assert_eq!(
        second.between_turns,
        vec![
            "before turn 2: board 1 engine Deadly Spore 100/100, screen Deadly Spore 95..105/110..120"
                .to_string()
        ]
    );
    let engine = second.seeds[0].engine.as_ref().expect("the turn finished");
    assert_eq!(
        (engine.board[0].attack, engine.board[0].health),
        (100, 110),
        "the agreeing range stays and the disagreeing one takes its bound"
    );
    let group = second
        .checks
        .iter()
        .find(|g| g.when == "before turn")
        .expect("the between-turn checks");
    assert!(
        group.checks.iter().any(|c| {
            c.field == "attack" && c.status == screen_replay::ScreenCheckStatus::Agree
        })
    );
    assert!(group.checks.iter().any(|c| {
        c.field == "health" && c.status == screen_replay::ScreenCheckStatus::Disagree
    }));
}

#[test]
fn a_null_stat_is_not_compared_and_keeps_the_engine_value() {
    let json = r#"{
      "source": "hand-written", "game": 32,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": 100, "health": 100,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 4, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": null, "health": 100,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 4, "tier": 1, "health": 30,
                 "board": [{"name": "Deadly Spore", "attack": 100, "health": 100,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
    assert!(
        second.between_turns.is_empty(),
        "{:?}",
        second.between_turns
    );
    let group = second
        .checks
        .iter()
        .find(|g| g.when == "before turn")
        .expect("the between-turn checks");
    assert!(group.checks.iter().any(|c| {
        c.field == "attack" && c.status == screen_replay::ScreenCheckStatus::CannotTell
    }));
}

#[test]
fn a_keyword_read_between_turns_is_one_line_and_resynced() {
    let json = r#"{
      "source": "hand-written", "game": 35,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                              "golden": false, "keywords": []}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 4, "tier": 1, "health": 30,
                   "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                              "golden": false,
                              "keyword_reads": {"divine_shield": true}}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 4, "tier": 1, "health": 30,
                 "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                            "golden": false,
                            "keyword_reads": {"divine_shield": true}}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert!(
        second
            .between_turns
            .iter()
            .any(|l| l.contains("keyword DIVINE_SHIELD engine no, screen yes")),
        "{:?}",
        second.between_turns
    );
    assert!(
        second.differences.is_empty(),
        "the keyword is resynced, so the end does not repeat it: {:?}",
        second.differences
    );
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
}

#[test]
fn a_full_keyword_list_between_turns_replaces_the_old_set() {
    let json = r#"{
      "source": "hand-written", "game": 36,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Buzzing Vermin", "attack": 1, "health": 1,
                              "golden": false, "keywords": ["taunt"]}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 4, "tier": 1, "health": 30,
                   "board": [{"name": "Buzzing Vermin", "attack": 1, "health": 1,
                              "golden": false, "keywords": []}],
                   "hand": [], "tavern": []},
         "steps": [],
         "end": {"gold": 4, "tier": 1, "health": 30,
                 "board": [{"name": "Buzzing Vermin", "attack": 1, "health": 1,
                            "golden": false, "keywords": []}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert!(
        second
            .between_turns
            .iter()
            .any(|l| l.contains("keyword TAUNT engine yes, screen no")),
        "{:?}",
        second.between_turns
    );
    assert!(
        second.differences.is_empty(),
        "the full list clears TAUNT, so the end does not repeat it: {:?}",
        second.differences
    );
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
}

#[test]
fn a_step_keyword_drift_gives_one_line_then_resyncs() {
    let json = r#"{
      "source": "hand-written", "game": 37,
      "turns": [{
        "turn": 3,
        "start": {"gold": 10, "tier": 1, "health": 30,
                  "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                             "golden": false, "keywords": []}],
                  "hand": [], "tavern": []},
        "steps": [{"step": "roll",
                   "seen": {"board": [{"name": "Deadly Spore", "attack": 1,
                                       "health": 1, "golden": false,
                                       "keyword_reads": {"divine_shield": true}}]}}],
        "end": {"gold": 9, "tier": 1, "health": 30,
                "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                           "golden": false,
                           "keyword_reads": {"divine_shield": true}}],
                "hand": []}
      }]
    }"#;
    let outcome = run(json, 1);
    assert_eq!(
        outcome.differences.len(),
        1,
        "one line at the step, none at the end: {:?}",
        outcome.differences
    );
    assert!(
        outcome.differences[0].contains("keyword DIVINE_SHIELD engine no, screen yes"),
        "{:?}",
        outcome.differences
    );
    assert_eq!(
        outcome.verdict,
        ScreenVerdict::Flagged,
        "{:?}",
        outcome.differences
    );
}

#[test]
fn a_read_keyword_that_disagrees_flags_the_end() {
    let json = r#"{
      "source": "hand-written", "game": 33,
      "turns": [{
        "turn": 3,
        "start": {"gold": 10, "tier": 1, "health": 30,
                  "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                             "golden": false}],
                  "hand": [], "tavern": []},
        "steps": [],
        "end": {"gold": 10, "tier": 1, "health": 30,
                "board": [{"name": "Deadly Spore", "attack": 1, "health": 1,
                           "golden": false,
                           "keyword_reads": {"divine_shield": true}}],
                "hand": []}
      }]
    }"#;
    let outcome = run(json, 1);
    assert_eq!(outcome.verdict, ScreenVerdict::Flagged);
    assert!(
        outcome
            .differences
            .iter()
            .any(|d| d.contains("keyword DIVINE_SHIELD")),
        "{:?}",
        outcome.differences
    );
    let end = outcome
        .checks
        .iter()
        .find(|g| g.when == "end")
        .expect("the end checks");
    assert!(end.checks.iter().any(|c| {
        c.field == "DIVINE_SHIELD" && c.status == screen_replay::ScreenCheckStatus::Disagree
    }));
}

#[test]
fn only_certain_uncertain_lines_make_the_uncertain_verdict() {
    let json = r#"{
      "source": "hand-written", "game": 34,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [{"name": "Utility Drone", "attack": 4, "health": 5,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 4, "tier": 1, "health": 30,
                   "board": [{"name": "Utility Drone", "attack": 6, "health": 6,
                              "golden": false}],
                   "hand": [], "tavern": []},
         "steps": [{"step": "roll",
                    "seen": {"board": [{"name": "Utility Drone", "attack": 9,
                                        "health": 9, "golden": false}]}}],
         "end": {"gold": 3, "tier": 1, "health": 30,
                 "board": [{"name": "Utility Drone", "attack": 9, "health": 9,
                            "golden": false}],
                 "hand": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert_eq!(
        second.verdict,
        ScreenVerdict::Uncertain,
        "diffs {:?} uncertain {:?}",
        second.differences,
        second.uncertain_lines
    );
    assert!(second.differences.is_empty(), "{:?}", second.differences);
    assert!(
        second
            .uncertain_lines
            .iter()
            .any(|d| d.contains("(uncertain after the fight)")),
        "{:?}",
        second.uncertain_lines
    );
}

#[test]
fn a_converted_card_leaves_its_script_numbers_unseen() {
    let data = Data::bundled();
    let ids = CardIdMap::default();
    let text = r#"{"source": "test", "game": 1, "turns": [
        {"turn": 6,
         "start": {"gold": 10, "tier": 1, "health": 30, "board": [],
                   "hand": [{"name": "Lockbox", "kind": "spell",
                             "numbers": [2, 0, 0, 0]}],
                   "tavern": []},
         "actions": []}]}"#;
    let specs = shop_replay::parse_action_files(&[("test.json".to_string(), text.to_string())])
        .expect("the action file parses");
    let screen = screen_replay::screen_from_action_file(&data, &ids, &specs[0], false);
    assert_eq!(
        screen.turns[0].start.hand.as_ref().expect("the hand")[0].numbers,
        None,
        "script numbers are hidden unless a video converter reads them"
    );
}

#[test]
fn a_converted_end_turn_does_not_write_the_next_turns_gold() {
    let data = Data::bundled();
    let ids = CardIdMap::default();
    let text = r#"{"source": "test", "game": 1, "turns": [
        {"turn": 7,
         "start": {"gold": 10, "tier": 1, "health": 30, "board": [], "hand": [],
                   "tavern": []},
         "actions": [{"step": "end_turn", "paid": {"gold": 0, "health": 0},
                      "after": {"gold": 0}}],
         "end": {"gold": 0}}]}"#;
    let specs = shop_replay::parse_action_files(&[("test.json".to_string(), text.to_string())])
        .expect("the action file parses");
    let screen = screen_replay::screen_from_action_file(&data, &ids, &specs[0], false);
    let seen = screen.turns[0].steps[0]
        .seen
        .as_ref()
        .expect("the end-turn step has a seen state");
    assert_eq!(seen.gold, None, "the post-turn Gold is not the shop-end Gold");
    assert_eq!(
        screen.turns[0].end.as_ref().and_then(|end| end.gold),
        None,
        "the end state's post-turn Gold is not compared either"
    );
}

#[test]
fn a_converted_trinket_pick_writes_the_paid_price() {
    let data = Data::bundled();
    let ids = CardIdMap::default();
    let text = r#"{"source": "test", "game": 1, "turns": [
        {"turn": 9,
         "start": {"gold": 16, "tier": 4, "health": 30, "board": [], "hand": [],
                   "tavern": []},
         "actions": [{"step": "trinket_pick", "slot_kind": "greater",
                      "options": [
                        {"card_id": "BG35_MagicItem_755", "name": "Chillmere Mosaic", "cost": 3},
                        {"card_id": "BG36_MagicItem_373", "name": "Multilayered Shield", "cost": 5}],
                      "picked": {"card_id": "BG35_MagicItem_755", "name": "Chillmere Mosaic",
                                 "cost": 5},
                      "paid": {"gold": 3, "health": 0},
                      "after": {"gold": 13}}],
         "end": null}]}"#;
    let specs = shop_replay::parse_action_files(&[("test.json".to_string(), text.to_string())])
        .expect("the action file parses");
    let screen = screen_replay::screen_from_action_file(&data, &ids, &specs[0], false);
    let picked = screen.turns[0].steps[0]
        .picked
        .as_ref()
        .expect("the step has a pick");
    let ScreenPick::Card(card) = picked else {
        panic!("a trinket pick writes a card");
    };
    assert_eq!(card.cost, Some(3), "the offer's paid price, not the list price");
}

#[test]
fn buying_a_tavern_spell_uses_the_card_not_the_screens_slot_number() {
    let json = r#"{
      "source": "hand-written", "game": 35,
      "turns": [{
        "turn": 8,
        "start": {"gold": 10, "tier": 1, "health": 30,
                  "board": [], "hand": [],
                  "tavern": [
                    {"name": "Southsea Busker", "attack": 3, "health": 1, "slot": 1},
                    {"name": "Southsea Busker", "attack": 3, "health": 1, "slot": 2},
                    {"name": "Southsea Busker", "attack": 3, "health": 1, "slot": 3},
                    {"name": "Alliance Flag", "attack": 0, "health": 0, "slot": 4,
                     "kind": "spell"},
                    {"name": "Southsea Busker", "attack": 3, "health": 1, "slot": 5},
                    {"name": "Southsea Busker", "attack": 3, "health": 1, "slot": 6}
                  ]},
        "steps": [{"step": "buy", "slot": 4,
                   "card": {"name": "Alliance Flag", "kind": "spell"}}],
        "end": {"gold": 9, "tier": 1, "health": 30,
                "board": [],
                "hand": [{"name": "Alliance Flag", "kind": "spell",
                          "attack": 0, "health": 0}],
                "tavern": [
                  {"name": "Southsea Busker", "attack": 3, "health": 1},
                  {"name": "Southsea Busker", "attack": 3, "health": 1},
                  {"name": "Southsea Busker", "attack": 3, "health": 1},
                  {"name": "Southsea Busker", "attack": 3, "health": 1},
                  {"name": "Southsea Busker", "attack": 3, "health": 1}
                ]}
      }]
    }"#;
    let outcome = run(json, 1);
    assert_eq!(outcome.verdict, ScreenVerdict::Ok, "{:?}", outcome.differences);
    let engine = outcome.seeds[0].engine.as_ref().expect("the turn finished");
    assert!(
        engine.hand.iter().any(|m| m.name == "Alliance Flag"),
        "{:?}",
        engine.hand
    );
    assert!(
        !engine.tavern.iter().any(|m| m.name == "Alliance Flag"),
        "{:?}",
        engine.tavern
    );
}

#[test]
fn a_converted_buy_writes_the_discounted_paid_price() {
    let data = Data::bundled();
    let ids = CardIdMap::default();
    let text = r#"{"source": "test", "game": 1, "turns": [
        {"turn": 8,
         "start": {"gold": 12, "tier": 5, "health": 30, "board": [], "hand": [],
                   "tavern": [{"card_id": "BG26_ICC_901", "name": "Drakkari Enchanter",
                               "attack": 1, "health": 5, "cost": 5}]},
         "actions": [{"step": "buy", "slot": 1,
                      "card": {"card_id": "BG26_ICC_901", "name": "Drakkari Enchanter",
                               "attack": 1, "health": 5, "cost": 5},
                      "paid": {"gold": 3, "health": 0},
                      "after": {"gold": 9}}],
         "end": null}]}"#;
    let specs = shop_replay::parse_action_files(&[("test.json".to_string(), text.to_string())])
        .expect("the action file parses");
    let screen = screen_replay::screen_from_action_file(&data, &ids, &specs[0], false);
    let card = screen.turns[0].steps[0]
        .card
        .as_ref()
        .expect("the step names the bought card");
    assert_eq!(card.cost, Some(3), "the screen shows the paid price, not the list price");
}

#[test]
fn a_hero_power_trinket_discover_writes_the_free_paid_price() {
    let data = Data::bundled();
    let ids = CardIdMap::default();
    let text = r#"{"source": "test", "game": 1, "turns": [
        {"turn": 5,
         "start": {"gold": 7, "tier": 3, "health": 30, "board": [], "hand": [],
                   "tavern": []},
         "actions": [{"step": "discover", "kind": "discover",
                      "source": {"card_id": "BG30_HERO_304p", "name": "Fantastic Treasure"},
                      "options": [{"card_id": "BG35_MagicItem_840", "name": "Chromatic Tear",
                                   "cost": 2}],
                      "picked": {"card_id": "BG35_MagicItem_840", "name": "Chromatic Tear",
                                 "cost": 2},
                      "paid": {"gold": 0, "health": 0},
                      "after": {"gold": 7}}],
         "end": null}]}"#;
    let specs = shop_replay::parse_action_files(&[("test.json".to_string(), text.to_string())])
        .expect("the action file parses");
    let screen = screen_replay::screen_from_action_file(&data, &ids, &specs[0], false);
    let picked = screen.turns[0].steps[0]
        .picked
        .as_ref()
        .expect("the step has a pick");
    let ScreenPick::Card(card) = picked else {
        panic!("a trinket Discover writes a card");
    };
    assert_eq!(card.cost, Some(0), "the hero power gives the trinket for free");
}

#[test]
fn a_health_price_charge_does_not_carry_into_the_next_turn() {
    let json = r#"{
      "source": "hand-written", "game": 36,
      "turns": [
        {"turn": 1,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [], "hand": [],
                   "tavern": [{"name": "Southsea Busker", "attack": 3, "health": 1,
                               "slot": 1, "costs_health": true}]},
         "steps": []},
        {"turn": 2,
         "start": {"gold": 10, "tier": 1, "health": 30,
                   "board": [], "hand": [],
                   "tavern": [{"name": "Harmless Bonehead", "attack": 1, "health": 1,
                               "slot": 1}]},
         "steps": [{"step": "buy", "slot": 1}],
         "end": {"gold": 7, "tier": 1, "health": 30,
                 "board": [],
                 "hand": [{"name": "Harmless Bonehead", "attack": 1, "health": 1,
                           "golden": false}],
                 "tavern": []}}
      ]
    }"#;
    let chained = run_chain(json, 1);
    let second = &chained[1];
    assert_eq!(
        second.verdict,
        ScreenVerdict::Ok,
        "{:?}",
        second.differences
    );
    let engine = second.seeds[0].engine.as_ref().expect("the turn finished");
    assert_eq!(engine.gold, 7, "the buy paid Gold, not a stale Health price");
}
