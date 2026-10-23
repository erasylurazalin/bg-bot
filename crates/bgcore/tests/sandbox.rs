//! The sandbox game: one player, a rigged Tavern, free Refreshes and the
//! mirror fight, plus the recording of its boards.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::record::{self, GameRecord, TurnRecord};
use bgcore::{Action, Data, Game, Rng, Tribe};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn sandbox(seed: u64, tribe: Tribe, free: u8) -> Game {
    Game::new_sandbox(data(), seed, tribe, free)
}

#[test]
fn rigged_tavern_shows_only_the_tribe_and_neutrals() {
    for tribe in Tribe::ALL {
        let mut g = sandbox(7, tribe, 3);
        let (mut tribal, mut neutral) = (0, 0);
        let mut rolls = 0;
        let mut rng = Rng::new(3);
        while !g.over && rolls < 400 {
            // level up whenever possible so every tier's Tavern gets rolled
            if g.is_legal(Action::Level) && rng.below(2) == 0 {
                g.step(Action::Level).unwrap();
            }
            if g.is_legal(Action::Roll) {
                g.step(Action::Roll).unwrap();
                rolls += 1;
            } else {
                g.step(Action::EndTurn).unwrap();
            }
            for m in &g.players[0].shop {
                let c = g.data.cards.get(m.card);
                if c.is_spell() {
                    continue;
                }
                assert!(
                    c.tribes.is_neutral() || c.tribes.has(tribe),
                    "{tribe:?} Tavern showed {}",
                    c.name
                );
                if c.tribes.is_neutral() {
                    neutral += 1;
                } else {
                    tribal += 1;
                }
            }
            g.check_invariants().unwrap();
        }
        assert!(tribal > 100 && neutral > 20, "{tribe:?}: {tribal} tribal, {neutral} neutral");
    }
}

#[test]
fn normal_games_are_not_rigged() {
    let mut g = Game::new(data(), 5);
    assert!(g.sandbox.is_none());
    assert!(g.players.iter().all(|p| p.rig.is_none()));
    let mut seen = [false; 5];
    let mut rng = Rng::new(1);
    for _ in 0..300 {
        if g.over {
            break;
        }
        let a = random_action(&g, &mut rng);
        g.step(a).unwrap();
        for m in &g.players[0].shop {
            for (i, t) in Tribe::ALL.iter().enumerate() {
                seen[i] |= g.data.cards.get(m.card).tribes.has(*t);
            }
        }
    }
    assert!(seen.iter().all(|&s| s), "a normal Tavern should show every tribe: {seen:?}");
}

#[test]
fn three_free_refreshes_then_paid() {
    let mut g = sandbox(11, Tribe::Beast, 3);
    assert_eq!(g.players[0].gold, 3);
    for left in [2, 1, 0] {
        g.step(Action::Roll).unwrap();
        assert_eq!(g.players[0].free_rolls, left);
        assert_eq!(g.players[0].gold, 3, "a free Refresh costs no Gold");
    }
    g.step(Action::Roll).unwrap();
    assert_eq!(g.players[0].gold, 2, "the fourth Refresh is paid");
    // the next turn gives the free Refreshes back
    g.step(Action::EndTurn).unwrap();
    assert_eq!(g.turn, 2);
    assert_eq!(g.players[0].free_rolls, 3);
    // and the flag sets the number
    let g0 = sandbox(11, Tribe::Beast, 0);
    assert_eq!(g0.players[0].free_rolls, 0);
}

fn put(g: &mut Game, names: &[&str]) {
    g.players[0].board.clear();
    for n in names {
        let m = g.data.cards.make(g.data.cards.by_name(n).unwrap(), false);
        g.players[0].board.push(m);
    }
}

#[test]
fn mirror_fight_applies_lobster_count_and_spell_bonus() {
    let mut g = sandbox(21, Tribe::Beast, 3);
    put(&mut g, &["Tasty Lobster", "Blue Whelp"]);
    let lobster_before = g.players[0].improve[0];
    let spell_before = g.players[0].spell_bonus;
    g.step(Action::EndTurn).unwrap();
    let fight = g.sandbox.as_ref().unwrap().last_fight.clone().unwrap();
    assert_eq!(fight.turn, 1);
    assert_eq!(fight.board.len(), 2, "the recorded board is the one that fought");
    assert!(g.players[0].improve[0] > lobster_before, "Lobster count did not grow");
    assert!(g.players[0].spell_bonus.1 > spell_before.1, "Blue Whelp's spell bonus did not grow");
    assert_eq!(g.players[0].spell_bonus.1 - spell_before.1, fight.side.spell_bonus_gain.1);
    g.check_invariants().unwrap();
}

#[test]
fn mirror_fight_never_changes_health() {
    let mut g = sandbox(33, Tribe::Dragon, 3);
    for _ in 0..12 {
        // a weak board that can lose to its own mirror
        put(&mut g, &["Glim Guardian", "Blue Whelp"]);
        g.step(Action::EndTurn).unwrap();
        assert_eq!(g.players[0].health, 30);
    }
    assert!(!g.over);
}

#[test]
fn shop_damage_ends_the_game() {
    let mut g = sandbox(41, Tribe::Demon, 3);
    put(&mut g, &["Wrath Weaver"]);
    g.players[0].health = 1;
    let w = g.data.cards.make(g.data.cards.by_name("Wrath Weaver").unwrap(), false);
    g.players[0].hand.push(w);
    // playing a Demon next to Wrath Weaver makes the hero take 1 damage
    g.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    assert!(g.over, "the hero should be knocked out by the Wrath Weaver damage");
}

fn record_of(g: &Game, turns: &[TurnRecord]) -> GameRecord {
    let tribe = g.sandbox.as_ref().unwrap().tribe;
    GameRecord {
        tribe: format!("{tribe:?}"),
        label: "test".into(),
        seed: 99,
        free_rolls: 3,
        date: record::today(),
        turns: turns.to_vec(),
    }
}

#[test]
fn a_recorded_game_round_trips_through_the_loader() {
    // Random play in a Mech sandbox for many turns, so boards carry
    // Magnetizations, improvements, golden minions and hands.
    let mut tried_attached = false;
    for seed in 0..6u64 {
        let mut g = sandbox(seed, Tribe::Mech, 3);
        let mut rng = Rng::new(seed + 100);
        let mut turns: Vec<TurnRecord> = Vec::new();
        let mut steps = 0;
        while !g.over && steps < 4000 {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            steps += 1;
            if let Some(f) = &g.sandbox.as_ref().unwrap().last_fight
                && turns.last().is_none_or(|l| l.turn != f.turn)
            {
                turns.push(TurnRecord::from_fight(f));
            }
        }
        assert!(turns.len() >= 5, "seed {seed}: only {} turns recorded", turns.len());
        tried_attached |= turns.iter().flat_map(|t| &t.board).any(|m| m.magnetizations > 0);
        let rec = record_of(&g, &turns);
        let text = record::to_toml(&g.data.cards, &rec).unwrap();
        let back = record::load_str(&g.data.cards, &text).unwrap();
        assert_eq!(back, rec, "seed {seed} did not round trip:\n{text}");
    }
    assert!(tried_attached, "no recorded board had a Magnetization, the test is too weak");
}

#[test]
fn the_loader_rejects_garbage() {
    let db = data().cards.clone();
    assert!(record::load_str(&db, "not toml at all =").is_err());
    let bad = "tribe=\"Beast\"\nlabel=\"x\"\nseed=1\nfree_rolls=3\ndate=\"2026-01-01\"\n\
        [[turn]]\nturn=1\ntier=1\nhealth=30\nresult=\"win\"\n[[turn.board]]\ncard=\"Nope\"\n\
        golden=false\nattack=1\nhealth=1\nkeywords=[]\n";
    assert!(record::load_str(&db, bad).unwrap_err().contains("Nope"));
}
