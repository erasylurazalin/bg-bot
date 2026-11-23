use std::sync::Arc;

use bgcore::Data;
use bgcore::real_boards::CardIdMap;
use bgcore::screen_replay::{self, ScreenTurnOutcome, ScreenVerdict};

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
    assert_eq!(second.uncertain, vec!["Deadly Spore", "player counters"]);
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
