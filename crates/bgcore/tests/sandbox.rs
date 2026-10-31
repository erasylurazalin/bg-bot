//! The sandbox game: one player, a fixed lobby, free Refreshes and the
//! mirror fight, plus the recording of its boards.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::record::{self, GameRecord, TurnRecord};
use bgcore::{Action, Data, Game, Rng, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn sandbox(seed: u64, lobby: Tribes, free: u8) -> Game {
    Game::new_sandbox(data(), seed, Some(lobby), free)
}

fn one(tribe: Tribe) -> Tribes {
    Tribes::of(tribe)
}

/// Answer whatever the sandbox player has open (a Discover, a trinket
/// offer), so a loop that only rolls and passes never stalls on a prompt.
fn answer(g: &mut Game) -> bool {
    let pl = &g.players[0];
    let a = if !pl.discover.is_empty() || !pl.power_offer.is_empty() || pl.pending_choose_one.is_some() {
        Action::Choose(0)
    } else if pl.pending_discard.is_some() {
        Action::Discard(0)
    } else if pl.pending_shop_target.is_some() {
        Action::TargetShop(0)
    } else if pl.pending_target.is_some() {
        Action::Target(0)
    } else if !pl.trinket_offer.is_empty() {
        Action::ChooseTrinket(0)
    } else {
        return false;
    };
    if !g.is_legal(a) {
        return false;
    }
    g.step(a).ok();
    true
}

#[test]
fn a_one_tribe_lobby_shows_only_the_tribe_and_neutrals() {
    for tribe in Tribe::EVERY {
        let lobby = one(tribe);
        let mut g = sandbox(7, lobby, 3);
        let (mut tribal, mut neutral) = (0, 0);
        let mut rolls = 0;
        let mut rng = Rng::new(3);
        while !g.over && rolls < 400 {
            if answer(&mut g) {
                continue;
            }
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
                    c.lobby_ok(lobby),
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
fn normal_games_roll_a_five_tribe_lobby() {
    let data = data();
    let mut seen = [false; Tribe::EVERY.len()];
    for seed in 0..30u64 {
        let mut g = Game::new(data.clone(), seed);
        assert!(g.sandbox.is_none());
        assert_eq!(g.lobby.len(), 5, "seed {seed}: {:?}", g.lobby);
        for t in Tribe::EVERY {
            seen[t as usize] |= g.lobby.has(t);
        }
        let mut rng = Rng::new(1);
        let mut rolls = 0;
        while !g.over && rolls < 60 {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            for m in g.players.iter().flat_map(|p| &p.shop) {
                assert!(
                    g.data.cards.get(m.card).lobby_ok(g.lobby),
                    "seed {seed}: {} is outside the lobby",
                    g.data.cards.get(m.card).name
                );
            }
            if g.is_legal(Action::Roll) {
                g.step(Action::Roll).unwrap();
                rolls += 1;
            }
        }
    }
    assert!(
        seen.iter().all(|&s| s),
        "30 seeds should roll every tribe into some lobby: {seen:?}"
    );
}

#[test]
fn three_free_refreshes_then_paid() {
    let mut g = sandbox(11, one(Tribe::Beast), 3);
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
    let g0 = sandbox(11, one(Tribe::Beast), 0);
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
    let mut g = sandbox(21, one(Tribe::Beast), 3);
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
    let mut g = sandbox(33, one(Tribe::Dragon), 3);
    for _ in 0..12 {
        // a weak board that can lose to its own mirror
        put(&mut g, &["Glim Guardian", "Blue Whelp"]);
        // A trinket picked on the way out of a turn can open a Discover at
        // the start of the next one, and that has to be answered first.
        while !g.is_legal(Action::EndTurn) {
            assert!(answer(&mut g), "EndTurn is illegal with nothing to answer");
        }
        g.step(Action::EndTurn).unwrap();
        assert_eq!(g.players[0].health, 30);
    }
    assert!(!g.over);
}

#[test]
fn shop_damage_ends_the_game() {
    let mut g = sandbox(41, one(Tribe::Demon), 3);
    put(&mut g, &["Wrath Weaver"]);
    g.players[0].health = 1;
    let w = g.data.cards.make(g.data.cards.by_name("Wrath Weaver").unwrap(), false);
    g.players[0].hand.push(w);
    // playing a Demon next to Wrath Weaver makes the hero take 1 damage
    g.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    assert!(g.over, "the hero should be knocked out by the Wrath Weaver damage");
}

fn record_of(turns: &[TurnRecord]) -> GameRecord {
    GameRecord {
        tribe: None,
        label: "test".into(),
        seed: 99,
        free_rolls: 3,
        date: record::today(),
        turns: turns.to_vec(),
    }
}

#[test]
fn a_recorded_game_round_trips_through_the_loader() {
    // Random play in a Mech lobby for many turns, so boards carry
    // Magnetizations, improvements, golden minions and hands.
    let mut tried_attached = false;
    for seed in 0..12u64 {
        let mut g = sandbox(seed, one(Tribe::Mech), 3);
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
        let rec = record_of(&turns);
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

/// The cards the player is now being offered by a Discover.
fn offered(g: &Game) -> Vec<bgcore::CardId> {
    g.players[0].discover.iter().copied().collect()
}

fn give_card(g: &mut Game, name: &str, golden: bool) {
    let id = g.data.cards.by_name(name).unwrap();
    let m = g.data.cards.make(id, golden);
    g.players[0].hand.push(m);
}

fn assert_in_lobby(g: &Game, lobby: Tribes, what: &str) {
    let o = offered(g);
    assert_eq!(o.len(), 3, "{what}: {o:?}");
    for id in o {
        let c = g.data.cards.get(id);
        assert!(c.lobby_ok(lobby), "{what} offered {}", c.name);
    }
}

#[test]
fn triple_reward_discover_stays_in_the_lobby() {
    for seed in 0..30 {
        let mut g = sandbox(seed, one(Tribe::Beast), 3);
        g.players[0].tier = 5;
        give_card(&mut g, "Wolf Pup", true);
        g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
        // the reward is a spell in hand now: casting it opens the Discover
        g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
        assert_in_lobby(&g, one(Tribe::Beast), "triple reward");
        g.check_invariants().unwrap();
    }
}

#[test]
fn generic_discover_stays_in_the_lobby() {
    // Hired Headhunter: "Discover a Battlecry minion", an any-minion draw
    for tribe in [Tribe::Beast, Tribe::Mech, Tribe::Demon] {
        for seed in 0..30 {
            let mut g = sandbox(seed, one(tribe), 3);
            g.players[0].tier = 6;
            give_card(&mut g, "Hired Headhunter", false);
            g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
            assert_in_lobby(&g, one(tribe), "Hired Headhunter");
            g.check_invariants().unwrap();
        }
    }
}

#[test]
fn named_tribe_discover_keeps_its_tribe() {
    // Primalfin Lookout in a lobby that also has Murlocs still discovers
    // Murlocs, not just any lobby card.
    let lobby = lobby(&[Tribe::Beast, Tribe::Murloc, Tribe::Mech, Tribe::Dragon, Tribe::Demon]);
    for seed in 0..30 {
        let mut g = sandbox(seed, lobby, 3);
        g.players[0].tier = 6;
        for n in ["Bubble Gunner", "Primalfin Lookout"] {
            give_card(&mut g, n, false);
        }
        g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
        g.step(Action::Play { hand: 0, pos: 1 }).unwrap();
        let o = offered(&g);
        assert_eq!(o.len(), 3, "seed {seed}");
        for id in o {
            let c = g.data.cards.get(id);
            assert!(c.tribes.has(Tribe::Murloc), "offered {} (not a Murloc)", c.name);
        }
    }
}

fn lobby(ts: &[Tribe]) -> Tribes {
    ts.iter().copied().fold(Tribes::NONE, |m, t| m.with(t))
}

#[test]
fn the_lobby_reaches_random_draws_and_the_fight() {
    let g = sandbox(1, one(Tribe::Beast), 3);
    let db = &g.data.cards;
    assert_eq!(g.players[0].counters(&g.data, g.lobby).lobby, g.lobby);
    let mut rng = Rng::new(9);
    for _ in 0..500 {
        let c = db.get(db.random_deathrattle_minion(6, g.lobby, &mut rng).unwrap());
        assert!(c.deathrattle.is_some() && c.lobby_ok(g.lobby));
        let c = db.get(db.random_tier_minion(3, g.lobby, &mut rng).unwrap());
        assert!(c.tier == 3 && c.lobby_ok(g.lobby));
    }
    // no lobby filter (Tribes::ALL): any tribe shows up
    let seen: std::collections::HashSet<_> =
        (0..500).map(|_| db.random_tier_minion(3, Tribes::ALL, &mut rng).unwrap()).collect();
    assert!(seen.iter().any(|&c| db.get(c).tribes.has(Tribe::Murloc)));
    // the mirror fight with a Ghastcoiler runs and keeps the pool whole
    let mut g = sandbox(2, one(Tribe::Beast), 3);
    put(&mut g, &["Ghastcoiler", "Tasty Lobster"]);
    g.step(Action::EndTurn).unwrap();
    g.check_invariants().unwrap();
}

#[test]
fn a_one_tribe_lobby_draws_at_every_tier() {
    let g = sandbox(1, one(Tribe::Beast), 3);
    let db = &g.data.cards;
    let mut rng = Rng::new(4);
    // every tier has a Beast or neutral minion, so every draw finds one
    for tier in 1..=6 {
        assert!(db.random_tier_minion(tier, g.lobby, &mut rng).is_some());
    }
}

#[test]
fn a_rolled_sandbox_lobby_keeps_its_tribes() {
    let mut g = Game::new_sandbox(data(), 3, None, 0);
    assert_eq!(g.lobby.len(), 5);
    assert_eq!(g.players[0].free_rolls, 0);
    let mut rolls = 0;
    while !g.over && rolls < 300 {
        if answer(&mut g) {
            continue;
        }
        if g.is_legal(Action::Level) {
            g.step(Action::Level).unwrap();
        }
        if g.is_legal(Action::Roll) {
            g.step(Action::Roll).unwrap();
            rolls += 1;
        } else {
            g.step(Action::EndTurn).unwrap();
        }
        for m in &g.players[0].shop {
            assert!(
                g.data.cards.get(m.card).lobby_ok(g.lobby),
                "{} is outside the lobby",
                g.data.cards.get(m.card).name
            );
        }
    }
    // a Discover offers other lobby tribes too, in a lobby with several
    let lobby = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]);
    let mut other = false;
    for seed in 0..30 {
        let mut g = Game::new_sandbox(data(), seed, Some(lobby), 0);
        g.players[0].tier = 6;
        give_card(&mut g, "Hired Headhunter", false);
        g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
        other |= offered(&g).iter().any(|&c| {
            let t = g.data.cards.get(c).tribes;
            !t.is_neutral() && !t.has(Tribe::Beast)
        });
    }
    assert!(other);
}

#[test]
fn an_unrigged_record_round_trips() {
    let mut g = Game::new_sandbox(data(), 5, None, 0);
    let mut turns = Vec::new();
    for _ in 0..3 {
        g.step(Action::EndTurn).unwrap();
        turns.push(TurnRecord::from_fight(g.sandbox.as_ref().unwrap().last_fight.as_ref().unwrap()));
    }
    let rec = GameRecord {
        tribe: None,
        label: "unlabeled".into(),
        seed: 5,
        free_rolls: 0,
        date: record::today(),
        turns,
    };
    let text = record::to_toml(&g.data.cards, &rec).unwrap();
    assert!(text.contains("rigged = false"));
    assert_eq!(record::load_str(&g.data.cards, &text).unwrap(), rec);
}
