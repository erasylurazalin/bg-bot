//! Hero powers the game writes under a second card id, and Gallywix's Gold
//! Coin. The log's own ids decide the aliases: Lucky Roll's wait
//! (`BG28_HERO_400p2`) and Spirit Swap's second pick (`BG20_HERO_201p2`).

use std::sync::Arc;

use bgcore::{Action, Data, Game};

#[test]
fn lucky_roll_wait_maps_to_the_built_power() {
    let data = Data::bundled();
    let base = data.heroes.power_by_card_id("BG28_HERO_400p").expect("Lucky Roll");
    assert_eq!(
        data.heroes.power_by_card_id("BG28_HERO_400p2"),
        Some(base),
        "the wait is the same built power"
    );
    assert_eq!(data.heroes.power(base).name, "Lucky Roll");
}

#[test]
fn spirit_swap_second_click_maps_to_the_built_power() {
    let data = Data::bundled();
    let base = data.heroes.power_by_card_id("BG20_HERO_201p").expect("Spirit Swap");
    assert_eq!(
        data.heroes.power_by_card_id("BG20_HERO_201p2"),
        Some(base),
        "the second pick is the same built power"
    );
    assert_eq!(data.heroes.power(base).name, "Spirit Swap");
}

#[test]
fn gallywixs_gold_coin_is_a_gold_gain_spell() {
    let data = Arc::new(Data::bundled());
    let card = data.cards.by_name("Gold Coin").expect("Gold Coin is built");
    let c = data.cards.get(card);
    assert!(c.is_spell(), "a spell");
    assert!(c.token, "never in the pool");
    assert_eq!(c.cost, 0);
    assert!(!c.tavern_spell, "a plain spell, not a Tavern spell");

    let mut g = Game::new_sandbox(data, 1, None, 0);
    g.players[0].gold = 4;
    assert!(g.give(0, "Gold Coin").is_some(), "the card goes to hand");
    g.step(Action::Play { hand: 0, pos: 0 }).expect("the Coin is playable");
    assert_eq!(g.players[0].gold, 5, "Gain 1 Gold");
}
