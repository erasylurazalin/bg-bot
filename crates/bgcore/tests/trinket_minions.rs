//! The minions only trinkets hand out (DESIGN.md "Generated minions outside
//! the pool": Tide Oracle Morgl, Arcane Behemoth, Dr. Boom's Monster,
//! Doubloon Grifter, Bassgill, Faceless Manipulator, Flourishing Frostling and
//! the Fish of N'Zoth). Each one's text is a card field a real card can use,
//! so every test here drives the mechanic, not the token's name: a kill
//! rider, a sell trigger that gains the sold minion's stats, two "wherever
//! this is" auras, "whenever you get a Pirate", summon from hand by Health, a
//! Battlecry that becomes a copy, and a Deathrattle the Fish of N'Zoth
//! inherits.

use std::sync::Arc;

use bgcore::fight::{Counters, fight_with, fight_with_hands};
use bgcore::{Action, CardId, Data, Game, Keywords, Minion, Rng, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn all_tribes() -> Tribes {
    Tribe::EVERY.into_iter().fold(Tribes::NONE, |m, t| m.with(t))
}

/// A hero-less game with every tribe in the lobby.
fn game(seed: u64) -> Game {
    Game::new_with_lobby(data(), seed, all_tribes())
}

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("{name}"))
}

/// Answer whatever the current player has open (a trinket offer, a Discover,
/// a pending target), so a helper that only wants the next turn never stalls.
fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let a = if !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || pl.pending_choose_one.is_some()
    {
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
    g.step(a).unwrap_or_else(|_| panic!("could not answer {a:?}"));
    true
}

fn act(g: &mut Game, p: usize, a: Action) {
    while g.current_player() != Some(p) {
        if !answer(g) {
            g.step(Action::Freeze).unwrap();
        }
    }
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

/// Take a copy out of the pool (a token has none: it is generated).
fn copies(g: &mut Game, c: CardId) -> u8 {
    if g.data.cards.get(c).token {
        return 0;
    }
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    1
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let n = copies(g, c);
    let m = g.players[p].make_card(&g.data.cards, c, false, n);
    g.players[p].board.push(m);
}

fn put_in_hand(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let n = copies(g, c);
    let m = g.players[p].make_card(&g.data.cards, c, false, n);
    g.players[p].hand.push(m);
}

fn play_from_hand(g: &mut Game, p: usize, name: &str, pos: usize) {
    put_in_hand(g, p, name);
    let hand = g.players[p].hand.len() - 1;
    act(g, p, Action::Play { hand: hand as u8, pos: pos as u8 });
}

fn make(db: &bgcore::CardDb, name: &str) -> Minion {
    db.make(db.by_name(name).unwrap_or_else(|| panic!("{name}")), false)
}

/// A minion that kills in one hit and never dies.
fn wall(db: &bgcore::CardDb) -> Minion {
    let mut m = make(db, "Microbot");
    m.attack = 50;
    m.health = 2000;
    m
}

// ---- Tide Oracle Morgl ----

#[test]
fn morgl_gives_the_killed_minions_maximum_stats_to_a_card_in_hand() {
    let d = data();
    let db = &d.cards;
    // A 3/2 in the hand; the killed minion is a Sewer Rat, a 3/2.
    let hand = [make(db, "Microbot")];
    for golden in [false, true] {
        let morgl = db.make(db.by_name("Tide Oracle Morgl").unwrap(), golden);
        let mut filler = make(db, "Microbot");
        filler.attack = 0;
        let board = [morgl, filler];
        let enemy = [make(db, "Sewer Rat")];
        let out = fight_with_hands(
            db,
            &board,
            &enemy,
            [&hand, &[]],
            [Counters::default(); 2],
            1,
            &mut Rng::new(3),
            &mut (),
        );
        // The left-most minion (Morgl) attacks first with two minions
        // against one, and Poisonous kills whatever it hits.
        let want = if golden { (6, 4) } else { (3, 2) };
        assert_eq!(
            out.sides[0].hand_stats[0], want,
            "the killed 3/2's maximum stats reached the card in hand (golden: {golden})"
        );
    }
}

// ---- Arcane Behemoth ----

#[test]
fn arcane_behemoth_gains_the_stats_of_a_sold_elemental() {
    let mut g = game(11);
    put_on_board(&mut g, 0, "Arcane Behemoth");
    play_from_hand(&mut g, 0, "Water Droplet", 1);
    assert_eq!(g.players[0].board.len(), 2);
    let before = g.players[0].board[0];
    act(&mut g, 0, Action::Sell(1));
    let after = g.players[0].board[0];
    assert_eq!(
        (after.attack, after.health),
        (before.attack + 3, before.health + 3),
        "the sold 3/3 Elemental gave its stats"
    );
    // A Minion that is not an Elemental gives nothing.
    put_on_board(&mut g, 0, "Microbot");
    let before = g.players[0].board[0];
    act(&mut g, 0, Action::Sell(1));
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (before.attack, before.health));
}

// ---- Dr. Boom's Monster ----

#[test]
fn booms_monster_grows_with_each_magnetization() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Auto Accelerator");
    assert_eq!(g.players[0].magnetizations, 0);
    let monster = id(&g, "Dr. Boom's Monster");
    let base = g.players[0].make_card(&g.data.cards, monster, false, 0);
    assert_eq!((base.attack, base.health), (2, 2), "no Magnetization yet");
    // Playing a Magnetic onto a Mech is one Magnetization.
    play_from_hand(&mut g, 0, "Satellite", 0);
    assert_eq!(g.players[0].magnetizations, 1);
    let one = g.players[0].make_card(&g.data.cards, monster, false, 0);
    assert_eq!((one.attack, one.health), (4, 4), "+2/+2 for the one");
    // Wherever it is: a golden one takes double.
    let gold = g.players[0].make_card(&g.data.cards, monster, true, 0);
    assert_eq!((gold.attack, gold.health), (8, 8), "the golden 4/4 plus 4/4 for the one");
}

// ---- Doubloon Grifter ----

#[test]
fn doubloon_grifter_raises_the_maximum_gold_every_four_pirates() {
    let mut g = game(13);
    put_on_board(&mut g, 0, "Doubloon Grifter");
    let before = g.players[0].extra_max_gold;
    for name in ["Southsea Busker", "Clever Castaway", "Surfing Sylvar"] {
        assert!(g.give(0, name).is_some(), "{name}");
    }
    assert_eq!(g.players[0].extra_max_gold, before, "three Pirates is not enough");
    assert!(g.give(0, "Greedy Conniver").is_some());
    assert_eq!(g.players[0].extra_max_gold, before + 1, "the fourth Pirate raised it");
    // A non-Pirate does not count.
    assert!(g.give(0, "Microbot").is_some());
    assert_eq!(g.players[0].extra_max_gold, before + 1);
    // The count runs on: four more Pirates raise it again.
    let more = g.players[0].hand.len();
    for name in ["Southsea Busker", "Clever Castaway", "Surfing Sylvar", "Azsharan Cutlassier"] {
        assert!(g.give(0, name).is_some(), "{name}");
    }
    assert!(g.players[0].hand.len() > more);
    assert_eq!(g.players[0].extra_max_gold, before + 2, "the counter runs on");
}

// ---- Bassgill ----

#[test]
fn bassgill_summons_the_highest_health_murloc_from_hand_for_the_fight() {
    let d = data();
    let db = &d.cards;
    for golden in [false, true] {
        let board = [db.make(db.by_name("Bassgill").unwrap(), golden)];
        let low = make(db, "Tad"); // 2/2
        let high = make(db, "Bream Counter"); // 6/6
        let hand = [low, high];
        // Two walls: the enemy has more minions, so it attacks first, and
        // Bassgill is the only target.
        let enemy = [wall(db), wall(db)];
        #[derive(Default)]
        struct Summons(Vec<CardId>);
        impl bgcore::fight::Watch for Summons {
            fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
                if side == 0 {
                    self.0.push(m.card);
                }
            }
        }
        let mut w = Summons::default();
        let _ = fight_with_hands(
            db,
            &board,
            &enemy,
            [&hand, &[]],
            [Counters::default(); 2],
            30,
            &mut Rng::new(5),
            &mut w,
        );
        let high_id = db.by_name("Bream Counter").unwrap();
        let low_id = db.by_name("Tad").unwrap();
        assert_eq!(
            w.0.iter().filter(|&&c| c == high_id).count(),
            1,
            "the highest-Health Murloc came out (golden: {golden})"
        );
        assert_eq!(
            w.0.iter().filter(|&&c| c == low_id).count(),
            if golden { 1 } else { 0 },
            "the second Murloc only with the golden one (golden: {golden})"
        );
    }
}

#[test]
fn a_hand_summoning_deathrattle_puts_the_hand_into_the_fight() {
    // The game only hands the fight its hand when something reads it.
    let d = data();
    assert!(d.cards.get(d.cards.by_name("Bassgill").unwrap()).reads_hand());
}

// ---- Faceless Manipulator ----

#[test]
fn faceless_manipulator_becomes_a_copy_of_the_minion_it_picks() {
    let mut g = game(14);
    put_on_board(&mut g, 0, "Auto Accelerator");
    put_on_board(&mut g, 0, "Microbot");
    play_from_hand(&mut g, 0, "Faceless Manipulator", 2);
    assert!(g.players[0].pending_target.is_some(), "the Battlecry asks for a minion");
    act(&mut g, 0, Action::Target(0));
    let copy = g.players[0].board[2];
    let target = g.players[0].board[0];
    assert_eq!(copy.card, target.card, "it became the minion it picked");
    assert_eq!((copy.attack, copy.health), (target.attack, target.health));
    assert_eq!(copy.pool_copies, 0, "a copy takes no pool copy");
    assert!(copy.no_reward, "it gives no Triple Reward: it was never played");
    let two = id(&g, "Auto Accelerator");
    let copies = g.players[0].board.iter().filter(|m| m.card == two).count();
    assert_eq!(copies, 2);
    assert_eq!(g.players[0].board.len(), 3, "the others are untouched");
}

#[test]
fn faceless_manipulator_copies_a_tavern_minion() {
    let mut g = game(15);
    let batter = id(&g, "Auto Accelerator");
    // Put it in the Tavern, returning what was there to the pool.
    let old: Vec<Minion> = g.players[0].shop.drain(..).collect();
    for m in old {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    let slot = g.data.cards.get(batter).pool_slot as usize;
    g.pool[slot] -= 1;
    g.players[0].shop.push(g.players[0].make_card(&g.data.cards, batter, false, 1));
    g.players[0].shop_spell = None;
    play_from_hand(&mut g, 0, "Faceless Manipulator", 0);
    // The Tavern minion is the only minion it could copy, so the choice
    // answers itself.
    assert!(g.players[0].pending_target.is_none());
    assert_eq!(g.players[0].board[0].card, batter, "the Tavern minion was copied");
    assert_eq!(g.players[0].shop.len(), 1, "the Tavern minion stays where it is");
}

// ---- Flourishing Frostling ----

#[test]
fn flourishing_frostling_grows_with_every_elemental_played() {
    let mut g = game(16);
    let frostling = id(&g, "Flourishing Frostling");
    let base = g.players[0].make_card(&g.data.cards, frostling, false, 0);
    assert_eq!((base.attack, base.health), (2, 1));
    play_from_hand(&mut g, 0, "Water Droplet", 0);
    play_from_hand(&mut g, 0, "Sellemental", 1);
    assert_eq!(g.players[0].elementals_played, 2);
    let two = g.players[0].make_card(&g.data.cards, frostling, false, 0);
    assert_eq!((two.attack, two.health), (6, 3), "+2/+1 for each of the two");
    let gold = g.players[0].make_card(&g.data.cards, frostling, true, 0);
    assert_eq!((gold.attack, gold.health), (12, 6), "the golden aura doubles");
    // A non-Elemental does not count.
    play_from_hand(&mut g, 0, "Microbot", 2);
    assert_eq!(g.players[0].elementals_played, 2);
}

#[test]
fn jarred_frostling_gives_three_elementals_the_frostling_deathrattle() {
    let mut g = game(17);
    put_on_board(&mut g, 0, "Water Droplet");
    put_on_board(&mut g, 0, "Sellemental");
    put_on_board(&mut g, 0, "Microbot");
    let i = g.data.trinkets.by_name("Jarred Frostling").unwrap();
    g.set_trinket(0, 1, Some(i));
    let counters = [g.players[0].counters(&g.data, g.lobby), Counters::default()];
    let board = g.players[0].board.clone();
    let enemy = [wall(&g.data.cards), wall(&g.data.cards)];
    #[derive(Default)]
    struct Summons(Vec<CardId>);
    impl bgcore::fight::Watch for Summons {
        fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
            if side == 0 {
                self.0.push(m.card);
            }
        }
    }
    let mut w = Summons::default();
    let _ = fight_with(
        &g.data.cards,
        &board,
        &enemy,
        counters,
        60,
        &mut Rng::new(2),
        &mut w,
    );
    let frostling = id(&g, "Flourishing Frostling");
    let got = w.0.iter().filter(|&&c| c == frostling).count();
    assert_eq!(got, 2, "the two Elementals each summoned a Frostling");
}

// ---- Fish of N'Zoth ----

#[test]
fn fish_of_nzoth_keeps_a_dead_deathrattle_and_fires_it() {
    let d = data();
    let db = &d.cards;
    // The Sewer Rat dies first (it has Taunt), so the Fish gains its
    // Deathrattle and fires it when it dies in turn: two Half-Shells.
    let fish = db.make(db.by_name("Fish of N'Zoth").unwrap(), false);
    let mut rat = make(db, "Sewer Rat");
    rat.keywords.set(Keywords::TAUNT);
    let board = [fish, rat];
    let enemy = [wall(db), wall(db), wall(db)];
    #[derive(Default)]
    struct Summons(Vec<CardId>);
    impl bgcore::fight::Watch for Summons {
        fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
            if side == 0 {
                self.0.push(m.card);
            }
        }
    }
    let mut w = Summons::default();
    let _ = fight_with(
        db,
        &board,
        &enemy,
        [Counters::default(); 2],
        60,
        &mut Rng::new(4),
        &mut w,
    );
    let shell = db.by_name("Half-Shell").unwrap();
    let got = w.0.iter().filter(|&&c| c == shell).count();
    assert_eq!(got, 2, "the Rat's own Half-Shell, then the Fish's gained one");
}
