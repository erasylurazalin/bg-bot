//! The Elemental tribe, with the real cards from cards.toml: the Ballers'
//! shared count, the Refresh amount, Ichoron's shield, Arcane Absorption and
//! Conflagration, free Refreshes, Elemental of Surprise triples, the two
//! counters on the minions (Ultraviolet Ascendant, Unbound Tempest), Stone
//! Age Slab, Living Prison, Wildfire Elemental and the spell counters.

use std::sync::Arc;

use bgcore::cards::Counter;
use bgcore::fight::{Watch, fight_with};
use bgcore::{Action, CardDb, CardId, Counters, Data, Game, Keywords, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Elemental, Tribe::Demon, Tribe::Pirate]))
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// A minion that never attacks and takes a long time to kill.
fn wall(db: &CardDb) -> Minion {
    let mut w = mk(db, "Microbot", false);
    w.attack = 0;
    w.health = 1000;
    w
}

fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        g.step(Action::Freeze).unwrap();
    }
}

fn act(g: &mut Game, p: usize, a: Action) {
    turn_of(g, p);
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

/// Everyone ends their turn until the next turn starts.
fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        g.step(Action::EndTurn).unwrap();
    }
}

/// A card from the pool straight onto the board; golden takes three copies.
fn put_on_board(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].board.push(m);
}

fn put_in_hand(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].hand.push(m);
}

fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for n in names {
        let c = id(&g.data.cards, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
}

/// Give a generated card (a spell or a minion token) and play it from hand.
fn cast(g: &mut Game, name: &str) {
    let h = g.give(0, name).map(|_| g.players[0].hand.len() - 1).unwrap();
    play(g, 0, h as u8, 0);
}

/// Put an Elemental from the pool into hand and play it to the right end.
fn play_new(g: &mut Game, name: &str) {
    put_in_hand(g, 0, name, false);
    let h = g.players[0].hand.len() as u8 - 1;
    let pos = g.players[0].board.len() as u8;
    play(g, 0, h, pos);
}

fn fight_deaths(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    max_attacks: u32,
    seed: u64,
) -> (bgcore::Outcome, Vec<(usize, CardId)>) {
    struct W(Vec<(usize, CardId)>);
    impl Watch for W {
        fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
            self.0.push((side, m.card));
        }
    }
    let mut w = W(Vec::new());
    let out = fight_with(db, a, b, [Counters::default(); 2], max_attacks, &mut Rng::new(seed), &mut w);
    (out, w.0)
}

/// The Attack and Health of the first attacker on side 0, as it attacked.
fn first_attacker(db: &CardDb, a: &[Minion], b: &[Minion]) -> (i32, i32) {
    struct W(Option<(i32, i32)>);
    impl Watch for W {
        fn attack(&mut self, side: usize, attacker: &Minion, _d: &Minion) {
            if side == 0 && self.0.is_none() {
                self.0 = Some((attacker.attack, attacker.health));
            }
        }
    }
    let mut w = W(None);
    let _ = fight_with(db, a, b, [Counters::default(); 2], 1, &mut Rng::new(1), &mut w);
    w.0.expect("side 0 attacked")
}

fn baller_count(g: &Game, p: usize) -> u16 {
    g.players[p].improve[Counter::Baller.index()]
}

// ================= the tribe and the cards =================

#[test]
fn all_the_elementals_are_in_the_pool_with_their_stats_and_tribes() {
    let db = db();
    let cards = [
        ("Crackling Cyclone", 1, (2, 1)),
        ("Dune Dweller", 1, (3, 3)),
        ("Fire Baller", 2, (4, 3)),
        ("Sellemental", 2, (3, 3)),
        ("Snow Baller", 2, (3, 4)),
        ("Waveling", 3, (5, 1)),
        ("Wildfire Elemental", 3, (6, 3)),
        ("Air Baller", 4, (6, 6)),
        ("En-Djinn Blazer", 4, (5, 5)),
        ("Ichoron the Protector", 4, (3, 1)),
        ("Leyline Surfacer", 4, (4, 6)),
        ("Living Prison", 4, (4, 5)),
        ("Refreshing Anomaly", 4, (4, 5)),
        ("Tavern Tempest", 4, (2, 2)),
        ("Air Revenant", 5, (3, 6)),
        ("Firelands Fugitive", 5, (5, 7)),
        ("Living Azerite", 5, (8, 6)),
        ("Elemental of Surprise", 6, (8, 8)),
        ("Ultraviolet Ascendant", 6, (6, 6)),
        ("Unbound Tempest", 6, (3, 12)),
        ("Stone Age Slab", 7, (10, 10)),
    ];
    for (n, tier, st) in cards {
        let c = id(&db, n);
        assert!((c as usize) < db.pool_cards, "{n} is in the pool");
        let card = db.get(c);
        assert_eq!(card.tier, tier, "{n}");
        assert_eq!((card.attack, card.health), st, "{n}");
        assert!(card.tribes.has(Tribe::Elemental), "{n}");
        assert!(!card.tribes.has(Tribe::Beast) && !card.tribes.is_neutral(), "{n}");
        assert_eq!(stats(&mk(&db, n, true)), (st.0 * 2, st.1 * 2), "{n} golden");
    }
    // Flaming Enforcer carries both of its tribes
    let fe = db.get(id(&db, "Flaming Enforcer"));
    assert!(fe.tribes.has(Tribe::Demon) && fe.tribes.has(Tribe::Elemental));
    // the Ichoron golden card has the doubled stats of the data
    assert_eq!(stats(&mk(&db, "Ichoron the Protector", true)), (6, 2));
    // keywords
    let cyc = mk(&db, "Crackling Cyclone", false);
    assert!(cyc.keywords.has(Keywords::DIVINE_SHIELD) && cyc.keywords.has(Keywords::WINDFURY));
    let sur = mk(&db, "Elemental of Surprise", false);
    assert!(sur.keywords.has(Keywords::DIVINE_SHIELD));
    // generated cards are never in the pool or the Tavern's spell draw
    for n in ["Arcane Absorption", "Conflagration", "Water Droplet"] {
        let c = id(&db, n);
        assert!(db.get(c).token, "{n} is generated only");
        assert!(!db.spells.contains(&c), "{n}");
        assert!(c as usize >= db.pool_cards, "{n}");
    }
    assert!(db.get(id(&db, "Water Droplet")).tribes.has(Tribe::Elemental));
    assert!(db.spells.contains(&id(&db, "Easterly Winds")));
    assert!(db.spells.contains(&id(&db, "Temperature Shift")));
    assert_eq!(db.get(id(&db, "Easterly Winds")).cost, 1);
    assert_eq!(db.get(id(&db, "Temperature Shift")).cost, 4);
}

#[test]
fn elementals_are_a_type_for_the_all_type_minions_and_the_sandbox_rig() {
    let db = db();
    let last = db.get(id(&db, "The Last One Standing"));
    assert!(last.tribes.has(Tribe::Elemental));
    let g = Game::new_sandbox(Arc::new(Data::bundled()), 3, Some(bgcore::Tribes::of(Tribe::Elemental)), 0);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        if !c.is_spell() {
            assert!(c.tribes.has(Tribe::Elemental) || c.tribes.is_neutral());
        }
    }
}

// ================= Ballers =================

#[test]
fn ballers_share_one_count_and_each_step_is_the_card_text() {
    let mut g = game(1);
    put_on_board(&mut g, 0, "Fire Baller", false);
    put_on_board(&mut g, 0, "Cord Puller", false); // 1/1
    // 3 Ballers sold before: a Fire Baller gives +4 Attack
    g.players[0].improve[Counter::Baller.index()] = 3;
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(stats(&g.players[0].board[0]), (1 + 4, 1));
    assert_eq!(baller_count(&g, 0), 4);

    // a golden Fire Baller's step is double: count 3 gives +8
    let mut g = game(2);
    put_on_board(&mut g, 0, "Fire Baller", true);
    put_on_board(&mut g, 0, "Cord Puller", false);
    g.players[0].improve[Counter::Baller.index()] = 3;
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(stats(&g.players[0].board[0]), (1 + 8, 1));
    assert_eq!(baller_count(&g, 0), 4, "a golden Baller is still one more step");

    // an Air Baller gives +2/+2 a step: count 3 gives +8/+8
    let mut g = game(3);
    put_on_board(&mut g, 0, "Air Baller", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    g.players[0].improve[Counter::Baller.index()] = 3;
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(stats(&g.players[0].board[0]), (1 + 8, 1 + 8));

    // Snow Baller: +1 Health a step
    let mut g = game(4);
    put_on_board(&mut g, 0, "Snow Baller", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    g.players[0].improve[Counter::Baller.index()] = 3;
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(stats(&g.players[0].board[0]), (1, 1 + 4));
    g.check_invariants().unwrap();
}

#[test]
fn selling_different_ballers_in_turn_raises_the_one_shared_count() {
    let mut g = game(5);
    put_on_board(&mut g, 0, "Fire Baller", false);
    put_on_board(&mut g, 0, "Snow Baller", true);
    put_on_board(&mut g, 0, "Air Baller", false);
    put_on_board(&mut g, 0, "Cord Puller", false); // 1/1
    // Fire Baller first: count 0, +1 Attack to the rest (the sold one is gone)
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(baller_count(&g, 0), 1);
    assert_eq!(stats(&g.players[0].board[2]), (1 + 1, 1));
    // golden Snow Baller: (1 + 1) steps of +1 Health, doubled: +4 Health
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(baller_count(&g, 0), 2);
    assert_eq!(stats(&g.players[0].board[1]), (2, 1 + 4));
    // Air Baller: (1 + 2) steps of +2/+2: +6/+6; only the Puller is left
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(baller_count(&g, 0), 3);
    assert_eq!(stats(&g.players[0].board[0]), (2 + 6, 5 + 6));
    // another player's count is their own
    assert_eq!(baller_count(&g, 1), 0);
    g.check_invariants().unwrap();
}

#[test]
fn the_sold_baller_is_not_one_of_your_minions_and_a_lone_baller_buffs_nothing() {
    let mut g = game(6);
    put_on_board(&mut g, 0, "Fire Baller", false);
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::Sell(0));
    assert!(g.players[0].board.is_empty());
    assert_eq!(baller_count(&g, 0), 1);
    assert_eq!(g.players[0].gold, gold + 1);
}

#[test]
fn temperature_shift_gets_two_generated_ballers_that_add_nothing_to_the_pool() {
    let mut g = game(7);
    let fire = id(&g.data.cards, "Fire Baller");
    let snow = id(&g.data.cards, "Snow Baller");
    let (fire_slot, snow_slot) = (fire as usize, snow as usize);
    let (pool_fire, pool_snow) = (g.pool[fire_slot], g.pool[snow_slot]);
    cast(&mut g, "Temperature Shift");
    let hand = &g.players[0].hand;
    assert_eq!(hand.len(), 2);
    assert_eq!(hand[0].card, fire);
    assert_eq!(hand[1].card, snow);
    assert!(hand.iter().all(|m| m.pool_copies == 0));
    assert_eq!((g.pool[fire_slot], g.pool[snow_slot]), (pool_fire, pool_snow));
    g.check_invariants().unwrap();
    // selling them adds no copies to the pool and still counts as Ballers
    play(&mut g, 0, 0, 0);
    play(&mut g, 0, 0, 1);
    act(&mut g, 0, Action::Sell(0));
    act(&mut g, 0, Action::Sell(0));
    assert_eq!((g.pool[fire_slot], g.pool[snow_slot]), (pool_fire, pool_snow));
    assert_eq!(baller_count(&g, 0), 2);
    g.check_invariants().unwrap();
}

// ================= Dune Dweller, Sellemental, Tavern Tempest =================

#[test]
fn dune_dweller_buffs_the_elementals_in_the_tavern_for_the_game() {
    let mut g = game(8);
    set_shop(&mut g, 0, &["Crackling Cyclone", "Cord Puller"]);
    let cyc = stats(&g.players[0].shop[0]);
    let puller = stats(&g.players[0].shop[1]);
    play_new(&mut g, "Dune Dweller");
    assert_eq!(stats(&g.players[0].shop[0]), (cyc.0 + 1, cyc.1 + 1));
    assert_eq!(stats(&g.players[0].shop[1]), puller, "not a Cord Puller");
    // a later Refresh shows buffed Elementals too
    g.players[0].gold = 10;
    for _ in 0..4 {
        act(&mut g, 0, Action::Roll);
        for m in &g.players[0].shop {
            let c = g.data.cards.get(m.card);
            if !c.is_spell() && c.tribes.has(Tribe::Elemental) {
                assert_eq!((m.attack - c.attack, m.health - c.health), (1, 1));
            }
        }
    }
    // golden: twice
    let mut g = game(9);
    put_in_hand(&mut g, 0, "Dune Dweller", true);
    set_shop(&mut g, 0, &["Crackling Cyclone"]);
    let cyc = stats(&g.players[0].shop[0]);
    play(&mut g, 0, 0, 0);
    assert_eq!(stats(&g.players[0].shop[0]), (cyc.0 + 2, cyc.1 + 2));
    g.check_invariants().unwrap();
}

#[test]
fn sellemental_gets_a_3_3_elemental_and_golden_two() {
    let mut g = game(10);
    put_on_board(&mut g, 0, "Sellemental", false);
    act(&mut g, 0, Action::Sell(0));
    let hand = &g.players[0].hand;
    assert_eq!(hand.len(), 1);
    assert_eq!(name(&g, &hand[0]), "Water Droplet");
    assert_eq!(stats(&hand[0]), (3, 3));
    assert_eq!(hand[0].pool_copies, 0);
    g.check_invariants().unwrap();
    let mut g = game(11);
    put_on_board(&mut g, 0, "Sellemental", true);
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].hand.len(), 2);
    assert!(g.players[0].hand.iter().all(|m| stats(m) == (3, 3) && !m.golden));
    g.check_invariants().unwrap();
}

#[test]
fn tavern_tempest_gets_a_random_elemental_up_to_your_tier() {
    for seed in 0..30 {
        let mut g = game(100 + seed);
        let pool_before = g.pool.clone();
        play_new_tempest(&mut g);
        let hand = &g.players[0].hand;
        assert_eq!(hand.len(), 1, "seed {seed}");
        let c = g.data.cards.get(hand[0].card);
        assert!(c.tribes.has(Tribe::Elemental), "seed {seed}");
        assert!(c.tier <= g.players[0].tier, "seed {seed}: tier {}", c.tier);
        assert_eq!(hand[0].pool_copies, 0, "generated: no pool copy");
        // only the played Tempest's own copy left the pool
        let tempest = id(&g.data.cards, "Tavern Tempest") as usize;
        let mut want = pool_before;
        want[tempest] -= 1;
        assert_eq!(g.pool, want);
        g.check_invariants().unwrap();
    }
    // golden: two
    let mut g = game(7);
    put_in_hand(&mut g, 0, "Tavern Tempest", true);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].hand.len(), 3, "two Elementals and the Triple Reward");
}

fn play_new_tempest(g: &mut Game) {
    play_new(g, "Tavern Tempest");
}

// ================= Refresh amount =================

#[test]
fn easterly_winds_and_the_blazer_add_into_one_amount_for_one_minion() {
    let mut g = game(12);
    cast(&mut g, "Easterly Winds");
    assert_eq!(g.players[0].refresh_buff, (9, 9));
    play_new(&mut g, "En-Djinn Blazer");
    assert_eq!(g.players[0].refresh_buff, (19, 19));
    // an explicit Refresh gives the whole amount to ONE Tavern minion
    g.players[0].gold = 10;
    set_shop(&mut g, 0, &[]);
    act(&mut g, 0, Action::Roll);
    let shop = &g.players[0].shop;
    let buffed: Vec<_> = shop
        .iter()
        .filter(|m| !g.data.cards.get(m.card).is_spell())
        .map(|m| {
            let c = g.data.cards.get(m.card);
            (m.attack - c.attack, m.health - c.health)
        })
        .collect();
    assert!(buffed.len() >= 3);
    assert_eq!(buffed.iter().filter(|&&d| d == (19, 19)).count(), 1, "{buffed:?}");
    assert_eq!(buffed.iter().filter(|&&d| d == (0, 0)).count(), buffed.len() - 1);
    // and again on the next Refresh (the amount is never used up)
    act(&mut g, 0, Action::Roll);
    let n = g.players[0]
        .shop
        .iter()
        .filter(|m| {
            let c = g.data.cards.get(m.card);
            !c.is_spell() && (m.attack - c.attack, m.health - c.health) == (19, 19)
        })
        .count();
    assert_eq!(n, 1);
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_blazer_and_a_golden_waveling_add_double() {
    let mut g = game(13);
    put_in_hand(&mut g, 0, "En-Djinn Blazer", true);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].refresh_buff, (20, 20));
    let db = db();
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 1000;
    let mut a = mk(&db, "Waveling", true);
    a.attack = 0;
    let (out, _) = fight_deaths(&db, &[a], &[killer], 5, 1);
    assert_eq!(out.sides[0].refresh_buff_gain, (8, 8));
}

#[test]
fn the_turn_start_roll_is_a_refresh_only_when_the_tavern_changed() {
    // an unfrozen Tavern always changed: the amount lands on one random
    // Tavern minion
    let mut g = game(14);
    g.players[0].refresh_buff = (50, 50);
    end_all_turns(&mut g);
    let big = g.players[0].shop.iter().filter(|m| m.attack >= 50).count();
    assert_eq!(big, 1);
    // the very next Refresh gives it again
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    let big = g.players[0].shop.iter().filter(|m| m.attack >= 50).count();
    assert_eq!(big, 1);

    // a fully frozen, full Tavern is not refreshed: no amount
    let mut g = game(14);
    g.players[0].refresh_buff = (50, 50);
    act(&mut g, 0, Action::Freeze);
    end_all_turns(&mut g);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        if !c.is_spell() {
            assert_eq!((m.attack, m.health), (c.attack, c.health), "{}", c.name);
        }
    }

    // frozen with one card bought: the roll fills that slot, the amount
    // lands again (on any Tavern minion, a frozen one included)
    let mut g = game(14);
    g.players[0].refresh_buff = (50, 50);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Freeze);
    end_all_turns(&mut g);
    assert!(!g.players[0].frozen, "the freeze was for that turn only");
    let big = g.players[0].shop.iter().filter(|m| m.attack >= 50).count();
    assert_eq!(big, 1);
    g.check_invariants().unwrap();
}

#[test]
fn a_free_refresh_and_a_refresh_effect_give_the_amount_too() {
    let mut g = game(15);
    g.players[0].refresh_buff = (7, 7);
    g.players[0].free_rolls = 1;
    act(&mut g, 0, Action::Roll);
    let n = g.players[0]
        .shop
        .iter()
        .filter(|m| {
            let c = g.data.cards.get(m.card);
            !c.is_spell() && (m.attack - c.attack, m.health - c.health) == (7, 7)
        })
        .count();
    assert_eq!(n, 1, "free Refresh");
    // Lost Staff of Hamuul refreshes the Tavern with one type
    let mut g = game(16);
    g.players[0].refresh_buff = (7, 7);
    put_on_board(&mut g, 0, "Cord Puller", false);
    cast(&mut g, "Lost Staff of Hamuul");
    act(&mut g, 0, Action::Target(0));
    let n = g.players[0]
        .shop
        .iter()
        .filter(|m| {
            let c = g.data.cards.get(m.card);
            !c.is_spell() && (m.attack - c.attack, m.health - c.health) == (7, 7)
        })
        .count();
    assert_eq!(n, 1, "refresh effect");
    g.check_invariants().unwrap();
}

#[test]
fn a_waveling_dying_in_combat_counts_and_two_add_up() {
    let db = db();
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 1000;
    let mut a = mk(&db, "Waveling", false);
    a.attack = 0;
    let (out, deaths) = fight_deaths(&db, &[a], &[killer], 5, 1);
    assert_eq!(deaths.len(), 1);
    assert_eq!(out.sides[0].refresh_buff_gain, (4, 4));
    let (out, _) = fight_deaths(&db, &[a, a], &[killer], 10, 1);
    assert_eq!(out.sides[0].refresh_buff_gain, (8, 8));
}

#[test]
fn two_dead_wavelings_give_one_minion_plus_8_plus_8_after_the_game_fight() {
    let mut g = game(17);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Waveling", false);
    put_on_board(&mut g, 0, "Waveling", false);
    for m in &mut g.players[0].board {
        m.attack = 0;
    }
    put_on_board(&mut g, q as usize, "Cage Gnawer", false);
    g.players[q as usize].board[0].attack = 50;
    g.players[q as usize].board[0].health = 1000;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].refresh_buff, (8, 8));
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    let diffs: Vec<_> = g.players[0]
        .shop
        .iter()
        .filter(|m| !g.data.cards.get(m.card).is_spell())
        .map(|m| {
            let c = g.data.cards.get(m.card);
            (m.attack - c.attack, m.health - c.health)
        })
        .collect();
    assert_eq!(diffs.iter().filter(|&&d| d == (8, 8)).count(), 1, "{diffs:?}");
    assert_eq!(diffs.iter().filter(|&&d| d == (4, 4)).count(), 0, "never +4/+4 to two minions");
    g.check_invariants().unwrap();
}

// ================= Ichoron =================

#[test]
fn ichoron_shields_elementals_played_from_hand_until_the_next_shop_phase() {
    let mut g = game(18);
    put_on_board(&mut g, 0, "Ichoron the Protector", false);
    // not an Elemental: no shield
    put_in_hand(&mut g, 0, "Buzzing Vermin", false);
    play(&mut g, 0, 0, 1);
    assert!(!g.players[0].board[1].keywords.has(Keywords::DIVINE_SHIELD));
    // an Elemental played from hand gets one
    play_new(&mut g, "Dune Dweller");
    let dd = g.players[0].board[2];
    assert!(dd.keywords.has(Keywords::DIVINE_SHIELD) && dd.temp_shield);
    // an Elemental placed on the board by other means does not
    put_on_board(&mut g, 0, "Waveling", false);
    assert!(!g.players[0].board[3].keywords.has(Keywords::DIVINE_SHIELD));
    // Flaming Enforcer is an Elemental too
    play_new(&mut g, "Flaming Enforcer");
    assert!(g.players[0].board[4].keywords.has(Keywords::DIVINE_SHIELD));
    // one that already has a shield keeps it for good
    play_new(&mut g, "Crackling Cyclone");
    assert!(g.players[0].board[5].keywords.has(Keywords::DIVINE_SHIELD));
    assert!(!g.players[0].board[5].temp_shield);
    // the shield goes at the start of the next shop phase, used or not
    end_all_turns(&mut g);
    let b = &g.players[0].board;
    assert_eq!(b.len(), 6);
    assert!(!b[2].keywords.has(Keywords::DIVINE_SHIELD));
    assert!(!b[2].temp_shield);
    assert!(!b[4].keywords.has(Keywords::DIVINE_SHIELD));
    assert!(b[0].keywords.has(Keywords::DIVINE_SHIELD), "Ichoron's own");
    assert!(b[5].keywords.has(Keywords::DIVINE_SHIELD), "Cyclone's own");
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_ichorons_shield_has_no_time_limit() {
    let mut g = game(19);
    put_on_board(&mut g, 0, "Ichoron the Protector", true);
    play_new(&mut g, "Dune Dweller");
    let dd = g.players[0].board[1];
    assert!(dd.keywords.has(Keywords::DIVINE_SHIELD) && !dd.temp_shield);
}

#[test]
fn ichorons_shield_comes_off_at_turn_start_in_a_game_that_keeps_the_board() {
    // Sandbox games keep the board through the mirror fight.
    let data = Arc::new(Data::bundled());
    let mut g = Game::new_sandbox(data, 4, Some(lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Elemental, Tribe::Demon, Tribe::Pirate])), 0);
    put_on_board(&mut g, 0, "Ichoron the Protector", false);
    play_new(&mut g, "Dune Dweller");
    play_new(&mut g, "Cord Puller");
    assert!(g.players[0].board[1].keywords.has(Keywords::DIVINE_SHIELD));
    g.step(Action::EndTurn).unwrap();
    assert_eq!(g.turn, 2);
    let b = &g.players[0].board;
    assert_eq!(b.len(), 3);
    assert!(!b[1].keywords.has(Keywords::DIVINE_SHIELD), "gone at the start of the next shop phase");
    assert!(!b[1].temp_shield);
    assert!(b[0].keywords.has(Keywords::DIVINE_SHIELD));
    // a Golden Ichoron's shield stays
    let data = Arc::new(Data::bundled());
    let mut g = Game::new_sandbox(data, 5, Some(lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Elemental, Tribe::Demon, Tribe::Pirate])), 0);
    put_on_board(&mut g, 0, "Ichoron the Protector", true);
    play_new(&mut g, "Dune Dweller");
    g.step(Action::EndTurn).unwrap();
    assert!(g.players[0].board[1].keywords.has(Keywords::DIVINE_SHIELD));
}

// ================= Arcane Absorption =================

#[test]
fn arcane_absorption_needs_a_friendly_elemental_and_gives_half_rounded_up() {
    let mut g = game(20);
    let id_aa = id(&g.data.cards, "Arcane Absorption");
    // no Elemental on the board: it can't be played
    put_on_board(&mut g, 0, "Cord Puller", false);
    assert!(!g.can_cast_spell(0, id_aa));
    g.give(0, "Arcane Absorption").unwrap();
    assert!(!g.is_legal_play(0, 0));
    // with an Elemental it can
    put_on_board(&mut g, 0, "Dune Dweller", false); // 3/3
    assert!(g.can_cast_spell(0, id_aa));
    set_shop(&mut g, 0, &["Cord Puller", "Tasty Lobster", "Cord Puller"]);
    g.players[0].shop[0].attack = 5;
    g.players[0].shop[0].health = 7;
    g.players[0].shop[1].attack = 9;
    g.players[0].shop[1].health = 3;
    g.players[0].shop[2].attack = 1;
    g.players[0].shop[2].health = 6;
    play(&mut g, 0, 0, 0);
    // only the Elemental is a target (the Cord Puller is not)
    act(&mut g, 0, Action::Target(1));
    // the highest Health is 7 (5/7): half rounded up is 3/4
    assert_eq!(stats(&g.players[0].board[1]), (3 + 3, 3 + 4));
    assert_eq!(stats(&g.players[0].board[0]), (1, 1), "the Cord Puller is untouched");
    g.check_invariants().unwrap();
}

#[test]
fn arcane_absorption_cannot_pick_a_non_elemental() {
    let mut g = game(21);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Dune Dweller", false);
    g.give(0, "Arcane Absorption").unwrap();
    play(&mut g, 0, 0, 0);
    assert!(!g.is_legal_test(0, Action::Target(0)));
    assert!(g.is_legal_test(0, Action::Target(1)));
}

#[test]
fn arcane_absorption_with_an_empty_tavern_gives_nothing_but_counts_as_a_cast() {
    let mut g = game(22);
    put_on_board(&mut g, 0, "Dune Dweller", false);
    put_on_board(&mut g, 0, "Felboar", false); // every 3 spells cast
    set_shop(&mut g, 0, &[]);
    let before = stats(&g.players[0].board[0]);
    cast(&mut g, "Arcane Absorption");
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), before);
    assert_eq!(g.players[0].board[1].improvements, 1, "a spell cast");
    g.check_invariants().unwrap();
}

#[test]
fn arcane_absorption_counts_for_living_azerite_even_with_an_empty_tavern() {
    let mut g = game(23);
    put_on_board(&mut g, 0, "Living Azerite", false);
    set_shop(&mut g, 0, &["Dune Dweller", "Cord Puller"]);
    let dd = stats(&g.players[0].shop[0]);
    let cp = stats(&g.players[0].shop[1]);
    cast(&mut g, "Arcane Absorption");
    act(&mut g, 0, Action::Target(0));
    // the Tavern is not empty here: the Azerite buffed its Elementals
    assert_eq!(stats(&g.players[0].shop[0]), (dd.0 + 4, dd.1 + 3));
    assert_eq!(stats(&g.players[0].shop[1]), cp);
    // now with an empty Tavern: still a cast, nothing to buff, no crash
    set_shop(&mut g, 0, &[]);
    cast(&mut g, "Arcane Absorption");
    act(&mut g, 0, Action::Target(0));
    g.check_invariants().unwrap();
}

#[test]
fn leyline_surfacer_gets_arcane_absorption_from_the_battlecry_and_the_deathrattle() {
    let mut g = game(24);
    play_new(&mut g, "Leyline Surfacer");
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(name(&g, &g.players[0].hand[0]), "Arcane Absorption");
    let mut g = game(25);
    put_in_hand(&mut g, 0, "Leyline Surfacer", true);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].hand.len(), 3, "golden: 2 Arcane Absorptions and the Triple Reward");
    // the Deathrattle works in combat
    let db = db();
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 1000;
    let mut a = mk(&db, "Leyline Surfacer", false);
    a.attack = 0;
    let (out, _) = fight_deaths(&db, &[a], &[killer], 5, 1);
    let aa = id(&db, "Arcane Absorption");
    assert_eq!(out.sides[0].cards_gained(), &[aa]);
    let mut b = mk(&db, "Leyline Surfacer", true);
    b.attack = 0;
    let (out, _) = fight_deaths(&db, &[b], &[killer], 5, 1);
    assert_eq!(out.sides[0].cards_gained(), &[aa, aa]);
}

// ================= Conflagration =================

#[test]
fn conflagration_is_improved_by_each_elemental_played_this_turn() {
    let mut g = game(26);
    put_on_board(&mut g, 0, "Cord Puller", false);
    cast(&mut g, "Conflagration");
    // a minion on the board or in the Tavern: pick the Puller (slot 0)
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (1 + 4, 1 + 4));
    // two Elementals played: +4/+4 plus 2 x +2/+2
    play_new(&mut g, "Crackling Cyclone");
    play_new(&mut g, "Crackling Cyclone");
    assert_eq!(g.players[0].elementals_played_turn, 2);
    cast(&mut g, "Conflagration");
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (5 + 8, 5 + 8));
    // a Tavern minion is a legal target too
    set_shop(&mut g, 0, &["Cord Puller"]);
    let before = stats(&g.players[0].shop[0]);
    cast(&mut g, "Conflagration");
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(stats(&g.players[0].shop[0]), (before.0 + 8, before.1 + 8));
    // the count resets with the turn
    end_all_turns(&mut g);
    assert_eq!(g.players[0].elementals_played_turn, 0);
    g.check_invariants().unwrap();
}

#[test]
fn firelands_fugitive_gets_a_conflagration_golden_two() {
    let mut g = game(27);
    play_new(&mut g, "Firelands Fugitive");
    assert_eq!(name(&g, &g.players[0].hand[0]), "Conflagration");
    assert_eq!(g.players[0].hand.len(), 1);
    let mut g = game(28);
    put_in_hand(&mut g, 0, "Firelands Fugitive", true);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].hand.len(), 3, "two Conflagrations and the Triple Reward");
}

// ================= Free Refreshes =================

#[test]
fn refreshing_anomaly_gives_free_refreshes_that_add_up_carry_over_and_are_used_first() {
    let mut g = game(29);
    play_new(&mut g, "Refreshing Anomaly");
    assert_eq!(g.players[0].free_rolls, 2);
    put_in_hand(&mut g, 0, "Refreshing Anomaly", true);
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 1);
    // a golden play also puts a Triple Reward spell in hand; no Discover opens
    assert!(g.players[0].discover.is_empty());
    assert_eq!(g.players[0].free_rolls, 2 + 4, "golden gives 4");
    // used before gold, and not gold spent
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].free_rolls, 5);
    assert_eq!(g.players[0].gold, gold);
    assert_eq!(g.players[0].gold_spent_turn, 0);
    // they carry over turns
    end_all_turns(&mut g);
    assert_eq!(g.players[0].free_rolls, 5);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].free_rolls, 4);
    g.check_invariants().unwrap();
}

// ================= Every Tavern spell counts =================

#[test]
fn living_azerite_gives_elementals_in_the_tavern_a_buff_for_every_spell() {
    let mut g = game(30);
    put_on_board(&mut g, 0, "Living Azerite", false);
    set_shop(&mut g, 0, &["Dune Dweller", "Cord Puller"]);
    let dd = stats(&g.players[0].shop[0]);
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(stats(&g.players[0].shop[0]), (dd.0 + 4, dd.1 + 3));
    // for the game: a later Refresh shows buffed Elementals
    g.players[0].gold = 10;
    for _ in 0..3 {
        act(&mut g, 0, Action::Roll);
        for m in &g.players[0].shop {
            let c = g.data.cards.get(m.card);
            if !c.is_spell() && c.tribes.has(Tribe::Elemental) {
                assert_eq!((m.attack - c.attack, m.health - c.health), (4, 3), "{}", c.name);
            }
        }
    }
    // golden: twice
    let mut g = game(31);
    put_on_board(&mut g, 0, "Living Azerite", true);
    set_shop(&mut g, 0, &["Dune Dweller"]);
    let dd = stats(&g.players[0].shop[0]);
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(stats(&g.players[0].shop[0]), (dd.0 + 8, dd.1 + 6));
}

#[test]
fn a_spell_cast_twice_counts_twice_for_the_azerite_and_balinda_casts_count() {
    // Balinda doubles a spell cast on a friendly minion: two spells
    let mut g = game(32);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Felboar", false);
    put_on_board(&mut g, 0, "Dune Dweller", false);
    set_shop(&mut g, 0, &["Tasty Lobster"]);
    cast(&mut g, "Arcane Absorption");
    // targets: Balinda (not an Elemental), Felboar (not), Dune Dweller
    act(&mut g, 0, Action::Target(2));
    assert_eq!(g.players[0].board[1].improvements, 2, "two spells cast");
    // a Bounty cast twice (Proud Privateer) also counts twice
    let mut g = game(33);
    put_on_board(&mut g, 0, "Proud Privateer", false);
    put_on_board(&mut g, 0, "Living Azerite", false);
    set_shop(&mut g, 0, &["Dune Dweller"]);
    let dd = stats(&g.players[0].shop[0]);
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(stats(&g.players[0].shop[0]), (dd.0 + 8, dd.1 + 6), "two casts, two buffs");
    g.check_invariants().unwrap();
}

#[test]
fn air_revenant_casts_easterly_winds_every_7_gold_and_it_counts_as_a_spell() {
    let mut g = game(34);
    put_on_board(&mut g, 0, "Air Revenant", false);
    put_on_board(&mut g, 0, "Felboar", false);
    put_on_board(&mut g, 0, "Living Azerite", false);
    g.players[0].gold = 10;
    for _ in 0..5 {
        act(&mut g, 0, Action::Roll);
    }
    assert_eq!(g.players[0].refresh_buff, (0, 0));
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(2));
    // the count carries over the turn
    end_all_turns(&mut g);
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(2));
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].refresh_buff, (0, 0));
    let felboar = g.players[0].board[1].improvements;
    act(&mut g, 0, Action::Roll);
    // the 7th gold cast Easterly Winds: it counts as a spell cast
    assert_eq!(g.players[0].refresh_buff, (9, 9));
    assert_eq!(g.players[0].board[1].improvements, felboar + 1);
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_air_revenant_casts_two_easterly_winds() {
    let mut g = game(35);
    put_on_board(&mut g, 0, "Air Revenant", true);
    put_on_board(&mut g, 0, "Felboar", false);
    g.players[0].gold = 10;
    for _ in 0..7 {
        act(&mut g, 0, Action::Roll);
    }
    assert_eq!(g.players[0].refresh_buff, (18, 18));
    assert_eq!(g.players[0].board[1].improvements, 2);
}

// ================= Elemental of Surprise =================

fn buy_slot0(g: &mut Game) {
    g.players[0].gold = 10;
    act(g, 0, Action::Buy(0));
}

#[test]
fn two_of_a_card_plus_a_surprise_make_a_golden_one() {
    let mut g = game(36);
    put_on_board(&mut g, 0, "Dune Dweller", false);
    put_in_hand(&mut g, 0, "Dune Dweller", false);
    let slab_surprise = id(&g.data.cards, "Elemental of Surprise") as usize;
    let before = g.pool[slab_surprise];
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    assert_eq!(g.pool[slab_surprise], before - 1);
    buy_slot0(&mut g);
    let p = &g.players[0];
    assert!(p.board.is_empty(), "the board copy was taken");
    assert_eq!(p.hand.len(), 1);
    let gold = p.hand[0];
    assert!(gold.golden);
    assert_eq!(name(&g, &gold), "Dune Dweller");
    assert_eq!(stats(&gold), (6, 6));
    // two pool copies in the golden card, the Surprise's copy is back
    assert_eq!(gold.pool_copies, 2);
    assert_eq!(g.pool[slab_surprise], before, "the Surprise's pool copy went back");
    g.check_invariants().unwrap();
}

#[test]
fn two_surprises_plus_a_card_make_a_golden_one_in_either_order() {
    // the card arrives last
    let mut g = game(37);
    put_in_hand(&mut g, 0, "Elemental of Surprise", false);
    put_on_board(&mut g, 0, "Elemental of Surprise", false);
    let surprise = id(&g.data.cards, "Elemental of Surprise") as usize;
    let before = g.pool[surprise];
    set_shop(&mut g, 0, &["Crackling Cyclone"]);
    buy_slot0(&mut g);
    let p = &g.players[0];
    assert_eq!(p.hand.len(), 1);
    assert!(p.hand[0].golden);
    assert_eq!(name(&g, &p.hand[0]), "Crackling Cyclone");
    assert!(p.board.is_empty());
    assert_eq!(g.pool[surprise], before + 2, "both Surprise copies went back");
    g.check_invariants().unwrap();
    // the second Surprise arrives last
    let mut g = game(38);
    put_in_hand(&mut g, 0, "Elemental of Surprise", false);
    put_in_hand(&mut g, 0, "Crackling Cyclone", false);
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    buy_slot0(&mut g);
    let p = &g.players[0];
    assert_eq!(p.hand.len(), 1);
    assert!(p.hand[0].golden);
    assert_eq!(name(&g, &p.hand[0]), "Crackling Cyclone");
    g.check_invariants().unwrap();
}

#[test]
fn a_surprise_arriving_last_completes_a_pair() {
    let mut g = game(39);
    put_in_hand(&mut g, 0, "Waveling", false);
    put_on_board(&mut g, 0, "Waveling", false);
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    buy_slot0(&mut g);
    let p = &g.players[0];
    assert_eq!(p.hand.len(), 1);
    assert_eq!(name(&g, &p.hand[0]), "Waveling");
    assert!(p.hand[0].golden);
    g.check_invariants().unwrap();
}

#[test]
fn the_surprise_does_not_triple_with_unrelated_cards() {
    let mut g = game(40);
    // two different Elementals and a Surprise: no triple
    put_in_hand(&mut g, 0, "Dune Dweller", false);
    put_in_hand(&mut g, 0, "Waveling", false);
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    buy_slot0(&mut g);
    assert_eq!(g.players[0].hand.len(), 3);
    assert!(g.players[0].hand.iter().all(|m| !m.golden));
    // two Cord Pullers (not Elementals) and a Surprise: no triple
    let mut g = game(41);
    put_in_hand(&mut g, 0, "Cord Puller", false);
    put_in_hand(&mut g, 0, "Cord Puller", false);
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    buy_slot0(&mut g);
    assert_eq!(g.players[0].hand.len(), 3);
    // three Surprises are a golden Surprise
    let mut g = game(42);
    put_in_hand(&mut g, 0, "Elemental of Surprise", false);
    put_in_hand(&mut g, 0, "Elemental of Surprise", false);
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    buy_slot0(&mut g);
    assert_eq!(g.players[0].hand.len(), 1);
    assert!(g.players[0].hand[0].golden);
    assert_eq!(name(&g, &g.players[0].hand[0]), "Elemental of Surprise");
    g.check_invariants().unwrap();
}

#[test]
fn a_surprise_triple_keeps_the_buffs_of_all_three_and_the_surprises_shield() {
    let mut g = game(43);
    put_on_board(&mut g, 0, "Waveling", false);
    put_in_hand(&mut g, 0, "Waveling", false);
    g.players[0].board[0].attack += 10;
    g.players[0].hand[0].health += 3;
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    g.players[0].shop[0].attack += 7; // buffed in the Tavern
    g.players[0].shop[0].health += 5;
    buy_slot0(&mut g);
    let gold = g.players[0].hand[0];
    assert_eq!(name(&g, &gold), "Waveling");
    // golden Waveling 10/2 + 10 Attack + 3 Health + the Surprise's +7/+5
    assert_eq!(stats(&gold), (10 + 10 + 7, 2 + 3 + 5));
    assert!(gold.keywords.has(Keywords::DIVINE_SHIELD), "the Surprise's shield carries");
    g.check_invariants().unwrap();
}

// ================= Ultraviolet Ascendant, Unbound Tempest =================

#[test]
fn ultraviolet_ascendant_gives_other_elementals_plus_3_plus_3_and_improves() {
    let db = db();
    let wall = wall(&db);
    let dd = mk(&db, "Dune Dweller", false); // 3/3
    let mut asc = mk(&db, "Ultraviolet Ascendant", false);
    // no improvement: +3/+3
    assert_eq!(first_attacker(&db, &[dd, asc, wall, wall], &[wall]), (6, 6));
    // two Elementals played: +9/+9
    asc.improvements = 2;
    assert_eq!(first_attacker(&db, &[dd, asc, wall, wall], &[wall]), (12, 12));
    // golden: twice as much
    let mut gasc = mk(&db, "Ultraviolet Ascendant", true);
    assert_eq!(first_attacker(&db, &[dd, gasc, wall, wall], &[wall]), (3 + 6, 3 + 6));
    gasc.improvements = 1;
    assert_eq!(first_attacker(&db, &[dd, gasc, wall, wall], &[wall]), (3 + 12, 3 + 12));
    // other Elementals only, not itself and not other tribes
    let cp = mk(&db, "Cord Puller", false); // 1/1 Mech
    assert_eq!(first_attacker(&db, &[cp, asc, wall, wall], &[wall]), (1, 1));
}

#[test]
fn the_ascendant_improves_after_each_elemental_played_while_it_is_on_the_board() {
    let mut g = game(44);
    // an Elemental played before it arrives does not count
    play_new(&mut g, "Crackling Cyclone");
    put_on_board(&mut g, 0, "Ultraviolet Ascendant", false);
    let asc = |g: &Game| g.players[0].board.iter().find(|m| m.card == id(&g.data.cards, "Ultraviolet Ascendant")).unwrap().improvements;
    assert_eq!(asc(&g), 0);
    play_new(&mut g, "Crackling Cyclone");
    assert_eq!(asc(&g), 1);
    play_new(&mut g, "Waveling");
    assert_eq!(asc(&g), 2);
    // not a non-Elemental
    put_in_hand(&mut g, 0, "Cord Puller", false);
    let h = g.players[0].hand.len() as u8 - 1;
    let pos = g.players[0].board.len() as u8;
    play(&mut g, 0, h, pos);
    assert_eq!(asc(&g), 2);
    // the count survives the turn
    end_all_turns(&mut g);
    assert_eq!(asc(&g), 2);
}

#[test]
fn unbound_tempest_gains_the_highest_health_tavern_stats_every_4_elementals() {
    let mut g = game(45);
    put_on_board(&mut g, 0, "Unbound Tempest", false);
    set_shop(&mut g, 0, &["Cord Puller", "Tasty Lobster", "Cord Puller"]);
    for (i, (a, h)) in [(2, 5), (9, 3), (1, 8)].into_iter().enumerate() {
        g.players[0].shop[i].attack = a;
        g.players[0].shop[i].health = h;
    }
    let base = stats(&g.players[0].board[0]);
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(4));
    for k in 1..=3 {
        play_new(&mut g, "Crackling Cyclone");
        assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(4 - k));
    }
    // a spell is not an Elemental
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(stats(&g.players[0].board[0]), base);
    play_new(&mut g, "Crackling Cyclone");
    // the 4th: gains 1/8, the highest-Health minion
    assert_eq!(stats(&g.players[0].board[0]), (base.0 + 1, base.1 + 8));
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(4), "repeating");
    g.check_invariants().unwrap();
}

#[test]
fn unbound_tempest_carries_its_count_over_turns_and_an_empty_tavern_still_resets_it() {
    let mut g = game(46);
    put_on_board(&mut g, 0, "Unbound Tempest", false);
    play_new(&mut g, "Crackling Cyclone");
    play_new(&mut g, "Crackling Cyclone");
    // keep the board through the turn
    let data = Arc::new(Data::bundled());
    let mut s = Game::new_sandbox(data, 6, Some(lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Elemental, Tribe::Demon, Tribe::Pirate])), 0);
    put_on_board(&mut s, 0, "Unbound Tempest", false);
    play_new(&mut s, "Crackling Cyclone");
    play_new(&mut s, "Crackling Cyclone");
    s.step(Action::EndTurn).unwrap();
    assert_eq!(s.players[0].board[0].events_left(&s.data.cards), Some(2), "carried over");
    // the Tavern is empty: the 4th does nothing but the count resets
    set_shop(&mut s, 0, &[]);
    let base = stats(&s.players[0].board[0]);
    play_new(&mut s, "Crackling Cyclone");
    play_new(&mut s, "Crackling Cyclone");
    assert_eq!(stats(&s.players[0].board[0]), base);
    assert_eq!(s.players[0].board[0].events_left(&s.data.cards), Some(4));
    s.check_invariants().unwrap();
    let _ = g;
}

#[test]
fn a_golden_unbound_tempest_gains_double() {
    let mut g = game(47);
    put_on_board(&mut g, 0, "Unbound Tempest", true);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 7;
    let base = stats(&g.players[0].board[0]);
    for _ in 0..4 {
        play_new(&mut g, "Crackling Cyclone");
    }
    assert_eq!(stats(&g.players[0].board[0]), (base.0 + 6, base.1 + 14));
}

// ================= Stone Age Slab, Living Prison =================

#[test]
fn stone_age_slab_buffs_then_doubles_once_per_turn() {
    let mut g = game(48);
    put_on_board(&mut g, 0, "Stone Age Slab", false);
    set_shop(&mut g, 0, &["Cord Puller", "Buzzing Vermin", "Flittering Bat"]);
    for m in &mut g.players[0].shop {
        m.attack = 3;
        m.health = 3;
    }
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    // +20/+20 then double: 3/3 -> 23/23 -> 46/46
    assert_eq!(stats(&g.players[0].hand[0]), (46, 46));
    // once per turn: the next buy is plain
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].hand[1]), (3, 3));
    // the Slab itself is untouched
    assert_eq!(stats(&g.players[0].board[0]), (10, 10));
    // next turn it fires again
    end_all_turns(&mut g);
    assert_eq!(name(&g, &g.players[0].board[0]), "Stone Age Slab");
    set_shop(&mut g, 0, &["Tasty Lobster"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 3;
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    let last = g.players[0].hand.last().unwrap();
    assert_eq!(stats(last), (46, 46));
    g.check_invariants().unwrap();
}

#[test]
fn two_slabs_fire_on_the_same_buy_left_to_right() {
    let mut g = game(49);
    put_on_board(&mut g, 0, "Stone Age Slab", false);
    put_on_board(&mut g, 0, "Stone Age Slab", false);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 3;
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].hand[0]), (132, 132));
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_slab_triples() {
    let mut g = game(50);
    put_on_board(&mut g, 0, "Stone Age Slab", true);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 3;
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    // +20/+20 then triple: 23 x 3
    assert_eq!(stats(&g.players[0].hand[0]), (69, 69));
}

#[test]
fn buying_a_spell_does_not_use_up_the_slab() {
    let mut g = game(51);
    put_on_board(&mut g, 0, "Stone Age Slab", false);
    g.players[0].gold = 10;
    let coin = id(&g.data.cards, "Tavern Coin");
    g.players[0].shop_spell = Some(coin);
    act(&mut g, 0, Action::BuySpell);
    assert!(!g.players[0].board[0].activated);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 3;
    act(&mut g, 0, Action::Buy(0));
    let m = g.players[0].hand.iter().find(|m| m.card == id(&g.data.cards, "Cord Puller")).unwrap();
    assert_eq!(stats(m), (46, 46));
}

#[test]
fn living_prison_gains_the_stats_of_the_next_minion_bought_this_turn() {
    let mut g = game(52);
    put_on_board(&mut g, 0, "Living Prison", false);
    set_shop(&mut g, 0, &["Cord Puller", "Cord Puller"]);
    g.players[0].shop[0].attack = 5;
    g.players[0].shop[0].health = 6; // Tavern buffs count: its current stats
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].gold, 9);
    assert_eq!(g.players[0].gold_spent_turn, 1);
    // once per turn
    assert!(!g.is_legal_test(0, Action::Activate(0)));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].board[0]), (4 + 5, 5 + 6));
    // only the next minion
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].board[0]), (4 + 5, 5 + 6));
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_prison_gains_double_and_an_unused_one_is_wasted() {
    let mut g = game(53);
    put_on_board(&mut g, 0, "Living Prison", true);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 5;
    g.players[0].shop[0].health = 6;
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].board[0]), (8 + 10, 10 + 12));
    // no buy this turn: wasted
    let data = Arc::new(Data::bundled());
    let mut s = Game::new_sandbox(data, 8, Some(lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Elemental, Tribe::Demon, Tribe::Pirate])), 0);
    put_on_board(&mut s, 0, "Living Prison", false);
    s.players[0].gold = 10;
    act(&mut s, 0, Action::Activate(0));
    s.step(Action::EndTurn).unwrap();
    assert_eq!(s.players[0].board[0].improvements, 0);
    set_shop(&mut s, 0, &["Cord Puller"]);
    let before = stats(&s.players[0].board[0]);
    act(&mut s, 0, Action::Buy(0));
    assert_eq!(stats(&s.players[0].board[0]), before, "last turn's Activate is gone");
    s.check_invariants().unwrap();
}

#[test]
fn a_slab_triple_fires_again_this_turn() {
    // the copy fired on a buy this turn; the golden from the triple has its
    // own once-per-turn use back
    let mut g = game(54);
    put_on_board(&mut g, 0, "Stone Age Slab", false);
    set_shop(&mut g, 0, &["Cord Puller", "Stone Age Slab", "Stone Age Slab"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 3;
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Buy(0));
    let bought = g.players[0].hand.iter().rev().find(|m| name(&g, m) == "Cord Puller").unwrap();
    assert_eq!(stats(bought), (46, 46), "the copy fired");
    act(&mut g, 0, Action::Buy(0)); // the second Slab
    act(&mut g, 0, Action::Buy(0)); // the third triples
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    play(&mut g, 0, gold as u8, 0);
    // the golden fires again on the very next buy: +20/+20 then triple
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 3;
    act(&mut g, 0, Action::Buy(0));
    let m = g.players[0].hand.iter().rev().find(|m| name(&g, m) == "Cord Puller").unwrap();
    assert_eq!(stats(m), (69, 69));
    g.check_invariants().unwrap();
}

#[test]
fn a_prison_triple_starts_unarmed() {
    // the copy Activated and armed; the golden from the triple is not armed,
    // and can arm itself again this turn
    let mut g = game(55);
    put_on_board(&mut g, 0, "Living Prison", false);
    set_shop(&mut g, 0, &["Cord Puller", "Living Prison", "Living Prison"]);
    g.players[0].shop[0].attack = 5;
    g.players[0].shop[0].health = 6;
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].board[0].improvements, 1, "armed");
    act(&mut g, 0, Action::Buy(1)); // the second Prison
    act(&mut g, 0, Action::Buy(1)); // the third triples
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    play(&mut g, 0, gold as u8, 0);
    assert_eq!(g.players[0].board[0].improvements, 0, "the golden starts unarmed");
    let base = stats(&g.players[0].board[0]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].board[0]), base, "an unarmed golden gains nothing");
    // its own Activate is fresh too: arm it, buy, gain double the stats
    act(&mut g, 0, Action::Activate(0));
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 5;
    g.players[0].shop[0].health = 6;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].board[0]), (base.0 + 10, base.1 + 12),
        "the armed golden takes double the bought stats");
    g.check_invariants().unwrap();
}

// ================= Wildfire Elemental =================

#[test]
fn wildfire_elemental_deals_excess_damage_to_one_adjacent_enemy() {
    let db = db();
    let wf = mk(&db, "Wildfire Elemental", false); // 6/3
    let w = wall(&db);
    let mut left = mk(&db, "Microbot", false);
    left.attack = 1;
    left.health = 4;
    let mut mid = left;
    mid.keywords.set(Keywords::TAUNT);
    mid.health = 2;
    let right = left;
    let board = [wf, w, w, w];
    let mut killed_one = false;
    let mut killed_other = false;
    for seed in 0..20 {
        let (_, deaths) = fight_deaths(&db, &board, &[left, mid, right], 1, seed);
        // the taunt minion died to 6 damage: excess 4 kills one neighbour
        let enemy: Vec<_> = deaths.iter().filter(|(s, _)| *s == 1).collect();
        assert_eq!(enemy.len(), 2, "seed {seed}: the target and one neighbour: {deaths:?}");
        let _ = (&mut killed_one, &mut killed_other);
        killed_one = true;
    }
    assert!(killed_one);
}

#[test]
fn wildfire_elemental_hits_a_random_neighbour_and_golden_hits_both() {
    use std::cell::RefCell;
    let db = db();
    let wf = mk(&db, "Wildfire Elemental", false); // 6/3
    let gwf = mk(&db, "Wildfire Elemental", true); // 12/6
    let w = wall(&db);
    // the neighbours differ in Attack so a death says which one it was
    let mut left = mk(&db, "Microbot", false);
    left.attack = 1;
    left.health = 4;
    let mut right = left;
    right.attack = 2;
    let mut mid = left;
    mid.keywords.set(Keywords::TAUNT);
    mid.health = 2;
    struct W(RefCell<Vec<i32>>);
    impl Watch for W {
        fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
            if side == 1 {
                self.0.borrow_mut().push(m.attack);
            }
        }
    }
    let run = |wf: Minion, seed: u64| {
        let mut watch = W(RefCell::new(Vec::new()));
        let _ = fight_with(
            &db,
            &[wf, w, w, w],
            &[left, mid, right],
            [Counters::default(); 2],
            1,
            &mut Rng::new(seed),
            &mut watch,
        );
        let mut v = watch.0.borrow().clone();
        v.sort();
        v
    };
    // excess 4 kills exactly one neighbour (and the taunt target)
    let mut seen = [false; 2];
    for seed in 0..40 {
        let deaths = run(wf, seed);
        assert_eq!(deaths.len(), 2, "seed {seed}: {deaths:?}");
        assert!(deaths[0] == 1 && (deaths[1] == 1 || deaths[1] == 2), "{deaths:?}");
        // the target is the taunt (Attack 1) and the other is left (1) or right (2)
        seen[(deaths[1] - 1) as usize] = true;
    }
    assert!(seen[0] && seen[1], "both neighbours get picked sometimes");
    // golden: both neighbours (excess 10)
    assert_eq!(run(gwf, 1), vec![1, 1, 2]);
}

#[test]
fn wildfire_needs_a_kill_and_an_excess() {
    let db = db();
    let wf = mk(&db, "Wildfire Elemental", false); // 6 Attack
    let w = wall(&db);
    let mut a = mk(&db, "Microbot", false);
    a.attack = 1;
    a.health = 3;
    let mut mid = a;
    mid.keywords.set(Keywords::TAUNT);
    // exactly lethal: no excess
    mid.health = 6;
    let (_, deaths) = fight_deaths(&db, &[wf, w, w, w], &[a, mid, a], 1, 1);
    assert_eq!(deaths.iter().filter(|(s, _)| *s == 1).count(), 1, "no excess, no splash");
    // not lethal: no splash
    mid.health = 20;
    let (_, deaths) = fight_deaths(&db, &[wf, w, w, w], &[a, mid, a], 1, 1);
    assert_eq!(deaths.iter().filter(|(s, _)| *s == 1).count(), 0);
    // a Divine Shield soaks the hit: no kill, no splash
    mid.health = 1;
    mid.keywords.set(Keywords::DIVINE_SHIELD);
    let (_, deaths) = fight_deaths(&db, &[wf, w, w, w], &[a, mid, a], 1, 1);
    assert_eq!(deaths.iter().filter(|(s, _)| *s == 1).count(), 0);
    // at the edge there is one neighbour and it takes the excess
    let mut edge = a;
    edge.keywords.set(Keywords::TAUNT);
    edge.health = 2;
    let mut next = a;
    next.health = 4;
    let (_, deaths) = fight_deaths(&db, &[wf, w, w, w], &[edge, next, a], 1, 1);
    assert_eq!(deaths.iter().filter(|(s, _)| *s == 1).count(), 2);
}

// ================= whole games =================

#[test]
fn whole_random_games_with_elementals_keep_the_pool_exact() {
    use bgcore::bots::random_action;
    let data = Arc::new(Data::bundled());
    for seed in 0..12u64 {
        let mut g = if seed % 2 == 0 {
            Game::new_sandbox(data.clone(), 900 + seed, Some(bgcore::Tribes::of(Tribe::Elemental)), 2)
        } else {
            Game::new(data.clone(), 900 + seed)
        };
        let mut rng = Rng::new(seed);
        let mut steps = 0;
        while !g.over && steps < 6000 {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
            steps += 1;
        }
    }
}

// ================= helper traits =================

trait LegalTest {
    fn is_legal_test(&mut self, p: usize, a: Action) -> bool;
    fn is_legal_play(&mut self, p: usize, hand: u8) -> bool;
}

impl LegalTest for Game {
    fn is_legal_test(&mut self, p: usize, a: Action) -> bool {
        turn_of(self, p);
        self.is_legal(a)
    }

    fn is_legal_play(&mut self, p: usize, hand: u8) -> bool {
        turn_of(self, p);
        self.is_legal(Action::Play { hand, pos: 0 })
    }
}

// ================= the second round of rulings =================

#[test]
fn the_ascendant_and_the_tempest_do_not_count_their_own_play() {
    let mut g = game(60);
    play_new(&mut g, "Ultraviolet Ascendant");
    assert_eq!(g.players[0].board[0].improvements, 0);
    play_new(&mut g, "Crackling Cyclone");
    assert_eq!(g.players[0].board[0].improvements, 1);
    let mut g = game(61);
    play_new(&mut g, "Unbound Tempest");
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(4));
    play_new(&mut g, "Crackling Cyclone");
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(3));
}

#[test]
fn slabs_buff_first_then_the_armed_prison_gains_the_buffed_stats() {
    let mut g = game(62);
    put_on_board(&mut g, 0, "Living Prison", false);
    put_on_board(&mut g, 0, "Stone Age Slab", false);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].shop[0].attack = 3;
    g.players[0].shop[0].health = 3;
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].hand[0]), (46, 46));
    assert_eq!(stats(&g.players[0].board[0]), (4 + 46, 5 + 46));
}

#[test]
fn arcane_absorption_picks_a_random_minion_on_a_health_tie() {
    let mut seen = [false; 2];
    for seed in 0..40 {
        let mut g = game(200 + seed);
        put_on_board(&mut g, 0, "Dune Dweller", false); // 3/3
        set_shop(&mut g, 0, &["Cord Puller", "Buzzing Vermin"]);
        g.players[0].shop[0].attack = 2;
        g.players[0].shop[0].health = 6;
        g.players[0].shop[1].attack = 8;
        g.players[0].shop[1].health = 6;
        cast(&mut g, "Arcane Absorption");
        act(&mut g, 0, Action::Target(0));
        match stats(&g.players[0].board[0]) {
            (4, 6) => seen[0] = true,
            (7, 6) => seen[1] = true,
            other => panic!("{other:?}"),
        }
    }
    assert!(seen[0] && seen[1]);
}

#[test]
fn a_golden_surprise_triples_with_two_copies_and_everything_goes_back_right() {
    let mut g = game(63);
    put_on_board(&mut g, 0, "Waveling", false);
    put_in_hand(&mut g, 0, "Waveling", false);
    put_in_hand(&mut g, 0, "Elemental of Surprise", true);
    let surprise = id(&g.data.cards, "Elemental of Surprise") as usize;
    let before = g.pool[surprise];
    set_shop(&mut g, 0, &["Waveling"]);
    // only two Wavelings in hand and board, the third is bought: that is a
    // plain triple. Take the board one out so the Surprise is needed.
    let m = g.players[0].board.remove(0);
    g.pool[id(&g.data.cards, "Waveling") as usize] += m.pool_copies;
    buy_slot0(&mut g);
    let p = &g.players[0];
    assert_eq!(p.hand.len(), 1, "{:?}", p.hand.iter().map(|m| name(&g, m)).collect::<Vec<_>>());
    let gold = p.hand[0];
    assert!(gold.golden && name(&g, &gold) == "Waveling");
    assert!(gold.keywords.has(Keywords::DIVINE_SHIELD), "the Surprise's shield");
    assert_eq!(gold.pool_copies, 2);
    assert_eq!(g.pool[surprise], before + 3, "its three copies went back");
    g.check_invariants().unwrap();
    // played from hand it puts a Triple Reward spell in hand
    play(&mut g, 0, 0, 0);
    assert!(g.players[0].hand.iter().any(|m| name(&g, m) == "Triple Reward"));
}

#[test]
fn a_golden_surprise_in_hand_completes_two_bought_copies() {
    let mut g = game(64);
    put_on_board(&mut g, 0, "Waveling", false);
    put_in_hand(&mut g, 0, "Elemental of Surprise", true);
    set_shop(&mut g, 0, &["Waveling"]);
    buy_slot0(&mut g);
    let p = &g.players[0];
    assert_eq!(p.hand.len(), 1);
    assert!(p.hand[0].golden && name(&g, &p.hand[0]) == "Waveling");
    assert!(p.board.is_empty());
    g.check_invariants().unwrap();
}
