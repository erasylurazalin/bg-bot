//! The hero frame and the first 14 heroes, with the real heroes.toml: the
//! pick before turn 1, armor before Health on every hero damage, the power
//! button, the passives, the two spells that waited for heroes (Armor Stash,
//! Unmasked Identity) and Finley's Discover.

use std::sync::Arc;

use bgcore::fight::fight_with;
use bgcore::{Action, CardDb, CardId, Data, Game, Keywords, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

/// A no-pick game with every tribe in the lobby, player 0 forced to `hero`.
fn hero_game(seed: u64, hero: &str) -> Game {
    hero_game_lobby(seed, hero, lobby(&Tribe::EVERY))
}

fn hero_game_lobby(seed: u64, hero: &str, tribes: bgcore::Tribes) -> Game {
    let d = data();
    let mut g = Game::new_with_lobby(d.clone(), seed, tribes);
    let h = d.heroes.by_name(hero).expect(hero);
    g.assign_hero(0, h);
    g
}

fn mk(db: &CardDb, name: &str) -> Minion {
    db.make(db.by_name(name).expect(name), false)
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
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

fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        g.step(Action::EndTurn).unwrap();
    }
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
}

/// Take every board minion off, pool copies back (a plain `clear()` would
/// break the exact pool accounting the tests check).
fn clear_board(g: &mut Game, p: usize) {
    for m in g.players[p].board.drain(..) {
        let c = g.data.cards.get(m.card);
        if m.pool_copies > 0 && !c.token && !c.is_spell() {
            g.pool[c.pool_slot as usize] += m.pool_copies;
        }
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
        let c = id(&g.data.cards, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
    g.players[p].shop_spell = None;
}

fn power(g: &Game, p: usize) -> &bgcore::heroes::HeroPower {
    &g.data.heroes.get(g.players[p].power.expect("no power").of).power
}

// ---- the data ----

#[test]
fn heroes_toml_parses_with_the_twenty_heroes() {
    let d = Data::bundled();
    assert_eq!(d.heroes.heroes.len(), 20);
    // Armor and card_ids from ref/bg-meta.json (matched by card_id).
    for (name, card_id, armor, extra_health) in [
        ("Nozdormu", "TB_BaconShop_HERO_57", 13, 0),
        ("Forest Warden Omu", "TB_BaconShop_HERO_74", 6, 0),
        ("Cap'n Hoggarr", "BG26_HERO_101", 14, 0),
        ("Trade Prince Gallywix", "TB_BaconShop_HERO_10", 5, 0),
        ("Patchwerk", "TB_BaconShop_HERO_34", 0, 30),
        ("Al'Akir", "TB_BaconShop_HERO_76", 15, 0),
        ("Forest Lord Cenarius", "BG32_HERO_001", 13, 0),
        ("George the Fallen", "TB_BaconShop_HERO_15", 15, 0),
        ("Lich Baz'hial", "TB_BaconShop_HERO_25", 18, 0),
        ("Pyramad", "TB_BaconShop_HERO_39", 14, 0),
        ("Reno Jackson", "TB_BaconShop_HERO_41", 12, 0),
        ("Death Speaker Blackthorn", "BG20_HERO_103", 18, 0),
        ("Alexstrasza", "TB_BaconShop_HERO_56", 8, 0),
        ("Sir Finley Mrrgglton", "TB_BaconShop_HERO_40", 14, 0),
        ("Exarch Othaar", "BG31_HERO_006", 15, 0),
        ("Shudderwock", "TB_BaconShop_HERO_23", 10, 0),
        ("Drest'agath", "BG36_HERO_000", 12, 0),
        ("Kith'ix", "BG36_HERO_002", 8, 0),
        ("Doctor Holli'dae", "BG28_HERO_801", 14, 0),
        ("Time Twister Chromie", "BG34_HERO_001", 14, 0),
    ] {
        let h = d.heroes.by_name(name).expect(name);
        let hero = d.heroes.get(h);
        assert_eq!(hero.card_id, card_id, "{name}");
        assert_eq!(hero.armor, armor, "{name}");
        assert_eq!(hero.extra_health, extra_health, "{name}");
    }
    // The tribe ties.
    assert!(d.heroes.get(d.heroes.by_name("Cap'n Hoggarr").unwrap()).tribes.has(Tribe::Pirate));
    assert!(d.heroes.get(d.heroes.by_name("Lich Baz'hial").unwrap()).tribes.has(Tribe::Demon));
    assert!(
        d.heroes.get(d.heroes.by_name("Death Speaker Blackthorn").unwrap()).tribes.has(Tribe::Quilboar)
    );
    assert!(d.heroes.get(d.heroes.by_name("Alexstrasza").unwrap()).tribes.has(Tribe::Dragon));
    assert!(d.heroes.get(d.heroes.by_name("Drest'agath").unwrap()).tribes.has(Tribe::Aberration));
    // Unmasked Identity's exclusion list holds Finley, A. F. Kay, Faelin
    // and Patchwerk (Era: his power can't come from Unmasked Identity).
    let ui = db().by_name("Unmasked Identity").unwrap();
    let excludes = db().get(ui).exclude_heroes.clone();
    for card_id in ["TB_BaconShop_HERO_40", "TB_BaconShop_HERO_16", "BG22_HERO_201", "TB_BaconShop_HERO_34"] {
        assert!(excludes.contains(&card_id.to_string()), "{card_id}");
    }
    // Smart Savings is a passive whatever the data's cost says, Bloodbound is
    // twice a turn, Gonna Be Rich! once a game, Queen of Dragons tier 4.
    let pw = |n: &str| d.heroes.get(d.heroes.by_name(n).unwrap()).power.clone();
    assert!(pw("Trade Prince Gallywix").passive);
    assert_eq!(pw("Death Speaker Blackthorn").uses_per_turn, 2);
    assert!(pw("Reno Jackson").once_per_game);
    assert_eq!(pw("Alexstrasza").unlock_tier, 4);
    // The 2026-10-30 batch: the two turn-3 unlocks, and the three costs that
    // are not in the game data (a null cost is 0: two presses are buttons).
    assert_eq!(pw("Exarch Othaar").unlock_turn, 3);
    assert_eq!(pw("Shudderwock").unlock_turn, 3);
    assert_eq!(pw("Drest'agath").cost, 1);
    assert_eq!(pw("Kith'ix").cost, 2);
    assert_eq!(pw("Doctor Holli'dae").cost, 1);
    assert_eq!(pw("Time Twister Chromie").cost, 0);
    assert!(!pw("Time Twister Chromie").passive);
}

#[test]
fn a_bad_heroes_toml_is_refused() {
    assert!(bgcore::heroes::HeroDb::from_toml("nonsense").is_err());
    let dupe = "[[hero]]\nname = 'A'\ncard_id = 'X'\npower = { name = 'p', card_id = 'q' }\n\
         [[hero]]\nname = 'B'\ncard_id = 'X'\npower = { name = 'r', card_id = 's' }";
    assert!(bgcore::heroes::HeroDb::from_toml(dupe).unwrap_err().contains("twice"));
    let bad_kw = "[[hero]]\nname = 'A'\ncard_id = 'X'\n\
                  power = { name = 'p', card_id = 'q', passive = true, effect = { soc_leftmost = ['flying'] } }";
    assert!(bgcore::heroes::HeroDb::from_toml(bad_kw).unwrap_err().contains("keyword"));
}

// ---- the pick ----

#[test]
fn the_pick_offers_two_heroes_no_repeats_and_starts_turn_1() {
    for seed in 0..20 {
        let mut g = Game::new_heroes_with_lobby(data(), seed, lobby(&Tribe::EVERY));
        assert_eq!(g.turn, 0, "the pick is before turn 1");
        let mut seen: Vec<u8> = Vec::new();
        // Every player answers their offer with Choose; nothing else is legal.
        while g.turn == 0 {
            let p = g.current_player().unwrap();
            // Finley's power Discover waits before his turn 1 starts.
            if !g.players[p].power_offer.is_empty() {
                g.step(Action::Choose(0)).unwrap();
                continue;
            }
            let offer: Vec<u8> = g.players[p]
                .hero_offer
                .iter()
                .copied()
                .filter(|&h| h != u8::MAX)
                .collect();
            assert!(!offer.is_empty(), "seed {seed}: player {p} got no hero");
            assert!(offer.len() <= 2, "seed {seed}: {} offers", offer.len());
            for &h in &offer {
                assert!(!seen.contains(&h), "seed {seed}: a hero offered twice");
                seen.push(h);
            }
            assert!(!g.is_legal(Action::EndTurn), "seed {seed}: something else was legal");
            assert!(!g.is_legal(Action::Roll), "seed {seed}: something else was legal");
            g.step(Action::Choose(0)).unwrap();
        }
        assert_eq!(g.turn, 1);
        for p in 0..8 {
            assert!(g.players[p].hero.is_some(), "seed {seed}: player {p} picked nothing");
            assert_eq!(g.players[p].armor as u32, g.data.heroes.get(g.players[p].hero.unwrap()).armor as u32);
        }
        g.check_invariants().unwrap();
    }
}

#[test]
fn tribe_tied_heroes_need_a_lobby_tribe() {
    // No Pirate, Demon, Quilboar or Dragon in the lobby: those four heroes
    // are never offered.
    let no_ties = lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Elemental, Tribe::Undead]);
    for seed in 0..40 {
        let mut g = Game::new_heroes_with_lobby(data(), seed, no_ties);
        while g.turn == 0 {
            let p = g.current_player().unwrap();
            if !g.players[p].power_offer.is_empty() {
                g.step(Action::Choose(0)).unwrap();
                continue;
            }
            for &h in g.players[p].hero_offer.iter() {
                if h == u8::MAX {
                    continue;
                }
                let name = g.data.heroes.get(h).name.clone();
                assert!(
                    !matches!(
                        name.as_str(),
                        "Cap'n Hoggarr"
                            | "Lich Baz'hial"
                            | "Death Speaker Blackthorn"
                            | "Alexstrasza"
                            | "Drest'agath"
                    ),
                    "seed {seed}: {name} offered without its tribe"
                );
            }
            g.step(Action::Choose(0)).unwrap();
        }
    }
    // With a Pirate lobby, Hoggarr can be offered (find him within 60 seeds).
    let pirates = lobby(&[Tribe::Pirate, Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon]);
    let mut found = false;
    for seed in 0..60 {
        let mut g = Game::new_heroes_with_lobby(data(), seed, pirates);
        while g.turn == 0 {
            let p = g.current_player().unwrap();
            if g.players[p].hero_offer.iter().any(|&h| {
                h != u8::MAX && g.data.heroes.get(h).name == "Cap'n Hoggarr"
            }) {
                found = true;
            }
            g.step(Action::Choose(0)).unwrap();
        }
    }
    assert!(found, "Hoggarr never offered in a Pirate lobby");
}

// ---- health, armor, damage ----

#[test]
fn patchwerk_has_sixty_health_and_no_armor() {
    let g = hero_game(1, "Patchwerk");
    assert_eq!((g.players[0].health, g.players[0].armor), (60, 0));
}

#[test]
fn armor_takes_fight_damage_first_and_never_comes_back() {
    // A hero with armor loses a fight against a big board: the cap (5 on
    // turn 1) eats armor first, then Health.
    let mut g = hero_game(2, "Al'Akir"); // 15 armor
    g.players[0].board.clear();
    // player 0 must be paired: find the opponent and load their board.
    let opp = match g.players[0].opponent {
        Some(bgcore::Opponent::Player(q)) => q as usize,
        _ => panic!("paired with the ghost on turn 1"),
    };
    g.players[opp].board.clear();
    let mut killer = mk(&db(), "Lullabot");
    killer.attack = 50;
    killer.health = 100;
    g.players[opp].board.push(killer);
    g.players[opp].tier = 5;
    g.players[0].tier = 1;
    // end everyone's turn and let the fight land
    let t = g.turn;
    while g.turn == t {
        g.step(Action::EndTurn).unwrap();
    }
    // winner tier 5 -> 5 damage, capped at 5 on turn 1
    assert_eq!(g.players[0].armor, 10, "armor took the 5 first");
    assert_eq!(g.players[0].health, 30, "none reached Health");
}

#[test]
fn health_costs_reach_less_health_through_armor() {
    // can_pay_health: armor absorbs a Health cost first, so a hero at low
    // Health with armor can pay more than without it.
    let mut g = hero_game(3, "George the Fallen"); // 15 armor
    g.players[0].health = 5;
    g.players[0].armor = 10;
    assert!(g.players[0].can_pay_health(12, &g.data.cards)); // 2 Health past armor
    assert!(!g.players[0].can_pay_health(15, &g.data.cards)); // exactly to 0
    g.players[0].armor = 0;
    assert!(!g.players[0].can_pay_health(12, &g.data.cards));
}

#[test]
fn a_rewind_gives_armor_back_as_armor() {
    // Graveyard Shift's 2 damage through 18 armor: armor takes it, and Soul
    // Rewinder on the board rewinds it as armor.
    let mut g = hero_game(4, "Lich Baz'hial");
    assert_eq!(g.players[0].armor, 18);
    put_on_board(&mut g, 0, "Soul Rewinder");
    set_shop(&mut g, 0, &["Lullabot"]);
    // Two candidates (the card and a spell) so the target prompt opens.
    g.players[0].shop_spell = Some(id(&db(), "Tavern Coin"));
    g.players[0].gold = 3;
    act(&mut g, 0, Action::HeroPower);
    // The steal takes any Tavern card: the minion in slot 0.
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(g.players[0].hand.len(), 1);
    // The rewind runs inside the hero-damage path and clears what it undid,
    // so by the time the action returns the armor is whole; the proof armor
    // took the 2 is that Health never moved.
    assert_eq!(g.players[0].health, 30, "Health never moved");
    assert_eq!(g.players[0].armor, 18, "the rewind returned the armor");
    assert_eq!(g.players[0].last_damage, 0, "the rewind cleared its record");
    g.check_invariants().unwrap();
}

// ---- the powers ----

#[test]
fn clairvoyance_gains_a_free_refresh_each_turn_and_stacks() {
    let mut g = hero_game(5, "Nozdormu");
    assert_eq!(g.players[0].free_rolls, 0, "turn 1 has not started its turn loop");
    end_all_turns(&mut g);
    assert_eq!(g.turn, 2);
    assert_eq!(g.players[0].free_rolls, 1, "one free Refresh per turn");
    // A free Refresh saved: it carries over and the next turn's adds on top.
    act(&mut g, 0, Action::Roll);
    // (the roll just spent one; back to 0)
    end_all_turns(&mut g);
    assert_eq!(g.players[0].free_rolls, 1);
    // two turns of powers stack when none is spent
    end_all_turns(&mut g);
    assert!(g.players[0].free_rolls >= 1);
    g.check_invariants().unwrap();
}

#[test]
fn everbloom_gains_two_gold_after_an_upgrade() {
    let mut g = hero_game(6, "Forest Warden Omu");
    g.players[0].gold = 5;
    g.players[0].upgrade_cost = 5;
    let before = g.players[0].gold;
    act(&mut g, 0, Action::Level);
    assert_eq!(g.players[0].gold, before - 5 + 2);
    assert_eq!(g.players[0].tier, 2);
}

#[test]
fn the_capn_now_gains_a_gold_after_buying_a_pirate() {
    let mut g = hero_game_lobby(7, "Cap'n Hoggarr", lobby(&[Tribe::Pirate, Tribe::Beast]));
    // A Pirate in the shop, and a Beast to prove the tie matters.
    let pirate = "Southsea Busker";
    let beast = "Flittering Bat";
    assert!(db().get(id(&db(), pirate)).tribes.has(Tribe::Pirate));
    assert!(db().get(id(&db(), beast)).tribes.has(Tribe::Beast));
    set_shop(&mut g, 0, &[pirate, beast]);
    g.players[0].gold = 6;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 6 - 3 + 1, "the Pirate buy gave 1 Gold back");
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 1, "the Beast buy gave nothing");
}

#[test]
fn smart_savings_gives_gold_next_turn_per_sell() {
    let mut g = hero_game(8, "Trade Prince Gallywix");
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Flittering Bat");
    g.players[0].gold = 0;
    act(&mut g, 0, Action::Sell(0));
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].gold_next_turn, 2);
    // The button is not legal: it is a passive.
    g.players[0].gold = 10;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    end_all_turns(&mut g);
    assert_eq!(g.turn, 2);
    // turn 2 gold is 4 base plus the 2 saved.
    assert_eq!(g.players[0].gold, 4 + 2);
}

#[test]
fn alakir_gives_the_leftmost_minion_the_three_keywords() {
    // The mask reaches the fight: the left-most minion attacks with Windfury,
    // Divine Shield and Taunt at its first swing (a Watch sees the keywords),
    // and only the left-most one: the Microbot next to it gets nothing.
    let mut g = hero_game(9, "Al'Akir");
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Lullabot");
    let counters = [g.players[0].counters(&g.data, g.lobby), bgcore::Counters::default()];
    assert_eq!(counters[0].soc_leftmost, Keywords::TAUNT | Keywords::DIVINE_SHIELD | Keywords::WINDFURY);

    #[derive(Default)]
    struct FirstSwings(Vec<(CardId, u8)>);
    impl bgcore::fight::Watch for FirstSwings {
        fn attack(&mut self, side: usize, attacker: &Minion, _: &Minion) {
            if side == 0 && !self.0.iter().any(|&(c, _)| c == attacker.card) {
                self.0.push((attacker.card, attacker.keywords.0));
            }
        }
    }
    let mut watch = FirstSwings::default();
    let board = g.players[0].board.clone();
    let mut wall = mk(&db(), "Lullabot");
    wall.attack = 0;
    wall.health = 1000;
    let out = fight_with(
        &db(),
        &board,
        &[wall, wall],
        counters,
        500,
        &mut Rng::new(1),
        &mut watch,
    );
    assert_eq!(out.winner, None, "1000-Health walls never die: a tie at the cap");
    let kw_of = |name: &str| {
        let c = id(&db(), name);
        watch.0.iter().find(|&&(card, _)| card == c).map(|&(_, kw)| kw)
    };
    // Taunt is on the Alleycat from the start, so both sides' target picks
    // see it; the Watch fires at the first swing, where the Alleycat still
    // carries the shield (it lost none: the walls have no attack).
    assert_eq!(
        kw_of("Flittering Bat"),
        Some(Keywords::TAUNT | Keywords::DIVINE_SHIELD | Keywords::WINDFURY),
        "the left-most minion got all three"
    );
    // Lullabot (a Magnetic Mech) keeps only its own keywords: none of the
    // three Al'Akir grants.
    if let Some(kw) = kw_of("Lullabot") {
        assert_eq!(
            kw & (Keywords::TAUNT | Keywords::DIVINE_SHIELD | Keywords::WINDFURY),
            0,
            "the second minion got nothing"
        );
    }
}

#[test]
fn wisdom_of_ancients_raises_max_gold_stacking_past_ten() {
    let mut g = hero_game(11, "Forest Lord Cenarius");
    // Turn 1 gold is 3; set to 10 for the press.
    g.players[0].gold = 10;
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].extra_max_gold, 1);
    assert_eq!(g.players[0].gold, 7, "no Gold this turn from the power");
    assert_eq!(g.players[0].power.unwrap().used_game, 1);
    // Only once per turn.
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    end_all_turns(&mut g);
    assert_eq!(g.players[0].gold, 4 + 1, "turn 2: base 4 plus the raised maximum");
    // It stacks once per turn, past 10.
    for _ in 0..11 {
        g.players[0].gold = 10;
        act(&mut g, 0, Action::HeroPower);
        end_all_turns(&mut g);
    }
    assert_eq!(g.players[0].extra_max_gold, 12, "one more per turn, past 10");
}

#[test]
fn boon_of_light_gives_a_minion_divine_shield() {
    let mut g = hero_game(12, "George the Fallen");
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Lullabot");
    g.players[0].gold = 3;
    act(&mut g, 0, Action::HeroPower);
    // The power opened a target prompt; answer board slot 1.
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].board[1].keywords.has(Keywords::DIVINE_SHIELD));
    assert!(!g.players[0].board[0].keywords.has(Keywords::DIVINE_SHIELD));
    assert_eq!(g.players[0].gold, 2);
    // One use per turn.
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    // No minion: not legal.
    end_all_turns(&mut g);
    clear_board(&mut g, 0);
    g.players[0].gold = 5;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
}

#[test]
fn graveyard_shift_steals_any_tavern_card_including_the_spell() {
    let mut g = hero_game(13, "Lich Baz'hial");
    g.players[0].gold = 2;
    set_shop(&mut g, 0, &["Lullabot"]);
    // A spell in the spell slot: one past the last shop card is the pick.
    let coin = id(&db(), "Tavern Coin");
    g.players[0].shop_spell = Some(coin);
    act(&mut g, 0, Action::HeroPower);
    turn_of(&mut g, 0);
    assert!(g.players[0].pending_target.is_some(), "two candidates: the card and the spell");
    // The minion slot (0) and the spell slot (1, one past the last card) are
    // both valid targets.
    assert!(g.is_legal(Action::TargetShop(0)));
    assert!(g.is_legal(Action::TargetShop(1)));
    assert!(!g.is_legal(Action::TargetShop(2)));
    act(&mut g, 0, Action::TargetShop(1));
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.players[0].hand[0].card, coin);
    assert_eq!(g.players[0].shop_spell, None);
    assert_eq!(g.players[0].armor, 16, "the 2 damage hit armor first");
    assert_eq!(g.players[0].health, 30);
    // 2 damage with no armor left: Health takes it. One candidate left, so
    // the press auto-steals it.
    g.players[0].armor = 0;
    g.players[0].power.as_mut().unwrap().uses_left = 1;
    g.players[0].gold = 2;
    put_on_board(&mut g, 0, "Flittering Bat"); // something to sell for room is not needed
    turn_of(&mut g, 0);
    let hand_before = g.players[0].hand.len();
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].hand.len(), hand_before + 1, "the lone card was stolen at once");
    assert_eq!(g.players[0].health, 28);
    // Stealing is not a buy: the pool copy left with the card, nothing else.
    assert_eq!(g.players[0].gold, 0);
    g.check_invariants().unwrap();
}

#[test]
fn brick_by_brick_steals_a_random_minion_and_doubles_its_health() {
    let mut g = hero_game(14, "Pyramad");
    g.players[0].gold = 2;
    let m = mk(&db(), "Lullabot");
    let base_health = m.health;
    set_shop(&mut g, 0, &["Lullabot"]);
    act(&mut g, 0, Action::HeroPower);
    assert!(g.players[0].pending_target.is_none(), "the steal is random");
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.players[0].hand[0].health, base_health * 2);
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.players[0].gold, 0);
    g.check_invariants().unwrap();
}

#[test]
fn gonna_be_rich_makes_a_minion_golden_once_per_game() {
    let mut g = hero_game(15, "Reno Jackson");
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Lullabot");
    let base = mk(&db(), "Flittering Bat");
    let before_stats = g.players[0].board[0];
    act(&mut g, 0, Action::HeroPower); // cost 0, opens the target prompt
    assert!(g.players[0].pending_target.is_some());
    act(&mut g, 0, Action::Target(0));
    let m = &g.players[0].board[0];
    assert!(m.golden);
    assert_eq!(m.attack, before_stats.attack + base.attack); // doubled printed Attack
    // No Triple Reward spell, and it does not count as a Golden played.
    let reward = id(&db(), "Triple Reward");
    assert!(!g.players[0].hand.iter().any(|h| h.card == reward));
    assert_eq!(g.players[0].golden_played, 0);
    // Once per game: never legal again.
    end_all_turns(&mut g);
    for _ in 0..5 {
        end_all_turns(&mut g);
    }
    put_on_board(&mut g, 0, "Lullabot");
    g.players[0].gold = 10;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "once per game");
    g.check_invariants().unwrap();
}

#[test]
fn bloodbound_gets_two_gems_twice_a_turn() {
    let mut g = hero_game_lobby(16, "Death Speaker Blackthorn", lobby(&[Tribe::Quilboar, Tribe::Beast]));
    let gem = id(&db(), "Blood Gem");
    g.players[0].gold = 2;
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::HeroPower);
    let gems = g.players[0].hand.iter().filter(|m| m.card == gem).count();
    assert_eq!(gems, 4, "2 Gems per press, twice a turn");
    assert_eq!(g.players[0].gold, 0);
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "both uses spent");
    end_all_turns(&mut g);
    g.players[0].gold = 1;
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower));
}

#[test]
fn queen_of_dragons_discovers_a_dragon_once_tier_4() {
    let mut g = hero_game_lobby(17, "Alexstrasza", lobby(&[Tribe::Dragon, Tribe::Beast]));
    g.players[0].gold = 1;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "locked below tier 4");
    // Levelling to tier 4 unlocks it mid-turn.
    while g.players[0].tier < 4 {
        g.players[0].gold = 20;
        g.players[0].upgrade_cost = 0;
        act(&mut g, 0, Action::Level);
    }
    g.players[0].gold = 1;
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower), "unlocked at tier 4");
    act(&mut g, 0, Action::HeroPower);
    assert!(!g.players[0].discover.is_empty(), "the Discover is open");
    // 3 Dragons of tier <= 4 from the lobby-filtered pool.
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in &g.players[0].discover {
        let card = g.data.cards.get(c);
        assert!(card.tribes.has(Tribe::Dragon), "{} is not a Dragon", card.name);
        assert!(card.tier <= 4, "{} is above the tier", card.name);
    }
    act(&mut g, 0, Action::Choose(0));
    let got = g.players[0].hand[0].card;
    assert!(g.data.cards.get(got).tribes.has(Tribe::Dragon));
    g.check_invariants().unwrap();
}

// ---- the 2026-10-30 batch: the six Easy powers that were left ----

#[test]
fn arcane_knowledge_discounts_the_next_tavern_spell_from_turn_3() {
    let mut g = hero_game(20, "Exarch Othaar");
    g.players[0].gold = 5;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "it unlocks on turn 3");
    end_all_turns(&mut g);
    end_all_turns(&mut g);
    assert_eq!(g.turn, 3);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].spell_discount, 1);
    // The next Tavern spell costs 1 less: a 1-cost spell is free, and the
    // discount is used up by it.
    let coin = id(&db(), "Tavern Coin");
    assert_eq!(g.data.cards.get(coin).cost, 1);
    g.players[0].shop_spell = Some(coin);
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].gold, 5, "the 1-cost spell was free");
    assert_eq!(g.players[0].spell_discount, 0, "the discount was used up");
    // A second spell in the same turn pays in full.
    g.players[0].shop_spell = Some(coin);
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].gold, 4, "no discount left");
    // Once per turn (it also costs 0).
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    g.check_invariants().unwrap();
}

#[test]
fn snicker_snack_triggers_a_chosen_battlecry_from_turn_3() {
    let mut g = hero_game(21, "Shudderwock");
    // Razorfen Geomancer gets 2 Blood Gems, Southsea Busker 1 Gold next turn.
    put_on_board(&mut g, 0, "Razorfen Geomancer");
    put_on_board(&mut g, 0, "Southsea Busker");
    g.players[0].gold = 4;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "it unlocks on turn 3");
    end_all_turns(&mut g);
    end_all_turns(&mut g);
    g.players[0].gold = 4;
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower), "unlocked on turn 3");
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].gold, 4, "the power costs 0");
    // Two Battlecry minions: the press asks which one.
    assert!(g.players[0].pending_target.is_some());
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::Target(0)));
    assert!(g.is_legal(Action::Target(1)));
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].gold_next_turn, 1, "the Busker's Battlecry ran");
    let gem = id(&db(), "Blood Gem");
    assert_eq!(
        g.players[0].hand.iter().filter(|m| m.card == gem).count(),
        0,
        "the Geomancer's Battlecry did not run"
    );
    // Once per turn, and nothing to trigger is not legal.
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    end_all_turns(&mut g);
    clear_board(&mut g, 0);
    g.players[0].gold = 4;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "no Battlecry on the board");
    g.check_invariants().unwrap();
}

#[test]
fn incubate_discards_a_card_for_a_random_aberration() {
    let lobby = lobby(&[Tribe::Aberration, Tribe::Beast]);
    let mut g = hero_game_lobby(22, "Drest'agath", lobby);
    g.players[0].gold = 5;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "nothing in hand to discard");
    // Buy a minion to throw away.
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].hand.len(), 1);
    let gold_before = g.players[0].gold;
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].gold, gold_before - 1, "the power costs 1");
    let pd = g.players[0].pending_discard.expect("the discard prompt is open");
    assert_eq!(pd.source, None, "a hero power has no board slot");
    act(&mut g, 0, Action::Discard(0));
    assert_eq!(g.players[0].discarded, 1, "a real discard");
    assert_eq!(g.players[0].hand.len(), 1, "one card in, one minion out");
    let got = g.players[0].hand[0];
    let card = g.data.cards.get(got.card);
    assert!(card.tribes.has(Tribe::Aberration), "{} is not an Aberration", card.name);
    assert!(card.tier <= g.players[0].tier);
    assert_eq!(got.pool_copies, 0, "generated, not from the pool");
    g.check_invariants().unwrap();
}

#[test]
fn dark_ritual_gives_a_linked_pair_and_playing_one_discards_the_other() {
    let mut g = hero_game(23, "Kith'ix");
    g.players[0].gold = 2;
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].gold, 0, "the power costs 2");
    assert_eq!(g.players[0].hand.len(), 2);
    let (a, b) = (g.players[0].hand[0], g.players[0].hand[1]);
    assert!(!g.data.cards.get(a.card).is_spell());
    assert!(!g.data.cards.get(b.card).is_spell());
    assert_ne!(a.linked, 0, "the pair is linked");
    assert_eq!(a.linked & 0x3fff, b.linked & 0x3fff, "one link group");
    assert_eq!(a.linked & 0xc000, 0, "a plain pair, not a golden four");
    // Playing one discards the other, as a real discard.
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].board.len(), 1);
    assert_eq!(g.players[0].hand.len(), 0, "the other one was discarded");
    assert_eq!(g.players[0].discarded, 1);
    // Once per turn.
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    // Both minions must fit in hand.
    end_all_turns(&mut g);
    g.players[0].gold = 2;
    while g.players[0].hand.len() + 2 > 10 {
        g.players[0].hand.pop();
    }
    let filler = {
        let coin = id(&db(), "Tavern Coin");
        g.players[0].make_card(&g.data.cards, coin, false, 0)
    };
    while g.players[0].hand.len() < 9 {
        g.players[0].hand.push(filler);
    }
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "one slot short of the pair");
    g.check_invariants().unwrap();
}

#[test]
fn blessing_of_the_nine_frogs_gets_a_random_tavern_spell() {
    let mut g = hero_game(24, "Doctor Holli'dae");
    g.players[0].gold = 1;
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].gold, 0, "the power costs 1");
    assert_eq!(g.players[0].hand.len(), 1);
    let card = g.data.cards.get(g.players[0].hand[0].card);
    assert!(card.is_spell(), "{} is not a spell", card.name);
    assert!(card.tavern_spell, "{} is not a Tavern spell", card.name);
    assert!(card.tier <= g.players[0].tier, "{} is above the tier", card.name);
    // Once per turn, and no room in hand means no press.
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    end_all_turns(&mut g);
    g.players[0].gold = 1;
    let filler = g.players[0].hand[0];
    while g.players[0].hand.len() < 10 {
        g.players[0].hand.push(filler);
    }
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower), "a full hand has no room");
    g.check_invariants().unwrap();
}

#[test]
fn mana_per_minute_refreshes_the_tavern_with_tavern_spells() {
    let mut g = hero_game(25, "Time Twister Chromie");
    let size = g.data.rules.shop_size(g.players[0].tier);
    set_shop(&mut g, 0, &["Flittering Bat", "Lullabot"]);
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].gold, gold, "the power costs 0");
    assert_eq!(g.players[0].shop.len(), size, "the Tavern is full of spells");
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert!(c.is_spell() && c.tavern_spell, "{} is not a Tavern spell", c.name);
    }
    let sp = g.players[0].shop_spell.expect("the spell slot is filled");
    assert!(g.data.cards.get(sp).is_spell());
    // Once per turn.
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::HeroPower));
    g.check_invariants().unwrap();
}

// ---- Finley and the hero power Discover ----

#[test]
fn finley_discovers_a_power_right_after_the_pick() {
    let d = data();
    // The hero deck is 20 deep now, so a seed may not deal Finley to player 0:
    // try more of them, and play the whole pick out (the order is shuffled, so
    // player 0's power Discover may be answered before or after the others).
    for seed in 0..80 {
        let mut g = Game::new_heroes_with_lobby(
            d.clone(),
            seed,
            lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Elemental, Tribe::Undead]),
        );
        let mut finley = false;
        while g.turn == 0 {
            let p = g.current_player().unwrap();
            if !g.players[p].power_offer.is_empty() {
                if p == 0 && finley {
                    // His own power Discover, before turn 1.
                    for &h in &g.players[0].power_offer {
                        let name = g.data.heroes.get(h).name.clone();
                        assert_ne!(name, "Sir Finley Mrrgglton", "his own exclusion list");
                        assert_ne!(
                            g.data.heroes.get(h).power.card_id, "TB_BaconShop_HP_057",
                            "Adventure! itself is never offered (it is Finley's own power)"
                        );
                    }
                    let old_power = g.players[0].power.unwrap().of;
                    g.step(Action::Choose(0)).unwrap();
                    assert_ne!(g.players[0].power.unwrap().of, old_power, "the power was replaced");
                    assert_eq!(g.players[0].power.unwrap().used_game, 0);
                    assert_eq!(g.players[0].power.unwrap().uses_left, power(&g, 0).uses_per_turn);
                    g.check_invariants().unwrap();
                    return;
                }
                g.step(Action::Choose(0)).unwrap();
                continue;
            }
            let offers_finley = g.players[p].hero_offer.iter().any(|&h| {
                h != u8::MAX && g.data.heroes.get(h).name == "Sir Finley Mrrgglton"
            });
            let k = if offers_finley {
                g.players[p]
                    .hero_offer
                    .iter()
                    .position(|&h| {
                        h != u8::MAX && g.data.heroes.get(h).name == "Sir Finley Mrrgglton"
                    })
                    .unwrap()
            } else {
                0
            };
            if p == 0 && offers_finley {
                finley = true;
            }
            g.step(Action::Choose(k as u8)).unwrap();
        }
        if finley {
            panic!("seed {seed}: Finley's power Discover never opened");
        }
    }
    panic!("Finley was never offered to player 0 in 80 seeds");
}

#[test]
fn unmasked_identity_replaces_the_power_and_respects_its_lists() {
    let mut g = hero_game(18, "George the Fallen");
    // Spend a use and some progress first: all of it is lost on the replace.
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Lullabot");
    g.players[0].gold = 4;
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::Target(0));
    assert_eq!(g.players[0].power.unwrap().used_game, 1);
    let ui = id(&db(), "Unmasked Identity");
    let m = g.players[0].make_card(&g.data.cards, ui, false, 0);
    g.players[0].hand.push(m);
    g.players[0].gold = 6;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert!(!g.players[0].power_offer.is_empty(), "the Discover opened");
    // Never the current power, never an excluded hero, never out of lobby.
    let current = g.players[0].power.unwrap().of;
    for &h in &g.players[0].power_offer {
        assert_ne!(h, current, "the current power is not offered");
        let hero = g.data.heroes.get(h);
        assert_ne!(hero.card_id, "TB_BaconShop_HERO_40", "Finley is excluded");
        assert!(
            hero.tribes.is_neutral() || hero.tribes.overlaps(g.lobby),
            "{} is tied outside the lobby",
            hero.name
        );
    }
    let offered: Vec<u8> = g.players[0].power_offer.to_vec();
    turn_of(&mut g, 0);
    g.step(Action::Choose(0)).unwrap();
    let st = g.players[0].power.unwrap();
    assert_ne!(st.of, current);
    assert_eq!(st.used_game, 0, "progress lost");
    assert_eq!(st.uses_left, g.data.heroes.get(offered[0]).power.uses_per_turn);
    assert!(offered.len() <= 3);
    g.check_invariants().unwrap();
}

#[test]
fn armor_stash_sets_armor_to_five() {
    let mut g = hero_game(19, "Al'Akir"); // 15 armor
    let stash = id(&db(), "Armor Stash");
    // Above 5: sets down.
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, stash, false, 0));
    g.players[0].gold = 3;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].armor, 5);
    // Below 5: sets up, from a fight-chipped hero.
    g.players[0].armor = 1;
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, stash, false, 0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].armor, 5);
}

// ---- bots and the observation ----

#[test]
fn bots_play_full_hero_games() {
    for seed in 0..5 {
        let mut g = Game::new_heroes(data(), seed);
        let mut rng = Rng::new(seed);
        let mut steps = 0;
        while !g.over {
            let a = bgcore::bots::random_action(&g, &mut rng);
            g.step(a).unwrap_or_else(|_| panic!("seed {seed}: illegal {a:?}"));
            steps += 1;
            if steps % 50 == 0 {
                g.check_invariants().unwrap();
            }
        }
        g.check_invariants().unwrap();
    }
}

#[test]
fn the_observation_carries_the_hero_features() {
    use bgcore::obs;
    let mut g = Game::new_heroes_with_lobby(
        data(),
        77,
        lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Elemental, Tribe::Undead]),
    );
    let mut out = vec![0.0f32; obs::OBS_SIZE];
    // During the pick: the offer is in the scalars.
    obs::write(&g, g.current_player().unwrap(), &mut out);
    assert_eq!(out[obs::SCALAR_PENDING_HERO], 1.0);
    let offered: Vec<usize> = (0..3)
        .filter(|&k| out[obs::SCALAR_HERO_OFFER + k] > 0.0)
        .map(|k| out[obs::SCALAR_HERO_OFFER + k] as usize)
        .collect();
    assert!(!offered.is_empty());
    // Pick everyone's hero, then check own and opponent features.
    while g.turn == 0 {
        g.step(Action::Choose(0)).unwrap();
    }
    for p in 0..8 {
        out.fill(0.0);
        obs::write(&g, p, &mut out);
        let hero = g.players[p].hero.unwrap() as usize + 1;
        assert_eq!(out[obs::SCALAR_HERO] as usize, hero);
        assert_eq!(out[obs::SCALAR_ARMOR] as i32, g.players[p].armor);
        assert_eq!(out[obs::SCALAR_POWER_HERO] as usize, g.players[p].power.unwrap().of as usize + 1);
        // Opponent block for the next player around holds their hero and armor.
        let q = (p + 1) % 8;
        let at = obs::OWN_END + obs::OPP_HERO;
        assert_eq!(out[at] as usize, g.players[q].hero.unwrap() as usize + 1);
        assert_eq!(out[at + 1] as i32, g.players[q].armor);
    }
}

// ---- Random heroes (training and bgbench games) ----

#[test]
fn random_hero_games_deal_everyone_a_different_hero_without_a_pick() {
    let d = data();
    for seed in 0..50 {
        let mut g = Game::new_random_heroes(d.clone(), seed);
        let mut seen = Vec::new();
        for p in 0..8 {
            let h = g.players[p].hero.expect("everyone has a hero");
            assert!(!seen.contains(&h), "seed {seed}: hero dealt twice");
            seen.push(h);
            assert!(g.players[p].hero_offer.iter().all(|&o| o == u8::MAX), "no pick");
        }
        // Turn 0 only waits for Finley's power Discover.
        while g.turn == 0 {
            let p = g.current_player().unwrap();
            assert!(!g.players[p].power_offer.is_empty(), "seed {seed}");
            g.step(Action::Choose(0)).unwrap();
        }
        assert_eq!(g.turn, 1);
        g.check_invariants().unwrap();
    }
}

#[test]
fn finley_finding_all_patched_up_starts_with_60_health_and_his_armor() {
    let d = data();
    for seed in 0..400 {
        let mut g = Game::new_random_heroes(d.clone(), seed);
        while g.turn == 0 {
            let p = g.current_player().unwrap();
            let k = g.players[p]
                .power_offer
                .iter()
                .position(|&h| g.data.heroes.get(h).name == "Patchwerk");
            let Some(k) = k else {
                g.step(Action::Choose(0)).unwrap();
                continue;
            };
            let armor = g.players[p].armor;
            assert_eq!(g.players[p].health, 30);
            g.step(Action::Choose(k as u8)).unwrap();
            assert_eq!(g.players[p].health, 60, "All Patched Up's 30 extra Health");
            assert_eq!(g.players[p].armor, armor, "Finley keeps his armor (UNSURE.md)");
            return;
        }
    }
    panic!("no Finley was offered All Patched Up in 400 seeds");
}
