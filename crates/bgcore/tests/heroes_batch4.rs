//! Trinkets block 5: Tickatus, whose Prize Wall Discovers a Darkmoon Prize
//! every four turns (docs/trinkets.md, "Rules for the last trinket batch").

use std::sync::Arc;

use bgcore::{Action, CardId, Data, Game, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn lobby() -> Tribes {
    Tribe::EVERY.into_iter().fold(Tribes::NONE, |m, t| m.with(t))
}

/// A no-pick game with every tribe in the lobby, player 0 forced to `hero`.
fn hero_game(seed: u64, hero: &str) -> Game {
    let d = data();
    let mut g = Game::new_with_lobby(d.clone(), seed, lobby());
    let h = d.heroes.by_name(hero).expect(hero);
    g.assign_hero(0, h);
    g
}

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).expect(name)
}

/// Answer whatever the current player has open, so a helper that only wants
/// the next turn never stalls on a prompt.
fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let mut cands = [Action::EndTurn; 4];
    let n = if !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || pl.pending_choose_one.is_some()
        || pl.wager_offer.is_some()
        || pl.holmes_offer.is_some()
    {
        cands[0] = Action::Choose(0);
        1
    } else if pl.pending_discard.is_some() {
        cands[0] = Action::Discard(0);
        1
    } else if pl.pending_shop_target.is_some() {
        cands[0] = Action::TargetShop(0);
        1
    } else if pl.pending_target.is_some() {
        cands[0] = Action::Target(0);
        cands[1] = Action::TargetShop(0);
        2
    } else if !pl.trinket_offer.is_empty() {
        cands[0] = Action::ChooseTrinket(0);
        1
    } else {
        return false;
    };
    for &a in &cands[..n] {
        if g.is_legal(a) {
            g.step(a).ok();
            return true;
        }
    }
    false
}

fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        if !answer(g) {
            g.step(Action::Freeze).unwrap();
        }
    }
}

/// End every turn until `turn` starts, keeping everyone alive.
fn to_turn(g: &mut Game, turn: u32) {
    while g.turn < turn && !g.over {
        for pl in &mut g.players {
            pl.health = pl.health.max(30);
        }
        let t = g.turn;
        while g.turn == t && !g.over {
            if !answer(g) {
                g.step(Action::EndTurn).unwrap();
            }
        }
    }
}

#[test]
fn tickatus_has_prize_wall_and_armor_from_the_game_data() {
    let g = hero_game(1, "Tickatus");
    let h = &g.data.heroes.get(g.players[0].hero.expect("a hero"));
    assert_eq!(h.name, "Tickatus");
    assert_eq!(h.card_id, "TB_BaconShop_HERO_94");
    assert_eq!(h.armor, 17);
    let p = g.data.heroes.power(g.players[0].power.expect("a power").of);
    assert_eq!(p.name, "Prize Wall");
    assert_eq!(p.card_id, "TB_BaconShop_HP_106");
    assert!(p.passive, "it has no button");
}

#[test]
fn prize_wall_discovers_a_prize_on_turns_4_8_12_and_16_with_the_tier_rising() {
    let mut g = hero_game(2, "Tickatus");
    // Turn 1 to 3: nothing.
    for turn in 1..=3u32 {
        to_turn(&mut g, turn);
        turn_of(&mut g, 0);
        assert!(g.players[0].discover.is_empty(), "turn {turn} has no Prize");
    }
    for (turn, tier) in [(4u32, 1u8), (8, 2), (12, 3), (16, 4)] {
        to_turn(&mut g, turn);
        turn_of(&mut g, 0);
        assert!(!g.players[0].discover.is_empty(), "turn {turn} offers a Prize");
        for &c in g.players[0].discover.clone().iter() {
            assert_eq!(
                g.data.cards.get(c).prize,
                Some(tier),
                "turn {turn} offers Tier {tier} Prizes"
            );
        }
        g.step(Action::Choose(0)).unwrap();
        // The pick lands in hand.
        assert!(
            g.players[0].hand.last().is_some_and(|m| g.data.cards.get(m.card).prize.is_some()),
            "the Prize is in hand"
        );
        g.check_invariants().unwrap();
    }
    // Tier 4 every four turns after 16, and never another tier.
    to_turn(&mut g, 20);
    turn_of(&mut g, 0);
    assert!(!g.players[0].discover.is_empty(), "turn 20 offers a Prize");
    for &c in g.players[0].discover.clone().iter() {
        assert_eq!(g.data.cards.get(c).prize, Some(4), "Tier 4 from turn 16 on");
    }
    g.step(Action::Choose(0)).unwrap();
    g.check_invariants().unwrap();
}

#[test]
fn the_observation_carries_the_turns_to_the_next_prize() {
    let mut g = hero_game(3, "Tickatus");
    // Between Prizes the power's numbers say the tier coming and the turns to
    // it; on a Prize turn the turns left are 0.
    to_turn(&mut g, 5);
    turn_of(&mut g, 0);
    let mut obs = vec![f32::NAN; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut obs);
    let tier = obs[bgcore::obs::SCALAR_POWER_PROGRESS];
    let left = obs[bgcore::obs::SCALAR_POWER_LEFT];
    assert_eq!(tier, 2.0, "the next Prize is a Tier 2 one");
    assert_eq!(left, 3.0, "three turns to it (turns 6, 7 and 8)");
    to_turn(&mut g, 8);
    turn_of(&mut g, 0);
    let mut obs = vec![f32::NAN; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut obs);
    assert_eq!(obs[bgcore::obs::SCALAR_POWER_PROGRESS], 2.0, "this turn's tier");
    assert_eq!(obs[bgcore::obs::SCALAR_POWER_LEFT], 0.0, "the Prize is now");
    assert!(!g.players[0].discover.is_empty());
}

#[test]
fn the_observation_says_whether_the_ice_block_is_held() {
    let mut g = hero_game(4, "Tickatus");
    let i = g.data.trinkets.by_name("Safety Patch").unwrap();
    let mut obs = vec![f32::NAN; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut obs);
    assert_eq!(obs[bgcore::obs::SCALAR_ICE_BLOCK], 0.0);
    turn_of(&mut g, 0);
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = 2;
    let _ = g.players[0].trinket_offer.try_push((i, 0));
    g.step(Action::ChooseTrinket(0)).unwrap();
    let mut obs = vec![f32::NAN; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut obs);
    assert_eq!(obs[bgcore::obs::SCALAR_ICE_BLOCK], 1.0, "the save is visible");
    assert!(g.players[0].secrets.ice_block());
}

#[test]
fn the_prizes_a_hero_power_hands_out_are_the_data_sets() {
    // Every prize in `CardDb.prizes` is a real card of that tier, and the
    // Prize discover never offers the same one twice (Era's rule: three
    // different Prizes of one tier).
    let mut g = hero_game(5, "Tickatus");
    to_turn(&mut g, 4);
    turn_of(&mut g, 0);
    let offer: Vec<CardId> = g.players[0].discover.to_vec();
    assert_eq!(offer.len(), 3);
    let mut s = offer.clone();
    s.sort_unstable();
    s.dedup();
    assert_eq!(s.len(), 3, "three different Prizes");
    let want = &g.data.cards.prizes[0];
    for c in offer {
        assert!(want.contains(&c), "{} is a Tier 1 Prize", g.data.cards.get(c).name);
    }
    // All-Seeing Elder is the one Tier 4 Prize that is a minion, and it comes
    // out of a Tier 4 Discover as a generated copy.
    let elder = id(&g, "All-Seeing Elder");
    assert_eq!(g.data.cards.get(elder).prize, Some(4));
    g.check_invariants().unwrap();
}
