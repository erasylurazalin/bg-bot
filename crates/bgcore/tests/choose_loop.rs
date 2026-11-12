//! A choice answered by the wrong prompt: a Discover and two Hero Powers open
//! at once (docs/briefs/choose-loop.md). The resolver answers the power offer
//! first, so the Discover's third option must not be legal there.

use std::sync::Arc;

use bgcore::{Action, CardId, Data, Game, Tribe, Tribes, N_ACTIONS};

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).expect(name)
}

#[test]
fn a_discover_beside_a_two_power_offer_does_not_offer_its_third_option() {
    let d = Arc::new(Data::bundled());
    let lobby = Tribe::EVERY.iter().copied().fold(Tribes::NONE, |m, t| m.with(t));
    let mut g = Game::new_with_lobby(d.clone(), 7, lobby);
    let h = d.heroes.by_name("Master Nguyen").expect("Master Nguyen");
    g.assign_hero(0, h);
    while g.current_player() != Some(0) {
        g.step(Action::EndTurn).expect("a hero-less seat skips its pick");
    }
    assert_eq!(g.players[0].power_offer.len(), 2, "Power of the Storm deals two");
    let cards = [
        id(&g, "Boon of Beetles"),
        id(&g, "Gem Confiscation"),
        id(&g, "Weapons Forge"),
    ];
    for c in cards {
        g.players[0].discover.push(c);
    }
    assert_eq!(g.players[0].discover.len(), 3);

    // The resolver answers the two-power offer first, so the Discover's third
    // option is not an answer to it. It used to be legal through the Discover
    // branch, and `pick_power` then picked nothing and left both prompts as
    // they were, which the model could choose forever.
    assert!(
        !g.is_legal(Action::Choose(2)),
        "the third option belongs to the Discover, not the power offer"
    );
    let mut mask = [false; N_ACTIONS];
    g.legal_mask(&mut mask);
    assert!(!mask[Action::Choose(2).index()]);
    assert!(g.is_legal(Action::Choose(0)));
    assert!(g.is_legal(Action::Choose(1)));

    // The loop: this action used to succeed without changing the state.
    assert!(g.step(Action::Choose(2)).is_err());
    assert_eq!(g.players[0].power_offer.len(), 2);
    assert_eq!(g.players[0].discover.len(), 3);

    // The power offer is still answerable on its own, and the Discover is
    // still there for after it.
    g.step(Action::Choose(0)).expect("the power answer");
    assert!(g.players[0].power_offer.is_empty());
    assert_eq!(g.players[0].discover.len(), 3);
    g.check_invariants().unwrap();
}
