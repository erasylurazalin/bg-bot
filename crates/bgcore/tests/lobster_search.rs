//! The lobster search mode's own tests (docs/briefs/lobster-search.md).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bgcore::action::N_ACTIONS;
use bgcore::{Action, Data, Game, Rng, Tribe, Tribes, shop_replay};

/// A digest of the shop-visible state after a fixed pseudo-random action line
/// on a plain sandbox game. The mode must leave this alone.
fn plain_game_digest() -> u64 {
    let data = Arc::new(Data::bundled());
    let mut game = Game::new_sandbox(data.clone(), 0x10b5_7e12, None, 0);
    let mut rng = Rng::new(0x5eed_1a2b);
    let mut hasher = DefaultHasher::new();
    let mut mask = [false; N_ACTIONS];
    for _ in 0..60 {
        game.legal_mask(&mut mask);
        let n = mask.iter().filter(|&&ok| ok).count() as u32;
        if n == 0 {
            break;
        }
        let k = rng.below(n) as usize;
        let a = mask
            .iter()
            .enumerate()
            .filter(|(_, ok)| **ok)
            .map(|(i, _)| i)
            .nth(k)
            .and_then(Action::from_index)
            .expect("a legal action");
        game.step(a).expect("the chosen action is legal");
        format!("{:?}", shop_replay::ShopSnapshot::of_game(&data, &game, 0)).hash(&mut hasher);
    }
    hasher.finish()
}

#[test]
#[ignore = "prints the baseline digest; the real test compares it"]
fn print_plain_game_digest() {
    println!("PLAIN_GAME_DIGEST={:#018x}", plain_game_digest());
}

#[test]
fn plain_game_is_unchanged() {
    assert_eq!(plain_game_digest(), 0x00ea_a43c_73c9_5b49);
}

#[test]
fn the_start_position_has_the_logged_minions() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let path = format!(
        "{root}boards/actions/Hearthstone_2026_10_28_22_11_50-g2.json"
    );
    let text = std::fs::read_to_string(&path).expect("the logged game");
    let name = "Hearthstone_2026_10_28_22_11_50-g2.json".to_string();
    let mut files = shop_replay::parse_action_files(&[(name, text)]).expect("the log parses");
    let ids_text = std::fs::read_to_string(format!("{root}ref/bg-cards.json"))
        .expect("the card id mapping");
    let ids = bgcore::CardIdMap::parse(&ids_text).expect("the card id mapping parses");
    shop_replay::seat_taught_spells(&mut files, &ids);
    let file = &files[0];
    let turn = file
        .turns
        .iter()
        .find(|t| t.turn == 10)
        .expect("turn 10");
    let data = Arc::new(Data::bundled());
    let (mut game, _missing) =
        shop_replay::turn_start_game(&data, &ids, file, turn, 0x10b5_7e12)
            .expect("turn 10 starts");
    game.players[shop_replay::ME].health = 500;
    let names: Vec<String> = game.players[shop_replay::ME]
        .board
        .iter()
        .map(|m| data.cards.get(m.card).name.clone())
        .collect();
    assert_eq!(names.len(), 7);
    assert_eq!(names[0], "Headhunter Gryphon");
    assert_eq!(names[1], "Tasty Lobster");
    assert_eq!(names[2], "Hoarding Hyena");
    assert_eq!(names[3], "Lurking Lionfish");
    assert_eq!(names[4], "Lurking Lionfish");
    assert_eq!(names[5], "Drakkari Enchanter");
    assert_eq!(names[6], "Deathstrider");
    assert!(game.players[shop_replay::ME].board[0].golden, "the log's Gryphon is golden");
}

fn mode_game(seed: u64, per_mille: u16) -> Game {
    let data = Arc::new(Data::bundled());
    let mut game = Game::new_sandbox(
        data,
        seed,
        Some(Tribes::of(Tribe::Beast).with(Tribe::Mech)),
        0,
    );
    game.players[0].health = 500;
    game.players[0].gold = 20;
    game.players[0].tier = 6;
    game.enable_lobster_mode(0, seed ^ 0xabc, per_mille);
    game.seat_lobster_tavern(0);
    game
}

fn shop_ids(game: &Game) -> (Vec<u16>, Option<u16>) {
    (
        game.players[0].shop.iter().map(|m| m.card).collect(),
        game.players[0].shop_spell,
    )
}

#[test]
fn tavern_streams_are_deterministic_and_path_independent() {
    let a = mode_game(11, 250);
    let b = mode_game(11, 250);
    assert_eq!(shop_ids(&a), shop_ids(&b), "the same seed gives the same Tavern");

    let mut direct = b;
    direct.step(Action::Roll).expect("a paid Roll is legal");

    let mut detour = a;
    let fish = detour
        .data
        .cards
        .by_name("Fishbait")
        .expect("Fishbait");
    // Anything that does not refresh: buying the first card leaves the stream
    // where it was.
    if detour.is_legal(Action::Buy(0)) && detour.players[0].shop[0].card != fish {
        detour.step(Action::Buy(0)).expect("the buy is legal");
    }
    detour.step(Action::Roll).expect("a paid Roll is legal");
    assert_eq!(
        shop_ids(&detour),
        shop_ids(&direct),
        "the Tavern after the first Refresh is the same on both paths"
    );
}

#[test]
fn beast_streams_are_deterministic_and_path_independent() {
    let mut a = mode_game(12, 250);
    let mut b = mode_game(12, 250);
    // A Refresh consumes a different number of game RNG draws but no Beast
    // stream entry.
    b.step(Action::Roll).expect("a paid Roll is legal");
    let mut va = Vec::new();
    let mut vb = Vec::new();
    for _ in 0..8 {
        va.push(a.lobster_next_beast(0));
        vb.push(b.lobster_next_beast(0));
    }
    assert_eq!(va, vb, "the same seed gives the same Beasts");
    assert!(va.iter().all(|c| c.is_some()));
}

#[test]
fn a_sold_shark_spends_the_next_tavern() {
    let mut shark_lane = mode_game(13, 250);
    let mut roll_lane = mode_game(13, 250);
    let shark = shark_lane
        .data
        .cards
        .by_name("Snarky Shark")
        .expect("Snarky Shark");
    shark_lane.give(0, "Snarky Shark");
    let hand = shark_lane.players[0]
        .hand
        .iter()
        .position(|m| m.card == shark)
        .expect("the Shark is in hand");
    shark_lane
        .step(Action::Play { hand: hand as u8, pos: 0 })
        .expect("the Shark is playable");
    let board = shark_lane.players[0]
        .board
        .iter()
        .position(|m| m.card == shark)
        .expect("the Shark is on the board");
    shark_lane
        .step(Action::Sell(board as u8))
        .expect("the Shark is sellable");
    roll_lane.step(Action::Roll).expect("a paid Roll is legal");

    let fish = shark_lane
        .data
        .cards
        .by_name("Fishbait")
        .expect("Fishbait");
    let (a_shop, a_spell) = shop_ids(&shark_lane);
    let (b_shop, b_spell) = shop_ids(&roll_lane);
    assert_eq!(a_shop[0], fish, "the Shark's Refresh puts a Fishbait first");
    assert_eq!(&a_shop[1..], &b_shop[1..], "the rest is the next stream roll");
    assert_eq!(a_spell, b_spell, "the spell slot is the next stream roll too");
}

