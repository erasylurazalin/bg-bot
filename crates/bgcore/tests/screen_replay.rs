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
    assert_eq!(outcome.verdict, ScreenVerdict::Ok, "{:?}", outcome.differences);
    assert!(
        !outcome
            .differences
            .iter()
            .any(|d| d.contains("Gold")),
        "{:?}",
        outcome.differences
    );
}
