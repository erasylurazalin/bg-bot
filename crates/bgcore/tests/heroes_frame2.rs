//! Hero frame 2 (docs/briefs/heroes-frame2.md): the second hero power, the
//! powers a hero only reaches later (Sulfuras, the four Invocations,
//! Galewing's flightpaths, the King of X family), the skipped turns of A. F.
//! Kay and Faelin, and Mukla's Bananas.

use std::sync::Arc;

use bgcore::fight::{fight_with, Counters};
use bgcore::{Action, CardId, Data, Game, Keywords, Minion, PowerState, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn lobby(ts: &[Tribe]) -> Tribes {
    ts.iter().copied().fold(Tribes::NONE, |m, t| m.with(t))
}

/// A no-pick game with every tribe in the lobby, player 0 forced to `hero`.
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

fn power_id(g: &Game, card_id: &str) -> u8 {
    g.data.heroes.power_by_card_id(card_id).expect(card_id)
}

/// A fresh power state for `of`, built on an existing one (the fields are
/// public, so no game path is needed to set up a second power).
fn make_power(g: &Game, of: u8) -> PowerState {
    let mut st = g.players[0].power.expect("the player needs some power first");
    st.of = of;
    st.uses_left = g.data.heroes.power(of).uses_per_turn;
    st.used_game = 0;
    st.progress = 0;
    st.last_pick = 0;
    st.unlocked = true;
    st
}

/// Answer whatever the current player has open (a Discover, a power offer, a
/// target, a discard, a trinket offer), so a helper that only wants the next
/// turn never stalls on a prompt. False when nothing is open.
fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let open = pl.hero_offer.iter().any(|&h| h != u8::MAX)
        || !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || !pl.secret_offer.is_empty()
        || pl.wager_offer.is_some()
        || pl.holmes_offer.is_some()
        || pl.pending_choose_one.is_some()
        || pl.pending_discard.is_some()
        || pl.pending_shop_target.is_some()
        || pl.pending_target.is_some()
        || !pl.trinket_offer.is_empty();
    if !open {
        return false;
    }
    // Every answer to every prompt is tried, in an order that lets a target
    // be picked before a Discover that is queued behind it (the engine only
    // allows the target while one is open) and lets a Tavern minion in any
    // slot be the target (Alliance Flag on the Tavern), which the old
    // fixed list of two actions could not.
    let mut cands: Vec<Action> = Vec::new();
    for s in 0..bgcore::BOARD as u8 {
        cands.push(Action::Target(s));
    }
    // One past the cards: a card steal or a power may take the spell slot.
    for s in 0..=bgcore::SHOP as u8 {
        cands.push(Action::TargetShop(s));
    }
    cands.push(Action::Discard(0));
    cands.push(Action::Choose(0));
    cands.push(Action::Choose(1));
    cands.push(Action::ChooseTrinket(0));
    for a in cands {
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

fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
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

/// Empty the hand, pool copies back (a plain `clear()` would break the exact
/// pool accounting the tests check).
fn clear_hand(g: &mut Game, p: usize) {
    for m in g.players[p].hand.drain(..) {
        let c = g.data.cards.get(m.card);
        if m.pool_copies > 0 && !c.token && !c.is_spell() {
            g.pool[c.pool_slot as usize] += m.pool_copies;
        }
    }
}

fn power_name(g: &Game, p: usize) -> String {
    let st = g.players[p].power.expect("no power");
    g.data.heroes.power(st.of).name.clone()
}

// ---- the data ----

#[test]
fn heroes_toml_parses_with_every_hero_and_the_later_powers() {
    let d = Data::bundled();
    assert_eq!(d.heroes.heroes.len(), 110);
    // Every power is its own entry, and a hero's starting power is its own
    // index, so a hero and its starting power are the same number.
    for (i, h) in d.heroes.heroes.iter().enumerate() {
        assert_eq!(h.power as usize, i, "{} starts with its own power", h.name);
    }
    // Sulfuras is reached from BUY, INSECT!, not started with.
    let sulfuras = d.heroes.power_by_card_id("TB_BaconShop_HP_087t").expect("Sulfuras");
    assert_eq!(d.heroes.power(sulfuras).name, "Sulfuras");
    let rag = d.heroes.hero_power(d.heroes.by_name("Ragnaros the Firelord").unwrap());
    match rag.effect {
        bgcore::PowerEffect::BuyMilestone { count, becomes } => {
            assert_eq!(count, 12);
            assert_eq!(becomes, sulfuras);
        }
        other => panic!("{other:?}"),
    }
    // Bru'kan's chooser lists the four Invocations, each of which comes back
    // to it.
    let brukan = d.heroes.hero_power(d.heroes.by_name("Bru'kan").unwrap());
    assert_eq!(brukan.options.len(), 4);
    assert_eq!(brukan.effect, bgcore::PowerEffect::ChooseSubPowers);
    // Galewing's lists three flightpaths.
    let galewing = d.heroes.hero_power(d.heroes.by_name("Galewing").unwrap());
    assert_eq!(galewing.options.len(), 3);
    // The powers reached later: their name, their chooser, no button.
    for (name, card, parent_card, turns) in [
        ("Earth Invocation", "BG22_HERO_001p_t1", "BG22_HERO_001p", 0),
        ("Fire Invocation", "BG22_HERO_001p_t2", "BG22_HERO_001p", 0),
        ("Water Invocation", "BG22_HERO_001p_t3", "BG22_HERO_001p", 0),
        ("Lightning Invocation", "BG22_HERO_001p_t4", "BG22_HERO_001p", 0),
        ("Westfall", "BG20_HERO_283p_t1", "BG20_HERO_283p", 1),
        ("Ironforge", "BG20_HERO_283p_t2", "BG20_HERO_283p", 2),
        ("Eastern Plaguelands", "BG20_HERO_283p_t3", "BG20_HERO_283p", 3),
    ] {
        let of = d.heroes.power_by_card_id(card).unwrap_or_else(|| panic!("{name}"));
        let p = d.heroes.power(of);
        assert_eq!(p.name, name);
        assert!(p.passive, "{name} has no button");
        assert_eq!(p.complete_turns, turns, "{name}");
        let parent = p.parent.expect("a chooser");
        assert_eq!(d.heroes.power(parent).card_id, parent_card, "{name} comes back to its chooser");
    }
    // The King of X family: ten powers, one per tribe in the game, all in the
    // same family, and none of them a hero's starting power.
    let rat = d.heroes.hero_power(d.heroes.by_name("The Rat King").unwrap());
    assert_eq!(rat.name, "A Tale of Kings");
    let family = rat.family;
    assert_ne!(family, 0, "the Rat King's power is a family");
    let members = d.heroes.family_options(family, Tribes::ALL, None);
    assert_eq!(members.len(), 10);
    for &m in &members {
        let p = d.heroes.power(m);
        assert!(p.tribe_of().is_some(), "{} names its tribe", p.name);
        assert_eq!(p.cost, 2);
        assert!(!d.heroes.heroes.iter().any(|h| h.power == m), "{} is not a starting power", p.name);
    }
    // The skipped turns and the Discovers that end them.
    let kay = d.heroes.hero_power(d.heroes.by_name("A. F. Kay").unwrap());
    assert_eq!(kay.skip_turns, 2);
    assert_eq!(
        kay.effect,
        bgcore::PowerEffect::TurnDiscovers { turn: 3, tiers: [3, 4, 0] }
    );
    let faelin = d.heroes.hero_power(d.heroes.by_name("Ambassador Faelin").unwrap());
    assert_eq!(faelin.skip_turns, 1);
    assert_eq!(
        faelin.effect,
        bgcore::PowerEffect::TurnDiscovers { turn: 1, tiers: [2, 4, 6] },
        "the game starts with his three Discovers, before turn 1 does anything"
    );
    // Mukla's Bananas are plain spells, not the Tavern dish Banana.
    let mukla = d.heroes.hero_power(d.heroes.by_name("King Mukla").unwrap());
    match mukla.effect {
        bgcore::PowerEffect::Bananarama { mine, others, big_percent, banana, big } => {
            assert_eq!((mine, others, big_percent), (2, 1, 33));
            let b = d.cards.get(banana);
            assert_eq!(b.name, "Bananas");
            assert!(b.is_spell());
            assert!(!b.tavern_spell, "a Banana is a spell, not a Tavern spell");
            assert_eq!(d.cards.get(big).name, "Big Banana");
            assert!(!d.cards.get(big).tavern_spell);
        }
        other => panic!("{other:?}"),
    }
    // The Elemental Earth Invocation's Deathrattle summons is in the data.
    let earth = d.heroes.power(d.heroes.power_by_card_id("BG22_HERO_001p_t1").unwrap());
    match earth.effect {
        bgcore::PowerEffect::SocGrantDeathrattle { count, card } => {
            assert_eq!(count, 4);
            let c = d.cards.get(card);
            assert_eq!((c.name.as_str(), c.attack, c.health), ("Stone Elemental", 1, 1));
        }
        other => panic!("{other:?}"),
    }
    // Reinvigorating Light is a free lesser trinket.
    let t = d.trinkets.by_name("Reinvigorating Light").expect("the trinket");
    assert_eq!(d.trinkets.get(t).slot, bgcore::trinkets::Slot::Lesser);
    assert_eq!(d.trinkets.get(t).cost, 0);
}

// ---- two powers ----

#[test]
fn genn_discovers_two_powers_on_turn_4_the_first_into_the_second_slot() {
    let mut g = hero_game(3, "Genn, Worgen King");
    assert_eq!(power_name(&g, 0), "King of Duality");
    // Turns 1 to 3: the button does nothing.
    for turn in 1..=3 {
        assert_eq!(g.turn, turn);
        turn_of(&mut g, 0);
        assert!(!g.is_legal(Action::HeroPower), "turn {turn}");
        end_all_turns(&mut g);
    }
    assert_eq!(g.turn, 4);
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower), "turn 4 unlocks it");
    let main_before = g.players[0].power.expect("a power").of;
    act(&mut g, 0, Action::HeroPower);
    // The first pick becomes the SECOND power...
    assert!(!g.players[0].power_offer.is_empty(), "a Hero Power Discover opened");
    assert!(g.players[0].power_offer_second, "and it fills the second slot");
    let first = g.players[0].power_offer[0];
    // The same exclusion list as Unmasked Identity (cards.toml).
    for &of in g.players[0].power_offer.clone().iter() {
        let hero = g.data.heroes.heroes.iter().find(|h| h.power == of).expect("a starting power");
        assert!(
            !["TB_BaconShop_HERO_40", "TB_BaconShop_HERO_16", "BG22_HERO_201", "TB_BaconShop_HERO_34"]
                .contains(&hero.card_id.as_str()),
            "{} makes no sense as a found power",
            hero.name
        );
    }
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].power2.expect("the second power").of, first);
    assert_eq!(g.players[0].power.expect("the main power").of, main_before, "the main one waits");
    // ... and the second pick the MAIN one.
    assert!(!g.players[0].power_offer.is_empty(), "the second Discover opened");
    assert!(!g.players[0].power_offer_second, "which fills the main slot");
    let second = g.players[0].power_offer[0];
    assert_ne!(second, first, "never a power the player already holds");
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].power.expect("the main power").of, second);
    assert_eq!(g.players[0].power2.expect("the second power").of, first);
    assert_eq!(g.players[0].power_offer_left, 0, "the power is spent");
    assert!(!g.is_legal(Action::HeroPower), "once per game");
    g.check_invariants().unwrap();
}

#[test]
fn the_second_power_is_its_own_button_with_its_own_cost_and_uses() {
    let mut g = hero_game(4, "George the Fallen");
    put_on_board(&mut g, 0, "Flittering Bat");
    g.players[0].gold = 10;
    assert_eq!(power_name(&g, 0), "Boon of Light");
    // A second power by hand, the way Genn's and the trinket's picks land.
    let kings = power_id(&g, "TB_BaconShop_HP_041a");
    g.players[0].power2 = Some(make_power(&g, kings));
    assert!(g.power_usable2(0), "the second power has its own button");
    act(&mut g, 0, Action::HeroPower2);
    assert_eq!(g.players[0].gold, 8, "the King's own price is 2");
    assert_eq!(g.players[0].power2.expect("the second power").used_game, 1);
    assert_eq!(g.players[0].power.expect("the main power").used_game, 0, "the main one is untouched");
    assert!(!g.is_legal(Action::HeroPower2), "one use a turn");
    // Its own Discover is open now: a minion of the King's tribe.
    assert!(!g.players[0].discover.is_empty());
    act(&mut g, 0, Action::Choose(0));
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower), "the main power is still usable");
    g.check_invariants().unwrap();
}

#[test]
fn reinvigorating_light_gives_a_second_power_and_three_gold() {
    let mut g = hero_game(5, "George the Fallen");
    let light = g.data.trinkets.by_name("Reinvigorating Light").expect("Reinvigorating Light");
    // Take it the way an offer does: its own slot, its own price (0), the
    // option holding a plain index into the trinket table.
    g.players[0].gold = 0;
    g.players[0].trinket_offer.push((light, 0));
    g.players[0].trinket_offer_slot = 1;
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::ChooseTrinket(0)));
    act(&mut g, 0, Action::ChooseTrinket(0));
    assert_eq!(g.players[0].gold, 3, "gain 3 Gold");
    assert_eq!(power_name(&g, 0), "Boon of Light", "the main power stays");
    assert!(!g.players[0].power_offer.is_empty(), "the second power Discover opened");
    assert!(g.players[0].power_offer_second);
    let pick = g.players[0].power_offer[0];
    assert_ne!(pick, g.players[0].power.expect("a power").of, "never the power it has");
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].power2.expect("the second power").of, pick);
    assert_eq!(power_name(&g, 0), "Boon of Light");
    g.check_invariants().unwrap();
}

#[test]
fn unmasked_identity_changes_only_the_main_power() {
    let mut g = hero_game(17, "George the Fallen");
    let kings = power_id(&g, "TB_BaconShop_HP_041a");
    g.players[0].power2 = Some(make_power(&g, kings));
    let ui = id(&g, "Unmasked Identity");
    let m = g.players[0].make_card(&g.data.cards, ui, false, 0);
    g.players[0].hand.push(m);
    g.players[0].gold = 6;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert!(!g.players[0].power_offer.is_empty(), "the Discover opened");
    assert!(!g.players[0].power_offer_second, "it changes the main power");
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].power2.expect("the second power").of, kings, "the second one stays");
    g.check_invariants().unwrap();
}

// ---- powers that turn into others ----

#[test]
fn ragnaros_turns_into_sulfuras_after_twelve_bought_cards() {
    let mut g = hero_game(6, "Ragnaros the Firelord");
    assert_eq!(power_name(&g, 0), "BUY, INSECT!");
    put_on_board(&mut g, 0, "Flittering Bat");
    // Eleven minions bought: still BUY, INSECT!.
    for i in 0..11 {
        let name = if i % 2 == 0 { "Buzzing Vermin" } else { "Cord Puller" };
        set_shop(&mut g, 0, &[name]);
        g.players[0].gold = 5;
        clear_hand(&mut g, 0);
        act(&mut g, 0, Action::Buy(0));
        assert_eq!(power_name(&g, 0), "BUY, INSECT!", "after {i} buys");
    }
    assert_eq!(g.players[0].power.expect("a power").progress, 11);
    // The twelfth is a Tavern spell, and it counts.
    g.players[0].shop_spell = Some(id(&g, "Tavern Coin"));
    g.players[0].gold = 5;
    clear_hand(&mut g, 0);
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(power_name(&g, 0), "Sulfuras", "spells count");
    // Sulfuras fires at the end of the same turn.
    let before = g.players[0].board[0];
    end_all_turns(&mut g);
    let got = g.players[0].board[0];
    assert_eq!(
        (got.attack - before.attack, got.health - before.health),
        (16, 16),
        "one minion is both ends, so it gets both buffs"
    );
}

#[test]
fn only_bought_cards_count_towards_buy_insect() {
    let mut g = hero_game(7, "Ragnaros the Firelord");
    // A card that arrives without a buy (a steal, a generated card) does not
    // come through the buy path, so the count stays where it was.
    clear_hand(&mut g, 0);
    assert!(g.give(0, "Buzzing Vermin").is_some());
    assert_eq!(power_name(&g, 0), "BUY, INSECT!");
    assert_eq!(g.players[0].power.expect("a power").progress, 0, "a steal is not a buy");
    // A free buy is still a buy, and a Tavern spell counts.
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].tp.free_buys = 1;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].power.expect("a power").progress, 1, "a free buy counts");
}

#[test]
fn brukan_chooses_an_element_and_the_chooser_comes_back_after_the_fight() {
    let mut g = hero_game(8, "Bru'kan");
    assert_eq!(power_name(&g, 0), "Embrace the Elements");
    put_on_board(&mut g, 0, "Flittering Bat");
    // The choice is a fixed list of four, not a random Discover.
    act(&mut g, 0, Action::HeroPower);
    let offered: Vec<u8> = g.players[0].power_offer.to_vec();
    assert_eq!(offered.len(), 4, "the four Elements");
    let names: Vec<String> = offered.iter().map(|&o| g.data.heroes.power(o).name.clone()).collect();
    for want in ["Earth Invocation", "Fire Invocation", "Water Invocation", "Lightning Invocation"] {
        assert!(names.contains(&want.to_string()), "{want} missing from {names:?}");
    }
    let earth = offered
        .iter()
        .position(|&o| g.data.heroes.power(o).name == "Earth Invocation")
        .unwrap();
    act(&mut g, 0, Action::Choose(earth as u8));
    assert_eq!(power_name(&g, 0), "Earth Invocation");
    assert!(!g.is_legal(Action::HeroPower), "the Invocation has no button");
    // The fight reads it as a Start of Combat counter.
    let counters = g.players[0].counters(&g.data, g.lobby);
    assert_eq!(counters.soc_earth, 4);
    assert_eq!(g.data.cards.get(counters.soc_earth_card).name, "Stone Elemental");
    // After the fight the chooser is back, and a new Element can be chosen.
    end_all_turns(&mut g);
    assert_eq!(power_name(&g, 0), "Embrace the Elements", "the button comes back");
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower));
    g.check_invariants().unwrap();
}

#[test]
fn the_fire_invocation_doubles_the_leftmost_minions_attack() {
    let db = &Data::bundled().cards;
    let mk = |name: &str| db.make(db.by_name(name).expect(name), false);
    // Two 2/3s on my side, one 2/3 against them: with the Fire Invocation my
    // left-most minion becomes 4/3, kills the 2/3 outright and the fight is
    // won at the first swing. Without it both live.
    let board = [mk("Clever Castaway"), mk("Clever Castaway")];
    let enemy = [mk("Clever Castaway")];
    let fire = Counters { soc_double_left: 1, ..Counters::default() };
    let mut rng = bgcore::Rng::new(31);
    let out = fight_with(db, &board, &enemy, [fire, Counters::default()], 1, &mut rng, &mut ());
    assert_eq!(out.winner, Some(0), "the doubled left-most minion killed the enemy");
    let mut rng = bgcore::Rng::new(31);
    let out = fight_with(db, &board, &enemy, [Counters::default(); 2], 1, &mut rng, &mut ());
    assert_eq!(out.winner, None, "2 Attack does not kill a 2/3");
}

#[test]
fn the_water_invocation_shields_the_rightmost_minion() {
    let db = &Data::bundled().cards;
    let mk = |name: &str| db.make(db.by_name(name).expect(name), false);
    // The enemy attacks first (two minions against my one), and its 3 Attack
    // kills my 2/3 in one swing. With the shield on the right-most (my only)
    // minion the blow is absorbed and my side is still there after it.
    let mine = [mk("Clever Castaway")];
    let enemy = [mk("Zoatroid"), mk("Zoatroid")]; // 3/2
    let water = Counters {
        soc_rightmost: Keywords::DIVINE_SHIELD | Keywords::TAUNT,
        ..Counters::default()
    };
    let mut rng = bgcore::Rng::new(32);
    let out = fight_with(db, &mine, &enemy, [water, Counters::default()], 1, &mut rng, &mut ());
    assert_eq!(out.winner, None, "with the shield my minion survives the swing");
    let mut rng = bgcore::Rng::new(32);
    let out = fight_with(db, &mine, &enemy, [Counters::default(); 2], 1, &mut rng, &mut ());
    assert_eq!(out.winner, Some(1), "without it the swing kills it");
}

#[test]
fn the_earth_invocation_deathrattle_summons_an_elemental() {
    let db = &Data::bundled().cards;
    let mk = |name: &str| db.make(db.by_name(name).expect(name), false);
    let stone = db.by_name("Stone Elemental").expect("Stone Elemental");
    let mine = [mk("Clever Castaway")]; // the only minion to grant to
    let enemy = [
        {
            let mut m = mk("Zoatroid");
            m.attack = 10;
            m
        },
        mk("Zoatroid"),
    ];
    let earth = Counters { soc_earth: 4, soc_earth_card: stone, ..Counters::default() };
    // Without the Deathrattle: one swing kills my 1/1 and the fight is over.
    let mut rng = bgcore::Rng::new(33);
    let out = fight_with(db, &mine, &enemy, [Counters::default(); 2], 40, &mut rng, &mut ());
    assert_eq!(out.attacks, 1);
    assert_eq!(out.winner, Some(1));
    // With it: the Elemental has to be killed too, so there is one more swing.
    let mut rng = bgcore::Rng::new(33);
    let out = fight_with(db, &mine, &enemy, [earth, Counters::default()], 40, &mut rng, &mut ());
    assert_eq!(out.attacks, 2, "the Elemental stood in the way");
    assert_eq!(out.winner, Some(1));
}

#[test]
fn the_lightning_invocation_hits_five_different_minions() {
    let db = &Data::bundled().cards;
    let mk = |name: &str| db.make(db.by_name(name).expect(name), false);
    let mine = [{
        let mut m = mk("Treasure Parrot");
        m.attack = 10;
        m
    }];
    let enemy: Vec<Minion> = (0..5).map(|_| mk("Mechagnome Interpreter")).collect();
    let bolt = Counters { soc_lightning: 5, soc_lightning_damage: 1, ..Counters::default() };
    // All five take a damage and die at 1 Health, right then: the fight is
    // over before a single attack.
    let mut rng = bgcore::Rng::new(34);
    let out = fight_with(db, &mine, &enemy, [bolt, Counters::default()], 40, &mut rng, &mut ());
    assert_eq!(out.winner, Some(0));
    assert_eq!(out.attacks, 0, "they die at Start of Combat, not at the next attack");
    // Without it, five 1/1s have to be killed one at a time.
    let mut rng = bgcore::Rng::new(34);
    let out = fight_with(db, &mine, &enemy, [Counters::default(); 2], 40, &mut rng, &mut ());
    assert!(out.attacks > 1, "{}", out.attacks);
}

#[test]
fn the_lightning_invocations_kill_is_a_real_death_with_deathrattles() {
    let db = &Data::bundled().cards;
    let mk = |name: &str| db.make(db.by_name(name).expect(name), false);
    // The enemy's Lightning hits my 2/1 Rescue Bot, whose Deathrattle gives a
    // Repair Job: at 0 attacks the fight is over, so the card can only be
    // there if the minion died at Start of Combat.
    let mine = [mk("Rescue Bot")];
    let enemy = [mk("Clever Castaway")];
    let bolt = Counters { soc_lightning: 1, soc_lightning_damage: 1, ..Counters::default() };
    let mut rng = bgcore::Rng::new(35);
    let out = fight_with(db, &mine, &enemy, [Counters::default(), bolt], 0, &mut rng, &mut ());
    assert_eq!(out.attacks, 0);
    assert_eq!(out.sides[0].n_cards, 1, "the Deathrattle ran: {:?}", out.sides[0].cards);
    assert_eq!(db.get(out.sides[0].cards[0]).name, "Repair Job");
    assert_eq!(out.winner, Some(1), "my board is gone");
    // Without the Invocation nothing dies and no card is given.
    let mut rng = bgcore::Rng::new(35);
    let out = fight_with(db, &mine, &enemy, [Counters::default(); 2], 0, &mut rng, &mut ());
    assert_eq!(out.sides[0].n_cards, 0);
    assert_eq!(out.winner, None);
}

#[test]
fn galewing_completes_a_flightpath_and_cannot_repeat_it() {
    let mut g = hero_game(10, "Galewing");
    assert_eq!(power_name(&g, 0), "Dungar's Gryphon");
    act(&mut g, 0, Action::HeroPower);
    let offered: Vec<u8> = g.players[0].power_offer.to_vec();
    assert_eq!(offered.len(), 3, "three flightpaths");
    let westfall =
        offered.iter().position(|&o| g.data.heroes.power(o).name == "Westfall").unwrap();
    act(&mut g, 0, Action::Choose(westfall as u8));
    assert_eq!(power_name(&g, 0), "Westfall");
    // "In 1 turn": the next turn start completes it.
    clear_hand(&mut g, 0);
    end_all_turns(&mut g);
    assert_eq!(power_name(&g, 0), "Dungar's Gryphon", "the chooser is back");
    assert_eq!(g.players[0].hand.len(), 1, "a random 1-Cost Tavern spell");
    let got = g.data.cards.get(g.players[0].hand[0].card);
    assert_eq!(got.cost, 1);
    assert!(got.tavern_spell);
    // The path just completed is not offered again.
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower));
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].power_offer.len(), 2, "the other two only");
    for &o in &g.players[0].power_offer {
        assert_ne!(g.data.heroes.power(o).name, "Westfall");
    }
    // Ironforge: two turns, then 2 Gold.
    let iron = g.players[0]
        .power_offer
        .iter()
        .position(|&o| g.data.heroes.power(o).name == "Ironforge")
        .unwrap();
    act(&mut g, 0, Action::Choose(iron as u8));
    end_all_turns(&mut g);
    assert_eq!(power_name(&g, 0), "Ironforge", "one turn of the two is left");
    g.players[0].gold = 0;
    g.players[0].free_rolls = 0;
    end_all_turns(&mut g);
    assert_eq!(power_name(&g, 0), "Dungar's Gryphon");
    assert_eq!(
        g.players[0].gold,
        g.data.rules.gold(g.turn) + 2,
        "+2 Gold on top of the turn's own"
    );
}

#[test]
fn the_eastern_plaguelands_discover_a_minion_of_your_tier() {
    let mut g = hero_game(18, "Galewing");
    let of = power_id(&g, "BG20_HERO_283p_t3");
    g.players[0].power = Some(make_power(&g, of));
    g.players[0].power.as_mut().unwrap().progress = 2;
    g.players[0].tier = 3;
    end_all_turns(&mut g);
    assert_eq!(power_name(&g, 0), "Dungar's Gryphon", "it completed");
    turn_of(&mut g, 0);
    assert!(!g.players[0].discover.is_empty(), "a Discover is open");
    for &c in &g.players[0].discover.clone() {
        assert_eq!(g.data.cards.get(c).tier, 3);
    }
}

#[test]
fn rat_king_swaps_to_a_lobby_tribe_every_turn_and_never_twice() {
    let d = data();
    let tribes = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon]);
    let mut g = Game::new_with_lobby(d.clone(), 11, tribes);
    let h = d.heroes.by_name("The Rat King").expect("the Rat King");
    g.assign_hero(0, h);
    let mut seen: Vec<Tribe> = Vec::new();
    for _ in 0..12 {
        let of = g.players[0].power.expect("a King power").of;
        let power = g.data.heroes.power(of);
        let tribe = power.tribe_of().expect("a King of X power");
        assert!(
            !seen.last().is_some_and(|&t| t == tribe),
            "{} followed {}",
            power.name,
            g.data.heroes.power(g.players[0].power.unwrap().of).name
        );
        assert!(g.lobby.has(tribe), "{} is not in the lobby", power.name);
        assert_eq!(power.cost, 2, "the King's price");
        seen.push(tribe);
        end_all_turns(&mut g);
    }
    // Every tribe of the lobby comes up, and no other.
    let mut kinds: Vec<String> = seen.iter().map(|t| format!("{t:?}")).collect();
    kinds.sort();
    kinds.dedup();
    assert_eq!(kinds, ["Beast", "Dragon", "Mech"], "{seen:?}");
}

// ---- skipped turns and locked cards ----

#[test]
fn af_kay_skips_two_turns_then_discovers_a_tier_3_and_a_tier_4_minion() {
    let mut g = hero_game(12, "A. F. Kay");
    assert_eq!(power_name(&g, 0), "Procrastinate");
    for turn in 1..=2 {
        assert_eq!(g.turn, turn);
        turn_of(&mut g, 0);
        // Only Freeze and End Turn: no buying, Refreshing, levelling or
        // selling.
        assert!(g.is_legal(Action::Freeze), "turn {turn}");
        assert!(g.is_legal(Action::EndTurn), "turn {turn}");
        assert!(!g.is_legal(Action::Roll), "turn {turn}");
        assert!(!g.is_legal(Action::Level), "turn {turn}");
        for s in 0..g.players[0].shop.len() as u8 {
            assert!(!g.is_legal(Action::Buy(s)), "turn {turn}");
        }
        // Freezing is allowed and does not end the turn.
        act(&mut g, 0, Action::Freeze);
        assert_eq!(g.turn, turn, "the turn is still open");
        end_all_turns(&mut g);
    }
    assert_eq!(g.turn, 3);
    turn_of(&mut g, 0);
    // Turn 3 starts with two Discovers, one from Tier 3 and one from Tier 4.
    let mut tiers: Vec<u8> = Vec::new();
    let mut guard = 0;
    while !g.players[0].discover.is_empty() {
        let opts: Vec<u16> = g.players[0].discover.to_vec();
        for &c in &opts {
            assert_eq!(g.data.cards.get(c).tier, tiers.len() as u8 + 3, "{opts:?}");
        }
        tiers.push(g.data.cards.get(opts[0]).tier);
        act(&mut g, 0, Action::Choose(0));
        guard += 1;
        assert!(guard < 10);
    }
    assert_eq!(tiers, [3, 4]);
    assert_eq!(g.players[0].hand.len(), 2, "both are in hand");
    // And the player shops again.
    assert!(g.is_legal(Action::EndTurn));
    g.check_invariants().unwrap();
}

#[test]
fn faelin_skips_turn_one_and_his_tier_minions_wait_for_their_tier() {
    let mut g = hero_game(13, "Ambassador Faelin");
    assert_eq!(power_name(&g, 0), "Expedition Plans");
    turn_of(&mut g, 0);
    assert_eq!(g.turn, 1);
    assert!(!g.is_legal(Action::Roll), "turn 1 is skipped");
    // The three Discovers are open on that skipped turn, and all three ways
    // out are legal: answer them, or freeze, or end the turn.
    assert!(!g.players[0].discover.is_empty(), "the Discovers open on turn 1");
    assert!(g.is_legal(Action::Choose(0)), "a Discover is answerable on a skipped turn");
    assert!(g.is_legal(Action::Freeze));
    assert!(g.is_legal(Action::EndTurn), "which answers the Discovers");
    let mut mask = [false; bgcore::N_ACTIONS];
    g.legal_mask(&mut mask);
    assert!(mask[Action::Choose(0).index()] && mask[Action::Freeze.index()]);
    assert!(mask[Action::EndTurn.index()]);
    assert!(!mask[Action::Roll.index()] && !mask[Action::Level.index()]);
    // Three Discovers, from Tiers 2, 4 and 6.
    let mut tiers: Vec<u8> = Vec::new();
    let mut guard = 0;
    while !g.players[0].discover.is_empty() {
        let c = g.players[0].discover[0];
        tiers.push(g.data.cards.get(c).tier);
        act(&mut g, 0, Action::Choose(0));
        guard += 1;
        assert!(guard < 10);
    }
    assert_eq!(tiers, [2, 4, 6]);
    assert_eq!(g.players[0].hand.len(), 3);
    for m in &g.players[0].hand {
        assert!(m.locked_tier > 0, "every one of them waits for its tier");
    }
    // Turn 1 is still spent: no shopping with them yet.
    assert!(!g.is_legal(Action::Play { hand: 0, pos: 0 }), "turn 1 is skipped");
    end_all_turns(&mut g);
    assert_eq!(g.turn, 2);
    turn_of(&mut g, 0);
    // At Tier 1 none of them can be played.
    assert!(!g.is_legal(Action::Play { hand: 0, pos: 0 }), "the Tier 3 card waits");
    // A locked card can be buffed in hand: Twilight Tidehunter's spell.
    put_on_board(&mut g, 0, "Twilight Tidehunter");
    let before = g.players[0].hand[0];
    let fortify = id(&g, "Fortify");
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, fortify, false, 0));
    g.players[0].gold = 5;
    let hand_slot = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: hand_slot, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    let after = g.players[0].hand[0];
    assert_eq!(after.card, before.card);
    assert_eq!(
        (after.attack - before.attack, after.health - before.health),
        (9, 9),
        "the locked minion is buffed in hand"
    );
    assert!(after.locked_tier > 0, "and stays locked");
    // Level to Tier 2: the Tier 2 one can be played, the Tier 4 one cannot.
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Level);
    assert_eq!(g.players[0].tier, 2);
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::Play { hand: 0, pos: 0 }), "the Tier 2 one unlocks");
    assert!(!g.is_legal(Action::Play { hand: 1, pos: 0 }), "the Tier 4 one still waits");
    g.check_invariants().unwrap();
}

#[test]
fn mukla_gives_two_bananas_to_himself_and_one_to_everyone_else() {
    let mut g = hero_game(14, "King Mukla");
    assert_eq!(power_name(&g, 0), "Bananarama");
    for p in 0..8 {
        clear_hand(&mut g, p);
    }
    end_all_turns(&mut g);
    let counts: Vec<usize> = (0..8).map(|p| g.players[p].hand.len()).collect();
    assert_eq!(counts[0], 2, "2 Bananas to Mukla");
    for (p, &n) in counts.iter().enumerate().skip(1) {
        assert_eq!(n, 1, "one to every other player, player {p}");
    }
    // Each Banana is the plain one or the Big one, never the Tavern dish.
    for p in 0..8 {
        for m in &g.players[p].hand {
            let c = g.data.cards.get(m.card);
            assert!(c.is_spell());
            assert!(!c.tavern_spell, "a Banana is not a Tavern spell");
            match c.name.as_str() {
                "Bananas" | "Big Banana" => {}
                other => panic!("{other} is not a Banana"),
            }
        }
    }
    // Over many turns about a third are Big Bananas.
    let (mut big, mut small) = (0, 0);
    for _ in 0..20 {
        for p in 0..8 {
            clear_hand(&mut g, p);
        }
        end_all_turns(&mut g);
        for p in 0..8 {
            for m in &g.players[p].hand {
                // Only the Bananas count: from turn 6 on a trinket can hand a
                // player a card of its own.
                match g.data.cards.get(m.card).name.as_str() {
                    "Big Banana" => big += 1,
                    "Bananas" => small += 1,
                    _ => {}
                }
            }
        }
    }
    let share = big as f64 / (big + small) as f64;
    assert!((0.2..0.45).contains(&share), "{big} of {} were big", big + small);
    g.check_invariants().unwrap();
}

#[test]
fn a_skipped_player_still_fights_and_takes_damage() {
    // Pairings are random, so over several skipped turns everyone with an
    // empty board has to lose one: A. F. Kay takes damage on turns 1 and 2.
    for seed in 0..8 {
        let mut g = hero_game(15 + seed, "A. F. Kay");
        for p in 1..8 {
            put_on_board(&mut g, p, "Treasure Parrot");
            g.players[p].tier = 2;
        }
        let before = (g.players[0].health, g.players[0].armor);
        turn_of(&mut g, 0);
        assert!(!g.is_legal(Action::Roll), "turn 1 is skipped");
        end_all_turns(&mut g);
        turn_of(&mut g, 0);
        assert!(!g.is_legal(Action::Roll), "turn 2 is skipped");
        end_all_turns(&mut g);
        let after = (g.players[0].health, g.players[0].armor);
        assert!(
            after.0 < before.0 || after.1 < before.1,
            "seed {seed}: the skipped turns fought {before:?} -> {after:?}"
        );
    }
}

// ---- the observation and the action space ----

#[test]
fn the_observation_shows_the_powers_on_offer() {
    let mut g = hero_game(19, "Bru'kan");
    let mut out = vec![0f32; bgcore::obs::OBS_SIZE];
    turn_of(&mut g, 0);
    bgcore::obs::write(&g, 0, &mut out);
    for k in 0..bgcore::CHOICES {
        assert_eq!(out[bgcore::obs::SCALAR_POWER_OFFER + k], 0.0, "nothing on offer yet");
    }
    assert_eq!(out[bgcore::obs::SCALAR_PENDING_POWER], 0.0);
    // The chooser offers its four Elements: each is in the observation, power
    // index + 1, one per Choose slot.
    act(&mut g, 0, Action::HeroPower);
    let offered: Vec<u8> = g.players[0].power_offer.to_vec();
    assert_eq!(offered.len(), bgcore::CHOICES);
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_PENDING_POWER], 1.0);
    for k in 0..bgcore::CHOICES {
        assert_eq!(
            out[bgcore::obs::SCALAR_POWER_OFFER + k] as u8 as usize,
            offered[k] as usize + 1,
            "slot {k}"
        );
    }
    // A Discover offers three, so the fourth slot stays empty.
    let mut g = hero_game(20, "George the Fallen");
    put_on_board(&mut g, 0, "Clever Castaway");
    let ui = id(&g, "Unmasked Identity");
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, ui, false, 0));
    g.players[0].gold = 6;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].power_offer.len(), bgcore::DISCOVER);
    bgcore::obs::write(&g, 0, &mut out);
    for k in 0..bgcore::DISCOVER {
        assert_eq!(
            out[bgcore::obs::SCALAR_POWER_OFFER + k] as u8 as usize,
            g.players[0].power_offer[k] as usize + 1
        );
    }
    assert_eq!(out[bgcore::obs::SCALAR_POWER_OFFER + bgcore::DISCOVER], 0.0, "an empty slot");
}

#[test]
fn the_observation_carries_the_second_power() {
    let mut g = hero_game(16, "George the Fallen");
    put_on_board(&mut g, 0, "Clever Castaway");
    let mut out = vec![0f32; bgcore::obs::OBS_SIZE];
    turn_of(&mut g, 0);
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER2_HERO], 0.0, "no second power yet");
    assert_eq!(
        out[bgcore::obs::SCALAR_POWER_HERO] as usize,
        g.players[0].power.expect("a power").of as usize + 1
    );
    let kings = power_id(&g, "TB_BaconShop_HP_041a");
    g.players[0].power2 = Some(make_power(&g, kings));
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER2_HERO] as usize, kings as usize + 1);
    assert_eq!(out[bgcore::obs::SCALAR_POWER2_COST], 2.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER2_USES_LEFT], 1.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER2_UNLOCKED], 1.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER2_USABLE], 1.0);
    // The two buttons are two action kinds, and the mask says which is legal.
    let mut mask = [false; bgcore::N_ACTIONS];
    g.legal_mask(&mut mask);
    assert!(mask[Action::HeroPower.index()]);
    assert!(mask[Action::HeroPower2.index()]);
    assert_ne!(Action::HeroPower.kind(), Action::HeroPower2.kind());
}


// ---- Start of Combat powers that trigger twice (docs/rules/heroes.md, 2026-11-01) ----
//
// Valdrakken Wind Chimes and Ancient Wishbone make a Start of Combat power
// run its effect twice, the older ones included: `tp.power_triggers` is what
// a trinket's `power_triggers = 2` writes.

#[test]
fn the_fire_invocation_triggers_twice() {
    let mut g = hero_game(50, "Bru'kan");
    let fire = power_id(&g, "BG22_HERO_001p_t2");
    g.players[0].power = Some(make_power(&g, fire));
    assert_eq!(g.players[0].counters(&g.data, g.lobby).soc_double_left, 1);
    g.players[0].tp.power_triggers = 2;
    let counters = g.players[0].counters(&g.data, g.lobby);
    assert_eq!(counters.soc_double_left, 2, "the doubling runs twice");
    // The fight doubles the left-most minion twice: a 2/3 becomes 8/3 and
    // kills a 5-Health wall that a 4/3 only wounds.
    let db = &g.data.cards;
    let mk = |name: &str, attack: i32, health: i32| {
        let mut m = db.make(db.by_name(name).expect(name), false);
        m.attack = attack;
        m.health = health;
        m
    };
    let mine = [mk("Clever Castaway", 2, 3)];
    let enemy = [mk("Microbot", 0, 5)];
    let mut rng = bgcore::Rng::new(50);
    let out = fight_with(db, &mine, &enemy, [counters, Counters::default()], 1, &mut rng, &mut ());
    assert_eq!(out.winner, Some(0), "8 Attack killed the 5-Health wall");
    let once = Counters { soc_double_left: 1, ..Counters::default() };
    let mut rng = bgcore::Rng::new(50);
    let out = fight_with(db, &mine, &enemy, [once, Counters::default()], 1, &mut rng, &mut ());
    assert_eq!(out.winner, None, "4 Attack leaves it standing");
}

#[test]
fn the_earth_invocation_triggers_twice() {
    let mut g = hero_game(51, "Bru'kan");
    let earth = power_id(&g, "BG22_HERO_001p_t1");
    g.players[0].power = Some(make_power(&g, earth));
    let once = g.players[0].counters(&g.data, g.lobby);
    assert_eq!(once.soc_earth, 4, "four minions get the Deathrattle");
    g.players[0].tp.power_triggers = 2;
    let twice = g.players[0].counters(&g.data, g.lobby);
    assert_eq!(twice.soc_earth, 8, "eight of them, twice the effect");
    assert_eq!(twice.soc_earth_card, once.soc_earth_card, "the same Elemental");
    assert_eq!(g.data.cards.get(twice.soc_earth_card).name, "Stone Elemental");
}

#[test]
fn the_lightning_invocation_triggers_twice() {
    let mut g = hero_game(52, "Bru'kan");
    let bolt = power_id(&g, "BG22_HERO_001p_t4");
    g.players[0].power = Some(make_power(&g, bolt));
    let once = g.players[0].counters(&g.data, g.lobby);
    assert_eq!((once.soc_lightning, once.soc_lightning_damage), (5, 1));
    g.players[0].tp.power_triggers = 2;
    let twice = g.players[0].counters(&g.data, g.lobby);
    assert_eq!(
        (twice.soc_lightning, twice.soc_lightning_damage),
        (10, 1),
        "twice the hits, the same damage"
    );
    // Ten hits clear seven 1-Health enemies at Start of Combat, where five
    // leave two of them standing to be fought.
    let db = &g.data.cards;
    let mk = |name: &str| db.make(db.by_name(name).expect(name), false);
    let mut mine = [mk("Treasure Parrot")];
    mine[0].attack = 10;
    let enemy: Vec<Minion> = (0..7).map(|_| mk("Mechagnome Interpreter")).collect();
    let mut rng = bgcore::Rng::new(52);
    let out = fight_with(db, &mine, &enemy, [twice, Counters::default()], 40, &mut rng, &mut ());
    assert_eq!((out.attacks, out.winner), (0, Some(0)), "all seven died at Start of Combat");
    let mut rng = bgcore::Rng::new(52);
    let out = fight_with(db, &mine, &enemy, [once, Counters::default()], 40, &mut rng, &mut ());
    assert!(out.attacks > 0, "five hits left two of them");
}

#[test]
fn a_keyword_grant_has_nothing_to_repeat() {
    // The Water Invocation gives the right-most minion two keywords. The same
    // two keywords twice are the same two keywords, so a doubled power keeps
    // the single grant (and the mask must not be multiplied).
    let mut g = hero_game(53, "Bru'kan");
    let water = power_id(&g, "BG22_HERO_001p_t3");
    g.players[0].power = Some(make_power(&g, water));
    let once = g.players[0].counters(&g.data, g.lobby).soc_rightmost;
    g.players[0].tp.power_triggers = 2;
    let twice = g.players[0].counters(&g.data, g.lobby).soc_rightmost;
    assert_eq!(once, twice, "the same grant twice");
    assert_eq!(twice, Keywords::DIVINE_SHIELD | Keywords::TAUNT);
}
