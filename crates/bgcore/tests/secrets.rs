//! Secrets and The Great Akazamzarak (docs/rules/heroes.md "Secrets"): the zone,
//! the four combat Secrets as the fight runs them, and Prestidigitation.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with};
use bgcore::secrets::{
    AUTODEFENSE_MATRIX, COMBAT_SECRETS, ICE_BLOCK, PACK_TACTICS, REDEMPTION, VENOMSTRIKE_TRAP,
};
use bgcore::{Action, CardDb, Counters, Data, Game, Keywords, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn mk(db: &CardDb, name: &str) -> Minion {
    db.make(db.by_name(name).expect(name), false)
}

fn name(db: &CardDb, m: &Minion) -> String {
    db.get(m.card).name.clone()
}

/// Counters holding these Secrets, in this order.
fn with_secrets(kinds: &[u8]) -> Counters {
    let mut c = Counters::default();
    for &k in kinds {
        c.secrets.add(k);
    }
    c
}

/// What a fight summoned and who attacked whom.
#[derive(Default)]
struct Log {
    summons: Vec<(usize, usize, Minion)>,
    attacks: Vec<(usize, Minion, Minion)>,
}

impl Watch for Log {
    fn attack(&mut self, side: usize, a: &Minion, d: &Minion) {
        self.attacks.push((side, *a, *d));
    }
    fn summon(&mut self, side: usize, pos: usize, m: &Minion) {
        self.summons.push((side, pos, *m));
    }
}

fn log() -> Log {
    Log::default()
}

// ---- the four Secrets ----

#[test]
fn venomstrike_trap_summons_a_venomous_cobra_when_a_minion_is_attacked() {
    let db = db();
    let mut target = mk(&db, "Microbot");
    target.attack = 0;
    target.health = 20;
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 5;
    attacker.health = 20;
    let c = with_secrets(&[VENOMSTRIKE_TRAP]);
    let mut w = log();
    // Two enemies to one minion: the enemy side attacks first, so the player's
    // minion is the one attacked.
    let out = fight_with(&db, &[target], &[attacker, attacker], [c, Counters::default()], 40,
        &mut Rng::new(3), &mut w);
    let cobra = w.summons.iter().find(|(side, _, _)| *side == 0).map(|&(_, _, m)| m).unwrap();
    assert_eq!(name(&db, &cobra), "Cobra");
    assert_eq!((cobra.attack, cobra.health), (2, 3), "a 2/3 Cobra");
    assert!(cobra.keywords.has(Keywords::VENOMOUS));
    assert!(out.sides[0].secrets_fired & (1 << VENOMSTRIKE_TRAP) != 0, "the Secret fired");
    // It stands to the right of the minion that was attacked.
    let (_, pos, _) = w.summons[0];
    assert_eq!(pos, 1, "summoned right of the attacked minion");
}

#[test]
fn autodefense_matrix_gives_divine_shield_before_the_hit_lands() {
    let db = db();
    let mut target = mk(&db, "Half-Shell"); // 2/3 Taunt
    target.attack = 0;
    target.health = 30;
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 20;
    attacker.health = 30;
    let c = with_secrets(&[AUTODEFENSE_MATRIX]);
    let mut w = log();
    let out = fight_with(&db, &[target], &[attacker, attacker], [c, Counters::default()], 10,
        &mut Rng::new(1), &mut w);
    // The first attack is watched just before its damage, so the defender
    // already carries the shield the Secret gave it.
    let first = w.attacks.iter().find(|(side, _, _)| *side == 1).unwrap();
    assert!(first.2.keywords.has(Keywords::DIVINE_SHIELD), "shielded before the hit");
    // The 20 damage was blocked: the first hit took no Health.
    // (The second attack finds the shield spent.)
    let second = w.attacks.iter().filter(|(side, _, _)| *side == 1).nth(1).unwrap();
    assert!(!second.2.keywords.has(Keywords::DIVINE_SHIELD), "the shield took one hit");
    assert!(out.sides[0].secrets_fired & (1 << AUTODEFENSE_MATRIX) != 0);
}

#[test]
fn pack_tactics_summons_a_three_three_copy_of_the_attacked_minion() {
    let db = db();
    let mut target = mk(&db, "Eternal Knight");
    target.attack = 11;
    target.health = 12;
    target.golden = true;
    target.keywords.set(Keywords::TAUNT);
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 1;
    attacker.health = 200;
    let c = with_secrets(&[PACK_TACTICS]);
    let mut w = log();
    let out = fight_with(&db, &[target], &[attacker, attacker], [c, Counters::default()], 10,
        &mut Rng::new(1), &mut w);
    let (_, pos, copy) = *w.summons.iter().find(|(side, _, _)| *side == 0).unwrap();
    assert_eq!(pos, 1, "to the right of the minion attacked");
    assert_eq!(name(&db, &copy), "Eternal Knight", "the same card");
    assert!(copy.golden, "Golden kept");
    assert!(copy.keywords.has(Keywords::TAUNT), "keywords kept");
    assert_eq!((copy.attack, copy.health), (3, 3), "its base stats are 3/3, buffs dropped");
    assert!(out.sides[0].secrets_fired & (1 << PACK_TACTICS) != 0);
}

#[test]
fn two_secrets_fire_on_one_attack_in_the_order_they_were_got() {
    let db = db();
    let mut target = mk(&db, "Microbot");
    target.attack = 0;
    target.health = 50;
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 1;
    attacker.health = 50;
    for (order, copy_shielded) in
        [(&[AUTODEFENSE_MATRIX, PACK_TACTICS][..], true), (&[PACK_TACTICS, AUTODEFENSE_MATRIX][..], false)]
    {
        let mut w = log();
        let out = fight_with(&db, &[target], &[attacker, attacker],
            [with_secrets(order), Counters::default()], 10, &mut Rng::new(1), &mut w);
        let copy = w.summons.iter().find(|(side, _, _)| *side == 0).map(|&(_, _, m)| m).unwrap();
        assert_eq!((copy.attack, copy.health), (3, 3));
        assert_eq!(
            copy.keywords.has(Keywords::DIVINE_SHIELD),
            copy_shielded,
            "the copy is of the minion as it stood when Pack Tactics fired"
        );
        assert_eq!(
            out.sides[0].secrets_fired,
            (1 << AUTODEFENSE_MATRIX) | (1 << PACK_TACTICS),
            "both fired, once"
        );
    }
}

#[test]
fn a_secret_that_summons_waits_for_room_and_stays() {
    let db = db();
    // Seven minions: nothing to summon into, so Venomstrike Trap stays.
    let mut board: Vec<Minion> = (0..7).map(|_| mk(&db, "Microbot")).collect();
    for m in &mut board {
        m.attack = 0;
        m.health = 90;
    }
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 1;
    attacker.health = 90;
    let c = with_secrets(&[VENOMSTRIKE_TRAP]);
    let mut w = log();
    let out = fight_with(&db, &board, &[attacker; 7], [c, Counters::default()], 3,
        &mut Rng::new(1), &mut w);
    assert!(!w.summons.iter().any(|(side, _, _)| *side == 0), "no Cobra on a full board");
    assert_eq!(out.sides[0].secrets_fired, 0, "it did not fire, so it stays");
}

#[test]
fn pack_tactics_fills_a_six_minion_board_and_venomstrike_then_waits() {
    let db = db();
    let mut board: Vec<Minion> = (0..6).map(|_| mk(&db, "Microbot")).collect();
    for m in &mut board {
        m.attack = 0;
        m.health = 90;
        m.keywords.set(Keywords::TAUNT);
    }
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 1;
    attacker.health = 90;
    let c = with_secrets(&[PACK_TACTICS, VENOMSTRIKE_TRAP]);
    let mut w = log();
    let out = fight_with(&db, &board, &[attacker; 7], [c, Counters::default()], 1,
        &mut Rng::new(1), &mut w);
    let got: Vec<String> =
        w.summons.iter().filter(|(s, _, _)| *s == 0).map(|(_, _, m)| name(&db, m)).collect();
    assert_eq!(got, ["Microbot"], "the 3/3 copy took the seventh slot");
    assert_eq!(
        out.sides[0].secrets_fired,
        1 << PACK_TACTICS,
        "Venomstrike Trap could not summon, so it stays"
    );
}

#[test]
fn a_pack_tactics_copy_of_an_eternal_knight_after_one_death_is_seven_five() {
    let db = db();
    let mut target = mk(&db, "Eternal Knight"); // 4/2, +4/+2 for every Knight that died
    target.attack = 0;
    target.health = 40;
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 1;
    attacker.health = 40;
    let mut c = with_secrets(&[PACK_TACTICS]);
    c.knight_deaths = 1;
    let mut w = log();
    fight_with(&db, &[target], &[attacker, attacker], [c, Counters::default()], 10,
        &mut Rng::new(1), &mut w);
    let copy = w.summons.iter().find(|(side, _, _)| *side == 0).map(|&(_, _, m)| m).unwrap();
    // 3/3 base, then the +4/+2 a Knight's death gives every Knight.
    assert_eq!((copy.attack, copy.health), (7, 5));
}

#[test]
fn goldrinns_beast_bonus_reaches_the_copy_and_the_cobra() {
    let db = db();
    // The shop's Goldrinn: "your Beasts have +7/+7 until next turn" arrives in
    // a fight as the Beast bonus, which every Beast summoned gets.
    let mut target = mk(&db, "Half-Shell"); // a 2/3 Beast
    target.attack = 0;
    target.health = 60;
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 1;
    attacker.health = 60;
    let mut c = with_secrets(&[PACK_TACTICS, VENOMSTRIKE_TRAP]);
    c.beast_attack = 7;
    c.beast_health = 7;
    let mut w = log();
    let out = fight_with(&db, &[target], &[attacker, attacker], [c, Counters::default()], 10,
        &mut Rng::new(1), &mut w);
    let summons: Vec<Minion> =
        w.summons.iter().filter(|(s, _, _)| *s == 0).map(|&(_, _, m)| m).collect();
    // The 3/3 copy first, then the 2/3 Cobra (the zone's order).
    assert_eq!(name(&db, &summons[0]), "Half-Shell");
    assert_eq!((summons[0].attack, summons[0].health), (10, 10), "3/3 + Goldrinn's +7/+7");
    assert_eq!(name(&db, &summons[1]), "Cobra");
    assert_eq!((summons[1].attack, summons[1].health), (9, 10), "2/3 + Goldrinn's +7/+7");
    assert_eq!(out.sides[0].secrets_fired, (1 << PACK_TACTICS) | (1 << VENOMSTRIKE_TRAP));
}

#[test]
fn a_goldrinn_that_dies_in_the_fight_buffs_the_cobra_it_already_summoned() {
    let db = db();
    // The player has one minion, Goldrinn, so the enemy side (two minions)
    // attacks first and has no choice of target.
    let goldrinn = mk(&db, "Goldrinn, the Great Wolf");
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 50;
    attacker.health = 200;
    let mut dummy = mk(&db, "Microbot");
    dummy.attack = 0;
    dummy.health = 1;
    let c = with_secrets(&[VENOMSTRIKE_TRAP]);
    let mut w = log();
    let out = fight_with(&db, &[goldrinn], &[attacker, dummy], [c, Counters::default()], 20,
        &mut Rng::new(1), &mut w);
    let cobra = w.summons.iter().find(|(s, _, _)| *s == 0).map(|&(_, _, m)| m).unwrap();
    assert_eq!(name(&db, &cobra), "Cobra");
    assert_eq!((cobra.attack, cobra.health), (2, 3), "summoned before Goldrinn died");
    // Goldrinn's Deathrattle gave the side +7/+7, which reached the Cobra (it
    // is a Beast) and everything summoned after it: the next attack it makes
    // shows the buff in its stats.
    assert_eq!(out.sides[0].beast_bonus_gain, (7, 7));
    let (_, cobra_now, _) = w
        .attacks
        .iter()
        .find(|(side, a, _)| *side == 0 && name(&db, a) == "Cobra")
        .expect("the Cobra attacks after Goldrinn died");
    assert_eq!((cobra_now.attack, cobra_now.health), (9, 10), "the Cobra carries +7/+7");
}

#[test]
fn redemption_returns_a_fresh_copy_with_one_health_where_it_stood() {
    let db = db();
    // Half-Shell prints Taunt, so the enemy's attacks always land on it.
    let filler = mk(&db, "Microbot");
    let shell = mk(&db, "Half-Shell");
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 10;
    attacker.health = 200;
    let mut dummies: Vec<Minion> = (0..6).map(|_| mk(&db, "Microbot")).collect();
    for d in &mut dummies {
        d.attack = 0;
        d.health = 1;
    }
    let mut enemy = vec![attacker];
    enemy.append(&mut dummies);
    let c = with_secrets(&[REDEMPTION]);
    let mut w = log();
    let out = fight_with(&db, &[filler, shell, filler], &enemy, [c, Counters::default()], 1,
        &mut Rng::new(1), &mut w);
    let (side, pos, copy) = *w.summons.iter().find(|(s, _, _)| *s == 0).unwrap();
    assert_eq!(side, 0);
    assert_eq!(pos, 1, "the spot the minion stood in");
    assert_eq!(name(&db, &copy), "Half-Shell", "the base card");
    assert_eq!((copy.attack, copy.health), (2, 1), "1 Health");
    assert!(copy.keywords.has(Keywords::TAUNT), "its printed keywords");
    assert!(
        out.sides[0].secrets_fired & (1 << REDEMPTION) != 0,
        "the Secret fired and left the zone"
    );
}

#[test]
fn redemption_waits_when_the_deathrattle_fills_the_board() {
    let db = db();
    // Seven minions, the left-most a Buzzing Vermin (Taunt, "Deathrattle:
    // summon a Beetle"). The player attacks first, so the Vermin is what dies,
    // and its Beetle takes the seventh slot back: no room for Redemption.
    let mut board: Vec<Minion> = std::iter::once(mk(&db, "Buzzing Vermin"))
        .chain((0..6).map(|_| mk(&db, "Microbot")))
        .collect();
    for m in &mut board {
        m.attack = 1;
        m.health = 1;
    }
    let mut wall = mk(&db, "Microbot");
    wall.attack = 9;
    wall.health = 400;
    let c = with_secrets(&[REDEMPTION]);
    let mut w = log();
    let out = fight_with(&db, &board, &[wall], [c, Counters::default()], 1,
        &mut Rng::new(1), &mut w);
    let got: Vec<String> =
        w.summons.iter().filter(|(s, _, _)| *s == 0).map(|(_, _, m)| name(&db, m)).collect();
    assert_eq!(got, ["Beetle"], "the Deathrattle filled the board, so Redemption waited");
    assert_eq!(out.sides[0].secrets_fired, 0, "it did not fire, so it stays");

    // One more death makes room: the waiting copy of the Vermin lands, and the
    // Secret fires then.
    let mut w = log();
    let out = fight_with(&db, &board, &[wall], [c, Counters::default()], 3,
        &mut Rng::new(1), &mut w);
    let got: Vec<(usize, Minion)> =
        w.summons.iter().filter(|(s, _, _)| *s == 0).map(|&(_, pos, m)| (pos, m)).collect();
    assert_eq!(name(&db, &got[0].1), "Beetle");
    assert_eq!(name(&db, &got[1].1), "Buzzing Vermin", "the Redemption copy");
    assert_eq!(got[1].1.health, 1);
    assert!(out.sides[0].secrets_fired & (1 << REDEMPTION) != 0, "it fires once there is room");
}

// ---- Prestidigitation and the zone ----

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

/// A no-pick game with every tribe in the lobby, player 0 forced to `hero`.
fn hero_game(seed: u64, hero: &str) -> Game {
    let tribes = Tribe::EVERY.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t));
    let d = data();
    let mut g = Game::new_with_lobby(d.clone(), seed, tribes);
    let h = d.heroes.by_name(hero).expect(hero);
    g.assign_hero(0, h);
    g
}

fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        if !answer(g) {
            g.step(Action::Freeze).unwrap();
        }
    }
}

/// Answer whatever is open for the player to act, False when nothing is.
fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let mut cands = [Action::EndTurn; 4];
    let n = if !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || !pl.secret_offer.is_empty()
        || pl.pending_choose_one.is_some()
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

#[test]
fn a_secret_fires_in_a_real_fight_and_leaves_the_zone_for_good() {
    // A sandbox mirror fight, so the whole path runs: the player's counters
    // carry the zone into the fight and the fight's outcome takes the fired
    // Secret back out.
    let d = data();
    let mut g = Game::new_sandbox(d.clone(), 5, None, 0);
    let shell = d.cards.by_name("Half-Shell").expect("Half-Shell");
    let mut m = d.cards.make(shell, false);
    m.health = 40;
    g.players[0].board.push(m);
    g.players[0].secrets.add(VENOMSTRIKE_TRAP);
    g.step(Action::EndTurn).unwrap();
    let side = g.sandbox.as_ref().and_then(|s| s.last_fight.as_ref()).expect("a fight").side;
    assert!(
        side.secrets_fired & (1 << VENOMSTRIKE_TRAP) != 0,
        "the mirror attacked, so the Cobra came in"
    );
    assert!(!g.players[0].secrets.has(VENOMSTRIKE_TRAP), "and the Secret is spent");
    g.check_invariants().unwrap();
}

#[test]
fn prestidigitation_is_free_and_once_a_turn_from_turn_1() {
    let mut g = hero_game(7, "The Great Akazamzarak");
    turn_of(&mut g, 0);
    assert_eq!(g.power_cost(0, false), 0, "a free press");
    assert!(g.power_usable(0), "usable on turn 1");
    let gold = g.players[0].gold;
    g.step(Action::HeroPower).unwrap();
    assert_eq!(g.players[0].gold, gold, "no Gold spent");
    assert!(!g.power_usable(0), "once a turn");
    // The offer waits for this player's next action, and nothing else is
    // legal beside it.
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::Choose(0)));
    assert!(!g.is_legal(Action::HeroPower));
    g.step(Action::Choose(0)).unwrap();
    assert_eq!(g.players[0].secrets.len(), 1, "the pick joined the zone");
}

#[test]
fn prestidigitation_offers_every_missing_secret_all_at_once() {
    let mut g = hero_game(11, "The Great Akazamzarak");
    turn_of(&mut g, 0);
    g.step(Action::HeroPower).unwrap();
    let offer: Vec<u8> = g.players[0].secret_offer.iter().copied().collect();
    assert_eq!(
        offer,
        (0..COMBAT_SECRETS).collect::<Vec<u8>>(),
        "all four, not the three of a Discover"
    );
    // Every offer slot is in the observation, as kind + 1.
    let mut obs = vec![f32::NAN; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut obs);
    for k in 0..COMBAT_SECRETS as usize {
        assert_eq!(obs[bgcore::obs::SCALAR_SECRET_OFFER + k], k as f32 + 1.0);
    }
    // Take Pack Tactics: the next turn offers the other three.
    let pack = offer.iter().position(|&k| k == PACK_TACTICS).unwrap();
    turn_of(&mut g, 0);
    g.step(Action::Choose(pack as u8)).unwrap();
    assert!(g.players[0].secrets.has(PACK_TACTICS));
    assert_eq!(g.players[0].secrets.len(), 1);
    end_turn(&mut g, 0);
    next_turn(&mut g, 0);
    g.step(Action::HeroPower).unwrap();
    let offer: Vec<u8> = g.players[0].secret_offer.iter().copied().collect();
    assert_eq!(offer, vec![VENOMSTRIKE_TRAP, AUTODEFENSE_MATRIX, REDEMPTION], "Pack Tactics held");
}

#[test]
fn the_ice_block_is_in_the_zone_but_is_never_offered() {
    let mut g = hero_game(13, "The Great Akazamzarak");
    g.players[0].secrets.add(ICE_BLOCK);
    g.players[0].secrets.add(VENOMSTRIKE_TRAP);
    g.players[0].secrets.add(AUTODEFENSE_MATRIX);
    g.players[0].secrets.add(REDEMPTION);
    turn_of(&mut g, 0);
    assert!(g.power_usable(0), "one combat Secret is still missing");
    g.step(Action::HeroPower).unwrap();
    let offer: Vec<u8> = g.players[0].secret_offer.iter().copied().collect();
    assert_eq!(offer, vec![PACK_TACTICS], "the Ice Block is not one of the four");
    turn_of(&mut g, 0);
    g.step(Action::Choose(0)).unwrap();
    assert_eq!(g.players[0].secrets.len(), 5, "the Ice Block is in the zone beside the four");
    assert!(g.players[0].secrets.ice_block());
    // With all four up the button is dead, however many turns go by.
    assert!(!g.power_usable(0));
    assert!(!g.is_legal(Action::HeroPower));
    end_turn(&mut g, 0);
    next_turn(&mut g, 0);
    assert!(!g.power_usable(0), "still all four");
}

#[test]
fn a_secret_that_fired_is_gone_from_the_zone_for_good() {
    let mut g = hero_game(17, "The Great Akazamzarak");
    g.players[0].secrets.add(VENOMSTRIKE_TRAP);
    g.players[0].secrets.add(PACK_TACTICS);
    let so = bgcore::SideOutcome {
        secrets_fired: 1 << PACK_TACTICS,
        ..Default::default()
    };
    g.apply_side_outcome(0, &so, true);
    assert_eq!(g.players[0].secrets.len(), 1);
    assert!(g.players[0].secrets.has(VENOMSTRIKE_TRAP), "only the one that fired is gone");
    end_turn(&mut g, 0);
    next_turn(&mut g, 0);
    assert!(!g.players[0].secrets.has(PACK_TACTICS), "it never comes back");
}

#[test]
fn the_bot_sees_its_own_secrets_and_never_the_opponents() {
    let mut g = hero_game(19, "The Great Akazamzarak");
    g.players[0].secrets.add(VENOMSTRIKE_TRAP);
    g.players[0].secrets.add(REDEMPTION);
    let mut mine = vec![f32::NAN; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 0, &mut mine);
    assert_eq!(mine[bgcore::obs::SCALAR_SECRET + VENOMSTRIKE_TRAP as usize], 1.0);
    assert_eq!(mine[bgcore::obs::SCALAR_SECRET + AUTODEFENSE_MATRIX as usize], 0.0);
    assert_eq!(mine[bgcore::obs::SCALAR_SECRET + PACK_TACTICS as usize], 0.0);
    assert_eq!(mine[bgcore::obs::SCALAR_SECRET + REDEMPTION as usize], 1.0);
    // Player 1 sits across from those Secrets: nothing of them shows.
    let mut theirs = vec![f32::NAN; bgcore::obs::OBS_SIZE];
    bgcore::obs::write(&g, 1, &mut theirs);
    for k in 0..COMBAT_SECRETS as usize {
        assert_eq!(theirs[bgcore::obs::SCALAR_SECRET + k], 0.0, "an opponent's Secret is hidden");
    }
    assert!(theirs[bgcore::obs::SCALAR_SECRET..bgcore::obs::SCALARS]
        .iter()
        .all(|&v| v == 0.0));
}

fn end_turn(g: &mut Game, p: usize) {
    turn_of(g, p);
    while g.current_player() == Some(p) {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

/// Play out the rest of the turn and the fights until it is player `p`'s turn
/// again.
fn next_turn(g: &mut Game, p: usize) {
    let turn = g.turn;
    let mut guard = 0;
    while g.turn == turn {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
        guard += 1;
        assert!(guard < 10000, "a turn that never ends");
    }
    turn_of(g, p);
}

#[test]
fn redemption_fires_before_reborn() {
    let db = db();
    // A full board of seven: a Half-Shell (Taunt) with Reborn and six 0
    // Attack Microbots, so only the enemy attacks and the Shell dies. One
    // slot opens, and the two copies want it. Era, 2026-11-23: Redemption
    // comes before Reborn, so the Secret's copy takes the slot and fires.
    let mut shell = mk(&db, "Half-Shell");
    shell.keywords.set(Keywords::REBORN);
    shell.attack = 0;
    let mut board = vec![shell];
    for _ in 0..6 {
        let mut m = mk(&db, "Microbot");
        m.attack = 0;
        m.health = 50;
        board.push(m);
    }
    let mut attacker = mk(&db, "Microbot");
    attacker.attack = 10;
    attacker.health = 200;
    let c = with_secrets(&[REDEMPTION]);
    let mut w = log();
    let out = fight_with(&db, &board, &[attacker], [c, Counters::default()], 1,
        &mut Rng::new(1), &mut w);
    let ours = w.summons.iter().filter(|(s, _, _)| *s == 0).count();
    assert_eq!(ours, 1, "one slot, one copy");
    assert!(
        out.sides[0].secrets_fired & (1 << REDEMPTION) != 0,
        "Redemption took the slot before Reborn could"
    );
}
