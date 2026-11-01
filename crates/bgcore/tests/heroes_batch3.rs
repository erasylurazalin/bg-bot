//! Hero batch 3 (docs/briefs/heroes-batch3.md): the 24 fight, kill and rest
//! powers, one test each. Era's rules for their non-obvious parts are in
//! docs/heroes.md "Hero batch rules (Era, 2026-11-01)" (the "Start of Combat
//! and fights", "Kills and counters" and "Other" lists) and DESIGN.md
//! "Order of events".

use std::sync::Arc;

use bgcore::fight::{Counters, Watch, fight_with, fight_with_hands};
use bgcore::{
    Action, CardId, CardDb, Data, Game, Keywords, Minion, Opponent, Rng, Tribe, Tribes,
};

// ---- shared helpers (the same ones heroes_batch2.rs uses) ----

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn cards() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> Tribes {
    ts.iter().copied().fold(Tribes::NONE, |m, t| m.with(t))
}

/// A no-pick game with every tribe in the lobby, player 0 forced to `hero`.
/// Every other player has no hero, so only player 0's power runs.
fn hero_game(seed: u64, hero: &str) -> Game {
    let d = data();
    let mut g = Game::new_with_lobby(d.clone(), seed, lobby(&Tribe::EVERY));
    let h = d.heroes.by_name(hero).expect(hero);
    g.assign_hero(0, h);
    g
}

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).expect(name)
}

fn power(g: &Game, p: usize) -> &bgcore::HeroPower {
    g.data.heroes.power(g.players[p].power.expect("no power").of)
}

fn power_name(g: &Game, p: usize) -> String {
    power(g, p).name.clone()
}

/// Answer whatever the current player has open (a Discover, a power offer, a
/// wager, a guess, a target, a discard, a trinket offer), so a helper that
/// only wants the next turn never stalls on a prompt. False when nothing is
/// open.
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

fn choose(g: &mut Game, p: usize, k: u8) {
    turn_of(g, p);
    g.step(Action::Choose(k)).unwrap_or_else(|_| panic!("Choose({k}) illegal for player {p}"));
}

fn act(g: &mut Game, p: usize, a: Action) {
    turn_of(g, p);
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

/// Take a card into the player's hand, taking a pool copy when the card has
/// one (a spell or a token has none), pool kept exact.
fn put_in_hand(g: &mut Game, p: usize, name: &str) -> Minion {
    let c = id(g, name);
    let card = g.data.cards.get(c);
    let copies = u8::from(!card.is_spell() && !card.token);
    if copies > 0 {
        g.pool[card.pool_slot as usize] -= 1;
    }
    let m = g.players[p].make_card(&g.data.cards, c, false, copies);
    g.players[p].hand.push(m);
    m
}

fn legal_for_0(g: &mut Game, a: Action) -> bool {
    turn_of(g, 0);
    g.is_legal(a)
}

/// Give the power its uses back, so a test can press it more than once in a
/// turn without walking whole rounds.
fn again(g: &mut Game, p: usize) {
    if let Some(st) = g.players[p].power.as_mut() {
        st.uses_left = g.data.heroes.power(st.of).uses_per_turn;
    }
}

/// Press the power and answer a target it opened, if any (with one legal
/// target the press applies it by itself).
fn press(g: &mut Game, p: usize) {
    act(g, p, Action::HeroPower);
    while g.players[p].pending_target.is_some() || g.players[p].pending_shop_target.is_some() {
        turn_of(g, p);
        if !answer(g) {
            break;
        }
    }
}

fn put_on_board(g: &mut Game, p: usize, name: &str) -> Minion {
    let m = put_in_hand(g, p, name);
    g.players[p].hand.pop();
    g.players[p].board.push(m);
    m
}

/// Every board minion off, pool copies back (a plain `clear()` would break
/// the exact pool accounting the tests check).
fn clear_board(g: &mut Game, p: usize) {
    for m in g.players[p].board.drain(..) {
        let c = g.data.cards.get(m.card);
        if m.pool_copies > 0 && !c.token && !c.is_spell() {
            g.pool[c.pool_slot as usize] += m.pool_copies;
        }
    }
}

/// Set the player's board to exactly these minions, pool kept exact.
fn set_board(g: &mut Game, p: usize, names: &[&str]) {
    clear_board(g, p);
    for n in names {
        put_on_board(g, p, n);
    }
}

/// Set the player's shop to exactly these minions, pool kept exact.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for n in names {
        let c = id(g, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
    g.players[p].shop_spell = None;
}

/// Leave only players 0 and 1 alive and paired with each other, so the next
/// fight is theirs whatever the round's pairing would have been.
fn duel(g: &mut Game) {
    for p in 2..8 {
        g.players[p].alive = false;
        g.players[p].done = true;
        g.players[p].placement = 3 + (p as u8 - 2);
    }
    g.players[1].health = 30;
    g.players[1].armor = 0;
    g.players[0].opponent = Some(Opponent::Player(1));
    g.players[1].opponent = Some(Opponent::Player(0));
}

fn hand_named(g: &Game, p: usize, name: &str) -> usize {
    let c = id(g, name);
    g.players[p].hand.iter().filter(|m| m.card == c).count()
}

// ---- fight helpers ----

fn board(db: &CardDb, names: &[&str]) -> Vec<Minion> {
    names.iter().map(|n| db.make(db.by_name(n).expect(n), false)).collect()
}

/// What one fight did, for the tests that watch the order of things.
#[derive(Default)]
struct Log {
    db: Option<CardDb>,
    /// (side, attacker name, attack, health, defender name), at the swing.
    attacks: Vec<(usize, String, i32, i32, String)>,
    deaths: Vec<(usize, String)>,
    /// (side, name, attack, health, taunt, divine shield), at the summon.
    summons: Vec<(usize, String, i32, i32, bool, bool)>,
}

impl Log {
    fn new(db: &CardDb) -> Log {
        Log { db: Some(db.clone()), ..Default::default() }
    }

    fn summoned(&self, name: &str) -> usize {
        self.summons.iter().filter(|s| s.1 == name).count()
    }
}

impl Watch for Log {
    fn attack(&mut self, side: usize, a: &Minion, d: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.attacks.push((
            side,
            db.get(a.card).name.clone(),
            a.attack,
            a.health,
            db.get(d.card).name.clone(),
        ));
    }
    fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.deaths.push((side, self.db.as_ref().unwrap().get(m.card).name.clone()));
    }
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.summons.push((
            side,
            self.db.as_ref().unwrap().get(m.card).name.clone(),
            m.attack,
            m.health,
            m.keywords.has(Keywords::TAUNT),
            m.keywords.has(Keywords::DIVINE_SHIELD),
        ));
    }
}

// ---- 1. Illidan Stormrage: Wingmen ----

#[test]
fn wingmen_attacks_first_and_a_start_of_combat_minion_it_kills_never_fires() {
    let db = cards();
    // Boom-in-a-Box (5 Attack, Taunt) deals 3 damage to every other minion at
    // Start of Combat. Wingmen's two minions hit for 7 (5 + the +2) each,
    // left then right, and kill it before its own Start of Combat.
    let mut a = board(&db, &["Microbot", "Blue Whelp"]);
    for m in &mut a {
        m.attack = 5;
        m.health = 20;
    }
    let b = board(&db, &["Boom-in-a-Box"]);
    let mut counters = [Counters::default(); 2];
    counters[0].wingmen = 1;
    let mut log = Log::new(&db);
    let out = fight_with(&db, &a, &b, counters, 500, &mut Rng::new(4), &mut log);
    assert_eq!(out.winner, Some(0), "Boom-in-a-Box died");
    assert_eq!(
        log.attacks,
        vec![
            (0, "Microbot".into(), 7, 21, "Boom-in-a-Box".into()),
            (0, "Blue Whelp".into(), 7, 21, "Boom-in-a-Box".into()),
        ],
        "left-most then right-most, both +2/+1, before the fight itself"
    );
    // Boom's 3 damage never landed (the second attacker still has 21 Health,
    // and its Deathrattle-free card left nothing behind.
    assert_eq!(log.summons.len(), 0);
    assert_eq!(log.deaths, vec![(1, "Boom-in-a-Box".into())]);
    assert_eq!(out.sides[0].friendly_attacks, 2);

    // A lone minion is both the left- and the right-most: one +2/+1 and one
    // attack, not two of each.
    let mut solo = board(&db, &["Microbot"]);
    solo[0].attack = 1;
    solo[0].health = 30;
    let mut fat = board(&db, &["Microbot"]);
    fat[0].attack = 0;
    fat[0].health = 200;
    let mut log = Log::new(&db);
    fight_with(&db, &solo, &fat, counters, 2, &mut Rng::new(5), &mut log);
    assert_eq!(log.attacks[0].2, 3, "1 + 2, once");
    assert_eq!(log.attacks[0].3, 31, "30 + 1, once");
}

#[test]
fn wingmens_buff_is_for_the_fight_only() {
    let mut g = hero_game(1, "Illidan Stormrage");
    duel(&mut g);
    put_on_board(&mut g, 0, "Microbot");
    g.players[0].board[0].attack = 4;
    g.players[0].board[0].health = 40;
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 0;
    g.players[1].board[0].health = 200;
    end_all_turns(&mut g);
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (4, 40),
        "the +2/+1 is for the fight"
    );
    g.check_invariants().unwrap();
}

// ---- 2. Queen Wagtoggle: Wax Warband ----

#[test]
fn wax_warband_buffs_one_minion_of_each_type_and_grows_with_gold_spent() {
    let db = cards();
    let mut a = board(&db, &["Buzzing Vermin", "Microbot", "Wrath Weaver"]);
    for m in &mut a {
        m.attack = 4;
        m.health = 30;
    }
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 400;
    let mut counters = [Counters::default(); 2];
    counters[0].wax_warband = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 3, &mut Rng::new(6), &mut log);
    let attacks: Vec<(i32, i32)> =
        log.attacks.iter().filter(|x| x.0 == 0).map(|x| (x.2, x.3)).collect();
    assert_eq!(
        attacks,
        vec![(5, 31), (5, 31), (5, 31)],
        "a Beast, a Mech and a Demon: +1/+1 each, once"
    );

    // 10 Gold spent: +2/+2.
    counters[0].gold_spent = 10;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 3, &mut Rng::new(7), &mut log);
    let attacks: Vec<i32> = log.attacks.iter().filter(|x| x.0 == 0).map(|x| x.2).collect();
    assert_eq!(attacks, vec![6, 6, 6], "+2/+2 at 10 Gold spent");

    // Two Beasts: one of them is buffed, not both.
    let mut a = board(&db, &["Buzzing Vermin", "Buzzing Vermin"]);
    for m in &mut a {
        m.attack = 4;
        m.health = 30;
    }
    counters[0].gold_spent = 0;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 2, &mut Rng::new(8), &mut log);
    let mut attacks: Vec<i32> = log.attacks.iter().filter(|x| x.0 == 0).map(|x| x.2).collect();
    attacks.sort();
    assert_eq!(attacks, vec![4, 5], "one minion of the type, not both");
}

// ---- 3. The Curator: Menagerist ----

#[test]
fn menagerist_starts_with_a_two_two_amalgam_of_every_type_and_venomous() {
    let mut g = hero_game(3, "The Curator");
    assert_eq!(g.players[0].board.len(), 1, "one minion at the start");
    let m = &g.players[0].board[0];
    assert_eq!(g.data.cards.get(m.card).name, "Amalgam");
    assert_eq!((m.attack, m.health), (2, 2));
    assert!(m.keywords.has(Keywords::VENOMOUS));
    assert_eq!(g.data.cards.tribes_of(m), Tribes::ALL, "every type");
    // It is a normal minion: it can be sold.
    assert!(legal_for_0(&mut g, Action::Sell(0)), "the Amalgam can be sold");
    act(&mut g, 0, Action::Sell(0));
    assert!(g.players[0].board.is_empty());
    g.check_invariants().unwrap();
}

// ---- 4. Lord Jaraxxus: Bloodfury ----

#[test]
fn bloodfury_becomes_nether_portal_at_150_damage_and_gives_two_demons_a_turn() {
    let mut g = hero_game(4, "Lord Jaraxxus");
    assert_eq!(power_name(&g, 0), "Bloodfury");
    assert!(power(&g, 0).passive);
    duel(&mut g);
    put_on_board(&mut g, 0, "Microbot");
    g.players[0].board[0].attack = 1;
    g.players[0].board[0].health = 100;
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 0;
    g.players[1].board[0].health = 200;
    // One point of damage from a friendly minion opens the portal.
    g.players[0].power.as_mut().unwrap().progress = 149;
    end_all_turns(&mut g);
    assert_eq!(power_name(&g, 0), "Nether Portal", "149 + a hit");
    assert_eq!(g.players[0].hand.len(), 2, "two Demons, at the next turn");
    end_all_turns(&mut g);
    assert_eq!(g.players[0].hand.len(), 4, "two more at every turn");
    for m in &g.players[0].hand {
        assert!(
            g.data.cards.tribes_of(m).has(Tribe::Demon),
            "{} is no Demon",
            g.data.cards.get(m.card).name
        );
    }
    g.check_invariants().unwrap();
}

// ---- 5. Arch-Villain Rafaam: I'll Take That! ----

#[test]
fn ill_take_that_gives_a_plain_copy_of_the_first_minion_killed() {
    let mut g = hero_game(5, "Arch-Villain Rafaam");
    duel(&mut g);
    assert_eq!(power(&g, 0).cost, 1);
    g.players[0].gold = 5;
    put_on_board(&mut g, 0, "Microbot");
    g.players[0].board[0].attack = 20;
    g.players[0].board[0].health = 200;
    set_board(&mut g, 1, &["Wrath Weaver", "Microbot"]);
    g.players[1].board[0].health = 5;
    g.players[1].board[0].attack = 0;
    g.players[1].board[1].health = 500;
    g.players[1].board[1].attack = 0;
    press(&mut g, 0);
    assert!(g.players[0].power.unwrap().armed, "the press arms it");
    end_all_turns(&mut g);
    // A plain copy of the first minion killed: a fresh card, no pool copy.
    let c = id(&g, "Wrath Weaver");
    let copied: Vec<Minion> = g.players[0].hand.iter().copied().filter(|m| m.card == c).collect();
    assert_eq!(copied.len(), 1, "a copy of the first kill");
    assert_eq!((copied[0].attack, copied[0].health), (1, 3), "plain stats");
    assert_eq!(copied[0].pool_copies, 0, "a copy made fresh takes no pool copy");
    // Only the first: no copy of the Microbot that died after it.
    assert_eq!(hand_named(&g, 0, "Microbot"), 0);
    g.check_invariants().unwrap();
}

// ---- 6. Deathwing: ALL Will Burn! ----

#[test]
fn all_will_burn_gives_every_minion_on_both_sides_two_attack_for_good() {
    let db = cards();
    let mut a = board(&db, &["Microbot"]);
    a[0].attack = 1;
    a[0].health = 20;
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 1;
    b[0].health = 20;
    let mut counters = [Counters::default(); 2];
    counters[0].all_will_burn = 1;
    let mut log = Log::new(&db);
    let out = fight_with(&db, &a, &b, counters, 60, &mut Rng::new(9), &mut log);
    // Both sides' first attackers have 3 Attack: every minion got +2.
    assert_eq!(log.attacks[0].2, 3);
    assert_eq!(log.attacks[1].2, 3);
    // Ours keeps it: the gain is permanent, by board slot.
    assert_eq!(out.sides[0].permanent_stats[0].0, 2);

    // In a game the board minion keeps the Attack after the fight.
    let mut g = hero_game(6, "Deathwing");
    duel(&mut g);
    put_on_board(&mut g, 0, "Microbot");
    g.players[0].board[0].attack = 4;
    g.players[0].board[0].health = 40;
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 0;
    g.players[1].board[0].health = 200;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].board[0].attack, 6, "4 + 2, permanently");
    g.check_invariants().unwrap();
}

// ---- 7. Aranna Starseeker: Demon Hunter Training ----

#[test]
fn demon_hunter_training_becomes_spectral_sight_after_fourteen_attacks() {
    let mut g = hero_game(7, "Aranna Starseeker");
    assert_eq!(power_name(&g, 0), "Demon Hunter Training");
    duel(&mut g);
    put_on_board(&mut g, 0, "Microbot");
    g.players[0].board[0].attack = 1;
    g.players[0].board[0].health = 400;
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 0;
    g.players[1].board[0].health = 400;
    // 13 attacks so far: the one this fight lands is the 14th.
    g.players[0].power.as_mut().unwrap().progress = 13;
    end_all_turns(&mut g);
    assert_eq!(power_name(&g, 0), "Spectral Sight", "once it triggers it stays");
    assert_eq!(g.players[0].free_minion_left, 1, "the first minion each turn is free");
    // Buying a minion costs nothing.
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.players[0].gold = 7;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 7, "a free minion");
    assert_eq!(g.players[0].free_minion_left, 0, "one a turn");
    // The next minion is paid for again.
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    act(&mut g, 0, Action::Buy(0));
    assert!(g.players[0].gold < 7, "the free buy is used up");
    g.check_invariants().unwrap();
}

// ---- 8. Lord Barov: Friendly Wager ----

#[test]
fn friendly_wager_pays_three_coins_for_a_win_and_one_gold_for_a_tie() {
    let mut g = hero_game(8, "Lord Barov");
    assert_eq!(power(&g, 0).cost, 1);
    // Four players alive, so the wager is about someone else's fight: the
    // pair is read from the offer the press opens.
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    g.players[0].gold = 5;
    press(&mut g, 0);
    let (first, second) = g.players[0].wager_offer.expect("a wager offer");
    assert_ne!(first as usize, 0, "never your own fight with four alive");
    assert_ne!(second as usize, 0);
    // Make the first of the two win and the other lose.
    set_board(&mut g, first as usize, &["Microbot"]);
    g.players[first as usize].board[0].attack = 30;
    g.players[first as usize].board[0].health = 200;
    set_board(&mut g, second as usize, &[]);
    choose(&mut g, 0, 0);
    assert_eq!(g.players[0].wagers.len(), 1);
    let before = hand_named(&g, 0, "Tavern Coin");
    end_all_turns(&mut g);
    assert_eq!(hand_named(&g, 0, "Tavern Coin"), before + 3, "3 Tavern Coins");

    // A tie pays 1 Gold next turn.
    turn_of(&mut g, 0);
    g.players[0].gold = 5;
    press(&mut g, 0);
    let (first, second) = g.players[0].wager_offer.expect("a wager offer");
    set_board(&mut g, first as usize, &[]);
    set_board(&mut g, second as usize, &[]);
    choose(&mut g, 0, 1);
    end_all_turns(&mut g);
    let turn = g.turn;
    assert_eq!(
        g.players[0].gold,
        g.data.rules.gold(turn) + 1,
        "a tied fight gives 1 Gold"
    );
    g.check_invariants().unwrap();
}

// ---- 9. Y'Shaarj: Embrace Your Rage ----

#[test]
fn embrace_your_rage_summons_a_minion_of_your_tier_and_adds_it_to_hand() {
    let mut g = hero_game(9, "Y'Shaarj");
    duel(&mut g);
    assert_eq!(power(&g, 0).cost, 2);
    g.players[0].gold = 5;
    let tier = g.players[0].tier;
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 0;
    g.players[1].board[0].health = 400;
    press(&mut g, 0);
    assert_eq!(g.players[0].gold, 3, "2 Gold");
    end_all_turns(&mut g);
    assert_eq!(g.players[0].hand.len(), 1, "the card went to hand");
    let got = g.players[0].hand[0];
    assert_eq!(g.data.cards.get(got.card).tier, tier, "a minion of your Tier");
    assert_eq!(got.pool_copies, 0, "a copy made fresh takes no pool copy");

    // A full board still gets the card and summons nothing new.
    clear_board(&mut g, 0);
    set_board(&mut g, 0, &["Microbot", "Microbot", "Microbot", "Microbot", "Microbot",
                            "Microbot", "Microbot"]);
    for m in &mut g.players[0].board {
        m.attack = 0;
        m.health = 400;
    }
    g.players[0].hand.clear();
    again(&mut g, 0);
    g.players[0].gold = 5;
    press(&mut g, 0);
    end_all_turns(&mut g);
    assert_eq!(g.players[0].hand.len(), 1, "the card goes to hand even if the board was full");
    assert_eq!(g.players[0].board.len(), 7);
    g.check_invariants().unwrap();
}

// ---- 10. N'Zoth: Avatar of N'Zoth ----

#[test]
fn avatar_of_nzoth_starts_with_a_fish_that_gains_your_deathrattles() {
    let g = hero_game(10, "N'Zoth");
    assert_eq!(g.players[0].board.len(), 1);
    let fish = g.players[0].board[0];
    assert_eq!(g.data.cards.get(fish.card).name, "Fish of N'Zoth");
    assert_eq!((fish.attack, fish.health), (2, 2));
    assert!(g.data.cards.get(fish.card).gains_friendly_deathrattles);
    g.check_invariants().unwrap();

    // In a fight it takes a friendly Deathrattle and fires it when it dies: a
    // Buzzing Vermin dies at Start of Combat, the Fish dies later and
    // summons a Beetle of its own.
    let db = cards();
    let mut a = board(&db, &["Buzzing Vermin", "Fish of N'Zoth"]);
    a[0].attack = 0;
    a[1].attack = 0;
    a[1].health = 40;
    let mut b = board(&db, &["Boom-in-a-Box"]);
    b[0].health = 500;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, [Counters::default(); 2], 500, &mut Rng::new(11), &mut log);
    let beetles = log.summons.iter().filter(|s| s.1 == "Beetle").count();
    assert_eq!(beetles, 2, "Buzzing Vermin's own Beetle and the Fish's copy of it");
}

// ---- 11. Greybough: Sprout It Out! ----

#[test]
fn sprout_it_out_gives_your_combat_summons_one_two_and_taunt() {
    let db = cards();
    // A Buzzing Vermin's Beetle is summoned in the fight: 2/2 base +1/+2, and
    // Taunt.
    let mut a = board(&db, &["Buzzing Vermin"]);
    a[0].attack = 0;
    a[0].health = 1;
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 5;
    b[0].health = 100;
    let mut counters = [Counters::default(); 2];
    counters[0].sprout_it_out = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 10, &mut Rng::new(12), &mut log);
    assert_eq!(
        log.summons.first().cloned(),
        Some((0, "Beetle".to_string(), 3, 4, true, false)),
        "a summoned Beetle gets +1/+2 and Taunt"
    );
    // The enemy's summons get nothing.
    let mut a2 = board(&db, &["Microbot"]);
    a2[0].attack = 5;
    a2[0].health = 100;
    let mut b2 = board(&db, &["Buzzing Vermin"]);
    b2[0].attack = 0;
    b2[0].health = 1;
    let mut counters = [Counters::default(); 2];
    counters[0].sprout_it_out = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a2, &b2, counters, 10, &mut Rng::new(13), &mut log);
    let theirs = log.summons.iter().find(|s| s.1 == "Beetle").cloned();
    assert_eq!(theirs, Some((1, "Beetle".to_string(), 2, 2, false, false)));
}

// ---- 12. Rokara: Glory of Combat ----

#[test]
fn glory_of_combat_gives_the_attacker_that_kills_one_attack_for_good() {
    let db = cards();
    let mut a = board(&db, &["Microbot"]);
    a[0].attack = 5;
    a[0].health = 50;
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 1;
    let mut counters = [Counters::default(); 2];
    counters[0].glory_of_combat = 1;
    let out = fight_with(&db, &a, &b, counters, 10, &mut Rng::new(14), &mut ());
    assert_eq!(out.winner, Some(0));
    assert_eq!(out.sides[0].permanent_stats[0].0, 1, "+1 Attack, kept");

    // In a game the board minion keeps the Attack after the fight.
    let mut g = hero_game(12, "Rokara");
    duel(&mut g);
    put_on_board(&mut g, 0, "Microbot");
    g.players[0].board[0].attack = 10;
    g.players[0].board[0].health = 100;
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 0;
    g.players[1].board[0].health = 1;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].board[0].attack, 11, "10 + 1 for the kill");
    g.check_invariants().unwrap();
}

// ---- 13. Master Nguyen: Power of the Storm ----

#[test]
fn power_of_the_storm_offers_two_new_powers_every_turn() {
    let mut g = hero_game(13, "Master Nguyen");
    assert_eq!(power_name(&g, 0), "Power of the Storm");
    assert_eq!(g.players[0].power_offer.len(), 2, "two powers, not three");
    // A fresh power each time, never one that makes no sense as a found power
    // (the hero power Discover exclusions) and never Power of the Storm.
    let excludes = g.data.cards.hero_power_excludes.clone();
    check_storm_offer(&g, &excludes);
    let picked = g.players[0].power_offer[1];
    choose(&mut g, 0, 1);
    assert_eq!(g.players[0].power.expect("a power").of, picked);
    assert!(g.players[0].storm_power[0], "still Power of the Storm's slot");
    // The observation carries the offer.
    let mut obs = vec![0.0f32; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut obs);
    assert_eq!(obs[bgcore::obs::SCALAR_PENDING_POWER], 0.0, "answered");
    // Next turn: two more.
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].power_offer.len(), 2);
    check_storm_offer(&g, &excludes);
    let mut obs = vec![0.0f32; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut obs);
    assert_eq!(obs[bgcore::obs::SCALAR_PENDING_POWER], 1.0);
    assert_eq!(
        obs[bgcore::obs::SCALAR_POWER_OFFER],
        g.players[0].power_offer[0] as f32 + 1.0,
        "the offered powers are in the observation"
    );
    g.check_invariants().unwrap();
}

fn check_storm_offer(g: &Game, excludes: &[String]) {
    for &of in &g.players[0].power_offer {
        let p = g.data.heroes.power(of);
        assert_ne!(p.name, "Power of the Storm", "never offered back");
        let hero = g.data.heroes.heroes.iter().find(|h| h.power == of).expect("a hero");
        assert!(
            !excludes.contains(&hero.card_id),
            "{} is on the exclusion list",
            hero.name
        );
        for slot in [false, true] {
            if let Some(st) = g.players[0].power_at(slot) {
                assert_ne!(st.of, of, "never one you already hold");
            }
        }
    }
}

// ---- 14. Cariel Roame: Conviction ----

#[test]
fn conviction_buffs_a_random_minion_and_each_fight_offers_a_stance() {
    let mut g = hero_game(14, "Cariel Roame");
    assert_eq!(power(&g, 0).cost, 0);
    assert_eq!(g.players[0].conviction, (1, 1, 1), "one minion, +1/+1");
    put_on_board(&mut g, 0, "Microbot");
    press(&mut g, 0);
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (2, 2),
        "one random friendly minion +1/+1"
    );
    // After the first fight: three stances.
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].power_offer.len(), 3, "three stances");
    let names: Vec<String> =
        g.players[0].power_offer.iter().map(|&o| g.data.heroes.power(o).name.clone()).collect();
    assert!(names.contains(&"United Stance".to_string()), "{names:?}");
    let united = names.iter().position(|n| n == "United Stance").unwrap() as u8;
    choose(&mut g, 0, united);
    assert_eq!(g.players[0].conviction, (2, 1, 1), "one more minion");
    // Two minions get +1/+1 now.
    put_on_board(&mut g, 0, "Microbot");
    again(&mut g, 0);
    press(&mut g, 0);
    let stats: Vec<(i32, i32)> = g.players[0]
        .board
        .iter()
        .map(|m| (m.attack, m.health))
        .collect();
    assert_eq!(stats, vec![(3, 3), (2, 2)], "+1/+1 to two minions");
    // United stops at 7: with the cap reached only the other two are offered.
    g.players[0].conviction.0 = 7;
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    let names: Vec<String> =
        g.players[0].power_offer.iter().map(|&o| g.data.heroes.power(o).name.clone()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(!names.contains(&"United Stance".to_string()), "{names:?}");
    let off = names.iter().position(|n| n == "Offensive Stance").unwrap() as u8;
    choose(&mut g, 0, off);
    assert_eq!(g.players[0].conviction, (7, 2, 1), "+1 Attack a cast");
    g.check_invariants().unwrap();
}

// ---- 15. Tamsin Roame: Fragrant Phylactery ----

#[test]
fn fragrant_phylactery_gives_the_lowest_attack_minion_a_stats_deathrattle() {
    let db = cards();
    // The lowest-Attack minion dies at Start of Combat (Boom-in-a-Box's 3
    // damage) and gives its Attack and maximum Health to the other one.
    let mut a = board(&db, &["Microbot", "Blue Whelp"]);
    a[0].attack = 3;
    a[0].health = 3;
    a[1].attack = 5;
    a[1].health = 20;
    let mut b = board(&db, &["Boom-in-a-Box"]);
    b[0].attack = 0;
    b[0].health = 500;
    let mut counters = [Counters::default(); 2];
    counters[0].phylactery = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 60, &mut Rng::new(15), &mut log);
    assert_eq!(
        log.attacks.first().cloned(),
        Some((0, "Blue Whelp".to_string(), 8, 20, "Boom-in-a-Box".to_string())),
        "+3 Attack and +3 Health (its maximum Health), less Boom's 3"
    );
    // Triggered twice (Valdrakken Wind Chimes and the like): twice the stats.
    let mut a = board(&db, &["Microbot", "Blue Whelp"]);
    a[0].attack = 3;
    a[0].health = 3;
    a[1].attack = 5;
    a[1].health = 20;
    let mut counters = [Counters::default(); 2];
    counters[0].phylactery = 2;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 60, &mut Rng::new(16), &mut log);
    assert_eq!(
        log.attacks.first().cloned(),
        Some((0, "Blue Whelp".to_string(), 11, 23, "Boom-in-a-Box".to_string())),
        "the Deathrattle twice"
    );
}

// ---- 16. Sneed: Pilot the Shredder ----

#[test]
fn pilot_the_shredder_starts_with_sneeds_new_shredder() {
    let g = hero_game(16, "Sneed");
    assert_eq!(g.players[0].board.len(), 1);
    let m = g.players[0].board[0];
    assert_eq!(g.data.cards.get(m.card).name, "Sneed's New Shredder");
    assert_eq!((m.attack, m.health), (2, 1));

    // Its Deathrattle summons the highest-Health minion from your hand and
    // gives it Divine Shield for the combat.
    let db = cards();
    let shredder = board(&db, &["Sneed's New Shredder"]);
    let mut hand = board(&db, &["Microbot", "Blue Whelp"]);
    hand[0].attack = 2;
    hand[0].health = 3;
    hand[1].attack = 4;
    hand[1].health = 9;
    let mut killer = board(&db, &["Microbot"]);
    killer[0].attack = 100;
    killer[0].health = 500;
    let mut log = Log::new(&db);
    fight_with_hands(
        &db,
        &shredder,
        &killer,
        [&hand, &[]],
        [Counters::default(); 2],
        500,
        &mut Rng::new(17),
        &mut log,
    );
    assert_eq!(
        log.summons.first().cloned(),
        Some((0, "Blue Whelp".to_string(), 4, 9, false, true)),
        "the highest-Health minion from hand, with Divine Shield"
    );
}

// ---- 17. Tavish Stormpike: Lock and Load ----

#[test]
fn lock_and_load_stores_a_tavern_minion_that_attacks_at_once_next_combat() {
    let mut g = hero_game(17, "Tavish Stormpike");
    assert_eq!(power(&g, 0).cost, 0, "a 0-cost power");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Wrath Weaver"]);
    g.players[0].shop[0].attack = 7;
    g.players[0].shop[0].health = 9;
    act(&mut g, 0, Action::HeroPower);
    assert!(g.players[0].pending_target.is_some(), "pick a Tavern minion");
    act(&mut g, 0, Action::TargetShop(0));
    let stored = g.players[0].locked_and_loaded.expect("a stored minion");
    assert_eq!(g.data.cards.get(stored.card).name, "Buzzing Vermin");
    assert_eq!((stored.attack, stored.health), (7, 9), "its stats are kept");
    assert_eq!(g.players[0].shop.len(), 1, "it left the Tavern");
    g.check_invariants().unwrap();
    // Next turn it is gone: it fought that one combat only.
    end_all_turns(&mut g);
    assert!(g.players[0].locked_and_loaded.is_none());
    g.check_invariants().unwrap();
}

#[test]
fn lock_and_loads_minion_is_summoned_and_attacks_at_once() {
    let db = cards();
    let card = db.by_name("Buzzing Vermin").unwrap();
    let mut counters = [Counters::default(); 2];
    let keywords = db.make(card, false).keywords;
    counters[0].locked_and_loaded = Some((card, false, 7, 9, keywords, 1));
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 100;
    let mut log = Log::new(&db);
    fight_with(&db, &[], &b, counters, 10, &mut Rng::new(18), &mut log);
    let first = log.summons.first().cloned();
    assert_eq!(
        first,
        Some((0, "Buzzing Vermin".to_string(), 7, 9, true, false)),
        "summoned when there was space"
    );
    assert_eq!(
        log.attacks.first(),
        Some(&(0, "Buzzing Vermin".to_string(), 7, 9, "Microbot".to_string())),
        "and it attacks at once"
    );
}

// ---- 18. Drek'Thar: Frostwolf Fervor ----

#[test]
fn frostwolf_fervor_copies_the_highest_attack_minion_when_there_is_space() {
    let db = cards();
    let mut a = board(&db, &["Microbot", "Blue Whelp"]);
    a[0].attack = 9;
    a[0].health = 12;
    a[0].keywords.set(Keywords::TAUNT);
    a[1].attack = 2;
    a[1].health = 30;
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 400;
    let mut counters = [Counters::default(); 2];
    counters[0].frostwolf_fervor = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 60, &mut Rng::new(19), &mut log);
    assert_eq!(
        log.summons.first().cloned(),
        Some((0, "Microbot".to_string(), 9, 12, true, false)),
        "an exact copy, keywords included"
    );
    assert_eq!(log.summons.len(), 1, "once per fight");

    // Unlocks on turn 7.
    let mut g = hero_game(18, "Drek'Thar");
    let read = |g: &Game| g.players[0].counters(&g.data, g.lobby).frostwolf_fervor;
    while g.turn < 6 {
        end_all_turns(&mut g);
    }
    assert_eq!(g.turn, 6);
    assert_eq!(read(&g), 0, "locked before turn 7");
    end_all_turns(&mut g);
    assert_eq!(g.turn, 7);
    assert_eq!(read(&g), 1, "unlocked on turn 7");
    g.check_invariants().unwrap();
}

// ---- 19. Vanndar Stormpike: Stormpike Strength ----

#[test]
fn stormpike_strength_copies_the_highest_health_minion_when_there_is_space() {
    let db = cards();
    let mut a = board(&db, &["Microbot", "Blue Whelp"]);
    a[0].attack = 9;
    a[0].health = 12;
    a[1].attack = 2;
    a[1].health = 30;
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 400;
    let mut counters = [Counters::default(); 2];
    counters[0].stormpike_strength = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 60, &mut Rng::new(20), &mut log);
    assert_eq!(
        log.summons.first().cloned(),
        Some((0, "Blue Whelp".to_string(), 2, 30, false, false)),
        "the highest-Health minion, copied exactly"
    );
    // Unlocks on turn 7.
    let mut g = hero_game(19, "Vanndar Stormpike");
    let read = |g: &Game| g.players[0].counters(&g.data, g.lobby).stormpike_strength;
    while g.turn < 6 {
        end_all_turns(&mut g);
    }
    assert_eq!(g.turn, 6);
    assert_eq!(read(&g), 0, "locked before turn 7");
    end_all_turns(&mut g);
    assert_eq!(g.turn, 7);
    assert_eq!(read(&g), 1, "unlocked on turn 7");
    g.check_invariants().unwrap();
}

// ---- 20. Ini Stormcoil: MechGyver ----

#[test]
fn mech_gyver_gives_a_random_mech_after_nine_friendly_deaths() {
    let mut g = hero_game(20, "Ini Stormcoil");
    assert!(power(&g, 0).passive);
    duel(&mut g);
    put_on_board(&mut g, 0, "Microbot");
    g.players[0].board[0].attack = 0;
    g.players[0].board[0].health = 1;
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 100;
    g.players[1].board[0].health = 500;
    // 8 deaths so far: the one this fight has is the ninth.
    g.players[0].power.as_mut().unwrap().progress = 8;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].hand.len(), 1, "a random Mech");
    let got = g.players[0].hand[0];
    assert!(g.data.cards.tribes_of(&got).has(Tribe::Mech));
    assert_eq!(g.players[0].power.unwrap().progress, 0, "the count starts over");
    g.check_invariants().unwrap();
}

// ---- 21. Onyxia: Broodmother ----

#[test]
fn broodmother_summons_a_whelp_that_attacks_and_improves_for_good() {
    let db = cards();
    let whelp = db.by_name("Onyxian Whelp").unwrap();
    let mut a = board(&db, &["Microbot", "Microbot", "Microbot", "Microbot"]);
    for m in &mut a {
        m.attack = 0;
        m.health = 1;
    }
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 100;
    b[0].health = 900;
    let mut counters = [Counters::default(); 2];
    counters[0].broodmother = Some((whelp, 0, 1));
    let mut log = Log::new(&db);
    let out = fight_with(&db, &a, &b, counters, 60, &mut Rng::new(21), &mut log);
    // Avenge (4): the fourth friendly death summons a 1/1 Whelp.
    assert_eq!(
        log.summons.first().cloned(),
        Some((0, "Onyxian Whelp".to_string(), 1, 1, false, false))
    );
    assert_eq!(
        log.attacks.last(),
        Some(&(0, "Onyxian Whelp".to_string(), 1, 1, "Microbot".to_string())),
        "the Whelp attacks at once"
    );
    assert_eq!(out.sides[0].broodmother_gain, 1, "improve by +1/+1, for the game");

    // The improvement carries: the next Whelp is 2/2.
    let mut counters = [Counters::default(); 2];
    counters[0].broodmother = Some((whelp, 1, 1));
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 60, &mut Rng::new(22), &mut log);
    assert_eq!(
        log.summons.first().cloned(),
        Some((0, "Onyxian Whelp".to_string(), 2, 2, false, false))
    );

    // In a game the growth is kept from fight to fight.
    let mut g = hero_game(21, "Onyxia");
    duel(&mut g);
    set_board(&mut g, 0, &["Microbot", "Microbot", "Microbot", "Microbot"]);
    for m in &mut g.players[0].board {
        m.attack = 0;
        m.health = 1;
    }
    set_board(&mut g, 1, &["Microbot"]);
    g.players[1].board[0].attack = 100;
    g.players[1].board[0].health = 900;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].power.unwrap().progress, 1, "the Whelps improve for good");
    g.check_invariants().unwrap();
}

// ---- 22. Murloc Holmes: Detective for Hire ----

#[test]
fn detective_for_hire_offers_two_minions_and_pays_for_a_right_guess() {
    let mut g = hero_game(22, "Murloc Holmes");
    duel(&mut g);
    set_board(&mut g, 1, &["Wrath Weaver", "Microbot"]);
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].opponent, Some(Opponent::Player(1)), "the next opponent");
    g.players[0].gold = 3;
    act(&mut g, 0, Action::HeroPower);
    let (options, correct) = g.players[0].holmes_offer.expect("a guess offer");
    let theirs: Vec<CardId> = g.players[0].next_board.iter().map(|m| m.card).collect();
    assert!(
        theirs.contains(&options[correct as usize]),
        "the right answer was in the next opponent's warband"
    );
    turn_of(&mut g, 0);
    assert!(
        g.is_legal(Action::Choose(correct)),
        "the guess is answered with Choose"
    );
    let coins = hand_named(&g, 0, "Tavern Coin");
    choose(&mut g, 0, correct);
    assert_eq!(hand_named(&g, 0, "Tavern Coin"), coins + 1, "a Tavern Coin");

    // A wrong guess pays nothing.
    again(&mut g, 0);
    act(&mut g, 0, Action::HeroPower);
    let (_, correct) = g.players[0].holmes_offer.expect("a guess offer");
    let coins = hand_named(&g, 0, "Tavern Coin");
    choose(&mut g, 0, 1 - correct);
    assert_eq!(hand_named(&g, 0, "Tavern Coin"), coins, "nothing for a wrong guess");
    g.check_invariants().unwrap();
}

// ---- 23. Ozumat: Tentacular ----

#[test]
fn tentacular_summons_a_taunt_tentacle_that_grows_with_every_sale() {
    let db = cards();
    let tentacle = db.by_name("Ozumat's Tentacle").unwrap();
    let mut a = board(&db, &["Microbot"]);
    a[0].attack = 0;
    a[0].health = 50;
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 50;
    let mut counters = [Counters::default(); 2];
    counters[0].tentacular = Some((tentacle, 1));
    counters[0].minions_sold = 3;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 4, &mut Rng::new(23), &mut log);
    assert_eq!(
        log.summons.first().cloned(),
        Some((0, "Ozumat's Tentacle".to_string(), 5, 5, true, false)),
        "2/2 base +1/+1 per sale, with Taunt"
    );
    assert_eq!(log.summons.len(), 1, "once per fight");

    // In a game the sales count.
    let mut g = hero_game(23, "Ozumat");
    assert!(power(&g, 0).passive);
    g.players[0].counts.minions_sold = 4;
    assert_eq!(g.players[0].counters(&g.data, g.lobby).tentacular, Some((tentacle, 1)));
    g.check_invariants().unwrap();
}

// ---- 24. Teron Gorefiend: Rapid Reanimation ----

#[test]
fn rapid_reanimation_destroys_the_chosen_minion_and_resummons_it_when_there_is_space() {
    let mut g = hero_game(24, "Teron Gorefiend");
    put_on_board(&mut g, 0, "Microbot");
    put_on_board(&mut g, 0, "Blue Whelp");
    g.players[0].gold = 3;
    act(&mut g, 0, Action::HeroPower);
    assert!(g.players[0].pending_target.is_some());
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].board[1].reanimate, "the chosen minion is marked");
    assert!(!g.players[0].board[0].reanimate);
    assert!(g.players[0].power.unwrap().armed, "the power is armed for the fight");
    g.check_invariants().unwrap();

    // The Deathrattle goes first, then the exact copy waits for space.
    let db = cards();
    let mut marked = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
    marked.reanimate = true;
    marked.attack = 4;
    marked.health = 5;
    let mut full = board(
        &db,
        &["Microbot", "Microbot", "Microbot", "Microbot", "Microbot", "Microbot"],
    );
    let mut a = vec![marked];
    a.append(&mut full);
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 300;
    let mut counters = [Counters::default(); 2];
    counters[0].rapid_reanimation = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 2, &mut Rng::new(24), &mut log);
    let first = log.summons.first().cloned();
    assert_eq!(
        first,
        Some((0, "Beetle".to_string(), 2, 2, false, false)),
        "the destroyed minion's Deathrattle fired first"
    );
    assert_eq!(log.summoned("Buzzing Vermin"), 0, "the copy waits: the board is full");

    // With a slot free the exact copy comes back.
    let mut marked = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
    marked.reanimate = true;
    marked.attack = 4;
    marked.health = 5;
    let mut a = vec![marked];
    a.push(db.make(db.by_name("Microbot").unwrap(), false));
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, counters, 2, &mut Rng::new(25), &mut log);
    assert_eq!(
        log.summoned("Buzzing Vermin"),
        1,
        "an exact copy: same stats, buffs included"
    );
    let copy = log.summons.iter().find(|s| s.1 == "Buzzing Vermin").cloned().unwrap();
    assert_eq!((copy.2, copy.3), (4, 5));
}

