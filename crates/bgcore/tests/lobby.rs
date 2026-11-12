//! The 5-tribe lobby: how a game rolls it, how it filters the pool and
//! every draw, the tied neutrals and spells, and the Deity rule.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::{CardDb, Data, Game, Rng, Tribe, Tribes};

fn lobby(ts: &[Tribe]) -> Tribes {
    ts.iter().copied().fold(Tribes::NONE, |m, t| m.with(t))
}

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

/// A fixed lobby for tests: Beast, Mech, Dragon, Murloc, Demon.
fn game(seed: u64) -> Game {
    Game::new_with_lobby(data(), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]))
}

/// Every card a player can see or hold: the Tavern, the spell slot, the
/// hand, the board and the open Discover options.
fn visible_cards(g: &Game) -> Vec<bgcore::CardId> {
    let mut out = Vec::new();
    for pl in &g.players {
        for m in pl.shop.iter().chain(&pl.hand).chain(&pl.board) {
            out.push(m.card);
        }
        for (c, _) in pl.board.iter().flat_map(|m| m.parts()) {
            out.push(c);
        }
        if let Some(s) = pl.shop_spell {
            out.push(s);
        }
        out.extend(&pl.discover);
    }
    out
}

#[test]
fn every_game_rolls_five_distinct_tribes_and_is_deterministic() {
    let d = data();
    let mut seen = [false; Tribe::EVERY.len()];
    let mut lobbies = std::collections::HashSet::new();
    for seed in 0..200u64 {
        let g = Game::new(d.clone(), seed);
        assert_eq!(g.lobby.len(), 5, "seed {seed}");
        for t in Tribe::EVERY {
            seen[t as usize] |= g.lobby.has(t);
        }
        lobbies.insert(g.lobby);
        // deterministic per seed: the same seed rolls the same lobby
        let again = Game::new(d.clone(), seed);
        assert_eq!(again.lobby, g.lobby, "seed {seed}");
    }
    assert!(lobbies.len() >= 100, "200 seeds should roll many lobbies (252 are possible)");
    assert!(
        seen.iter().all(|&s| s),
        "200 seeds should put every tribe in some lobby: {seen:?}"
    );
}

#[test]
fn a_fixed_lobby_is_respected_by_the_pool() {
    let d = data();
    let g = game(1);
    for c in 0..d.cards.pool_cards {
        let card = &d.cards.cards[c];
        if card.pool_slot as usize != c {
            continue;
        }
        // Dark Paradox: only the one version this game picked is in the pool
        // (every version is an all-type minion, so all five are lobby-ok).
        let in_pool = card.lobby_ok(g.lobby) && (!card.paradox || g.paradox == Some(c as u16));
        if in_pool {
            // the first Tavern may hold some of them: the exact accounting is
            // check_invariants' job, this asserts the card is in the pool
            assert!(g.pool[c] > 0, "{} has no copies in the pool", card.name);
        } else {
            assert_eq!(g.pool[c], 0, "{}", card.name);
        }
    }
    g.check_invariants().unwrap();
    // the game reports the lobby it was given
    assert_eq!(g.lobby, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]));
}

#[test]
fn tied_neutrals_and_spells_only_exist_in_their_tribes_lobby() {
    let d = data();
    let ties = [
        ("Disguised Graverobber", Tribe::Undead),
        ("Nadina the Red", Tribe::Dragon),
        ("Butchering", Tribe::Undead),
        ("Tomb Turning", Tribe::Undead),
        ("Wealthy Bounty", Tribe::Pirate),
        ("Cloning Conch", Tribe::Murloc),
        ("Boon of Beetles", Tribe::Beast),
        ("Brood of Nozdormu", Tribe::Dragon),
        ("Corrupted Coin", Tribe::Aberration),
        ("Corrupted Cupcakes", Tribe::Demon),
        ("Easterly Winds", Tribe::Elemental),
        ("Blood Gem Barrage", Tribe::Quilboar),
    ];
    for (name, tied) in ties {
        let id = d.cards.by_name(name).unwrap();
        let card = d.cards.get(id);
        assert_eq!(card.lobby, Some(tied), "{name}");
        // a lobby with the tribe: the neutral is in the pool
        let with = Tribes::of(tied)
            .with(Tribe::EVERY[(tied as usize + 1) % Tribe::EVERY.len()])
            .with(Tribe::EVERY[(tied as usize + 2) % Tribe::EVERY.len()])
            .with(Tribe::EVERY[(tied as usize + 3) % Tribe::EVERY.len()])
            .with(Tribe::EVERY[(tied as usize + 4) % Tribe::EVERY.len()]);
        // a lobby without it: the first 5 tribes that are not the tied one
        let mut without = Tribes::NONE;
        for t in Tribe::EVERY {
            if t != tied && without.len() < 5 {
                without = without.with(t);
            }
        }
        assert!(!without.has(tied) && with.has(tied));
        assert!(card.lobby_ok(with), "{name} should be in");
        assert!(!card.lobby_ok(without), "{name} should be out");
        if !card.is_spell() {
            let g_with = Game::new_with_lobby(d.clone(), 1, with);
            assert!(g_with.pool[card.pool_slot as usize] > 0, "{name} in the pool");
            let g_without = Game::new_with_lobby(d.clone(), 1, without);
            assert_eq!(g_without.pool[card.pool_slot as usize], 0, "{name} out of the pool");
        }
    }
    // a spell draw outside Undead never offers a tied spell, inside it does
    let with = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Undead]);
    let without = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]);
    let mut rng = Rng::new(5);
    let mut seen_undead = false;
    for _ in 0..4000 {
        if let Some(id) = d.cards.random_spell(6, without, &mut rng) {
            assert!(d.cards.get(id).lobby_ok(without), "{} offered", d.cards.get(id).name);
        }
        if let Some(id) = d.cards.random_spell(6, with, &mut rng) {
            let name = d.cards.get(id).name.clone();
            assert!(d.cards.get(id).lobby_ok(with), "{name} offered");
            seen_undead |= name == "Butchering" || name == "Tomb Turning";
        }
    }
    assert!(seen_undead, "an Undead lobby draws its tied spells");
}

#[test]
fn a_dual_tribe_minion_is_in_when_either_tribe_is() {
    let d = data();
    // Felboar is Demon and Quilboar, Timecap'n Hooktail Dragon and Pirate:
    // either tribe puts it in, neither keeps it out.
    let in_demon = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]));
    let in_quilboar = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Quilboar]));
    let neither = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Undead]));
    let felboar_id = d.cards.by_name("Felboar").unwrap();
    let felboar = d.cards.get(felboar_id);
    let copies = d.rules.copies(felboar.tier);
    let felboar = felboar.pool_slot as usize;
    assert_eq!(in_demon.pool[felboar], copies, "Felboar in with Demons");
    assert_eq!(in_quilboar.pool[felboar], copies, "Felboar in with Quilboar");
    assert_eq!(neither.pool[felboar], 0, "Felboar out with neither tribe");
    let in_dragon = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]));
    let in_pirate = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Pirate]));
    let hooktail_id = d.cards.by_name("Timecap'n Hooktail").unwrap();
    let copies = d.rules.copies(d.cards.get(hooktail_id).tier);
    let hooktail = d.cards.get(hooktail_id).pool_slot as usize;
    assert_eq!(in_dragon.pool[hooktail], copies, "Hooktail in with Dragons");
    assert_eq!(in_pirate.pool[hooktail], copies, "Hooktail in with Pirates");
    let no_dragon_no_pirate = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Undead]));
    assert_eq!(no_dragon_no_pirate.pool[hooktail], 0, "Hooktail out with neither tribe");
    // an all-type minion is in every lobby
    let amalgam = d.cards.cards.iter().position(|c| c.tribes == Tribes::ALL).unwrap();
    for seed in 0..20 {
        let g = Game::new(d.clone(), seed);
        assert_eq!(g.pool[amalgam], d.rules.copies(d.cards.cards[amalgam].tier), "seed {seed}");
    }
}

#[test]
fn no_deity_without_aberration() {
    let d = data();
    let without = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]));
    for pl in &without.players {
        assert!(pl.deity.is_none());
    }
    let with = Game::new_with_lobby(d.clone(), 1, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Aberration]));
    let god = with.players[0].deity.unwrap().card;
    for pl in &with.players {
        // the whole lobby gets the same god
        assert_eq!(pl.deity.unwrap().card, god);
        assert!(
            d.cards.get(pl.deity.unwrap().card).deity.is_some(),
            "the Deity is one of the two god tokens"
        );
    }
    // every random game: a Deity exactly with Aberrations in the lobby
    for seed in 0..60 {
        let g = Game::new(d.clone(), seed);
        assert_eq!(
            g.players[0].deity.is_some(),
            g.lobby.has(Tribe::Aberration),
            "seed {seed}"
        );
    }
}

#[test]
fn every_draw_respects_the_lobby() {
    let d = data();
    let beast_lobby = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]);
    type Draw = Box<dyn Fn(&CardDb, &mut Rng) -> Option<bgcore::CardId>>;
    let draws: Vec<Draw> = vec![
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_pool_minion(Tribe::Beast, 0, beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_deathrattle_minion(0, beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_tier_minion(2, beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_choose_one_card(6, beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_tier7_minion(beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_spell(6, beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_spell_up_to(6, &[], beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_tier_spell(3, &[], beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| db.random_spell_of_cost(2, beast_lobby, rng)),
        Box::new(move |db: &CardDb, rng: &mut Rng| {
            let t = db.get(db.random_pool_minion(Tribe::Beast, 0, beast_lobby, rng).unwrap()).tribes;
            db.random_different_minion(t, 0, 0, beast_lobby, rng)
        }),
    ];
    let mut rng = Rng::new(11);
    for _ in 0..1000 {
        for f in &draws {
            if let Some(c) = f(&d.cards, &mut rng) {
                assert!(d.cards.get(c).lobby_ok(beast_lobby), "{} offered", d.cards.get(c).name);
            }
        }
    }
    // group-only draws: none without their tribe, some with it
    let no_pirates = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]);
    let with_pirates = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Pirate]);
    assert!(d.cards.random_bounty(no_pirates, &mut rng).is_none());
    assert!(d.cards.random_bounty(with_pirates, &mut rng).is_some());
    // the Chromadrakes are all Dragons, the Volumizers all Mechs
    let no_dragons = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Pirate]);
    let with_dragons = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]);
    assert!(d.cards.random_chromadrake(no_dragons, &mut rng).is_none());
    assert!(d.cards.random_chromadrake(with_dragons, &mut rng).is_some());
    let no_mechs = lobby(&[Tribe::Beast, Tribe::Dragon, Tribe::Murloc, Tribe::Demon, Tribe::Pirate]);
    let with_mechs = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]);
    assert!(d.cards.random_volumizer(no_mechs, &mut rng).is_none());
    assert!(d.cards.random_volumizer(with_mechs, &mut rng).is_some());
}

#[test]
fn a_random_choose_one_card_can_be_a_spell() {
    // "Get a random Choose One card" (Bramble Tunneler's Rally, the Glass of
    // Perspective) takes minions and Tavern spells alike:
    // `Hearthstone_2026_11_03_15_22_36-g3` turn 9 step 52 hands over Boundless
    // Potential, a Choose One Tavern spell.
    let d = data();
    let any_lobby = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Pirate]);
    let mut rng = Rng::new(3);
    let mut spells = 0;
    let mut minions = 0;
    for _ in 0..200 {
        let c = d.cards.random_choose_one_card(6, any_lobby, &mut rng).unwrap();
        let card = d.cards.get(c);
        assert!(card.choose_one.is_some(), "{} is a Choose One card", card.name);
        if card.is_spell() {
            spells += 1;
        } else {
            minions += 1;
        }
    }
    assert!(spells > 0, "a Tavern spell with a Choose One is in the pool");
    assert!(minions > 0, "and so are the Choose One minions");
}

#[test]
fn nothing_outside_the_lobby_ever_shows_up_in_whole_games() {
    let d = data();
    for seed in 0..40u64 {
        let mut g = Game::new(d.clone(), seed);
        let mut rng = Rng::new(seed ^ 0x1ea);
        let mut steps = 0;
        while let Some(p) = g.current_player() {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            for c in visible_cards(&g) {
                let card = g.data.cards.get(c);
                // A Timewarped minion is the Candelabra's, not a lobby
                // draw: its list ignores the lobby by rule.
                assert!(
                    card.lobby_ok(g.lobby) || card.timewarp.is_some(),
                    "seed {seed}, step {steps}, player {p}: {} is outside the lobby",
                    card.name
                );
            }
            g.check_invariants().unwrap();
            steps += 1;
        }
    }
}

#[test]
fn the_sandbox_plays_a_fixed_or_rolled_lobby() {
    let d = data();
    let fixed = Game::new_sandbox(d.clone(), 3, Some(lobby(&[Tribe::Pirate, Tribe::Elemental, Tribe::Undead, Tribe::Quilboar, Tribe::Aberration])), 2);
    assert_eq!(fixed.lobby, lobby(&[Tribe::Pirate, Tribe::Elemental, Tribe::Undead, Tribe::Quilboar, Tribe::Aberration]));
    for c in visible_cards(&fixed) {
        assert!(fixed.data.cards.get(c).lobby_ok(fixed.lobby));
    }
    let rolled = Game::new_sandbox(d.clone(), 3, None, 0);
    assert_eq!(rolled.lobby.len(), 5);
}

/// Patch 37.0.2: Expert Aviator left the pool. It keeps its id, its place in
/// old logs and boards and its effect, but no draw may offer it again
/// (docs/rules/design.md "The pool").
#[test]
fn a_card_that_left_the_pool_keeps_its_effect_and_is_never_drawn() {
    let d = data();
    let aviator = d.cards.by_name("Expert Aviator").unwrap();
    let card = d.cards.get(aviator);
    assert!(card.out_of_pool, "marked out of the pool, never deleted");
    assert!(card.rally.is_some(), "its effect is still built for old logs");
    assert!(!card.token, "it is not a token: its id and pool slot stay");
    for lobby in [Tribes::ALL, lobby(&[Tribe::Murloc, Tribe::Beast])] {
        assert!(!card.lobby_ok(lobby), "no draw may hand it out");
    }
    // A fresh game's pool has no copies of it, and the accounting agrees.
    let g = game(1);
    assert_eq!(g.pool[card.pool_slot as usize], 0, "no copies in the pool");
    let mut rng = Rng::new(9);
    for _ in 0..2000 {
        let draws = [
            d.cards.random_pool_minion(Tribe::Murloc, 2, Tribes::ALL, &mut rng),
            d.cards.random_any_minion(bgcore::MAX_TIER, Tribes::ALL, &mut rng),
            d.cards.random_tier_minion(2, Tribes::ALL, &mut rng),
            d.cards.random_battlecry_minion(2, Tribes::ALL, &mut rng),
            d.cards.random_deathrattle_minion(2, Tribes::ALL, &mut rng),
            d.cards.random_magnetic_minion(2, Tribes::ALL, &mut rng),
            d.cards.random_different_minion(Tribes::ALL, aviator, 2, Tribes::ALL, &mut rng),
        ];
        for draw in draws {
            assert_ne!(draw, Some(aviator), "a random draw offered it");
        }
    }
}
