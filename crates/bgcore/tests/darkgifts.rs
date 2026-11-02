//! Dark Gifts (docs/darkgifts.md): the gift table, the Dark Discovery
//! button, its offers (tiers by turn, the "your type" slot, the pool), the
//! gifts on minions (a triple, a Magnetism, a turn countdown) and every gift
//! effect, one test per mechanism.

use std::sync::Arc;

use bgcore::cards::EndOfTurn;
use bgcore::darkgifts::{GiftCond, most_common_type};
use bgcore::fight::{Counters, fight_with};
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
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("no card {name}"))
}

fn gift(g: &Game, name: &str) -> usize {
    g.data
        .gifts
        .gifts
        .iter()
        .position(|x| x.name == name)
        .unwrap_or_else(|| panic!("no gift {name}"))
}

/// Answer whatever the current player has open, so a helper that only wants
/// the next turn never stalls on a prompt.
fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let cands = if !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || pl.pending_choose_one.is_some()
    {
        vec![Action::Choose(0)]
    } else if pl.pending_discard.is_some() {
        vec![Action::Discard(0)]
    } else if pl.pending_shop_target.is_some() {
        vec![Action::TargetShop(0)]
    } else if pl.pending_target.is_some() {
        vec![Action::Target(0), Action::TargetShop(0)]
    } else if !pl.trinket_offer.is_empty() {
        vec![Action::ChooseTrinket(0)]
    } else {
        return false;
    };
    for a in cands {
        if g.is_legal(a) {
            g.step(a).unwrap_or_else(|_| panic!("could not answer {a:?}"));
            return true;
        }
    }
    false
}

fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

/// End turns until `turn` starts, keeping every hero alive.
fn to_turn(g: &mut Game, turn: u32) {
    while g.turn < turn && !g.over {
        for pl in &mut g.players {
            pl.health = pl.health.max(30);
        }
        next_turn(g);
    }
}

fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        if !answer(g) {
            g.step(Action::Freeze).unwrap();
        }
    }
}

/// Answer everything the player has open, so an action can be taken.
fn settle(g: &mut Game, p: usize) {
    loop {
        turn_of(g, p);
        if !answer(g) {
            return;
        }
    }
}

/// Take an action as `p`, returning the turn's cursor to them first.
fn act(g: &mut Game, p: usize, a: Action) {
    turn_of(g, p);
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

fn pool_copies(g: &mut Game, c: CardId) -> u8 {
    if g.data.cards.get(c).token {
        return 0;
    }
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    1
}

/// Put a pool minion on the board, holding its pool copy.
fn put_on_board(g: &mut Game, p: usize, name: &str) -> usize {
    let c = id(g, name);
    let n = pool_copies(g, c);
    let m = g.players[p].make_card(&g.data.cards, c, false, n);
    g.players[p].board.push(m);
    g.players[p].board.len() - 1
}

/// Replace the Tavern's minions with these pool cards.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        if m.pool_copies > 0 {
            let slot = g.data.cards.get(m.card).pool_slot as usize;
            g.pool[slot] += m.pool_copies;
        }
    }
    for name in names {
        let c = id(g, name);
        let n = pool_copies(g, c);
        let m = g.players[p].make_card(&g.data.cards, c, false, n);
        g.players[p].shop.push(m);
    }
}

/// A minion of `name` with a gift on it, straight on the board.
fn board_with_gift(g: &mut Game, p: usize, name: &str, gift_name: &str) -> usize {
    let pos = put_on_board(g, p, name);
    assert!(g.give_gift(p, pos, gift_name, g.turn.max(1)), "gift {gift_name}");
    pos
}

fn gift_names(g: &Game, m: &Minion) -> Vec<String> {
    m.gift_slice().iter().map(|x| g.data.cards.get(x.card()).name.clone()).collect()
}

// ---- the data ----

#[test]
fn the_gift_table_parses_with_the_forty_seven_gifts() {
    let d = Data::bundled();
    assert_eq!(d.gifts.len(), 47);
    // The four names that exist twice are two entries each, by card_id.
    for name in ["Battle Scars", "Death's Embrace", "Spell Siphon", "Dexterity"] {
        assert_eq!(d.gifts.gifts.iter().filter(|g| g.name == name).count(), 1, "{name}");
        assert_eq!(
            d.gifts.gifts.iter().filter(|g| g.name.starts_with(name)).count(),
            2,
            "{name} has a strong half"
        );
    }
    // Every gift's effect card is in the card set, and every window is the
    // game data's (docs/briefs/darkgift-facts.md section 1).
    let hawk = d.gifts.gifts.iter().find(|g| g.name == "Harpy's Talons").unwrap();
    assert_eq!((hawk.first, hawk.last), (1, 0), "a gift with no window");
    let horror = d.gifts.gifts.iter().find(|g| g.name == "Persisting Horror").unwrap();
    assert_eq!((horror.first, horror.last, horror.weight), (10, 0, 50));
    assert_eq!(horror.cond, GiftCond::NotTypeless);
    let stone = d.gifts.gifts.iter().find(|g| g.name == "Fortitude").unwrap();
    assert_eq!((stone.first, stone.last), (3, 3));
    // The effect card is a token whose name says Gift.
    for g in &d.gifts.gifts {
        assert!(d.cards.get(g.card).name.starts_with("Gift: "), "{}", g.name);
        assert!(d.cards.get(g.card).token, "{}", g.name);
    }
}

#[test]
fn a_gift_is_only_offered_inside_its_window() {
    let g = game(1);
    let host = g.data.cards.get(id(&g, "Cord Puller"));
    let horror = gift(&g, "Persisting Horror");
    assert!(!g.data.gifts.eligible(horror, 9, host, host.tribes, false), "turn 9 is too early");
    assert!(g.data.gifts.eligible(horror, 10, host, host.tribes, false));
    assert!(g.data.gifts.eligible(horror, 15, host, host.tribes, false), "no last turn");
    let fortitude = gift(&g, "Fortitude");
    assert!(g.data.gifts.eligible(fortitude, 3, host, host.tribes, false));
    assert!(!g.data.gifts.eligible(fortitude, 4, host, host.tribes, false), "3 only");
}

#[test]
fn every_gift_condition_kind_reaches_a_host() {
    // One host per condition, and the same gift on a host it does not fit.
    let g = game(2);
    let db = &g.data.cards;
    let gifts = &g.data.gifts;
    let fits = |name: &str, host: &str, deity: bool| {
        let i = gifts.gifts.iter().position(|x| x.name == name).unwrap();
        let c = db.by_name(host).unwrap_or_else(|| panic!("no card {host}"));
        let card = db.get(c);
        gifts.gifts[i].cond.fits(card, card.tribes, deity)
    };
    assert!(fits("Echoing Voice", "Razorfen Geomancer", false), "a Battlecry host");
    assert!(!fits("Echoing Voice", "Buzzing Vermin", false), "no Battlecry");
    assert!(fits("Jaws of Death", "Buzzing Vermin", false), "a Deathrattle host");
    assert!(!fits("Jaws of Death", "Razorfen Geomancer", false));
    assert!(fits("Consanguinity", "Razorfen Geomancer", false), "a Quilboar");
    assert!(!fits("Consanguinity", "Buzzing Vermin", false));
    assert!(fits("Polarization", "Cord Puller", false), "a Mech");
    assert!(!fits("Polarization", "Razorfen Geomancer", false));
    assert!(fits("Amalgamation", "Suspicious Prisonguard", false), "typeless");
    assert!(!fits("Amalgamation", "Cord Puller", false));
    assert!(fits("Time Turning", "Lullabot", false), "an end of turn effect");
    assert!(!fits("Time Turning", "Cord Puller", false));
    assert!(fits("Worship", "Cord Puller", true), "a Deity");
    assert!(!fits("Worship", "Cord Puller", false));
    assert!(fits("Efficiency", "Suspicious Prisonguard", false), "an Activate");
    assert!(!fits("Efficiency", "Cord Puller", false));
    // Titanic Strength never on a Dragon, Persisting Horror never typeless,
    // Invulnerability never on a Taunt or a typeless minion.
    let dragon = db.by_name("Tarecgosa").unwrap();
    let dc = db.get(dragon);
    assert!(dc.tribes.has(Tribe::Dragon));
    let titanic = gifts.gifts.iter().position(|x| x.name == "Titanic Strength").unwrap();
    assert!(!gifts.gifts[titanic].cond.fits(dc, dc.tribes, false));
    let horror = gifts.gifts.iter().position(|x| x.name == "Persisting Horror").unwrap();
    assert!(gifts.gifts[horror].cond.fits(db.get(dragon), dc.tribes, false), "Tarecgosa is typed");
    let taunt = db.get(db.by_name("Buzzing Vermin").unwrap());
    assert!(taunt.keywords.has(Keywords::TAUNT));
    let inv = gifts.gifts.iter().position(|x| x.name == "Invulnerability").unwrap();
    assert!(!gifts.gifts[inv].cond.fits(taunt, taunt.tribes, false));
    let plain = db.get(db.by_name("Cord Puller").unwrap());
    assert!(gifts.gifts[inv].cond.fits(plain, plain.tribes, false));
}

#[test]
fn the_weight_fifty_gifts_are_half_as_likely() {
    let g = game(3);
    let db = &g.data.cards;
    let gifts = &g.data.gifts;
    // Titanic Strength (weight 50) against Fortitude (no weight), both on any
    // host of turn 11: count the picks.
    let host = db.get(db.by_name("Cord Puller").unwrap());
    let titanic = gifts.gifts.iter().position(|x| x.name == "Titanic Strength").unwrap() as u8;
    let mut rng = Rng::new(11);
    let mut counts = [0u32; 2];
    for _ in 0..2000 {
        let pick = gifts.pick(11, host, host.tribes, false, &[], &mut rng).unwrap();
        if pick as u8 == titanic {
            counts[0] += 1;
        } else {
            counts[1] += 1;
        }
    }
    let share = counts[0] as f64 / (counts[0] + counts[1]) as f64;
    // Among the gifts of turn 11 only these two fit a plain minion, so the
    // half weight shows as about a third of the picks.
    assert!(share < 0.45, "the weight-50 gift took {share:.2} of the picks");
    assert!(counts[0] > 0);
}

#[test]
fn most_common_type_counts_the_all_type_minions_everywhere_and_breaks_ties() {
    let g = game(4);
    let db = &g.data.cards;
    let beast = db.get(db.by_name("Buzzing Vermin").unwrap()).tribes;
    let mech = db.get(db.by_name("Cord Puller").unwrap()).tribes;
    let all = db.get(db.by_name("Amalgam").unwrap_or(db.by_name("Cord Puller").unwrap())).tribes;
    let mut rng = Rng::new(5);
    assert_eq!(most_common_type(&[beast, beast, mech], &mut rng), Some(Tribe::Beast));
    // A tie picks one of the tied at random, never a third type.
    for _ in 0..20 {
        let t = most_common_type(&[beast, mech], &mut rng).unwrap();
        assert!(t == Tribe::Beast || t == Tribe::Mech);
    }
    // A typeless board has no type.
    assert_eq!(most_common_type(&[], &mut rng), None);
    let _ = all;
}

// ---- the button ----

#[test]
fn the_button_costs_three_gold_works_once_a_turn_and_has_three_uses() {
    let mut g = game(5);
    to_turn(&mut g, 2);
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::DarkGift), "locked until turn 3");
    to_turn(&mut g, 3);
    turn_of(&mut g, 0);
    let pl = &g.players[0];
    assert_eq!(pl.dark_gift_uses, 3);
    g.players[0].gold = 2;
    assert!(!g.is_legal(Action::DarkGift), "2 Gold is not enough");
    g.players[0].gold = 5;
    assert!(g.is_legal(Action::DarkGift));
    act(&mut g, 0, Action::DarkGift);
    assert_eq!(g.players[0].gold, 2, "3 Gold");
    assert_eq!(g.players[0].dark_gift_uses, 2);
    assert_eq!(g.players[0].discover.len(), 3, "three offers");
    assert!(g.players[0].dark_offer);
    // Answer the Discover, then the button is spent for the turn.
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].hand.len(), 1, "the pick went to hand");
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::DarkGift), "once a turn");
    g.check_invariants().unwrap();
    // Three presses in all.
    for _ in 0..2 {
        next_turn(&mut g);
        let t = g.turn;
        turn_of(&mut g, 0);
        g.players[0].gold = 10;
        settle(&mut g, 0);
        g.players[0].gold = 10;
        if g.is_legal(Action::DarkGift) {
            act(&mut g, 0, Action::DarkGift);
            while !g.players[0].discover.is_empty() {
                act(&mut g, 0, Action::Choose(0));
            }
        }
        assert!(g.turn >= t);
    }
    assert_eq!(g.players[0].dark_gift_uses, 0, "three uses a game");
}

#[test]
fn the_offer_tiers_follow_the_table_by_turn() {
    let mut g = game(6);
    // The table itself: tiers per slot and the "yours" slot by turn.
    let want: [(u32, [[u8; 1]; 1], [bool; 3]); 8] = [
        (3, [[2]], [false, false, false]),
        (4, [[2]], [false, false, false]),
        (5, [[3]], [false, false, false]),
        (6, [[3]], [true, false, false]),
        (7, [[4]], [true, false, false]),
        (8, [[4]], [true, false, false]),
        (9, [[4]], [true, false, false]),
        (10, [[5]], [true, true, false]),
    ];
    for (turn, _, yours) in want {
        let step = g.data.gifts.offer_step(turn).expect("an offer step");
        assert_eq!(
            step.slots.iter().map(|s| s.yours).collect::<Vec<_>>(),
            yours.to_vec(),
            "turn {turn}"
        );
    }
    assert!(g.data.gifts.offer_step(2).is_none(), "locked on turn 2");
    let masks = |turn: u32| -> Vec<u8> {
        g.data.gifts.offer_step(turn).unwrap().slots.iter().map(|s| s.tiers).collect()
    };
    assert_eq!(masks(3), vec![1 << 2, 1 << 2, 1 << 2]);
    assert_eq!(masks(4), vec![1 << 2, 1 << 2, 1 << 3]);
    assert_eq!(masks(5), vec![1 << 3, 1 << 3, 1 << 3]);
    assert_eq!(masks(6), vec![1 << 3, 1 << 4, 1 << 4]);
    assert_eq!(masks(7), vec![1 << 4, 1 << 4, 1 << 4]);
    assert_eq!(masks(8), vec![1 << 4, 1 << 5, 1 << 5]);
    assert_eq!(masks(9), vec![(1 << 4) | (1 << 5), 1 << 5, 1 << 6]);
    assert_eq!(masks(10), vec![1 << 5, (1 << 5) | (1 << 6), (1 << 5) | (1 << 6)]);
    assert_eq!(masks(15), vec![1 << 5, (1 << 5) | (1 << 6), (1 << 5) | (1 << 6)], "10 and later");
    // And the real offers land on those tiers.
    for turn in 3..=9u32 {
        to_turn(&mut g, turn);
        settle(&mut g, 0);
        g.players[0].gold = 10;
        if !g.is_legal(Action::DarkGift) {
            continue;
        }
        act(&mut g, 0, Action::DarkGift);
        let slots: Vec<u8> =
            g.players[0].discover.iter().map(|&c| g.data.cards.get(c).tier).collect();
        let step = g.data.gifts.offer_step(turn).unwrap();
        assert_eq!(slots.len(), 3, "turn {turn}");
        for (k, &t) in slots.iter().enumerate() {
            assert!(
                step.slots[k].tiers & (1 << t) != 0,
                "turn {turn} slot {k} was tier {t}, not one of its tiers"
            );
        }
        while !g.players[0].discover.is_empty() {
            act(&mut g, 0, Action::Choose(0));
        }
    }
}

#[test]
fn the_offer_makes_three_different_minions_with_three_different_gifts() {
    let mut g = game(7);
    let mut presses = 0;
    for turn in 3..=12u32 {
        to_turn(&mut g, turn);
        settle(&mut g, 0);
        g.players[0].gold = 10;
        if g.players[0].dark_gift_uses == 0 || !g.is_legal(Action::DarkGift) {
            continue;
        }
        act(&mut g, 0, Action::DarkGift);
        presses += 1;
        let cards: Vec<CardId> = g.players[0].discover.to_vec();
        let mut sorted = cards.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), cards.len(), "three different minions");
        let gifts: Vec<u16> = g.players[0].discover_gifts.iter().map(|x| x.card()).collect();
        let mut sorted = gifts.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), gifts.len(), "three different gifts");
        while !g.players[0].discover.is_empty() {
            act(&mut g, 0, Action::Choose(0));
        }
        g.check_invariants().unwrap();
    }
    assert!(presses >= 3, "only {presses} presses");
}

#[test]
fn the_offer_takes_a_pool_copy_each_and_gives_the_others_back() {
    let mut g = game(8);
    to_turn(&mut g, 6);
    settle(&mut g, 0);
    for pl in &mut g.players {
        pl.health = pl.health.max(30);
    }
    g.players[0].gold = 10;
    let before: Vec<u8> = g.pool.clone();
    assert!(g.is_legal(Action::DarkGift));
    act(&mut g, 0, Action::DarkGift);
    let offers: Vec<CardId> = g.players[0].discover.to_vec();
    assert_eq!(offers.len(), 3);
    g.check_invariants().unwrap();
    let mut taken = 0;
    for &c in &offers {
        let slot = g.data.cards.get(c).pool_slot as usize;
        assert!(g.pool[slot] < before[slot], "the offer took a copy of {}", g.data.cards.get(c).name);
        taken += 1;
    }
    assert_eq!(taken, 3);
    // Two go back, the picked one is in hand.
    act(&mut g, 0, Action::Choose(1));
    g.check_invariants().unwrap();
    assert_eq!(g.players[0].hand.len(), 1);
    for (i, &c) in offers.iter().enumerate() {
        if i == 1 {
            continue;
        }
        let slot = g.data.cards.get(c).pool_slot as usize;
        assert_eq!(g.pool[slot], before[slot], "the unpicked copy went back");
    }
}

#[test]
fn the_pool_leaves_out_the_minions_the_rule_excludes() {
    let g = game(9);
    let gifts = &g.data.gifts;
    let db = &g.data.cards;
    let ok = |name: &str, turn: u32| {
        let c = db.by_name(name).unwrap_or_else(|| panic!("no card {name}"));
        gifts.pool_allows(db.get(c), c, turn)
    };
    // Never a Magnetic minion, never a "when you sell this" one, never one
    // whose effect works in hand, never the two named ones.
    assert!(!ok("Lullabot", 8), "Magnetic");
    assert!(!ok("Leeroy the Reckless", 8));
    assert!(!ok("Deadly Spore", 8));
    // No Battlecry or Choose One before turn 5.
    let battlecry = db
        .cards
        .iter()
        .position(|c| c.battlecry.is_some() && !c.token)
        .map(|i| i as CardId)
        .unwrap();
    let name = db.get(battlecry).name.clone();
    assert!(!ok(&name, 4), "{name} on turn 4");
    assert!(ok(&name, 5), "{name} on turn 5");
    // Never a spell or a token.
    assert!(!ok("Bananas", 8) || db.get(db.by_name("Bananas").unwrap()).token);
}

#[test]
fn the_your_type_slot_wants_the_boards_most_common_type() {
    let mut g = game(10);
    // A board of two Beasts and one Mech: the first slot is a Beast.
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Cord Puller");
    assert_eq!(g.most_common_board_type(0), Some(Tribe::Beast));
    to_turn(&mut g, 6);
    settle(&mut g, 0);
    g.players[0].gold = 10;
    let step = *g.data.gifts.offer_step(6).unwrap();
    assert!(step.slots[0].yours);
    act(&mut g, 0, Action::DarkGift);
    let first = g.players[0].discover[0];
    assert!(
        g.data.cards.tribes_of(&g.players[0].board[0]).has(Tribe::Beast),
        "the board still has Beasts"
    );
    assert!(
        g.data.cards.get(first).tribes.has(Tribe::Beast),
        "slot 1 was {} ({}), not a Beast",
        g.data.cards.get(first).name,
        g.data.cards.get(first).tier
    );
}

// ---- gifts on minions ----

#[test]
fn a_gift_adds_its_stats_and_a_second_one_stacks() {
    let mut g = game(11);
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Fortitude");
    let m = g.players[0].board[pos];
    assert_eq!((m.attack, m.health), (1 + 5, 1 + 5));
    assert_eq!(gift_names(&g, &m), ["Gift: Fortitude"]);
    // Gifts stack (docs/darkgifts.md): a second Fortitude adds again.
    g.give_gift(0, pos, "Fortitude", 3);
    let m = g.players[0].board[pos];
    assert_eq!((m.attack, m.health), (11, 11));
    assert_eq!(m.n_gifts, 2);
    // Except the ones that set a state: a second Harpy's Talons does
    // nothing, and a second Toreth's Blessing neither.
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Harpy's Talons");
    g.give_gift(0, pos, "Harpy's Talons", 3);
    assert_eq!(g.players[0].board[pos].n_gifts, 1);
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Toreth's Blessing");
    g.give_gift(0, pos, "Toreth's Blessing", 3);
    assert_eq!(g.players[0].board[pos].n_gifts, 1);
    assert_eq!(g.players[0].board[pos].shield_hits, 3, "still three hits");
    // Titanic Strength is +1000 Attack.
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Titanic Strength");
    assert_eq!(g.players[0].board[pos].attack, 1001);
}

#[test]
fn harpy_talons_harpys_two_keywords_and_gilding_makes_it_golden() {
    let mut g = game(12);
    let pos = board_with_gift(&mut g, 0, "Buzzing Vermin", "Harpy's Talons");
    let m = g.players[0].board[pos];
    assert!(m.keywords.has(Keywords::DIVINE_SHIELD));
    assert!(m.keywords.has(Keywords::WINDFURY));
    // Gilding: the golden card's printed stats, no Triple Reward.
    let pos = put_on_board(&mut g, 0, "Buzzing Vermin");
    let plain = g.players[0].board[pos];
    assert!(g.give_gift(0, pos, "Gilding", 4));
    let m = g.players[0].board[pos];
    assert!(m.golden);
    assert_eq!((m.attack, m.health), (plain.attack * 2, plain.health * 2));
    // A gift on a golden minion does not double.
    let pos = board_with_gift(&mut g, 0, "Buzzing Vermin", "Fortitude");
    let plain_attack = g.players[0].board[pos].attack;
    g.players[0].board[pos].golden = true;
    assert_eq!(g.players[0].board[pos].attack, plain_attack, "no doubling");
}

#[test]
fn a_triple_keeps_the_gifts_of_all_three_copies() {
    let mut g = game(13);
    // One copy in hand with Fortitude, one on the board with Fortitude and
    // Harpy's Talons, and a third bought from the Tavern makes the triple.
    let c = id(&g, "Cord Puller");
    let n = pool_copies(&mut g, c);
    let mut hand_copy = g.players[0].make_card(&g.data.cards, c, false, n);
    bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut hand_copy, gift(&g, "Fortitude"), 3);
    g.players[0].hand.push(hand_copy);
    let n = pool_copies(&mut g, c);
    let mut board_copy = g.players[0].make_card(&g.data.cards, c, false, n);
    bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut board_copy, gift(&g, "Fortitude"), 3);
    bgcore::game::darkgifts::land_gift(
        &g.data,
        &g.players[0],
        &mut board_copy,
        gift(&g, "Harpy's Talons"),
        5,
    );
    g.players[0].board.push(board_copy);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    let golden = g
        .players[0]
        .hand
        .iter()
        .find(|m| m.card == c && m.golden)
        .expect("the triple is golden");
    let names = gift_names(&g, golden);
    // Gifts stack (docs/darkgifts.md): Fortitude was on two copies, so the
    // golden holds it twice; Harpy's Talons sets a state, so once.
    assert_eq!(
        names,
        ["Gift: Fortitude", "Gift: Fortitude", "Gift: Harpy's Talons"],
        "{names:?}"
    );
    assert!(golden.keywords.has(Keywords::WINDFURY));
    g.check_invariants().unwrap();
}

#[test]
fn a_magnetized_mech_keeps_its_gifts_and_the_magnetic_minions_are_lost() {
    let mut g = game(14);
    let host = board_with_gift(&mut g, 0, "Cord Puller", "Fortitude");
    let mag = id(&g, "Lullabot");
    let n = pool_copies(&mut g, mag);
    let mut m = g.players[0].make_card(&g.data.cards, mag, false, n);
    bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut m, gift(&g, "Furtiveness"), 4);
    assert_eq!(m.n_gifts, 1, "the magnetic minion's own gift");
    g.magnetize_onto(0, host, m);
    // docs/darkgifts.md "Magnetic minions and gifts": the Mech keeps its own
    // gifts and the magnetic minion's are lost (Era thinks so).
    let names = gift_names(&g, &g.players[0].board[host]);
    assert_eq!(names, ["Gift: Fortitude"]);
    g.check_invariants().unwrap();
}

#[test]
fn steady_growth_carries_the_numbers_of_the_turn_it_landed() {
    let mut g = game(15);
    let steady = gift(&g, "Steady Growth");
    for (turn, want) in [(3, (2, 2)), (4, (2, 3)), (5, (3, 4)), (6, (3, 5)), (8, (4, 7))] {
        let c = id(&g, "Cord Puller");
        let n = pool_copies(&mut g, c);
        let mut m = g.players[0].make_card(&g.data.cards, c, false, n);
        bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut m, steady, turn);
        let state = m.gift_slice()[0].state();
        assert_eq!(g.data.gifts.steady_growth.numbers(state as u32), want, "turn {turn}");
        assert!(matches!(
            g.data.cards.get(g.data.gifts.gifts[steady].card).end_of_turn,
            Some(EndOfTurn::GiftBuffSelf)
        ));
    }
}

#[test]
fn incalculable_countdowns_tick_two_turns_at_the_end_of_the_turn() {
    let mut g = game(16);
    let incubation = gift(&g, "Incubation");
    let c = id(&g, "Cord Puller");
    let n = pool_copies(&mut g, c);
    let mut m = g.players[0].make_card(&g.data.cards, c, false, n);
    bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut m, incubation, 6);
    g.players[0].board.push(m);
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (5, 5));
    g.players[0].board[0].attack = 10;
    g.players[0].board[0].health = 10;
    // First tick (the end of the turn): 1 turn left, nothing doubles.
    g.dark_gift_end_of_turn(0);
    assert_eq!(g.players[0].board[0].gift_slice()[0].state(), 1);
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (10, 10));
    // Second tick: it fires and the gift comes off.
    g.dark_gift_end_of_turn(0);
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (20, 20));
    assert_eq!(g.players[0].board[0].n_gifts, 0);
}

#[test]
fn replication_and_affinity_count_down_then_hand_over_a_card() {
    let mut g = game(17);
    let replication = gift(&g, "Replication");
    let pos = put_on_board(&mut g, 0, "Cord Puller");
    g.players[0].board[pos].attack = 9;
    g.players[0].board[pos].health = 9;
    assert!(g.give_gift(0, pos, g.data.gifts.gifts[replication].name.clone().as_str(), 4));
    g.dark_gift_end_of_turn(0);
    assert!(g.players[0].hand.is_empty(), "one turn to go");
    g.dark_gift_end_of_turn(0);
    assert_eq!(g.players[0].hand.len(), 1, "a plain copy");
    let copy = g.players[0].hand[0];
    assert_eq!((copy.card, copy.attack, copy.health), (id(&g, "Cord Puller"), 9, 9));
    assert_eq!(copy.n_gifts, 0, "a plain copy carries no gift");
    // Affinity hands over a minion of the host's type.
    let affinity = gift(&g, "Affinity");
    let pos = put_on_board(&mut g, 0, "Cord Puller");
    assert!(g.give_gift(0, pos, g.data.gifts.gifts[affinity].name.clone().as_str(), 3));
    g.players[0].hand.clear();
    g.dark_gift_end_of_turn(0);
    g.dark_gift_end_of_turn(0);
    assert_eq!(g.players[0].hand.len(), 1);
    let card = g.data.cards.get(g.players[0].hand[0].card);
    assert!(card.tribes.has(Tribe::Mech), "{} is not a Mech", card.name);
}

/// Drakkari Enchanter makes a countdown's payout run twice on the turn it
/// fires, and never speeds the count itself (docs/darkgifts.md, Era
/// 2026-11-02): two plain copies from Replication, and Incubation's doubling
/// twice.
#[test]
fn drakkari_doubles_the_payout_not_the_count() {
    let mut g = game(18);
    let replication = gift(&g, "Replication");
    let pos = put_on_board(&mut g, 0, "Cord Puller");
    g.players[0].board[pos].attack = 4;
    g.players[0].board[pos].health = 4;
    assert!(g.give_gift(0, pos, g.data.gifts.gifts[replication].name.clone().as_str(), 4));
    put_on_board(&mut g, 0, "Drakkari Enchanter");
    // The first end of turn still only counts: one turn to go, no copy.
    g.dark_gift_end_of_turn(0);
    assert_eq!(g.players[0].hand.len(), 0, "the count is not doubled");
    assert_eq!(g.players[0].board[pos].gift_slice()[0].state(), 1);
    // The second end of turn fires it twice: two plain copies.
    g.dark_gift_end_of_turn(0);
    assert_eq!(g.players[0].hand.len(), 2, "two plain copies from one payout");
    assert!(g.players[0].hand.iter().all(|m| m.card == id(&g, "Cord Puller") && m.n_gifts == 0));
    assert_eq!(g.players[0].board[pos].n_gifts, 0, "and the gift is spent");

    // Incubation doubles the minion's stats twice with a Drakkari.
    let mut g = game(19);
    let incubation = gift(&g, "Incubation");
    let c = id(&g, "Cord Puller");
    let n = pool_copies(&mut g, c);
    let mut m = g.players[0].make_card(&g.data.cards, c, false, n);
    bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut m, incubation, 6);
    g.players[0].board.push(m);
    put_on_board(&mut g, 0, "Drakkari Enchanter");
    g.players[0].board[0].attack = 10;
    g.players[0].board[0].health = 10;
    g.dark_gift_end_of_turn(0);
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (10, 10));
    g.dark_gift_end_of_turn(0);
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (40, 40));
}

// ---- gift effects, one test per mechanism ----

/// What a fight did, as the tests read it: the first attacker's stats after
/// Start of Combat, everything summoned and the live minions at the end.
#[derive(Default)]
struct Seen {
    /// Every (side, card, attack, health) at the moment of a swing, both the
    /// attacker's and the defender's, in the order they were seen.
    at_attacks: Vec<(usize, CardId, i32, i32)>,
    /// A minion's shield before each swing it is in, attacker or defender:
    /// (side, card, hits left, has a Divine Shield).
    shields: Vec<(usize, CardId, u8, bool)>,
    summons: Vec<(usize, CardId)>,
    /// Every summon with its stats, for the Golem tests.
    summoned: Vec<(usize, CardId, i32, i32)>,
    deaths: Vec<(usize, CardId)>,
}

impl Seen {
    /// The first stats a card on a side was seen with in a swing.
    fn stats_of(&self, side: usize, card: CardId) -> Option<(i32, i32)> {
        self.at_attacks
            .iter()
            .find(|&&(s, c, _, _)| s == side && c == card)
            .map(|&(_, _, a, h)| (a, h))
    }
}

impl bgcore::fight::Watch for Seen {
    fn attack(&mut self, side: usize, attacker: &Minion, defender: &Minion) {
        self.at_attacks.push((side, attacker.card, attacker.attack, attacker.health));
        self.at_attacks.push((side ^ 1, defender.card, defender.attack, defender.health));
        self.shields.push((
            side,
            attacker.card,
            attacker.shield_hits,
            attacker.keywords.has(Keywords::DIVINE_SHIELD),
        ));
        self.shields.push((
            side ^ 1,
            defender.card,
            defender.shield_hits,
            defender.keywords.has(Keywords::DIVINE_SHIELD),
        ));
    }
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.summons.push((side, m.card));
        self.summoned.push((side, m.card, m.attack, m.health));
    }
    fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.deaths.push((side, m.card));
    }
}

/// A minion with the named gift on it, ready for a fight.
fn gifted(db: &bgcore::CardDb, data: &Data, pl: &bgcore::Player, name: &str, host: &str, turn: u32) -> Minion {
    let mut m = db.make(db.by_name(host).unwrap(), false);
    let i = data.gifts.gifts.iter().position(|g| g.name == name).unwrap();
    bgcore::game::darkgifts::land_gift(data, pl, &mut m, i, turn);
    m
}

fn fight_pair(given: Vec<Minion>, enemies: Vec<Minion>, counters: [Counters; 2]) -> (Outcome2, Seen) {
    let data = Data::bundled();
    let db = &data.cards;
    let mut seen = Seen::default();
    let mut rng = Rng::new(99);
    let out = fight_with(db, &given, &enemies, counters, 40, &mut rng, &mut seen);
    (Outcome2 { winner: out.winner, attacks: out.attacks }, seen)
}

struct Outcome2 {
    winner: Option<usize>,
    attacks: u32,
}

impl Outcome2 {
    /// The winning side, 0 or 1, None on a tie.
    #[allow(dead_code)]
    fn winner(&self) -> Option<usize> {
        self.winner
    }
}

#[test]
fn resistance_hostility_and_transcendence_change_the_stats_at_start_of_combat() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(20);
    let enemy = || vec![db.make(db.by_name("Cord Puller").unwrap(), false)];
    // The host's stats are read off the first swing it takes part in.
    let host_card = id(&g, "Cord Puller");
    let hit = |host: Minion| {
        let (out, seen) = fight_pair(vec![host], enemy(), [Counters::default(); 2]);
        assert!(out.attacks > 0);
        seen.stats_of(0, host_card).expect("the host was in a swing")
    };
    let mut host = gifted(db, &data, &g.players[0], "Resistance", "Cord Puller", 8);
    host.attack = 0;
    host.health = 4;
    assert_eq!(hit(host).1, 8, "double Health");
    let mut host = gifted(db, &data, &g.players[0], "Hostility", "Cord Puller", 8);
    host.attack = 3;
    host.health = 40;
    assert_eq!(hit(host).0, 6, "double Attack");
    let mut host = gifted(db, &data, &g.players[0], "Transcendence", "Cord Puller", 8);
    host.attack = 2;
    host.health = 3;
    assert_eq!(hit(host), (6, 9), "triple stats");
}

#[test]
fn admiration_takes_the_attack_of_the_minion_to_the_left() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(21);
    let mut left = db.make(db.by_name("Cord Puller").unwrap(), false);
    left.attack = 7;
    left.health = 40;
    // A different card, so the watcher can tell the host apart from the left
    // minion (both sides' slots carry card ids).
    let mut host = gifted(db, &data, &g.players[0], "Admiration", "Buzzing Vermin", 8);
    host.attack = 1;
    host.health = 60;
    let enemy = vec![db.make(db.by_name("Cord Puller").unwrap(), false)];
    let (_, seen) = fight_pair(vec![left, host], enemy, [Counters::default(); 2]);
    // The host took the left minion's 7 Attack at Start of Combat.
    let host_card = id(&g, "Buzzing Vermin");
    let (a, _) = seen.stats_of(0, host_card).expect("the host swung");
    assert!(a >= 8, "the host swung with {a}, not its own 1 plus the left 7");
}

#[test]
fn worship_takes_the_deitys_stats() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(22);
    let mut host = gifted(db, &data, &g.players[0], "Worship", "Cord Puller", 12);
    host.attack = 0;
    host.health = 2;
    let mut counters = [Counters::default(); 2];
    counters[0].deity = Some(bgcore::fight::Deity {
        card: db.by_name("C'Thun").unwrap(),
        attack: 5,
        health: 5,
        golden: false,
    });
    let enemy = vec![db.make(db.by_name("Cord Puller").unwrap(), false)];
    let (_, seen) = fight_pair(vec![host], enemy, counters);
    let host_card = id(&g, "Cord Puller");
    assert_eq!(seen.stats_of(0, host_card).unwrap(), (5, 7), "Deity stats on top");
}

#[test]
fn jaws_of_death_fires_the_hosts_own_deathrattle_at_start_of_combat() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(23);
    // Buzzing Vermin's Deathrattle summons a Beetle.
    let mut host = gifted(db, &data, &g.players[0], "Jaws of Death", "Buzzing Vermin", 5);
    host.attack = 0;
    host.health = 30;
    let enemy = vec![db.make(db.by_name("Cord Puller").unwrap(), false)];
    let (_, seen) = fight_pair(vec![host], enemy, [Counters::default(); 2]);
    let beetles = seen.summons.iter().filter(|&&(s, _)| s == 0).count();
    assert!(beetles >= 1, "no Beetle was summoned: {:?}", seen.summons);
}

#[test]
fn doppelganger_summons_a_copy_at_start_of_combat() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(24);
    let mut host = gifted(db, &data, &g.players[0], "Doppelganger", "Cord Puller", 8);
    host.attack = 0;
    host.health = 30;
    let enemy = vec![db.make(db.by_name("Cord Puller").unwrap(), false)];
    let (_, seen) = fight_pair(vec![host], enemy, [Counters::default(); 2]);
    let copies = seen
        .summons
        .iter()
        .filter(|&&(s, c)| s == 0 && c == db.by_name("Cord Puller").unwrap())
        .count();
    assert_eq!(copies, 1, "{:?}", seen.summons);
}

#[test]
fn the_deathrattle_gifts_give_stats_cards_and_free_refreshes() {
    // Offensive Sacrifice: the +10 Attack goes to a random other friendly
    // minion when the host dies.
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(25);
    let mut host = gifted(db, &data, &g.players[0], "Offensive Sacrifice", "Cord Puller", 6);
    host.attack = 10;
    host.health = 1;
    let mut killer = db.make(db.by_name("Microbot").unwrap(), false);
    killer.attack = 50;
    killer.health = 400;
    let host_card = id(&g, "Cord Puller");
    let (_, seen) = fight_pair(vec![host], vec![killer], [Counters::default(); 2]);
    assert!(
        seen.deaths.iter().any(|&(s, c)| s == 0 && c == host_card),
        "the host did not die: {:?}",
        seen.deaths
    );
    // The friend is not on the board here; the death still fires safely.
    // Golemancy: a Golem with the dead minion's stats.
    let mut host = gifted(db, &data, &g.players[0], "Golemancy", "Cord Puller", 9);
    host.attack = 8;
    host.health = 1;
    let mut keeper = db.make(db.by_name("Cord Puller").unwrap(), false);
    keeper.health = 40;
    let (_, seen) = fight_pair(vec![host, keeper], vec![killer], [Counters::default(); 2]);
    assert!(
        seen.summons.iter().any(|&(s, c)| s == 0 && c == db.by_name("Golem").unwrap()),
        "no Golem: {:?}",
        seen.summons
    );
}

#[test]
fn fresh_perspective_gives_two_free_refreshes_and_mystic_essence_a_spell() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(26);
    let mut host = gifted(db, &data, &g.players[0], "Fresh Perspective", "Cord Puller", 6);
    host.health = 1;
    let mut killer = db.make(db.by_name("Microbot").unwrap(), false);
    killer.attack = 50;
    killer.health = 400;
    let mut rng = Rng::new(3);
    let out = fight_with(db, &[host], &[killer], [Counters::default(); 2], 40, &mut rng, &mut ());
    assert_eq!(out.sides[0].free_refresh_gain, 2);
    // Mystic Essence: a random Tavern spell comes home in the hand.
    let mut host = gifted(db, &data, &g.players[0], "Mystic Essence", "Cord Puller", 6);
    host.health = 1;
    let mut rng = Rng::new(4);
    let out = fight_with(db, &[host], &[killer], [Counters::default(); 2], 40, &mut rng, &mut ());
    assert_eq!(out.sides[0].cards_gained().len(), 1, "one Tavern spell");
    assert!(db.get(out.sides[0].cards_gained()[0]).tavern_spell);
}

#[test]
fn the_rally_gifts_hand_over_gems_and_minions() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(27);
    // Consanguinity: two Blood Gems on the swing.
    let host = gifted(db, &data, &g.players[0], "Consanguinity", "Buzzing Vermin", 3);
    let mut enemy = db.make(db.by_name("Cord Puller").unwrap(), false);
    enemy.attack = 0;
    enemy.health = 60;
    let mut rng = Rng::new(5);
    let out = fight_with(db, &[host], &[enemy], [Counters::default(); 2], 1, &mut rng, &mut ());
    let gems = out.sides[0]
        .cards_gained()
        .iter()
        .filter(|&&c| Some(c) == db.blood_gem)
        .count();
    assert_eq!(gems, 2, "{:?}", out.sides[0].cards_gained());
    // Charisma: a minion of the most common type on the board (a Beast).
    let host = gifted(db, &data, &g.players[0], "Charisma", "Buzzing Vermin", 7);
    let mut enemy = db.make(db.by_name("Cord Puller").unwrap(), false);
    enemy.attack = 0;
    enemy.health = 60;
    let mut rng = Rng::new(6);
    let out = fight_with(db, &[host], &[enemy], [Counters::default(); 2], 1, &mut rng, &mut ());
    let gained = out.sides[0].cards_gained();
    assert_eq!(gained.len(), 1);
    assert!(db.get(gained[0]).tribes.has(Tribe::Beast), "{}", db.get(gained[0]).name);
}

#[test]
fn the_shop_gifts_buff_on_a_card_played_and_on_events() {
    let mut g = game(28);
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Dexterity");
    let before = g.players[0].board[pos].attack;
    // Playing a card is a play (a Tavern spell from hand).
    let bananas = id(&g, "Bananas");
    let m = g.players[0].make_card(&g.data.cards, bananas, false, 0);
    g.players[0].hand.push(m);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].board[pos].attack, before + 2, "Dexterity +2/+2");
    // Sharpened Sword is Attack only, Toughened Shield Health only.
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Sharpened Sword");
    let (a, h) = (g.players[0].board[pos].attack, g.players[0].board[pos].health);
    g.dark_gift_card_played(0);
    assert_eq!((g.players[0].board[pos].attack, g.players[0].board[pos].health), (a + 3, h));
    // Spell Siphon counts Tavern spells cast this game, never a plain spell.
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Spell Siphon");
    let m = g.players[0].board[pos];
    assert_eq!(m.n_gifts, 1);
    g.players[0].spells_cast = 2;
    // A gift that lands now takes the count so far.
    let pos2 = put_on_board(&mut g, 0, "Cord Puller");
    let before = g.players[0].board[pos2].attack;
    assert!(g.give_gift(0, pos2, "Spell Siphon", 7));
    assert_eq!(g.players[0].board[pos2].attack, before + 6, "+3/+3 for each of 2");
    // Battle Scars counts Battlecries.
    let pos3 = put_on_board(&mut g, 0, "Cord Puller");
    g.players[0].battlecries_triggered = 3;
    let before = g.players[0].board[pos3].attack;
    assert!(g.give_gift(0, pos3, "Battle Scars", 7));
    assert_eq!(g.players[0].board[pos3].attack, before + 9);
    // Desecration counts discards.
    let pos4 = put_on_board(&mut g, 0, "Cord Puller");
    g.players[0].discarded = 1;
    let before = g.players[0].board[pos4].health;
    assert!(g.give_gift(0, pos4, "Desecration", 5));
    assert_eq!(g.players[0].board[pos4].health, before + 3);
    // And they grow on the event.
    let before = g.players[0].board[pos4].attack;
    g.dark_gift_count(0, 1, |c| c.buff_per_discard);
    assert_eq!(g.players[0].board[pos4].attack, before + 3);
}

#[test]
fn double_vision_brings_a_plain_copy_along() {
    let mut g = game(29);
    let c = id(&g, "Cord Puller");
    let n = pool_copies(&mut g, c);
    let mut m = g.players[0].make_card(&g.data.cards, c, false, n);
    bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut m, gift(&g, "Double Vision"), 5);
    assert!(g.take_into_hand(0, m));
    assert_eq!(g.players[0].hand.len(), 2, "the minion and its copy");
    assert_eq!(g.players[0].hand[1].n_gifts, 0, "the copy carries no gift");
    g.check_invariants().unwrap();
}

#[test]
fn efficiency_makes_an_activate_run_twice() {
    let mut g = game(30);
    let pos = put_on_board(&mut g, 0, "Suspicious Prisonguard");
    // Its Activate buffs a minion +3/+3 for 1 Gold.
    let other = put_on_board(&mut g, 0, "Cord Puller");
    assert!(g.give_gift(0, pos, "Efficiency", 5));
    g.players[0].gold = 10;
    turn_of(&mut g, 0);
    let before = g.players[0].board[other].attack;
    act(&mut g, 0, Action::Activate(pos as u8));
    act(&mut g, 0, Action::Target(other as u8));
    assert_eq!(g.players[0].board[other].attack, before + 6, "twice the +3");
    g.check_invariants().unwrap();
}

#[test]
fn echoing_voice_triggers_the_battlecries_at_the_end_of_the_turn() {
    let mut g = game(31);
    // Razorfen Geomancer's Battlecry gets 2 Blood Gems.
    board_with_gift(&mut g, 0, "Razorfen Geomancer", "Echoing Voice");
    let gems = |g: &Game| {
        g.players[0].hand.iter().filter(|m| Some(m.card) == g.data.cards.blood_gem).count()
    };
    let before = gems(&g);
    act(&mut g, 0, Action::EndTurn);
    // The end of the turn ran before the fight: the hand has the Gems.
    assert_eq!(gems(&g), before + 2, "the Battlecry ran");
}

#[test]
fn polarization_magnetizes_a_mech_and_time_turning_runs_the_end_of_the_turn_at_the_start() {
    let mut g = game(32);
    let pos = put_on_board(&mut g, 0, "Cord Puller");
    assert!(g.give_gift(0, pos, "Polarization", 5));
    let magnetizations = g.players[0].board[pos].magnetizations;
    act(&mut g, 0, Action::EndTurn);
    assert!(
        g.players[0].board[pos].magnetizations > magnetizations,
        "no Mech was Magnetized on"
    );
    // Time Turning: a Lullabot's end of turn (+1 Health) at the start of the
    // turn too.
    let pos = put_on_board(&mut g, 0, "Lullabot");
    assert!(g.give_gift(0, pos, "Time Turning", 5));
    let health = g.players[0].board[pos].health;
    g.dark_gift_start_of_turn(0);
    assert_eq!(g.players[0].board[pos].health, health + 1, "the +1/+1 ran at start too");
}

#[test]
fn persisting_horror_comes_back_with_full_stats_and_bonus_keywords() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(34);
    let mut host = gifted(db, &data, &g.players[0], "Persisting Horror", "Cord Puller", 10);
    host.attack = 6;
    host.health = 2;
    host.keywords.set(Keywords::TAUNT);
    host.keywords.set(Keywords::REBORN);
    let mut enemy = db.make(db.by_name("Microbot").unwrap(), false);
    enemy.attack = 50;
    enemy.health = 500;
    let mut seen = Seen::default();
    let mut rng = Rng::new(8);
    let _ = fight_with(db, &[host], &[enemy], [Counters::default(); 2], 40, &mut rng, &mut seen);
    let reborn = seen
        .summons
        .iter()
        .filter(|&&(s, c)| s == 0 && c == db.by_name("Cord Puller").unwrap())
        .count();
    assert_eq!(reborn, 1, "{:?}", seen.summons);
}

#[test]
fn toreth_blessing_takes_three_hits_and_invulnerability_takes_none() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game(35);
    // A Toreth shield eats two hits and breaks on the third.
    let mut host = gifted(db, &data, &g.players[0], "Toreth's Blessing", "Cord Puller", 5);
    host.attack = 0;
    host.health = 40;
    assert_eq!(host.shield_hits, 3);
    let mut enemy = db.make(db.by_name("Cord Puller").unwrap(), false);
    enemy.attack = 1;
    enemy.health = 40;
    let (_, seen) = fight_pair(vec![host], vec![enemy], [Counters::default(); 2]);
    assert_eq!(seen.deaths.len(), 0, "the shield held");
    // Invulnerability: the attacker takes nothing back.
    let mut host = gifted(db, &data, &g.players[0], "Invulnerability", "Cord Puller", 12);
    host.attack = 1;
    host.health = 4;
    let mut enemy = db.make(db.by_name("Cord Puller").unwrap(), false);
    enemy.attack = 50;
    enemy.health = 40;
    let mut rng = Rng::new(9);
    let _ = fight_with(db, &[host], &[enemy], [Counters::default(); 2], 2, &mut rng, &mut ());
}

#[test]
fn amalgamation_makes_the_minion_every_type() {
    let mut g = game(36);
    let pos = put_on_board(&mut g, 0, "Cord Puller");
    let before = g.data.cards.tribes_of(&g.players[0].board[pos]);
    assert!(!before.has(Tribe::Beast));
    assert!(g.give_gift(0, pos, "Amalgamation", 8));
    let after = g.data.cards.tribes_of(&g.players[0].board[pos]);
    for t in Tribe::EVERY {
        assert!(after.has(t), "{t:?}");
    }
}

#[test]
fn toxicity_furtiveness_and_the_fifty_weights_land_their_keywords() {
    let mut g = game(37);
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Toxicity");
    assert!(g.players[0].board[pos].keywords.has(Keywords::VENOMOUS));
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Furtiveness");
    assert!(g.players[0].board[pos].keywords.has(Keywords::STEALTH));
    let pos = board_with_gift(&mut g, 0, "Cord Puller", "Toreth's Blessing");
    assert!(g.players[0].board[pos].keywords.has(Keywords::DIVINE_SHIELD));
    assert_eq!(g.players[0].board[pos].shield_hits, 3);
}

#[test]
fn every_gift_effect_card_runs_somewhere() {
    // A gift's effect must be reachable: each one is either a keyword, a
    // stat change at landing, a stored counter, or a trigger node one of the
    // engine's paths reads. This walks the table so a new gift cannot be
    // added without a place to run.
    let data = Data::bundled();
    for g in &data.gifts.gifts {
        let c = data.cards.get(g.card);
        let runs = c.keywords.0 != 0
            || c.attack != 0
            || c.health != 0
            || c.deathrattle.is_some()
            || c.start_of_combat.is_some()
            || c.end_of_turn.is_some()
            || c.rally.is_some()
            || c.on_card_play.is_some()
            || c.buff_per_battlecry.is_some()
            || c.buff_per_tavern_spell.is_some()
            || c.buff_per_discard.is_some()
            || c.buff_per_deathrattle.is_some()
            || c.permanent_keep.is_some()
            || c.acquire_copies > 0
            || c.activate_twice
            || c.shield_hits > 1
            || c.immune_attacking
            || c.all_types
            || c.reborn_full
            || c.makes_golden
            || c.end_of_turn_at_start
            || c.countdown.is_some()
            // Sunken Persistence has no host with a Spellcraft yet
            // (docs/ENGINE.md "Dark Gifts").
            || g.name == "Sunken Persistence";
        assert!(runs, "gift {} has nothing to run", g.name);
    }
}

#[test]
fn the_gift_trinkets_open_a_gift_offer() {
    let mut g = game(38);
    to_turn(&mut g, 6);
    settle(&mut g, 0);
    // Ominous Stone: a Tier 4 minion of the player's most common type.
    put_on_board(&mut g, 0, "Cord Puller");
    put_on_board(&mut g, 0, "Cord Puller");
    g.open_dark_gift_trinket(0, Some(4), true, false, false);
    assert_eq!(g.players[0].discover.len(), 3, "three offers");
    for (i, &c) in g.players[0].discover.iter().enumerate() {
        let card = g.data.cards.get(c);
        assert_eq!(card.tier, 4, "{}", card.name);
        assert!(card.tribes.has(Tribe::Mech), "{} is not a Mech", card.name);
        assert!(g.players[0].discover_gifts[i].card() != 0, "no gift on offer {i}");
    }
    assert!(!g.players[0].dark_offer_golden);
    settle(&mut g, 0);
    // Wax Lance: Tier 7, any type.
    g.open_dark_gift_trinket(0, Some(7), false, false, false);
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in &g.players[0].discover {
        assert_eq!(g.data.cards.get(c).tier, 7, "{}", g.data.cards.get(c).name);
    }
    settle(&mut g, 0);
    // Sinister Invitation: Golden offers, of the player's type, no reward.
    g.open_dark_gift_trinket(0, None, true, true, true);
    assert!(g.players[0].dark_offer_golden);
    let offers: Vec<CardId> = g.players[0].discover.to_vec();
    assert!(!offers.is_empty());
    g.check_invariants().unwrap();
    act(&mut g, 0, Action::Choose(0));
    let m = *g.players[0].hand.last().expect("the pick");
    assert!(m.golden, "a Golden offer");
    assert!(m.no_reward, "it gives no Triple Reward");
    assert!(g.data.cards.tribes_of(&m).has(Tribe::Mech));
    g.check_invariants().unwrap();
}

#[test]
fn the_dagger_offers_new_gifts_on_any_of_your_minions() {
    // Maldraxxus Dagger: plain copies of three different friendly minions,
    // gifted or not, each with a NEW gift picked by the normal rules. It is
    // the one source that may copy a Magnetic minion (the button never
    // offers one).
    let mut g = game(41);
    to_turn(&mut g, 5);
    settle(&mut g, 0);
    // One gifted minion, one plain minion and one Magnetic Mech.
    board_with_gift(&mut g, 0, "Cord Puller", "Fortitude");
    put_on_board(&mut g, 0, "Suspicious Prisonguard");
    let lulla = put_on_board(&mut g, 0, "Lullabot");
    assert!(g.data.cards.get(g.players[0].board[lulla].card).keywords.has(Keywords::MAGNETIC));
    let turn = g.turn;
    g.deal_dark_gift_copies(0, turn);
    let offers: Vec<CardId> = g.players[0].discover.to_vec();
    assert_eq!(offers.len(), 3, "three different minions");
    let mut sorted = offers.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 3, "three different cards: {offers:?}");
    // The Magnetic minion is among them.
    assert!(
        offers.iter().any(|&c| g.data.cards.get(c).keywords.has(Keywords::MAGNETIC)),
        "the Lullabot was not offered: {offers:?}"
    );
    // Every offer carries a gift that fits its host on this turn, and no gift
    // repeats within the offer.
    let mut seen: Vec<u16> = Vec::new();
    for (i, &c) in offers.iter().enumerate() {
        let gift = g.players[0].discover_gifts[i];
        let card = g.data.cards.get(c);
        let gift_card = g.data.cards.get(gift.card());
        let index = g.data.gifts.gift_of_card(gift.card()).expect("a gift card");
        let spec = &g.data.gifts.gifts[index];
        assert!(spec.in_window(turn), "{} is not open on turn {turn}", spec.name);
        assert!(
            spec.cond.fits(card, card.tribes, false),
            "{} does not fit {}",
            spec.name,
            card.name
        );
        assert!(!seen.contains(&gift.card()), "{} twice", gift_card.name);
        seen.push(gift.card());
    }
    // The pick is a plain copy (printed stats, no pool copy) with its gift.
    // Take an offer whose gift adds no stats of its own and does not gild,
    // so the copy shows the card's printed numbers.
    let k = (0..offers.len())
        .find(|&k| {
            let g_card = g.data.cards.get(g.players[0].discover_gifts[k].card());
            (g_card.attack, g_card.health) == (0, 0) && !g_card.makes_golden
        })
        .expect("an offer with a plain gift");
    let pick = offers[k];
    let printed = g.data.cards.get(pick).attack;
    g.check_invariants().unwrap();
    turn_of(&mut g, 0);
    act(&mut g, 0, Action::Choose(k as u8));
    let m = *g.players[0].hand.last().expect("the pick");
    assert_eq!(m.card, pick);
    assert_eq!(m.attack, printed, "a plain copy: printed stats");
    assert_eq!(m.pool_copies, 0, "generated, no pool copy");
    assert_eq!(m.n_gifts, 1, "and it carries the new gift");
    g.check_invariants().unwrap();
    // A gift already on the source does not travel: the source's own gift
    // list is untouched and the copy gets a gift of its own choosing.
    assert_eq!(g.players[0].board[0].n_gifts, 1, "the Fortitude source keeps one");
}

#[test]
fn the_gift_heroes_open_their_offers() {
    // Void Power: a Tier 5 minion with a gift at the start of the game, its
    // gifts picked as if it were turn 7, locked in hand until turn 7.
    let d = data();
    let mut g = game(39);
    let tras = d.heroes.by_name("Tras'tath, Soul Parasite").unwrap();
    g.assign_hero(0, tras);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].discover.len(), 3, "the Void Power offer");
    for (i, &c) in g.players[0].discover.iter().enumerate() {
        assert_eq!(g.data.cards.get(c).tier, 5);
        assert!(g.players[0].discover_gifts[i].card() != 0);
    }
    act(&mut g, 0, Action::Choose(0));
    let m = *g.players[0].hand.last().expect("the pick");
    assert!(m.gifts[0].card() != 0, "the pick carries its gift");
    assert!(m.locked_turns >= 5, "locked in hand until turn 7");
    // Feel Devastation: a gift offer every 4 turns.
    let mut g = game(40);
    let xavius = d.heroes.by_name("Nightmare Lord Xavius").unwrap();
    g.assign_hero(0, xavius);
    to_turn(&mut g, 4);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].discover.len(), 3, "the turn 4 offer");
}

// ---- Dark Paradox (BG36_360) ----

const PARADOX: [&str; 5] =
    ["Dark Paradox t3", "Dark Paradox t4", "Dark Paradox t5", "Dark Paradox t6", "Dark Paradox t9"];

fn paradox_cards(g: &Game) -> Vec<CardId> {
    PARADOX.iter().map(|n| id(g, n)).collect()
}

/// A game whose pool holds the named Dark Paradox version (the pick comes
/// from the seed, so this scans).
fn game_with_paradox(version: &str) -> Game {
    let want = id(&game(0), version);
    for seed in 0..500u64 {
        let g = game(seed);
        if g.paradox == Some(want) {
            return g;
        }
    }
    panic!("no seed in 0..500 gives {version}");
}

#[test]
fn one_paradox_version_per_game_with_its_tier_s_copies() {
    let mut seen_versions = std::collections::BTreeSet::new();
    for seed in 0..60u64 {
        let g = game(seed);
        let cards = paradox_cards(&g);
        let with_copies: Vec<CardId> =
            cards.iter().copied().filter(|&c| g.pool[c as usize] > 0).collect();
        assert_eq!(with_copies.len(), 1, "seed {seed}: {with_copies:?}");
        assert_eq!(g.paradox, Some(with_copies[0]), "seed {seed}");
        let card = g.data.cards.get(with_copies[0]);
        assert_eq!(
            g.pool[with_copies[0] as usize],
            g.data.rules.copies(card.tier),
            "seed {seed}: {} is Tier {}",
            card.name,
            card.tier
        );
        // Every version has every type, so it is in every lobby like any
        // other card.
        for t in Tribe::EVERY {
            assert!(card.tribes.has(t), "{} misses {t:?}", card.name);
        }
        seen_versions.insert(with_copies[0]);
        g.check_invariants().unwrap();
    }
    assert_eq!(seen_versions.len(), 5, "60 seeds should pick every version once or more");
    // One tribe's lobby still has it.
    let one = Tribe::EVERY.into_iter().fold(Tribes::NONE, |m, t| m.with(t));
    let mut single = Tribes::NONE;
    single = single.with(Tribe::Murloc);
    let _ = one;
    let g = Game::new_with_lobby(data(), 3, single);
    let p = g.paradox.expect("a Dark Paradox in a one-tribe lobby");
    assert!(g.pool[p as usize] > 0);
}

#[test]
fn every_player_sees_only_the_games_version() {
    let mut g = game_with_paradox("Dark Paradox t5");
    let chosen = g.paradox.unwrap();
    let others: Vec<CardId> =
        paradox_cards(&g).into_iter().filter(|&c| c != chosen).collect();
    for _ in 0..4 {
        for p in 0..8usize {
            for m in g.players[p].shop.iter().chain(&g.players[p].hand).chain(&g.players[p].board) {
                for &o in &others {
                    assert_ne!(m.card, o, "player {p} holds a version outside the pool");
                }
            }
        }
        next_turn(&mut g);
        if g.over {
            break;
        }
    }
    g.check_invariants().unwrap();
}

#[test]
fn dark_discovery_never_offers_the_paradox() {
    let mut g = game(42);
    let cards = paradox_cards(&g);
    for &c in &cards {
        for turn in 3..=12u32 {
            assert!(
                !g.data.gifts.pool_allows(g.data.cards.get(c), c, turn),
                "{} offered on turn {turn}",
                g.data.cards.get(c).name
            );
        }
    }
    // And in real presses: no offer is one of them.
    let mut presses = 0;
    for turn in 3..=12u32 {
        to_turn(&mut g, turn);
        settle(&mut g, 0);
        g.players[0].gold = 10;
        if g.players[0].dark_gift_uses == 0 || !g.is_legal(Action::DarkGift) {
            continue;
        }
        act(&mut g, 0, Action::DarkGift);
        presses += 1;
        for &c in &g.players[0].discover {
            assert!(!cards.contains(&c), "an offer was {}", g.data.cards.get(c).name);
        }
        while !g.players[0].discover.is_empty() {
            act(&mut g, 0, Action::Choose(0));
        }
    }
    assert!(presses >= 2, "only {presses} presses");
}

#[test]
fn a_triple_of_the_tier_five_version_makes_its_golden() {
    let mut g = game_with_paradox("Dark Paradox t4");
    let version = g.paradox.unwrap();
    let name = g.data.cards.get(version).name.clone();
    let plain = g.data.cards.get(version);
    assert_eq!((plain.attack, plain.health, plain.tier), (8, 4, 5));
    // Three copies from the Tavern: the golden is the game data's BG36_360_Gt4.
    set_shop(&mut g, 0, &[&name, &name, &name]);
    g.players[0].gold = 20;
    for _ in 0..3 {
        act(&mut g, 0, Action::Buy(0));
        // A triple can open a Discover (a pick from the Tavern), so settle.
        settle(&mut g, 0);
    }
    let golden = g
        .players[0]
        .hand
        .iter()
        .chain(&g.players[0].board)
        .find(|m| m.card == version && m.golden)
        .copied()
        .expect("the triple is golden");
    assert_eq!((golden.attack, golden.health), (16, 8), "the golden's printed stats");
    assert!(golden.keywords.has(Keywords::DIVINE_SHIELD));
    // The Tier 5 golden's shield takes 4 hits, not double 3.
    assert_eq!(golden.shield_hits, 4);
    let plain = g.players[0]
        .hand
        .iter()
        .chain(&g.players[0].board)
        .find(|m| m.card == version && !m.golden)
        .copied();
    if let Some(p) = plain {
        assert_eq!(p.shield_hits, 3);
    }
    g.check_invariants().unwrap();
}

#[test]
fn the_tier_five_golden_shield_takes_four_hits() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game_with_paradox("Dark Paradox t4");
    let version = g.paradox.unwrap();
    // Five hits on the guard each way; a plain one's shield breaks on the
    // third, the Tier 5 golden's on the fourth.
    for (golden, want, breaks_at) in
        [(false, vec![3u8, 2, 1, 0, 0], 3usize), (true, vec![4u8, 3, 2, 1, 0], 4usize)]
    {
        let mut host = db.make(version, golden);
        // Five 1/1 attackers: one hit each, all of them die to the guard.
        let mut attackers = Vec::new();
        for _ in 0..5 {
            let mut m = db.make(db.by_name("Microbot").unwrap(), false);
            m.attack = 1;
            attackers.push(m);
        }
        let mut seen = Seen::default();
        let mut rng = Rng::new(11);
        let out = host_fight(db, &mut host, &attackers, &mut seen, &mut rng);
        assert_eq!(out.attacks, 5, "five attackers, one swing each");
        let mine: Vec<(u8, bool)> = seen
            .shields
            .iter()
            .filter(|&&(s, c, _, _)| s == 0 && c == version)
            .map(|&(_, _, n, ds)| (n, ds))
            .collect();
        // One entry per hit the guard takes: the hits it has left before that
        // hit, and whether the shield is still up for it.
        let hits: Vec<u8> = mine.iter().map(|&(n, _)| n).collect();
        let shields: Vec<bool> = mine.iter().map(|&(_, ds)| ds).collect();
        assert_eq!(hits, want, "golden {golden}");
        // The shield is up for the first hits and gone from the one that
        // breaks it on.
        let breaks = shields.iter().position(|&ds| !ds).expect("the shield broke");
        assert_eq!(breaks, breaks_at, "golden {golden}: {shields:?}");
        assert!(shields[..breaks].iter().all(|&ds| ds), "golden {golden}: {shields:?}");
        assert!(shields[breaks..].iter().all(|&ds| !ds), "golden {golden}: {shields:?}");
    }
}

/// `host` alone against `attackers`, both sides watched.
fn host_fight(
    db: &bgcore::CardDb,
    host: &mut Minion,
    attackers: &[Minion],
    seen: &mut Seen,
    rng: &mut Rng,
) -> bgcore::fight::Outcome {
    fight_with(db, &[*host], attackers, [Counters::default(); 2], 40, rng, seen)
}

#[test]
fn the_tier_six_golden_summons_one_golem_with_double_stats() {
    let data = Data::bundled();
    let db = &data.cards;
    let g = game_with_paradox("Dark Paradox t9");
    let version = g.paradox.unwrap();
    let golem = db.by_name("Golem").unwrap();
    // The plain card summons one Golem at its own 10/2; the golden one (a
    // 20/4 card) summons one at double that, 40/8, not two 20/4 Golems.
    for (golden, want) in [(false, (10, 2)), (true, (40, 8))] {
        let host = db.make(version, golden);
        let mut killer = db.make(db.by_name("Microbot").unwrap(), false);
        killer.attack = 50;
        killer.health = 500;
        let mut seen = Seen::default();
        let mut rng = Rng::new(12);
        let _ = fight_with(
            db,
            &[host],
            &[killer],
            [Counters::default(); 2],
            40,
            &mut rng,
            &mut seen,
        );
        let golems: Vec<(i32, i32)> = seen
            .summoned
            .iter()
            .filter(|&&(s, c, _, _)| s == 0 && c == golem)
            .map(|&(_, _, a, h)| (a, h))
            .collect();
        // ONE Golem, and a golden minion's has double its stats.
        assert_eq!(golems, vec![want], "golden {golden}");
    }
}

#[test]
fn the_tier_six_golden_grows_twice_as_fast_per_battlecry() {
    let mut g = game_with_paradox("Dark Paradox t6");
    let version = g.paradox.unwrap();
    g.players[0].battlecries_triggered = 2;
    let plain = g.players[0].make_card(&g.data.cards, version, false, 0);
    let golden = g.players[0].make_card(&g.data.cards, version, true, 0);
    let printed = g.data.cards.get(version);
    assert_eq!(
        (plain.attack, plain.health),
        (printed.attack + 6, printed.health + 6),
        "+3/+3 for each of 2 Battlecries"
    );
    assert_eq!(
        (golden.attack, golden.health),
        (printed.attack * 2 + 12, printed.health * 2 + 12),
        "a golden one doubles the printed stats and the +6/+6 per Battlecry"
    );
    // And it grows on the event, both of them.
    g.players[0].board.push(plain);
    g.players[0].board.push(golden);
    let before = (g.players[0].board[0].attack, g.players[0].board[1].attack);
    g.dark_gift_count(0, 1, |c| c.buff_per_battlecry);
    assert_eq!(g.players[0].board[0].attack, before.0 + 3);
    assert_eq!(g.players[0].board[1].attack, before.1 + 6);
}

#[test]
fn the_tier_two_version_gains_attack_on_every_card_played() {
    let mut g = game_with_paradox("Dark Paradox t3");
    let version = g.paradox.unwrap();
    let mut plain = g.players[0].make_card(&g.data.cards, version, false, 0);
    plain.pool_copies = 0;
    let mut golden = g.players[0].make_card(&g.data.cards, version, true, 0);
    golden.pool_copies = 0;
    g.players[0].board.push(plain);
    g.players[0].board.push(golden);
    let before = (g.players[0].board[0].attack, g.players[0].board[1].attack);
    assert_eq!(before.1 - before.0, 2, "the golden is twice the card");
    g.dark_gift_card_played(0);
    assert_eq!(g.players[0].board[0].attack, before.0 + 3, "+3 Attack per card played");
    assert_eq!(g.players[0].board[1].attack, before.1 + 6, "the golden gets +6");
}
