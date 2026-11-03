//! The neutral minions and spells of the 5-tribe lobby, with the real cards
//! from cards.toml: board-or-Tavern targets, stealing, Rally effects that
//! read the target, Holy Vanguard's Health aura, Leeroy and Nadina, the
//! Discover and refresh spells, and the Stage 2 fixes (Chef's Choice tier
//! cap, Natural Blessing on dual types, Balinda, Patient Scout).

use std::sync::Arc;

use bgcore::fight::{Counters, Watch, fight_with, fight_watched};
use bgcore::{Action, CardDb, CardId, Data, Game, Keywords, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("no card {name}"))
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon, Tribe::Pirate]))
}

/// Answer whatever the current player has open (a Discover, a target, a
/// discard, or a trinket offer), so a helper that only wants the next turn
/// never stalls on a prompt. False when nothing is open.
fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let mut cands = [Action::EndTurn; 4];
    let n = if !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
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

fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        if !answer(g) {
            g.step(Action::Freeze).unwrap();
        }
    }
}

fn act(g: &mut Game, p: usize, a: Action) {
    turn_of(g, p);
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

fn is_legal(g: &mut Game, p: usize, a: Action) -> bool {
    turn_of(g, p);
    g.is_legal(a)
}

/// Everyone ends their turn until the next turn starts.
fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

fn put_on_board(g: &mut Game, p: usize, name: &str) -> usize {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let mut m = g.players[p].make_card(&g.data.cards, c, false, 1);
    m.uses_left = m.turn_uses(&g.data.cards);
    g.players[p].board.push(m);
    g.players[p].board.len() - 1
}

/// Replace a player's shop with the named cards, through the pool.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        if !g.data.cards.get(m.card).is_spell() {
            let slot = g.data.cards.get(m.card).pool_slot as usize;
            g.pool[slot] += m.pool_copies;
        }
    }
    for n in names {
        let c = id(g, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

fn board_pos(g: &Game, p: usize, name: &str) -> usize {
    g.players[p]
        .board
        .iter()
        .position(|m| g.data.cards.get(m.card).name == name)
        .unwrap_or_else(|| panic!("{name} is not on the board"))
}

fn hand_pos(g: &Game, p: usize, name: &str) -> usize {
    g.players[p]
        .hand
        .iter()
        .position(|m| g.data.cards.get(m.card).name == name)
        .unwrap_or_else(|| panic!("{name} is not in hand"))
}

fn play_card(g: &mut Game, p: usize, name: &str) {
    let h = hand_pos(g, p, name);
    act(g, p, Action::Play { hand: h as u8, pos: 0 });
}

/// A fight watcher that records every attack and death.
#[derive(Default)]
struct Rec {
    attacks: Vec<(CardId, i32)>,
    deaths: Vec<CardId>,
}

impl Watch for Rec {
    fn attack(&mut self, _side: usize, attacker: &Minion, _defender: &Minion) {
        self.attacks.push((attacker.card, attacker.attack));
    }
    fn death(&mut self, _side: usize, _pos: usize, m: &Minion) {
        self.deaths.push(m.card);
    }
}

// ---- board or Tavern targets ----

#[test]
fn prisonguard_buffs_a_tavern_minion_permanently() {
    let mut g = game(1);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Suspicious Prisonguard");
    set_shop(&mut g, 0, &["Cord Puller"]);
    // Only the Prisonguard is on the board and only one minion is in the
    // Tavern, so the one legal target is picked at once.
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(stats(&g.players[0].shop[0]), (4, 4));
    // The buff is permanent: the bought copy keeps it.
    act(&mut g, 0, Action::Buy(0));
    let m = &g.players[0].hand[0];
    assert_eq!(g.data.cards.get(m.card).name, "Cord Puller");
    assert_eq!(stats(m), (4, 4));
    g.check_invariants().unwrap();
}

#[test]
fn prisonguard_can_buff_a_board_minion() {
    let mut g = game(2);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Suspicious Prisonguard");
    put_on_board(&mut g, 0, "Cord Puller");
    set_shop(&mut g, 0, &[]);
    // Two board minions: the Prisonguard is excluded, Cord Puller is the
    // only target, so it is taken at once.
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(stats(&g.players[0].board[1]), (4, 4));
    g.check_invariants().unwrap();
}

#[test]
fn a_prisonguard_triple_starts_with_its_activate_fresh() {
    // the copy's Activate was used this turn; the golden from the triple
    // can Activate again right away (uses belong to the minion)
    let mut g = game(5);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Suspicious Prisonguard");
    put_on_board(&mut g, 0, "Cord Puller");
    set_shop(&mut g, 0, &[]);
    act(&mut g, 0, Action::Activate(0)); // Cord Puller is the one target
    assert!(g.players[0].board[0].activated);
    set_shop(&mut g, 0, &["Suspicious Prisonguard"]);
    act(&mut g, 0, Action::Buy(0)); // the second copy
    set_shop(&mut g, 0, &["Suspicious Prisonguard"]);
    act(&mut g, 0, Action::Buy(0)); // the third triples
    assert_eq!(g.players[0].board.len(), 1, "only Cord Puller is left");
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    act(&mut g, 0, Action::Play { hand: gold as u8, pos: 0 });
    assert!(g.players[0].board[0].golden);
    // fresh uses: the golden can Activate again this very turn
    assert!(is_legal(&mut g, 0, Action::Activate(0)));
    g.check_invariants().unwrap();
}

#[test]
fn tyrael_sets_a_tavern_minions_stats() {
    let mut g = game(3);
    g.players[0].gold = 10;
    let tyrael = put_on_board(&mut g, 0, "Tyrael");
    put_on_board(&mut g, 0, "Cord Puller");
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Activate(tyrael as u8));
    // Board other and Tavern minion: the player picks.
    assert!(g.players[0].pending_target.is_some());
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(stats(&g.players[0].shop[0]), (50, 50));
    g.check_invariants().unwrap();
}

/// A game whose lobby has the Undead, for the cards that carry the Undead
/// bonus and the Eternal Knight's counter.
fn undead_game(seed: u64) -> Game {
    Game::new_with_lobby(
        Arc::new(Data::bundled()),
        seed,
        lobby(&[Tribe::Undead, Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc]),
    )
}

#[test]
fn a_stat_set_keeps_the_bonuses_that_apply_wherever_the_card_is() {
    // Tyrael: "Activate (1): Set another minion's stats to 50/50". Era's log
    // has a 19/7 Drustfallen Butcher with the Undead bonus at 17 become 67/50:
    // the set reaches 50/50 and the bonuses that apply wherever the card is go
    // on top, the rule Pack Tactics' 3/3 copy gets from being summoned
    // (docs/rules/design.md "Exact copies", docs/rules/heroes.md "Secrets").
    let mut g = undead_game(5);
    g.players[0].undead_bonus = (17, 0);
    g.players[0].gold = 10;
    let tyrael = put_on_board(&mut g, 0, "Tyrael");
    put_on_board(&mut g, 0, "Cord Puller");
    set_shop(&mut g, 0, &["Handless Forsaken"]); // Undead, 2/1 plus the bonus
    assert_eq!(stats(&g.players[0].shop[0]), (19, 1));
    act(&mut g, 0, Action::Activate(tyrael as u8));
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(stats(&g.players[0].shop[0]), (67, 50));

    // The other set of stats, Perfect Vision (20/20), keeps it too.
    let undead = put_on_board(&mut g, 0, "Handless Forsaken");
    assert_eq!(stats(&g.players[0].board[undead]), (19, 1));
    g.give(0, "Perfect Vision").unwrap();
    g.players[0].gold = 10;
    play_card(&mut g, 0, "Perfect Vision");
    act(&mut g, 0, Action::Target(undead as u8));
    assert_eq!(stats(&g.players[0].board[undead]), (37, 20), "20/20 plus 17 Attack");

    // And a counter a card scales with: an Eternal Knight's deaths.
    let mut g = undead_game(6);
    g.players[0].knight_deaths = 2;
    g.players[0].gold = 10;
    let tyrael = put_on_board(&mut g, 0, "Tyrael");
    put_on_board(&mut g, 0, "Cord Puller");
    set_shop(&mut g, 0, &["Eternal Knight"]);
    let knight = g.data.cards.get(id(&g, "Eternal Knight"));
    let printed = (knight.attack, knight.health);
    let before = stats(&g.players[0].shop[0]);
    let bonus = (before.0 - printed.0, before.1 - printed.1);
    assert!(bonus.0 > 0, "the deaths are in its stats to begin with: {bonus:?}");
    act(&mut g, 0, Action::Activate(tyrael as u8));
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(stats(&g.players[0].shop[0]), (50 + bonus.0, 50 + bonus.1));
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_tyrael_sets_100_100() {
    let mut g = game(4);
    g.players[0].gold = 10;
    let c = id(&g, "Tyrael");
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 3;
    let m = g.players[0].make_card(&g.data.cards, c, true, 3);
    g.players[0].board.push(m);
    put_on_board(&mut g, 0, "Cord Puller");
    set_shop(&mut g, 0, &[]);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(stats(&g.players[0].board[1]), (100, 100));
    g.check_invariants().unwrap();
}

// ---- stealing ----

#[test]
fn decoy_conjurer_steals_from_the_pool_and_a_sale_returns_it() {
    let mut g = game(5);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Decoy Conjurer");
    let cord = id(&g, "Cord Puller");
    let slot = g.data.cards.get(cord).pool_slot as usize;
    set_shop(&mut g, 0, &["Cord Puller"]);
    let in_shop = g.pool[slot];
    act(&mut g, 0, Action::Activate(0));
    // The copy left the Tavern into hand and did not go back to the pool.
    assert!(g.players[0].shop.is_empty());
    let h = hand_pos(&g, 0, "Cord Puller");
    assert_eq!(g.pool[slot], in_shop);
    // Play it and sell it: the copy returns to the pool.
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    let b = board_pos(&g, 0, "Cord Puller");
    act(&mut g, 0, Action::Sell(b as u8));
    assert_eq!(g.pool[slot], in_shop + 1);
    g.check_invariants().unwrap();
}

#[test]
fn decoy_conjurer_breaks_a_tie_for_the_highest_attack_at_random() {
    // docs/rules/design.md "Shop replay answers": when several Tavern minions
    // tie for the highest Attack the stolen one is random among them, not the
    // left-most.
    let mut seen = [false; 2];
    for seed in 0..24 {
        let mut g = game(seed);
        g.players[0].gold = 10;
        put_on_board(&mut g, 0, "Decoy Conjurer");
        // Cord Puller and Buzzing Vermin are both 1/1, so they tie.
        set_shop(&mut g, 0, &["Cord Puller", "Buzzing Vermin"]);
        act(&mut g, 0, Action::Activate(0));
        let stolen_cord = g.players[0]
            .hand
            .iter()
            .any(|m| g.data.cards.get(m.card).name == "Cord Puller");
        seen[usize::from(!stolen_cord)] = true;
        g.check_invariants().unwrap();
    }
    assert!(seen[0] && seen[1], "both tied minions are stolen by some seed: {seen:?}");
}

#[test]
fn enchanted_lasso_steals_a_random_tavern_minion() {
    let mut g = game(6);
    g.players[0].gold = 10;
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.give(0, "Enchanted Lasso").unwrap();
    play_card(&mut g, 0, "Enchanted Lasso");
    assert!(g.players[0].shop.is_empty());
    assert_eq!(hand_pos(&g, 0, "Cord Puller"), 0);
    g.check_invariants().unwrap();
}

// ---- Rally effects that read the target ----

#[test]
fn heroic_underdog_gains_the_targets_attack_for_the_fight() {
    let d = db();
    let underdog = d.by_name("Heroic Underdog").unwrap();
    let mut a = vec![mk(&d, "Heroic Underdog", false), mk(&d, "Microbot", false)];
    let mut b = vec![mk(&d, "Microbot", false)];
    b[0].attack = 7;
    b[0].health = 1000;
    let mut w = Rec::default();
    fight_watched(&d, &a, &b, 1, &mut Rng::new(11), &mut w);
    let hit = w.attacks.iter().find(|(c, _)| *c == underdog).expect("Underdog attacks");
    assert_eq!(hit.1, 1 + 7);
    // Golden doubles the gain.
    a[0] = mk(&d, "Heroic Underdog", true);
    let mut w = Rec::default();
    fight_watched(&d, &a, &b, 1, &mut Rng::new(11), &mut w);
    let hit = w.attacks.iter().find(|(c, _)| *c == underdog).unwrap();
    assert_eq!(hit.1, 2 + 14);
}

#[test]
fn sindorei_removes_reborn_and_taunt_before_the_hit() {
    let d = db();
    let warden = d.by_name("Bronze Warden").unwrap();
    let straight = d.by_name("Sin'dorei Straight Shot").unwrap();
    let a = vec![mk(&d, "Sin'dorei Straight Shot", false), mk(&d, "Microbot", false)];
    let b = vec![mk(&d, "Bronze Warden", false)];
    let mut w = Rec::default();
    let out = fight_watched(&d, &a, &b, 10, &mut Rng::new(12), &mut w);
    assert_eq!(out.winner, Some(0));
    // Divine Shield takes the first swing; the second kills it, and Reborn
    // was removed before the hit, so it dies exactly once.
    assert!(w.attacks.iter().any(|(c, _)| *c == straight));
    assert_eq!(w.deaths.iter().filter(|&&c| c == warden).count(), 1);
}

// ---- Holy Vanguard ----

#[test]
fn holy_vanguard_gains_at_15_health() {
    let d = db();
    let a = vec![mk(&d, "Holy Vanguard", false), mk(&d, "Microbot", false)];
    let b = vec![mk(&d, "Microbot", false)];
    let vanguard = d.by_name("Holy Vanguard").unwrap();
    for (health, expected) in [(Some(15), 40), (Some(16), 10), (None, 10)] {
        let counters = [Counters { health, ..Counters::default() }, Counters::default()];
        let mut w = Rec::default();
        fight_with(&d, &a, &b, counters, 1, &mut Rng::new(13), &mut w);
        let attack = w.attacks.iter().find(|(c, _)| *c == vanguard).unwrap().1;
        assert_eq!(attack, expected, "health {health:?}");
    }
    // Golden: 20/20 plus +60/+60.
    let a = vec![mk(&d, "Holy Vanguard", true), mk(&d, "Microbot", false)];
    let counters = [Counters { health: Some(15), ..Counters::default() }, Counters::default()];
    let mut w = Rec::default();
    fight_with(&d, &a, &b, counters, 1, &mut Rng::new(13), &mut w);
    assert_eq!(w.attacks.iter().find(|(c, _)| *c == vanguard).unwrap().1, 80);
}

#[test]
fn holy_vanguard_does_not_show_a_rewound_hit() {
    let mut g = game(14);
    g.players[0].health = 16;
    put_on_board(&mut g, 0, "Wrath Weaver");
    put_on_board(&mut g, 0, "Soul Rewinder");
    put_on_board(&mut g, 0, "Holy Vanguard");
    g.give(0, "Demon Fodder").unwrap();
    play_card(&mut g, 0, "Demon Fodder");
    // Wrath Weaver dealt 1 and Soul Rewinder gave it back: the hero is at
    // 16, so Holy Vanguard has no bonus.
    assert_eq!(g.players[0].health, 16);
    let counters = [g.players[0].counters(&g.data, g.lobby), Counters::default()];
    let d = db();
    let a = vec![mk(&d, "Holy Vanguard", false), mk(&d, "Microbot", false)];
    let b = vec![mk(&d, "Microbot", false)];
    let vanguard = d.by_name("Holy Vanguard").unwrap();
    let mut w = Rec::default();
    fight_with(&d, &a, &b, counters, 1, &mut Rng::new(15), &mut w);
    assert_eq!(w.attacks.iter().find(|(c, _)| *c == vanguard).unwrap().1, 10);
}

// ---- Leeroy and Nadina ----

#[test]
fn leeroy_destroys_its_killer_through_divine_shield() {
    let d = db();
    let mut leeroy = mk(&d, "Leeroy the Reckless", false);
    leeroy.attack = 0; // its own hit doesn't pop the killer's shield
    let mut killer = mk(&d, "Microbot", false);
    killer.attack = 10;
    killer.health = 500;
    killer.keywords.set(Keywords::DIVINE_SHIELD);
    let a = vec![killer, mk(&d, "Microbot", false)];
    let b = vec![leeroy];
    let killer_id = d.by_name("Microbot").unwrap();
    let leeroy_id = d.by_name("Leeroy the Reckless").unwrap();
    let mut w = Rec::default();
    let out = fight_watched(&d, &a, &b, 1, &mut Rng::new(16), &mut w);
    assert_eq!(out.winner, Some(0));
    assert!(w.deaths.contains(&leeroy_id));
    // The killer still had its Divine Shield when Leeroy died and was
    // destroyed anyway.
    assert_eq!(w.deaths.iter().filter(|&&c| c == killer_id).count(), 1);
}

#[test]
fn nadina_prefers_dragons_without_divine_shield() {
    let d = db();
    let mut nadina = mk(&d, "Nadina the Red", false);
    nadina.health = 1;
    nadina.keywords.set(Keywords::TAUNT);
    let a = vec![
        nadina,
        mk(&d, "Tarecgosa", false),
        mk(&d, "Tarecgosa", false),
        mk(&d, "Tarecgosa", false),
        mk(&d, "Bronze Warden", false),
        mk(&d, "Bronze Warden", false),
    ];
    let mut wall = mk(&d, "Microbot", false);
    wall.attack = 100;
    wall.health = 100_000;
    let b = vec![wall];
    let out = fight_watched(&d, &a, &b, 500, &mut Rng::new(17), &mut Rec::default());
    // Nadina died and gave all 3 unshielded Tarecgosas a shield; no grant
    // was wasted on the two Bronze Wardens that already had one.
    let shield = Keywords::DIVINE_SHIELD;
    for slot in 1..=3 {
        assert!(
            out.sides[0].permanent_keywords[slot] & shield != 0,
            "Tarecgosa in slot {slot} should keep Divine Shield"
        );
    }
}

// ---- Misplaced Tea Set, Balinda and Gatekeeper ----

#[test]
fn tea_set_buffs_a_dual_tribe_minion_once() {
    let mut g = game(18);
    put_on_board(&mut g, 0, "Gearfin");
    g.give(0, "Misplaced Tea Set").unwrap();
    play_card(&mut g, 0, "Misplaced Tea Set");
    // Gearfin is Mech and Murloc and is the only minion: one +4/+4.
    assert_eq!(stats(&g.players[0].board[0]), (10, 9));
    g.check_invariants().unwrap();
}

#[test]
fn balinda_does_not_double_tea_set() {
    let mut g = game(19);
    put_on_board(&mut g, 0, "Gearfin");
    put_on_board(&mut g, 0, "Balinda Stonehearth");
    g.give(0, "Misplaced Tea Set").unwrap();
    play_card(&mut g, 0, "Misplaced Tea Set");
    assert_eq!(stats(&g.players[0].board[0]), (10, 9));
    g.check_invariants().unwrap();
}

#[test]
fn balinda_doubles_only_targeted_spells() {
    let mut g = game(20);
    put_on_board(&mut g, 0, "Balinda Stonehearth");
    put_on_board(&mut g, 0, "Gearfin");
    g.give(0, "Shiny Ring").unwrap();
    play_card(&mut g, 0, "Shiny Ring");
    // No target: cast once.
    assert_eq!(stats(&g.players[0].board[1]), (7, 6));
    g.give(0, "Tavern Dish Banana").unwrap();
    play_card(&mut g, 0, "Tavern Dish Banana");
    act(&mut g, 0, Action::Target(1));
    // A targeted spell: cast twice.
    assert_eq!(stats(&g.players[0].board[1]), (11, 10));
    g.check_invariants().unwrap();
}

#[test]
fn gatekeeper_casts_tea_set_and_hooktail_does_not_see_it() {
    let mut g = game(21);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam");
    put_on_board(&mut g, 0, "Timecap'n Hooktail");
    put_on_board(&mut g, 0, "Boom-in-a-Box");
    g.give(0, "Tavern Dish Banana").unwrap();
    play_card(&mut g, 0, "Tavern Dish Banana");
    let gatekeeper = board_pos(&g, 0, "Gatekeeper Amalgam");
    act(&mut g, 0, Action::Target(gatekeeper as u8));
    // Only the Banana is a spell you cast, so Hooktail gives every minion +1
    // Attack once. The Misplaced Tea Set the Gatekeeper casts for you is not
    // your cast (Era 2026-11-02), so it adds nothing. Boom-in-a-Box is
    // neutral and no Tea Set type reaches it, so it shows exactly +1.
    assert_eq!(stats(&g.players[0].board[2]), (6, 10));
    g.check_invariants().unwrap();
}

// ---- Robust Evolution ----

#[test]
fn robust_evolution_keeps_stats_and_goes_one_tier_up() {
    let mut g = game(22);
    let slot = put_on_board(&mut g, 0, "Wrath Weaver");
    g.players[0].board[slot].attack = 2;
    g.players[0].board[slot].health = 1;
    g.give(0, "Robust Evolution").unwrap();
    play_card(&mut g, 0, "Robust Evolution");
    act(&mut g, 0, Action::Target(slot as u8));
    let m = g.players[0].board[slot];
    assert_eq!(g.data.cards.get(m.card).tier, 2);
    assert_eq!(stats(&m), (2, 1));
    assert!(!m.golden);
    g.check_invariants().unwrap();
}

#[test]
fn robust_evolution_takes_a_tier_6_into_another_tier_6() {
    let mut g = game(23);
    let slot = put_on_board(&mut g, 0, "Tyrael");
    g.give(0, "Robust Evolution").unwrap();
    play_card(&mut g, 0, "Robust Evolution");
    act(&mut g, 0, Action::Target(slot as u8));
    let m = g.players[0].board[slot];
    assert_eq!(g.data.cards.get(m.card).tier, 6);
    assert_eq!(stats(&m), (10, 10));
    g.check_invariants().unwrap();
}

#[test]
fn robust_evolution_makes_a_golden_target_non_golden_with_its_stats() {
    let mut g = game(24);
    let c = id(&g, "Wrath Weaver");
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 3;
    let mut m = g.players[0].make_card(&g.data.cards, c, true, 3);
    m.attack = 9;
    m.health = 8;
    g.players[0].board.push(m);
    g.give(0, "Robust Evolution").unwrap();
    play_card(&mut g, 0, "Robust Evolution");
    act(&mut g, 0, Action::Target(0));
    let m = g.players[0].board[0];
    assert!(!m.golden);
    assert_eq!(stats(&m), (9, 8));
    assert_eq!(g.data.cards.get(m.card).tier, 2);
    g.check_invariants().unwrap();
}

// ---- Hasty Excavation ----

#[test]
fn hasty_excavation_costs_health_and_respects_a_rewind() {
    let mut g = game(25);
    g.players[0].tier = 2;
    g.players[0].health = 3;
    g.players[0].gold = 10;
    let hasty = id(&g, "Hasty Excavation");
    g.players[0].shop_spell = Some(hasty);
    assert!(!is_legal(&mut g, 0, Action::BuySpell));
    // A rewind on board makes the Health cost payable again.
    put_on_board(&mut g, 0, "Soul Rewinder");
    assert!(is_legal(&mut g, 0, Action::BuySpell));
    act(&mut g, 0, Action::BuySpell);
    // 3 Health paid and rewound: still 3.
    assert_eq!(g.players[0].health, 3);
    assert_eq!(hand_pos(&g, 0, "Hasty Excavation"), 0);
    g.check_invariants().unwrap();
}

// ---- Search Through Time ----

#[test]
fn search_through_time_locks_the_discovered_minion_for_a_turn() {
    let mut g = game(26);
    g.players[0].tier = 2;
    g.give(0, "Search Through Time").unwrap();
    play_card(&mut g, 0, "Search Through Time");
    assert_eq!(g.players[0].discover.len(), 3);
    act(&mut g, 0, Action::Choose(0));
    let h = g.players[0]
        .hand
        .iter()
        .position(|m| m.locked_turns > 0)
        .expect("the discovered minion is locked");
    assert_eq!(g.data.cards.get(g.players[0].hand[h].card).tier, 2);
    assert!(!is_legal(&mut g, 0, Action::Play { hand: h as u8, pos: 0 }));
    next_turn(&mut g);
    assert_eq!(g.players[0].hand[h].locked_turns, 0);
    assert!(is_legal(&mut g, 0, Action::Play { hand: h as u8, pos: 0 }));
    g.check_invariants().unwrap();
}

// ---- Saloon's Finest and Lost Staff ----

#[test]
fn saloon_finest_fills_the_tavern_with_spells_and_takes_no_fodders() {
    let mut g = game(27);
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    g.give(0, "Saloon's Finest").unwrap();
    play_card(&mut g, 0, "Saloon's Finest");
    assert_eq!(g.players[0].fodder_refreshes.len(), 1);
    assert_eq!(g.players[0].fodder_refreshes[0], 1);
    assert!(!g.players[0].shop.is_empty());
    assert!(
        g.players[0]
            .shop
            .iter()
            .all(|m| g.data.cards.get(m.card).is_spell())
    );
    assert!(g.players[0].shop_spell.is_some());
    // A spell in a minion slot is bought for its own cost and goes to hand.
    g.players[0].gold = 10;
    let cost = g.data.cards.get(g.players[0].shop[0].card).cost;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 10 - cost.min(10));
    g.check_invariants().unwrap();
}

#[test]
fn lost_staff_refreshes_with_the_targets_type_and_takes_no_fodders() {
    let mut g = game(28);
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    let target = put_on_board(&mut g, 0, "Buzzing Vermin");
    set_shop(&mut g, 0, &[]);
    g.give(0, "Lost Staff of Hamuul").unwrap();
    play_card(&mut g, 0, "Lost Staff of Hamuul");
    act(&mut g, 0, Action::Target(target as u8));
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), &[1]);
    assert!(!g.players[0].shop.is_empty());
    assert!(g.players[0].shop.iter().all(|m| {
        g.data.cards.get(m.card).tribes.has(Tribe::Beast)
    }));
    g.check_invariants().unwrap();
}

// ---- Channel the Devourer ----

#[test]
fn channel_the_devourer_sells_and_fires_the_sell_trigger() {
    let mut g = game(29);
    let tad = put_on_board(&mut g, 0, "Tad");
    let cord = put_on_board(&mut g, 0, "Cord Puller");
    let gold = g.players[0].gold;
    g.give(0, "Channel the Devourer").unwrap();
    play_card(&mut g, 0, "Channel the Devourer");
    act(&mut g, 0, Action::Target(tad as u8));
    // Tad sold for 1 Gold, its sell trigger gave a random Murloc, and its
    // 2/2 went to Cord Puller.
    assert_eq!(g.players[0].gold, gold + 1);
    assert_eq!(g.players[0].board.len(), 1);
    assert_eq!(stats(&g.players[0].board[0]), (3, 3));
    assert!(
        g.players[0]
            .hand
            .iter()
            .any(|m| g.data.cards.get(m.card).tribes.has(Tribe::Murloc))
    );
    let _ = cord;
    g.check_invariants().unwrap();
}

// ---- Fruit Vendor, Rodeo Performer, Planar Telescope ----

#[test]
fn fruit_vendor_gives_two_bananas() {
    let mut g = game(33);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Fruit Vendor");
    act(&mut g, 0, Action::Activate(0));
    let bananas = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).name == "Tavern Dish Banana")
        .count();
    assert_eq!(bananas, 2);
    g.check_invariants().unwrap();
}

#[test]
fn rodeo_performer_discovers_tavern_spells_up_to_the_tier() {
    let mut g = game(34);
    g.give(0, "Rodeo Performer").unwrap();
    play_card(&mut g, 0, "Rodeo Performer");
    // The Battlecry is a Discover of spells: 3 options.
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(g.players[0].discover.iter().all(|&c| {
        let card = g.data.cards.get(c);
        card.is_spell() && card.tier <= g.players[0].tier
    }));
    act(&mut g, 0, Action::Choose(0));
    assert!(g.players[0].hand.iter().any(|m| g.data.cards.get(m.card).is_spell()));
    g.check_invariants().unwrap();
}

#[test]
fn planar_telescope_discovers_the_most_common_type() {
    let mut g = game(35);
    // Two Beasts and one Mech: Beasts win.
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Lullabot");
    g.players[0].tier = 3;
    g.give(0, "Planar Telescope").unwrap();
    play_card(&mut g, 0, "Planar Telescope");
    assert!(!g.players[0].discover.is_empty());
    assert!(
        g.players[0]
            .discover
            .iter()
            .all(|&c| g.data.cards.get(c).tribes.has(Tribe::Beast))
    );
    g.check_invariants().unwrap();
}

// ---- Eonar's Favor, Staff of Enrichment, Tricky Trousers, Sludge Corrosion ----

#[test]
fn eonars_favor_buffs_tavern_minions_of_the_targets_type_this_game() {
    let mut g = game(36);
    let target = put_on_board(&mut g, 0, "Lullabot"); // Mech
    set_shop(&mut g, 0, &["Cord Puller", "Flittering Bat"]); // Mech and Beast
    g.give(0, "Eonar's Favor").unwrap();
    play_card(&mut g, 0, "Eonar's Favor");
    act(&mut g, 0, Action::Target(target as u8));
    assert_eq!(stats(&g.players[0].shop[0]), (4, 4)); // Cord Puller +3/+3
    assert_eq!(stats(&g.players[0].shop[1]), (1, 4)); // Beast untouched
    // The buff lasts the game: a new Mech drawn later has it too.
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    let mech = g.players[0]
        .shop
        .iter()
        .find(|m| g.data.cards.get(m.card).tribes.has(Tribe::Mech));
    if let Some(m) = mech {
        let base = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (base.attack + 3, base.health + 3));
    }
    g.check_invariants().unwrap();
}

#[test]
fn eonars_favor_on_an_all_type_target_buffs_every_typed_tavern_minion() {
    // "Choose a minion. Give minions of its type in the Tavern +3/+3 this
    // game": an Amalgam has every type, so every Tavern minion with a type
    // gets it (the log's En-Djinn Blazer and Firescale Hoarder next to an
    // untouched tribeless minion), and the plus is the card's current numbers:
    // the printed +3/+3 and the player's Tavern spell bonus, which the log
    // shows as +6/+8.
    let mut g = game(38);
    g.players[0].spell_bonus = (3, 5);
    let amalgam = put_on_board(&mut g, 0, "Gatekeeper Amalgam");
    set_shop(&mut g, 0, &["Cord Puller", "Flittering Bat", "Boom-in-a-Box"]);
    let printed: Vec<(i32, i32)> = g.players[0]
        .shop
        .iter()
        .map(|m| {
            let c = g.data.cards.get(m.card);
            (c.attack, c.health)
        })
        .collect();
    g.give(0, "Eonar's Favor").unwrap();
    play_card(&mut g, 0, "Eonar's Favor");
    act(&mut g, 0, Action::Target(amalgam as u8));
    assert_eq!(
        stats(&g.players[0].shop[0]),
        (printed[0].0 + 6, printed[0].1 + 8),
        "Cord Puller, a Mech"
    );
    assert_eq!(
        stats(&g.players[0].shop[1]),
        (printed[1].0 + 6, printed[1].1 + 8),
        "Flittering Bat, a Beast"
    );
    assert_eq!(stats(&g.players[0].shop[2]), printed[2], "a tribeless minion gets nothing");
    // It lasts the game: a minion dealt later gets the same plus.
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        if c.tribes.is_empty() || c.is_spell() {
            continue;
        }
        assert_eq!(
            (m.attack, m.health),
            (c.attack + 6, c.health + 8),
            "{} dealt later",
            c.name
        );
    }
    g.check_invariants().unwrap();
}

#[test]
fn a_target_that_says_a_minion_reaches_the_tavern() {
    // docs/rules/design.md "Targets in the shop": a spell, Battlecry or Activate that
    // targets "a minion" (or "another Murloc", "a Pirate", "a Beast") reaches
    // a Tavern minion; only "a friendly minion" stays on the board.
    let mut g = game(71);
    g.players[0].gold = 10;
    let escapee = put_on_board(&mut g, 0, "Sewer Escapee");
    // Two Murlocs, so the pick is a real prompt rather than its only answer.
    set_shop(&mut g, 0, &["Bile Spitter", "Bile Spitter"]);
    act(&mut g, 0, Action::Activate(escapee as u8));
    act(&mut g, 0, Action::TargetShop(0));
    let m = g.players[0].shop[0];
    let c = g.data.cards.get(m.card);
    assert_eq!((m.attack, m.health), (c.attack + 7, c.health + 7), "another Murloc, in the Tavern");

    // Lovesick Balladist's "Give a Pirate +2 Health": a Tavern Pirate, which
    // Era's log shows.
    let mut g = game(72);
    g.players[0].gold = 10;
    set_shop(&mut g, 0, &["Southsea Busker", "Southsea Busker"]);
    g.give(0, "Lovesick Balladist").unwrap();
    let before = g.players[0].shop[0].health;
    play_card(&mut g, 0, "Lovesick Balladist");
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(g.players[0].shop[0].health, before + 2, "a Pirate in the Tavern");
    g.check_invariants().unwrap();
}

#[test]
fn a_choose_one_with_a_target_asks_the_option_then_a_tavern_target() {
    // Sprightly Scarab is a Beast with a Choose One Battlecry that targets a
    // Beast: the option comes first, then the target, and a Tavern Beast is a
    // Beast (Era's log has it on one).
    let mut g = game(73);
    g.players[0].gold = 10;
    set_shop(&mut g, 0, &["Flittering Bat", "Flittering Bat"]);
    g.give(0, "Sprightly Scarab").unwrap();
    play_card(&mut g, 0, "Sprightly Scarab");
    assert!(g.players[0].pending_choose_one.is_some(), "the option is asked first");
    act(&mut g, 0, Action::Choose(0));
    assert!(g.players[0].pending_target.is_some(), "then the target");
    act(&mut g, 0, Action::TargetShop(0));
    let m = g.players[0].shop[0];
    let c = g.data.cards.get(m.card);
    assert_eq!((m.attack, m.health), (c.attack + 1, c.health + 1), "a Tavern Beast");
    assert!(m.keywords.has(Keywords::REBORN), "and Reborn");
    g.check_invariants().unwrap();
}

#[test]
fn malchezaars_last_free_refresh_spends_one_of_his_health_refreshes() {
    // docs/rules/design.md "Malchezaar and free Refreshes" (a bug in the real game we
    // copy): free Refreshes go first, and the last one also uses up one of
    // Malchezaar's two Health Refreshes, so a turn with any free Refresh has
    // one Health Refresh left.
    let mut g = Game::new_with_lobby(
        Arc::new(Data::bundled()),
        74,
        lobby(&[Tribe::Demon, Tribe::Undead, Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc]),
    );
    g.players[0].gold = 20;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 2);
    g.players[0].free_rolls = 2;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 2, "one free Refresh left");
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].free_rolls, 0);
    assert_eq!(
        g.players[0].health_refreshes_left(&g.data.cards),
        1,
        "the last free Refresh used one Health Refresh"
    );
    let health = g.players[0].health;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 0, "the Health Refresh");
    assert_eq!(g.players[0].health, health - g.data.rules.health_refresh_cost);
    // A Malchezaar that reaches the board without being played has its uses
    // at once (docs/notes/KNOWN-GAPS.md, the first row).
    let m = g.players[0].make_card(&g.data.cards, id(&g, "Malchezaar, Prince of Dance"), false, 0);
    assert_eq!(m.uses_left, 2, "a made Malchezaar arrives with both");
    g.check_invariants().unwrap();
}

#[test]
fn eonars_favor_cannot_target_a_neutral_minion() {
    let mut g = game(37);
    put_on_board(&mut g, 0, "Boom-in-a-Box"); // neutral
    set_shop(&mut g, 0, &[]);
    g.give(0, "Eonar's Favor").unwrap();
    assert!(!is_legal(&mut g, 0, Action::Play { hand: 0, pos: 0 }));
}

#[test]
fn staff_of_enrichment_buffs_the_tavern_this_game() {
    let mut g = game(38);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.give(0, "Staff of Enrichment").unwrap();
    play_card(&mut g, 0, "Staff of Enrichment");
    assert_eq!(stats(&g.players[0].shop[0]), (3, 3));
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    for m in &g.players[0].shop {
        let base = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (base.attack + 2, base.health + 2));
    }
    g.check_invariants().unwrap();
}

#[test]
fn tricky_trousers_adds_then_removes_taunt() {
    let mut g = game(39);
    let target = put_on_board(&mut g, 0, "Cord Puller"); // 1/1
    for _ in 0..2 {
        g.give(0, "Tricky Trousers").unwrap();
        play_card(&mut g, 0, "Tricky Trousers");
        act(&mut g, 0, Action::Target(target as u8));
    }
    let m = &g.players[0].board[target];
    assert_eq!(stats(m), (3, 5));
    assert!(!m.keywords.has(Keywords::TAUNT));
    g.check_invariants().unwrap();
}

#[test]
fn sludge_corrosion_buffs_the_board() {
    let mut g = game(40);
    put_on_board(&mut g, 0, "Cord Puller");
    put_on_board(&mut g, 0, "Flittering Bat");
    g.give(0, "Sludge Corrosion").unwrap();
    play_card(&mut g, 0, "Sludge Corrosion");
    assert_eq!(stats(&g.players[0].board[0]), (2, 2));
    assert_eq!(stats(&g.players[0].board[1]), (2, 5));
    g.check_invariants().unwrap();
}

#[test]
fn weapons_forge_gives_three_pointy_arrows() {
    let mut g = game(41);
    let target = put_on_board(&mut g, 0, "Cord Puller");
    g.give(0, "Weapons Forge").unwrap();
    play_card(&mut g, 0, "Weapons Forge");
    let arrows = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).name == "Pointy Arrow")
        .count();
    assert_eq!(arrows, 3);
    let a = hand_pos(&g, 0, "Pointy Arrow");
    act(&mut g, 0, Action::Play { hand: a as u8, pos: 0 });
    act(&mut g, 0, Action::Target(target as u8));
    assert_eq!(stats(&g.players[0].board[target]), (5, 1));
    g.check_invariants().unwrap();
}

#[test]
fn nightmare_par_tea_guest_gets_a_tea_set() {
    let mut g = game(42);
    g.give(0, "Nightmare Par-tea Guest").unwrap();
    play_card(&mut g, 0, "Nightmare Par-tea Guest");
    assert!(g.players[0].hand.iter().any(|m| {
        g.data.cards.get(m.card).name == "Misplaced Tea Set"
    }));
    g.check_invariants().unwrap();
}

#[test]
fn nightmare_par_tea_guest_deathrattle_gets_a_tea_set() {
    let d = db();
    let tea = d.by_name("Misplaced Tea Set").unwrap();
    let a = vec![mk(&d, "Nightmare Par-tea Guest", false)];
    let mut wall = mk(&d, "Microbot", false);
    wall.attack = 100;
    wall.health = 100;
    let b = vec![wall];
    let out = fight_with(
        &d,
        &a,
        &b,
        [Counters::default(); 2],
        10,
        &mut Rng::new(18),
        &mut (),
    );
    assert!(out.sides[0].cards_gained().contains(&tea));
}

// ---- Pointy Arrow in the pool, and Balinda's double casts ----

#[test]
fn pointy_arrow_only_comes_from_weapons_forge() {
    let d = db();
    let arrow = d.by_name("Pointy Arrow").unwrap();
    assert!(!d.spells.contains(&arrow), "Pointy Arrow is not in the spell pool");
    let mut rng = Rng::new(7);
    for _ in 0..1000 {
        assert_ne!(d.random_spell(1, bgcore::Tribes::ALL, &mut rng), Some(arrow));
        assert_ne!(d.random_tier_spell(1, &[], bgcore::Tribes::ALL, &mut rng), Some(arrow));
    }
}

#[test]
fn balinda_casts_robust_evolution_twice() {
    let mut g = game(44);
    put_on_board(&mut g, 0, "Balinda Stonehearth");
    let slot = put_on_board(&mut g, 0, "Wrath Weaver"); // tier 1
    g.players[0].board[slot].attack = 2;
    g.players[0].board[slot].health = 1;
    g.give(0, "Robust Evolution").unwrap();
    play_card(&mut g, 0, "Robust Evolution");
    act(&mut g, 0, Action::Target(slot as u8));
    // Two casts: the second hits what the first produced, in the same slot.
    let m = g.players[0].board[slot];
    assert_eq!(g.data.cards.get(m.card).tier, 3);
    assert_eq!(stats(&m), (2, 1));
    assert!(!m.golden);
    g.check_invariants().unwrap();
}

#[test]
fn balinda_casts_lost_staff_twice_on_a_devilish_distractor() {
    let mut g = game(45);
    put_on_board(&mut g, 0, "Balinda Stonehearth");
    let dd = put_on_board(&mut g, 0, "Devilish Distractor");
    set_shop(&mut g, 0, &[]);
    g.give(0, "Lost Staff of Hamuul").unwrap();
    play_card(&mut g, 0, "Lost Staff of Hamuul");
    act(&mut g, 0, Action::Target(dd as u8));
    // Both casts refreshed the Tavern with Demons, and Devilish Distractor's
    // "whenever you cast a spell on this" went off for each: +2/+4 in all.
    let buff = g.players[0]
        .tavern_buffs
        .iter()
        .find(|b| b.tribe.is_none())
        .copied()
        .expect("Devilish Distractor's Tavern buff");
    assert_eq!((buff.attack, buff.health), (2, 4));
    assert!(!g.players[0].shop.is_empty());
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert!(c.tribes.has(Tribe::Demon));
        assert_eq!((m.attack, m.health), (c.attack + 2, c.health + 4));
    }
    g.check_invariants().unwrap();
}

#[test]
fn balinda_channel_the_devourer_sells_only_once() {
    let mut g = game(46);
    put_on_board(&mut g, 0, "Balinda Stonehearth");
    let tad = put_on_board(&mut g, 0, "Tad");
    put_on_board(&mut g, 0, "Cord Puller");
    let gold = g.players[0].gold;
    let murlocs = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).tribes.has(Tribe::Murloc))
        .count();
    g.give(0, "Channel the Devourer").unwrap();
    play_card(&mut g, 0, "Channel the Devourer");
    act(&mut g, 0, Action::Target(tad as u8));
    // One sale, not two: Tad is gone, one Gold and its one Murloc.
    assert_eq!(g.players[0].gold, gold + 1);
    assert_eq!(g.players[0].board.len(), 2);
    let after = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).tribes.has(Tribe::Murloc))
        .count();
    assert_eq!(after, murlocs + 1);
    g.check_invariants().unwrap();
}

// ---- Stage 2 fixes ----

#[test]
fn chefs_choice_stays_at_the_players_tier_and_needs_a_type() {
    let mut g = game(30);
    let target = put_on_board(&mut g, 0, "Lullabot"); // Mech, tier 1
    g.give(0, "Chef's Choice").unwrap();
    play_card(&mut g, 0, "Chef's Choice");
    act(&mut g, 0, Action::Target(target as u8));
    let h = g.players[0].hand.len() - 1;
    let m = g.players[0].hand[h];
    let card = g.data.cards.get(m.card);
    assert!(card.tier <= g.players[0].tier);
    assert!(card.tribes.has(Tribe::Mech));
    assert_ne!(m.card, id(&g, "Lullabot"));
    g.check_invariants().unwrap();
}

#[test]
fn natural_blessing_picks_one_type_of_a_dual_minion() {
    let mut g = game(31);
    let gearfin = put_on_board(&mut g, 0, "Gearfin"); // Mech and Murloc
    put_on_board(&mut g, 0, "Lullabot"); // Mech
    put_on_board(&mut g, 0, "Tad"); // Murloc
    g.give(0, "Natural Blessing").unwrap();
    play_card(&mut g, 0, "Natural Blessing");
    act(&mut g, 0, Action::Target(gearfin as u8));
    assert_eq!(stats(&g.players[0].board[0]), (8, 6)); // Gearfin +2/+1
    let mech = stats(&g.players[0].board[1]);
    let murloc = stats(&g.players[0].board[2]);
    // Exactly one of the two single-type minions shares the picked type.
    assert!(
        (mech == (4, 3) && murloc == (2, 2)) || (mech == (2, 2) && murloc == (4, 3)),
        "mech {mech:?}, murloc {murloc:?}"
    );
    g.check_invariants().unwrap();
}

#[test]
fn natural_blessing_reaches_the_taverns_minions_of_the_type() {
    // "Give all minions that share a type with it +2/+1" is every such
    // minion, the Tavern's included (Era, 2026-11-02; log 15_25_26-g2 turn 7
    // step 27, where a Tavern Abyssal Envoy gained it).
    let mut g = game(34);
    put_on_board(&mut g, 0, "Lullabot"); // Mech
    set_shop(&mut g, 0, &["Lullabot", "Tad"]); // a Mech, a Murloc
    let before_mech = stats(&g.players[0].shop[0]);
    let before_murloc = stats(&g.players[0].shop[1]);
    g.give(0, "Natural Blessing").unwrap();
    play_card(&mut g, 0, "Natural Blessing");
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (4, 3), "the board's Mech: +2/+1");
    assert_eq!(
        stats(&g.players[0].shop[0]),
        (before_mech.0 + 2, before_mech.1 + 1),
        "the Tavern's Mech gains it too"
    );
    assert_eq!(stats(&g.players[0].shop[1]), before_murloc, "the Murloc does not");
    g.check_invariants().unwrap();
}

#[test]
fn a_same_tribe_buff_reaches_a_minion_through_either_of_its_tribes() {
    let mut g = game(33);
    put_on_board(&mut g, 0, "Gearfin"); // Mech and Murloc
    put_on_board(&mut g, 0, "Lullabot"); // Mech
    put_on_board(&mut g, 0, "Tad"); // Murloc
    g.give(0, "Natural Blessing").unwrap();
    play_card(&mut g, 0, "Natural Blessing");
    act(&mut g, 0, Action::Target(1)); // the Lullabot is a Mech
    // the buff reaches Gearfin through its Mech tribe, never Tad
    assert_eq!(stats(&g.players[0].board[0]), (6 + 2, 5 + 1));
    assert_eq!(stats(&g.players[0].board[1]), (2 + 2, 2 + 1));
    assert_eq!(stats(&g.players[0].board[2]), (2, 2));
    g.check_invariants().unwrap();
}

#[test]
fn patient_scout_improves_and_discovers_its_tier() {
    let mut g = game(32);
    put_on_board(&mut g, 0, "Patient Scout");
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.players[0].board[0].improvements, 2);
    let h = g.players[0].board.len() - 1;
    act(&mut g, 0, Action::Sell(h as u8));
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(
        g.players[0]
            .discover
            .iter()
            .all(|&c| g.data.cards.get(c).tier == 3)
    );
    g.check_invariants().unwrap();
}

/// Cataclysmic Harbinger: "At the end of your turn, get a copy of the last
/// Tavern spell you cast." The one it copies is the last Tavern spell the
/// player cast themselves: a Blood Gem or another non-Tavern spell played
/// after it does not take its place (docs/rules/design.md "Spells versus
/// Tavern spells"), and neither does a spell a minion cast for them ("A
/// spell a minion casts for you").
#[test]
fn cataclysmic_harbinger_copies_the_last_tavern_spell_you_cast() {
    let mut g = game(41);
    put_on_board(&mut g, 0, "Cataclysmic Harbinger");
    let target = put_on_board(&mut g, 0, "Cord Puller");
    let trousers = id(&g, "Tricky Trousers");
    let gem = id(&g, "Blood Gem");
    g.give(0, "Tricky Trousers").unwrap();
    play_card(&mut g, 0, "Tricky Trousers");
    act(&mut g, 0, Action::Target(target as u8));
    // A Blood Gem played after it is a spell, but not a Tavern spell.
    g.give(0, "Blood Gem").unwrap();
    play_card(&mut g, 0, "Blood Gem");
    act(&mut g, 0, Action::Target(target as u8));
    let count = |g: &Game, c: CardId| g.players[0].hand.iter().filter(|m| m.card == c).count();
    let before = count(&g, trousers);
    assert_eq!(count(&g, gem), 0, "the Gem was played, not kept");
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(
        count(&g, trousers),
        before + 1,
        "the end of the turn copies the Tricky Trousers, the last Tavern spell"
    );
    assert_eq!(count(&g, gem), 0, "the Gem is not a Tavern spell to copy");
    g.check_invariants().unwrap();
}
