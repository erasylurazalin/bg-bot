//! D4: the observation for the five-tribe lobby, and curve bots that never
//! get stuck.

use std::sync::Arc;

use bgcore::bots::{random_action, rush_action, standard_action};
use bgcore::game::{PendingTarget, TargetKind, TavernBuff};
use bgcore::obs::*;
use bgcore::{Action, Data, Game, Minion, N_ACTIONS, Opponent, PLAYERS, Rng, Tribe};

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]))
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

fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        g.step(Action::EndTurn).unwrap();
    }
}

fn id(g: &Game, name: &str) -> u16 {
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("no card {name}"))
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
}

fn view(g: &Game, p: usize) -> Vec<f32> {
    let mut out = vec![f32::NAN; OBS_SIZE];
    write(g, p, &mut out);
    assert!(out.iter().all(|x| x.is_finite()));
    out
}

fn board_slot(out: &[f32], s: usize) -> &[f32] {
    &out[BOARD_START + s * SLOT_FEATURES..BOARD_START + (s + 1) * SLOT_FEATURES]
}

fn hand_slot(out: &[f32], s: usize) -> &[f32] {
    &out[HAND_START + s * SLOT_FEATURES..HAND_START + (s + 1) * SLOT_FEATURES]
}

fn legal(g: &Game) -> Vec<Action> {
    let mut mask = [false; N_ACTIONS];
    g.legal_mask(&mut mask);
    (0..N_ACTIONS).filter(|&i| mask[i]).map(|i| Action::from_index(i).unwrap()).collect()
}

// ---- Observation ----

#[test]
fn layout_adds_up() {
    assert_eq!(OWN_END, SCALARS + OWN_SLOTS * SLOT_FEATURES);
    assert_eq!(OBS_SIZE, OWN_END + (PLAYERS - 1) * (OPP_SCALARS + bgcore::BOARD * SLOT_FEATURES));
    assert_eq!(DISCOVER_START + bgcore::DISCOVER * SLOT_FEATURES, OWN_END);
}

#[test]
fn slots_show_tribes_keywords_and_counters() {
    let mut g = game(1);
    turn_of(&mut g, 0);
    g.players[0].health = 20;
    put_on_board(&mut g, 0, "Holy Vanguard"); // 0: neutral 10/10, Divine Shield
    put_on_board(&mut g, 0, "Gatekeeper Amalgam"); // 1: all types
    put_on_board(&mut g, 0, "Heroic Underdog"); // 2: Stealth
    put_on_board(&mut g, 0, "Deadly Spore"); // 3: Venomous
    put_on_board(&mut g, 0, "Suspicious Prisonguard"); // 4: Activate (1)
    put_on_board(&mut g, 0, "Eredar Escapist"); // 5: "after 4 damage (N left!)"
    g.players[0].board[5].improvements = 3;
    g.players[0].board[1].extra_magnetize = 2;

    let out = view(&g, 0);
    let vanguard = board_slot(&out, 0);
    assert_eq!(vanguard[SLOT_CARD_ID], id(&g, "Holy Vanguard") as f32 + 1.0);
    assert_eq!(vanguard[SLOT_TIER], 4.0);
    assert_eq!(&vanguard[SLOT_TRIBES..SLOT_TRIBES + 5], &[0.0; 5], "a neutral has no tribe");
    assert_eq!(vanguard[SLOT_KEYWORDS + 1], 1.0, "divine shield");
    assert_eq!(vanguard[SLOT_FROM_POOL], 1.0);
    // above 15 Health, Holy Vanguard fights with its own stats
    assert_eq!((vanguard[SLOT_ATTACK], vanguard[SLOT_FIGHT_ATTACK]), (10.0, 10.0));
    assert_eq!((vanguard[SLOT_HEALTH], vanguard[SLOT_FIGHT_HEALTH]), (10.0, 10.0));

    let amalgam = board_slot(&out, 1);
    assert_eq!(&amalgam[SLOT_TRIBES..SLOT_TRIBES + 5], &[1.0; 5], "all types set every tribe");
    assert_eq!(amalgam[SLOT_EXTRA_MAGNETIZE], 2.0);
    assert_eq!(board_slot(&out, 2)[SLOT_STEALTH], 1.0);
    assert_eq!(board_slot(&out, 2)[SLOT_KEYWORDS + 3], 0.0);
    assert_eq!(board_slot(&out, 3)[SLOT_KEYWORDS + 3], 1.0, "venomous");
    assert_eq!(board_slot(&out, 4)[SLOT_ACTIVATE_COST], 1.0);
    assert_eq!(board_slot(&out, 3)[SLOT_ACTIVATE_COST], 0.0);
    let escapist = board_slot(&out, 5);
    assert_eq!(escapist[SLOT_DEMON], 1.0);
    assert_eq!(escapist[SLOT_IMPROVEMENTS], 3.0);
    assert_eq!(escapist[SLOT_EVENTS_LEFT], 1.0);
    assert_eq!(board_slot(&out, 4)[SLOT_EVENTS_LEFT], 0.0);
    assert_eq!(escapist[SLOT_COST], g.data.rules.buy_cost as f32);

    // at 15 Health the +30/+30 it gets at fight start shows in the fight stats
    g.players[0].health = 15;
    let out = view(&g, 0);
    let vanguard = board_slot(&out, 0);
    assert_eq!((vanguard[SLOT_ATTACK], vanguard[SLOT_FIGHT_ATTACK]), (10.0, 40.0));
    assert_eq!(vanguard[SLOT_FIGHT_HEALTH], 40.0);
    assert_eq!(board_slot(&out, 1)[SLOT_FIGHT_ATTACK], 6.0, "only the Vanguard gets it");
    // golden doubles it
    g.players[0].board[0].golden = true;
    let out = view(&g, 0);
    assert_eq!(board_slot(&out, 0)[SLOT_FIGHT_ATTACK], 70.0);
}

#[test]
fn hand_slots_show_taught_spells_locks_and_the_goldrinn_bonus() {
    let mut g = game(2);
    turn_of(&mut g, 0);
    g.players[0].health = 30;
    let ring = id(&g, "Shiny Ring");
    let apprentice = g.give(0, "Magicfin Apprentice").unwrap();
    g.players[0].hand[0].taught_spell = Some(ring);
    let slamma = g.give(0, "Banana Slamma").unwrap(); // a Beast, 3/6
    g.players[0].hand[1].locked_turns = 1;
    g.give(0, "Shiny Ring").unwrap();
    put_on_board(&mut g, 0, "Banana Slamma");
    // a shop Goldrinn: board Beasts carry the bonus already, a Beast in
    // hand gets it when played
    g.players[0].beast_bonus = (7, 7);
    g.players[0].board[0].attack += 7;
    g.players[0].board[0].health += 7;

    let out = view(&g, 0);
    let a = hand_slot(&out, 0);
    assert_eq!(a[SLOT_CARD_ID], apprentice.card as f32 + 1.0);
    assert_eq!(a[SLOT_TAUGHT_SPELL], ring as f32 + 1.0);
    assert_eq!(a[SLOT_FROM_POOL], 0.0, "a generated token");
    assert_eq!(a[SLOT_MURLOC], 1.0);
    let s = hand_slot(&out, 1);
    assert_eq!(s[SLOT_LOCKED], 1.0);
    assert_eq!((s[SLOT_ATTACK], s[SLOT_FIGHT_ATTACK]), (slamma.attack as f32, slamma.attack as f32 + 7.0));
    assert_eq!(s[SLOT_FIGHT_HEALTH], slamma.health as f32 + 7.0);
    let spell = hand_slot(&out, 2);
    assert_eq!(spell[SLOT_IS_SPELL], 1.0);
    assert_eq!(spell[SLOT_COST], 2.0);
    assert_eq!(spell[SLOT_FIGHT_ATTACK], 0.0);
    let b = board_slot(&out, 0);
    assert_eq!(b[SLOT_ATTACK], b[SLOT_FIGHT_ATTACK], "no double count on the board");
    assert_eq!(out[SCALAR_BEAST_BONUS_ATTACK], 7.0);
    assert_eq!(out[SCALAR_BEAST_BONUS_HEALTH], 7.0);
}

#[test]
fn scalars_show_player_counters() {
    let mut g = game(3);
    turn_of(&mut g, 0);
    g.players[0].health = 30;
    g.players[0].improve[0] = 3;
    g.players[0].fodder_refreshes.clear();
    for n in [2, 0, 1] {
        g.players[0].fodder_refreshes.push(n);
    }
    g.players[0].deathrattles_triggered = 9;
    g.players[0].overconfidence_win = 3;
    g.players[0].next_turn_board_buffs.push((1, 2, 3));
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Magicfin Mycologist");
    let cards = g.data.cards.clone();
    g.players[0].add_tavern_buff(&cards, TavernBuff { tribe: Some(Tribe::Demon), attack: 2, health: 1, this_turn: true });
    g.players[0].add_tavern_buff(&cards, TavernBuff { tribe: None, attack: 1, health: 1, this_turn: false });
    g.players[0].add_tavern_buff(&cards, TavernBuff { tribe: None, attack: 1, health: 0, this_turn: false });
    let sprout = id(&g, "A New Sprout");
    g.players[0].shop_spell = Some(sprout);
    g.players[0].spell_discount = 1;
    g.players[0].free_rolls = 0;

    let out = view(&g, 0);
    assert_eq!(out[SCALAR_TASTY_LOBSTER], 3.0);
    assert_eq!(out[SCALAR_FODDER_NEXT], 2.0);
    assert_eq!(out[SCALAR_FODDER_QUEUED], 3.0);
    assert_eq!(out[SCALAR_FODDER_REFRESHES], 3.0);
    assert_eq!(out[SCALAR_DEATHRATTLES], 9.0);
    assert_eq!(out[SCALAR_OVERCONFIDENCE_WIN], 3.0);
    assert_eq!(out[SCALAR_NEXT_TURN_BUFF_ATTACK], 3.0);
    assert_eq!(out[SCALAR_NEXT_TURN_BUFF_HEALTH], 6.0);
    assert_eq!(out[SCALAR_HEALTH_REFRESHES_LEFT], 2.0);
    assert_eq!(out[SCALAR_NEXT_ROLL_HEALTH], 1.0);
    assert_eq!(out[SCALAR_CAN_REWIND], 0.0);
    assert_eq!(out[SCALAR_MAGICFIN_LEFT], 1.0);
    assert_eq!(out[SCALAR_SHOP_SPELL_COST], 2.0, "A New Sprout costs 3, minus 1");
    // filter 0 is "any", then the tribes in Tribe::ALL order (Demon is 5)
    let tb = |filter: usize, k: usize| out[SCALAR_TAVERN_BUFFS + filter * 4 + k];
    assert_eq!((tb(0, 0), tb(0, 1), tb(0, 2), tb(0, 3)), (2.0, 1.0, 0.0, 0.0));
    assert_eq!((tb(5, 0), tb(5, 1), tb(5, 2), tb(5, 3)), (0.0, 0.0, 2.0, 1.0));
    assert_eq!(tb(1, 0) + tb(2, 0) + tb(3, 0) + tb(4, 0), 0.0);

    // a free roll comes first; used Health Refreshes and Magicfin teachings count down
    g.players[0].free_rolls = 1;
    g.players[0].health_refreshes_used = 1;
    g.players[0].mycologist_uses = 1;
    let out = view(&g, 0);
    assert_eq!(out[SCALAR_NEXT_ROLL_HEALTH], 0.0);
    assert_eq!(out[SCALAR_HEALTH_REFRESHES_LEFT], 1.0);
    assert_eq!(out[SCALAR_MAGICFIN_LEFT], 0.0);
}

#[test]
fn pending_spell_target_shows_its_card_and_numbers() {
    let mut g = game(4);
    turn_of(&mut g, 0);
    put_on_board(&mut g, 0, "Deadly Spore");
    put_on_board(&mut g, 0, "Holy Vanguard");
    let banana = id(&g, "Tavern Dish Banana");
    g.give(0, "Tavern Dish Banana").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let pt = g.players[0].pending_target.expect("two targets, so it waits");
    let out = view(&g, 0);
    let flag = if pt.kind.takes_shop() { SCALAR_PENDING_TARGET_SHOP } else { SCALAR_PENDING_TARGET };
    assert_eq!(out[flag], 1.0);
    assert_eq!(out[SCALAR_PENDING_CHOOSE_ONE] + out[SCALAR_PENDING_DISCOVER], 0.0);
    assert_eq!(out[SCALAR_PENDING_SOURCE], 0.0, "a spell has no source minion");
    assert_eq!(out[SCALAR_PENDING_CARD], banana as f32 + 1.0);
    assert_eq!(out[SCALAR_TARGET_KIND + 2], 1.0, "a spell target");
    assert_eq!((out[SCALAR_PENDING_ATTACK], out[SCALAR_PENDING_HEALTH]), (2.0, 2.0));
    assert_eq!(out[SCALAR_PENDING_RUNS], 1.0);
    // answering clears it
    act(&mut g, 0, Action::Target(1));
    let out = view(&g, 0);
    assert_eq!(out[SCALAR_PENDING_TARGET] + out[SCALAR_PENDING_TARGET_SHOP], 0.0);
    assert_eq!(out[SCALAR_PENDING_CARD], 0.0);
}

#[test]
fn pending_choose_one_and_discover_are_shown() {
    let mut g = game(5);
    turn_of(&mut g, 0);
    put_on_board(&mut g, 0, "Banana Slamma");
    let scarab = id(&g, "Sprightly Scarab");
    g.give(0, "Sprightly Scarab").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert!(g.players[0].pending_choose_one.is_some());
    let out = view(&g, 0);
    assert_eq!(out[SCALAR_PENDING_CHOOSE_ONE], 1.0);
    assert_eq!(out[SCALAR_PENDING_SOURCE], 2.0, "board slot 1, plus one");
    assert_eq!(out[SCALAR_PENDING_CARD], scarab as f32 + 1.0);
    let opt = |j: usize, k: usize| out[SCALAR_CHOOSE_ONE + j * CHOOSE_FEATURES + k];
    // both options are a minion target: +1/+1 Reborn, +4/+0 Windfury
    assert_eq!((opt(0, 0), opt(0, 4), opt(0, 5)), (1.0, 1.0, 1.0));
    assert_eq!((opt(1, 0), opt(1, 4), opt(1, 5)), (1.0, 4.0, 0.0));
    act(&mut g, 0, Action::Choose(1));

    // Search Through Time: a locked Discover
    g.players[0].gold = 10;
    g.give(0, "Search Through Time").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h, pos: 0 });
    assert!(!g.players[0].discover.is_empty());
    let out = view(&g, 0);
    assert_eq!(out[SCALAR_PENDING_DISCOVER], 1.0);
    assert_eq!(out[SCALAR_DISCOVER_LOCKED], 1.0);
    assert_eq!(out[SCALAR_DISCOVER_DAMAGE], 0.0);
    assert_eq!(out[SCALAR_PENDING_CHOOSE_ONE], 0.0);
    assert_eq!(out[DISCOVER_START + SLOT_PRESENT], 1.0);
    assert_eq!(out[DISCOVER_START + SLOT_TIER], g.players[0].tier as f32);
}

#[test]
fn the_opponent_is_hidden_until_you_fight() {
    let mut g = game(7);
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    let q = q as usize;
    put_on_board(&mut g, q, "Sewer Lord");
    // q's board is not visible before the fight...
    let opp = |out: &[f32]| {
        let start = OWN_END + ((q + PLAYERS - 1) % PLAYERS) * OPP_BLOCK;
        out[start..start + OPP_BLOCK].to_vec()
    };
    let o = opp(&view(&g, 0));
    assert_eq!(o[OPP_NEXT], 1.0); // next opponent
    assert_eq!(o[OPP_SEEN], 0.0); // never seen
    assert_eq!(o[OPP_SCALARS + SLOT_PRESENT], 0.0); // no minion shown
    // ...and is after it
    next_turn(&mut g);
    let o = opp(&view(&g, 0));
    assert_eq!(o[OPP_SEEN], 1.0);
    assert_eq!(o[OPP_SCALARS + SLOT_PRESENT], 1.0);
    assert_eq!(o[OPP_SCALARS + SLOT_ATTACK], 4.0); // Sewer Lord's attack
}

#[test]
fn opponents_show_last_opponent_tribes_and_placement() {
    let mut g = game(6);
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    let q = q as usize;
    put_on_board(&mut g, q, "Gatekeeper Amalgam");
    put_on_board(&mut g, q, "Eredar Escapist");
    put_on_board(&mut g, q, "Holy Vanguard");
    next_turn(&mut g);
    let out = view(&g, 0);
    let block = |q: usize| {
        let start = OWN_END + ((q + PLAYERS - 1) % PLAYERS) * OPP_BLOCK;
        out[start..start + OPP_BLOCK].to_vec()
    };
    let o = block(q);
    assert_eq!(o[OPP_SEEN], 1.0);
    assert_eq!(o[OPP_LAST], 1.0, "fought last turn");
    assert_eq!(o[OPP_PLACEMENT], 0.0);
    // Beast, Mech, Dragon, Murloc each 1 (the Amalgam), Demon 2
    assert_eq!(&o[OPP_TRIBES..OPP_TRIBES + 5], &[1.0, 1.0, 1.0, 1.0, 2.0]);
    for r in 1..PLAYERS {
        if r != q {
            assert_eq!(block(r)[OPP_LAST], 0.0);
        }
    }
    // their Holy Vanguard shows the bonus once their hero is low
    let vanguard = OPP_SCALARS + 2 * SLOT_FEATURES;
    assert_eq!(o[vanguard + SLOT_FIGHT_ATTACK], o[vanguard + SLOT_ATTACK]);
    g.players[q].health = 10;
    let out2 = view(&g, 0);
    let start = OWN_END + (q - 1) * OPP_BLOCK;
    assert_eq!(out2[start + vanguard + SLOT_FIGHT_ATTACK], o[vanguard + SLOT_ATTACK] + 30.0);
}

// ---- Bots ----

/// Play a whole game with the given seat policies; panics on an illegal
/// action.
fn play_out(g: &mut Game, rng: &mut Rng, seat: impl Fn(usize) -> u8) {
    while let Some(p) = g.current_player() {
        let a = match seat(p) {
            0 => standard_action(g),
            1 => rush_action(g),
            _ => random_action(g, rng),
        };
        if g.step(a).is_err() {
            panic!("seat {p} (kind {}) picked illegal {a:?} on turn {}", seat(p), g.turn);
        }
    }
    g.check_invariants().unwrap();
    let mut places: Vec<u8> = g.players.iter().map(|pl| pl.placement).collect();
    places.sort();
    assert_eq!(places, (1..=PLAYERS as u8).collect::<Vec<_>>());
}

#[test]
fn curve_bots_finish_many_games_against_each_other() {
    for seed in 0..60 {
        let mut g = game(1000 + seed);
        let mut rng = Rng::new(seed);
        // standard and rush bots, and in some games a few random seats
        play_out(&mut g, &mut rng, |p| if seed % 3 == 0 && p >= 6 { 2 } else { (p % 2) as u8 });
    }
}

#[test]
fn curve_bots_play_every_card() {
    let data = Arc::new(Data::bundled());
    let names: Vec<String> = data.cards.cards.iter().map(|c| c.name.clone()).collect();
    // every tribe: the point is that the curve bots can be handed every card
    let lobby = Tribe::EVERY.into_iter().fold(bgcore::Tribes::NONE, |m, t| m.with(t));
    for (k, name) in names.iter().enumerate() {
        let mut g = Game::new_with_lobby(data.clone(), 5000 + k as u64, lobby);
        // two bots get the card at the start (when pool copies allow), golden for one
        for p in [0usize, 1] {
            if g.give(p, name).is_some() && p == 1 {
                let last = g.players[p].hand.len() - 1;
                let card = g.players[p].hand[last].card;
                if !g.data.cards.get(card).is_spell() {
                    let pool_copies = g.players[p].hand[last].pool_copies;
                    let mut m: Minion = g.players[p].make_card(&g.data.cards, card, true, pool_copies);
                    m.pool_copies = pool_copies;
                    g.players[p].hand[last] = m;
                }
            }
            g.players[p].gold = 10;
        }
        let mut rng = Rng::new(k as u64);
        play_out(&mut g, &mut rng, |p| (p % 2) as u8);
    }
}

#[test]
fn curve_bot_answers_a_target_only_the_tavern_can_take() {
    let mut g = game(7);
    turn_of(&mut g, 0);
    put_on_board(&mut g, 0, "Tyrael");
    g.players[0].pending_target = Some(PendingTarget {
        source: Some(0),
        kind: TargetKind::AnySetStats { attack: 50, health: 50 },
        runs: 1,
        tavern: true,
        gem: false,
    });
    assert!(!g.players[0].shop.is_empty());
    for a in [standard_action(&g), rush_action(&g)] {
        assert!(matches!(a, Action::TargetShop(_)), "{a:?}");
        assert!(legal(&g).contains(&a));
    }
}

#[test]
fn curve_bot_does_not_pick_a_lethal_percussionist_discover() {
    let mut g = game(8);
    turn_of(&mut g, 0);
    put_on_board(&mut g, 0, "Imposing Percussionist");
    g.players[0].health = 3;
    // offer a tier 1 and two higher Demons, the way the Discover takes them out of the pool
    let demons: Vec<u16> = (0..g.data.cards.cards.len() as u16)
        .filter(|&c| {
            let card = g.data.cards.get(c);
            card.tribes.has(Tribe::Demon) && !card.token && !card.is_spell() && card.pool_slot == c
        })
        .collect();
    let low = *demons.iter().find(|&&c| g.data.cards.get(c).tier == 1).unwrap();
    let mut high = demons.iter().filter(|&&c| g.data.cards.get(c).tier >= 4);
    let opts = [*high.next().unwrap(), low, *high.next().unwrap()];
    for c in opts {
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        g.players[0].discover.push(c);
    }
    g.players[0].discover_damage_from = Some(0);
    assert_eq!(view(&g, 0)[SCALAR_DISCOVER_DAMAGE], 1.0);
    assert_eq!(standard_action(&g), Action::Choose(1));
    act(&mut g, 0, Action::Choose(1));
    assert!(g.players[0].alive);
}

#[test]
fn curve_bot_uses_activates_and_keeps_health_when_low() {
    // Suspicious Prisonguard's +3/+3 for 1 Gold is worth more than a roll
    let mut g = game(9);
    turn_of(&mut g, 0);
    g.players[0].board.clear();
    put_on_board(&mut g, 0, "Suspicious Prisonguard");
    put_on_board(&mut g, 0, "Deadly Spore");
    g.players[0].gold = 2; // too little to buy
    let a = standard_action(&g);
    assert_eq!(a, Action::Activate(0));
    act(&mut g, 0, a);
    // the +3/+3 waits on a target; the bot picks one and goes on
    while g.players[0].pending_target.is_some() {
        turn_of(&mut g, 0);
        let a = standard_action(&g);
        act(&mut g, 0, a);
    }
    assert_eq!(g.players[0].gold, 1);
    assert_eq!(g.players[0].board[1].attack, 1 + 3, "the Spore got the +3/+3");

    // with Malchezaar the next Refresh costs Health: not at 11 Health
    let mut g = game(10);
    turn_of(&mut g, 0);
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    g.players[0].shop.clear();
    g.players[0].shop_spell = None;
    g.players[0].free_rolls = 0;
    g.players[0].gold = 3;
    g.players[0].health = 11;
    assert_ne!(standard_action(&g), Action::Roll);
    g.players[0].health = 30;
    assert_eq!(standard_action(&g), Action::Roll);
}

#[test]
fn a_used_activate_shows_no_cost_until_the_next_turn() {
    let mut g = game(62);
    g.players[0].gold = 9;
    put_on_board(&mut g, 0, "Suspicious Prisonguard");
    turn_of(&mut g, 0);
    assert_eq!(board_slot(&view(&g, 0), 0)[SLOT_ACTIVATE_COST], 1.0);
    act(&mut g, 0, Action::Activate(0));
    if g.players[0].pending_target.is_some() {
        turn_of(&mut g, 0);
        let mut mask = [false; N_ACTIONS];
        g.legal_mask(&mut mask);
        let pick = Action::from_index(mask.iter().position(|&b| b).unwrap()).unwrap();
        act(&mut g, 0, pick);
    }
    assert_eq!(board_slot(&view(&g, 0), 0)[SLOT_ACTIVATE_COST], 0.0);
    next_turn(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(board_slot(&view(&g, 0), 0)[SLOT_ACTIVATE_COST], 1.0);
}
