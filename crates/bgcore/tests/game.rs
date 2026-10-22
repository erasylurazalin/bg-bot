use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::{Action, Data, Game, Keywords, Minion, N_ACTIONS, Opponent, Rng};

fn v01_data() -> Data {
    Data::from_toml(include_str!("data/v01-cards.toml"), include_str!("data/v01-rules.toml"))
        .unwrap()
}

fn game(seed: u64) -> Game {
    Game::new(Arc::new(v01_data()), seed)
}

fn s2_game(seed: u64) -> Game {
    Game::new(Arc::new(Data::bundled()), seed)
}

/// Let the other players toggle freeze until it's `p`'s action.
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
fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        g.step(Action::EndTurn).unwrap();
    }
}

fn take(g: &mut Game, name: &str) -> Minion {
    let c = g.data.cards.by_name(name).unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    g.players[0].make_card(&g.data.cards, c, false, 1)
}

/// Put a card from the pool straight onto a board.
fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = g.data.cards.by_name(name).unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
}

/// Replace a player's shop with the named cards, through the pool.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for n in names {
        let c = g.data.cards.by_name(n).unwrap();
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

fn legal(g: &Game) -> Vec<Action> {
    let mut mask = [false; N_ACTIONS];
    g.legal_mask(&mut mask);
    (0..N_ACTIONS).filter(|&i| mask[i]).map(|i| Action::from_index(i).unwrap()).collect()
}

#[test]
fn gold_goes_up_by_one_to_ten() {
    let mut g = game(1);
    let mut golds = vec![];
    for _ in 0..10 {
        golds.push(g.players[0].gold);
        next_turn(&mut g);
    }
    assert_eq!(golds, [3, 4, 5, 6, 7, 8, 9, 10, 10, 10]);
}

#[test]
fn upgrade_cost_drops_each_turn_and_resets_on_upgrade() {
    let mut g = game(2);
    assert_eq!(g.players[0].upgrade_cost, 5);
    next_turn(&mut g);
    assert_eq!(g.players[0].upgrade_cost, 4);
    act(&mut g, 0, Action::Level);
    assert_eq!((g.players[0].tier, g.players[0].gold, g.players[0].upgrade_cost), (2, 0, 7));
    next_turn(&mut g);
    assert_eq!(g.players[0].upgrade_cost, 6);
    for _ in 0..10 {
        next_turn(&mut g);
    }
    assert_eq!(g.players[0].upgrade_cost, 0);
}

#[test]
fn shop_size_and_tiers_follow_the_tavern_tier() {
    let mut g = game(3);
    assert_eq!(g.players[0].shop.len(), 3);
    assert!(g.players[0].shop.iter().all(|m| g.data.cards.get(m.card).tier == 1));
    next_turn(&mut g);
    act(&mut g, 0, Action::Level);
    next_turn(&mut g);
    assert_eq!(g.players[0].shop.len(), 4);
    assert!(g.players[0].shop.iter().all(|m| g.data.cards.get(m.card).tier <= 2));
}

#[test]
fn buy_sell_roll_cost_and_pay() {
    let mut g = game(4);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!((g.players[0].gold, g.players[0].hand.len(), g.players[0].shop.len()), (0, 1, 2));
    assert!(!g.is_legal(Action::Roll) || g.current_player() != Some(0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Sell(0));
    assert_eq!((g.players[0].gold, g.players[0].board.len()), (1, 0));
    act(&mut g, 0, Action::Roll);
    assert_eq!((g.players[0].gold, g.players[0].shop.len()), (0, 3));
}

#[test]
fn hand_holds_ten() {
    let mut g = game(5);
    for _ in 0..10 {
        g.give(0, "Wolf Pup").unwrap();
    }
    g.players[0].gold = 10;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Buy(0)));
    assert!(!legal(&g).iter().any(|a| matches!(a, Action::Buy(_))));
}

#[test]
fn board_holds_seven() {
    let mut g = game(6);
    for n in ["Wolf Pup", "Ram", "Viper", "Rust Bot", "Den Mother", "Warden", "Brute"] {
        put_on_board(&mut g, 0, n);
    }
    g.give(0, "Stag").unwrap();
    turn_of(&mut g, 0);
    assert!(!legal(&g).iter().any(|a| matches!(a, Action::Play { .. })));
}

#[test]
fn triple_makes_a_golden_with_double_stats_plus_buffs() {
    let mut g = game(7);
    g.give(0, "Wolf Pup").unwrap();
    g.give(0, "Wolf Pup").unwrap();
    g.players[0].hand[1].attack += 1; // a buffed copy
    act(&mut g, 0, Action::Play { hand: 1, pos: 0 });
    set_shop(&mut g, 0, &["Wolf Pup", "Rust Bot", "Rust Bot"]);
    act(&mut g, 0, Action::Buy(0));
    let p = &g.players[0];
    assert!(p.board.is_empty());
    assert_eq!(p.hand.len(), 1);
    let m = p.hand[0];
    assert!(m.golden);
    assert_eq!((m.attack, m.health), (5, 4));
}

#[test]
fn playing_a_golden_discovers_from_one_tier_up() {
    let mut g = game(8);
    for _ in 0..2 {
        g.give(0, "Wolf Pup").unwrap();
    }
    set_shop(&mut g, 0, &["Wolf Pup", "Rust Bot", "Rust Bot"]);
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let p = &g.players[0];
    assert_eq!(p.discover.len(), 3);
    assert!(p.discover.iter().all(|&c| g.data.cards.get(c).tier == 2));
    let mut uniq = p.discover.to_vec();
    uniq.dedup();
    assert_eq!(uniq.len(), 3);
    // only Choose is legal now
    turn_of(&mut g, 0);
    assert!(legal(&g).iter().all(|a| matches!(a, Action::Choose(_))));
    let pick = g.players[0].discover[2];
    act(&mut g, 0, Action::Choose(2));
    assert!(g.players[0].discover.is_empty());
    assert_eq!(g.players[0].hand[0].card, pick);
}

#[test]
fn selling_a_golden_returns_three_copies() {
    let mut g = game(9);
    let ram = g.data.cards.by_name("Ram").unwrap() as usize;
    g.give(0, "Ram").unwrap();
    g.give(0, "Ram").unwrap();
    g.players[0].tier = 2;
    set_shop(&mut g, 0, &["Ram", "Wolf Pup", "Wolf Pup"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Choose(0));
    let before = g.pool[ram];
    let at = g.players[0].board.iter().position(|m| m.golden).unwrap();
    act(&mut g, 0, Action::Sell(at as u8));
    assert_eq!(g.pool[ram], before + 3);
}

#[test]
fn targeted_battlecry_waits_for_a_target() {
    let mut g = game(10);
    put_on_board(&mut g, 0, "Wolf Pup");
    put_on_board(&mut g, 0, "Rust Bot");
    g.give(0, "Scrap Hound").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    assert!(g.players[0].pending_target.is_some());
    turn_of(&mut g, 0);
    let l = legal(&g);
    assert_eq!(l, [Action::Target(0), Action::Target(1)]);
    act(&mut g, 0, Action::Target(1));
    let b = &g.players[0].board;
    assert_eq!((b[1].attack, b[1].health), (2, 3));
    assert_eq!((b[0].attack, b[0].health), (2, 2));
}

#[test]
fn targeted_battlecry_with_one_target_picks_it() {
    let mut g = game(11);
    put_on_board(&mut g, 0, "Wolf Pup");
    put_on_board(&mut g, 0, "Rust Bot");
    g.give(0, "Tinkerer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let p = &g.players[0];
    assert!(p.pending_target.is_none());
    assert_eq!(name(&g, &p.board[2]), "Rust Bot");
    assert_eq!((p.board[2].attack, p.board[2].health), (3, 4));
    assert_eq!((p.board[1].attack, p.board[1].health), (2, 2));
}

#[test]
fn targeted_battlecry_with_no_target_does_nothing() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Wolf Pup");
    g.give(0, "Tinkerer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    let p = &g.players[0];
    assert!(p.pending_target.is_none());
    assert_eq!((p.board[0].attack, p.board[1].attack), (2, 3));
}

#[test]
fn stag_buffs_other_beasts_on_the_board_only() {
    let mut g = game(13);
    put_on_board(&mut g, 0, "Wolf Pup");
    put_on_board(&mut g, 0, "Rust Bot");
    g.give(0, "Stag").unwrap();
    g.give(0, "Viper").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    let p = &g.players[0];
    assert_eq!((p.board[0].attack, p.board[0].health), (3, 3)); // Pup
    assert_eq!((p.board[1].attack, p.board[1].health), (1, 2)); // Rust Bot
    assert_eq!((p.board[2].attack, p.board[2].health), (4, 4)); // Stag itself
    assert_eq!((p.hand[0].attack, p.hand[0].health), (2, 1)); // Viper in hand
}

#[test]
fn golden_battlecry_doubles() {
    let mut g = game(14);
    put_on_board(&mut g, 0, "Wolf Pup");
    for _ in 0..2 {
        g.give(0, "Stag").unwrap();
    }
    g.players[0].tier = 3;
    g.players[0].gold = 10;
    set_shop(&mut g, 0, &["Stag", "Wolf Pup", "Wolf Pup"]);
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    let pup = g.players[0].board[0];
    assert_eq!((pup.attack, pup.health), (4, 4));
}

#[test]
fn frozen_shop_is_kept_and_topped_up() {
    let mut g = game(15);
    let kept: Vec<_> = g.players[0].shop[1..].to_vec();
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Freeze);
    next_turn(&mut g);
    let p = &g.players[0];
    assert!(!p.frozen);
    assert_eq!(p.shop.len(), 3);
    assert_eq!(p.shop[..2], kept[..]);
}

#[test]
fn rolling_clears_freeze() {
    let mut g = game(16);
    act(&mut g, 0, Action::Freeze);
    act(&mut g, 0, Action::Roll);
    assert!(!g.players[0].frozen);
}

#[test]
fn turn_ends_after_the_action_cap() {
    let mut g = game(17);
    let cap = g.data.rules.max_actions_per_turn;
    for _ in 0..cap {
        act(&mut g, 0, Action::Freeze);
    }
    assert!(g.players[0].done);
    assert!(g.current_player() != Some(0));
}

#[test]
fn damage_cap_by_turn_and_players_left() {
    let r = Data::bundled().rules;
    let cases = [(1, 8, Some(5)), (4, 8, Some(5)), (5, 8, Some(10)), (7, 5, Some(10)), (8, 8, Some(15)), (30, 5, Some(15)), (30, 4, None), (2, 4, None), (6, 3, None)];
    for (turn, alive, cap) in cases {
        assert_eq!(r.damage_cap(turn, alive), cap, "turn {turn}, {alive} alive");
    }
}

#[test]
fn losing_a_fight_costs_tier_plus_survivors_capped() {
    let mut g = game(18);
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Brute");
    next_turn(&mut g);
    // tier 1 + Brute's tier 5 = 6, capped at 5 on turn 1
    assert_eq!(g.players[q as usize].health, 25);
    assert_eq!(g.players[0].health, 30);
    // what each saw of the other
    assert_eq!(g.players[q as usize].seen[0].board.len(), 1);
    assert_eq!(g.players[q as usize].seen[0].turn, 1);
}

#[test]
fn ties_deal_no_damage() {
    let mut g = game(19);
    next_turn(&mut g);
    assert!(g.players.iter().all(|p| p.health == 30));
}

fn random_game(seed: u64, record: Option<&mut Vec<Action>>) -> Game {
    let mut g = game(seed);
    let mut rng = Rng::new(seed ^ 0xabcdef);
    let mut rec = record;
    while !g.over {
        let alive = g.alive();
        let ghosts = g.players.iter().filter(|p| p.alive && matches!(p.opponent, Some(Opponent::Ghost(_)))).count();
        assert_eq!(ghosts, alive % 2, "turn {}", g.turn);
        let a = random_action(&g, &mut rng);
        if let Some(r) = rec.as_deref_mut() {
            r.push(a);
        }
        g.step(a).unwrap();
        g.check_invariants().unwrap();
    }
    g
}

#[test]
fn random_games_finish_with_a_full_ranking() {
    for seed in 0..30 {
        let g = random_game(seed, None);
        let mut p = g.placements();
        p.sort();
        assert_eq!(p, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(g.turn <= g.data.rules.max_turns);
    }
}

#[test]
fn seed_and_actions_replay_a_game_exactly() {
    let mut actions = vec![];
    let a = random_game(42, Some(&mut actions));
    let mut b = game(42);
    for &x in &actions {
        b.step(x).unwrap();
    }
    assert!(b.over);
    assert_eq!(a.players, b.players);
    assert_eq!(a.pool, b.pool);
    assert_eq!(a.rng, b.rng);
}

#[test]
fn pairing_avoids_the_last_opponent() {
    let mut g = game(20);
    let mut repeats = 0;
    for _ in 0..30 {
        let last: Vec<_> = g.players.iter().map(|p| p.last_opponent).collect();
        for (p, l) in g.players.iter().zip(&last) {
            if l.is_some() && p.opponent == *l {
                repeats += 1;
            }
        }
        next_turn(&mut g);
    }
    assert_eq!(repeats, 0);
}

#[test]
fn illegal_actions_are_refused() {
    let mut g = game(21);
    assert!(g.step(Action::Sell(0)).is_err());
    assert!(g.step(Action::Target(0)).is_err());
    assert!(g.step(Action::Level).is_err()); // 3 gold, costs 5
}

#[test]
fn rule_bots_play_whole_games() {
    for seed in 0..20 {
        let mut g = game(seed);
        while !g.over {
            g.step(bgcore::bots::rule_action(&g)).unwrap();
        }
        g.check_invariants().unwrap();
    }
}

#[test]
fn observation_shows_only_what_the_player_saw() {
    use bgcore::obs::{OBS_SIZE, write};
    let mut g = game(22);
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    let q = q as usize;
    put_on_board(&mut g, q, "Brute");
    let mut out = vec![f32::NAN; OBS_SIZE];
    write(&g, 0, &mut out);
    assert!(out.iter().all(|x| x.is_finite()));
    let before = out.clone();
    // q's board isn't visible before the fight...
    let opp = |out: &[f32]| {
        let k = q - 1; // seat q sits q places after seat 0
        let start = bgcore::obs::SCALARS
            + (bgcore::SHOP + bgcore::HAND + bgcore::BOARD + bgcore::DISCOVER) * bgcore::obs::MINION_FEATURES
            + k * (bgcore::obs::OPP_SCALARS + bgcore::BOARD * bgcore::obs::MINION_FEATURES);
        out[start..start + bgcore::obs::OPP_SCALARS + bgcore::BOARD * bgcore::obs::MINION_FEATURES].to_vec()
    };
    let o = opp(&before);
    assert_eq!(o[5], 1.0); // next opponent
    assert_eq!(o[3], 0.0); // never seen
    assert_eq!(o[bgcore::obs::OPP_SCALARS], 0.0); // no minion shown
    // ...and is after it
    next_turn(&mut g);
    write(&g, 0, &mut out);
    let o = opp(&out);
    assert_eq!(o[3], 1.0);
    assert_eq!(o[bgcore::obs::OPP_SCALARS], 1.0);
    assert_eq!(o[bgcore::obs::OPP_SCALARS + 2], 8.0); // Brute's attack
}

#[test]
fn magnetic_nesting_and_utility_drone() {
    let mut g = s2_game(100);
    put_on_board(&mut g, 0, "Cord Puller"); // 1/1 Mech at slot 0
    put_on_board(&mut g, 0, "Utility Drone"); // 4/5 Mech at slot 1

    // Build a Lullabot carrying another Lullabot directly in hand.
    let mut l1 = take(&mut g, "Lullabot"); // 2/2, pool_copies = 1
    let l2 = take(&mut g, "Lullabot"); // 2/2, pool_copies = 1
    let l_slot = g.data.cards.get(l2.card).pool_slot as usize;
    g.consumed[l_slot] += l2.pool_copies as u32;
    l1.magnetize(&g.data.cards, l2);
    assert_eq!((l1.attack, l1.health, l1.magnetizations), (4, 4, 1));
    g.players[0].hand.push(l1);

    // Merge the nested Lullabot into Cord Puller at slot 0.
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let host = g.players[0].board[0];
    // Cord Puller (1/1) + nested Lullabot (4/4) = 5/5, with 2 Magnetizations.
    assert_eq!((host.attack, host.health), (5, 5));
    assert_eq!(host.magnetizations, 2);
    assert_eq!(host.n_attached, 2);

    // End player 0's turn: 2 Lullabots give +2 Health, Utility Drone gives +8/+10 (2 * +4/+5).
    act(&mut g, 0, Action::EndTurn);
    let host = g.players[0].board[0];
    assert_eq!((host.attack, host.health), (5 + 8, 5 + 2 + 10));
    g.check_invariants().unwrap();
}

#[test]
fn magnetic_merge_into_full_board_and_normal_placement() {
    let mut g = s2_game(101);
    // Place a Beast at slot 0, and a Magnetic placed not next to a Mech places normally.
    put_on_board(&mut g, 0, "Buzzing Vermin"); // Beast at slot 0
    g.give(0, "Lullabot").unwrap();
    // Playing Lullabot at pos 0 (left of a Beast) or pos 1 (right end) places it normally.
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].board.len(), 2);
    assert_eq!(name(&g, &g.players[0].board[0]), "Lullabot");
    assert_eq!(g.players[0].board[0].magnetizations, 0);

    // Fill the rest of the board to 7 minions (slot 0 is Lullabot, a Mech; slots 1..7 are Beasts).
    for _ in 0..5 {
        put_on_board(&mut g, 0, "Buzzing Vermin");
    }
    assert!(g.players[0].board.is_full());
    g.give(0, "Lullabot").unwrap();
    turn_of(&mut g, 0);
    // Board is full: merging into the Mech at slot 0 is legal; placing at slot 1 (Beast) or 7 is not.
    assert!(g.is_legal(Action::Play { hand: 0, pos: 0 }));
    assert!(!g.is_legal(Action::Play { hand: 0, pos: 1 }));
    assert!(!g.is_legal(Action::Play { hand: 0, pos: 7 }));

    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].board.len(), 7);
    assert_eq!(g.players[0].board[0].magnetizations, 1);
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (4, 4));
    g.check_invariants().unwrap();
}

#[test]
fn volumizer_bonus_reaches_hand_shop_and_host_with_volumizers() {
    let mut g = s2_game(102);
    g.players[0].tier = 2;
    put_on_board(&mut g, 0, "Cord Puller"); // 1/1 host at slot 0
    // Magnetize 3 Volumizers (Blue, Green, Red) into Cord Puller so it holds 3 Volumizers.
    for v_name in ["Blue Volumizer", "Green Volumizer", "Red Volumizer"] {
        g.give(0, v_name).unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    }
    assert_eq!(g.players[0].board[0].volumizers, 3);
    // Bonus so far: +3/+0 (Red) + +1/+1 (Green) + +0/+3 (Blue) = +4/+4.
    assert_eq!(g.players[0].volumizer_bonus, (4, 4));

    // Put a Red Volumizer in the shop and a Blue Volumizer in hand, and another Green Volumizer to play.
    set_shop(&mut g, 0, &["Red Volumizer", "Zoatroid", "Zoatroid"]);
    g.give(0, "Blue Volumizer").unwrap();
    g.give(0, "Green Volumizer").unwrap();

    let shop_before = (g.players[0].shop[0].attack, g.players[0].shop[0].health);
    let hand_before = (g.players[0].hand[0].attack, g.players[0].hand[0].health);
    let host_before = (g.players[0].board[0].attack, g.players[0].board[0].health);

    // Play the Green Volumizer (hand slot 1) as a standalone minion at board pos 1.
    // Its +1/+1 bonus reaches the shop Volumizer (+1/+1), the hand Volumizer (+1/+1),
    // and the host with 3 Volumizers inside (+3/+3).
    act(&mut g, 0, Action::Play { hand: 1, pos: 1 });
    assert_eq!(
        (g.players[0].shop[0].attack, g.players[0].shop[0].health),
        (shop_before.0 + 1, shop_before.1 + 1)
    );
    assert_eq!(
        (g.players[0].hand[0].attack, g.players[0].hand[0].health),
        (hand_before.0 + 1, hand_before.1 + 1)
    );
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (host_before.0 + 3, host_before.1 + 3)
    );
    g.check_invariants().unwrap();
}

#[test]
fn spark_snapper_and_mechagnome_interpreter_distinguish_play_from_direct_attach() {
    let mut g = s2_game(103);
    put_on_board(&mut g, 0, "Cord Puller"); // 1/1 at slot 0
    put_on_board(&mut g, 0, "Spark Snapper"); // 6/8 at slot 1
    put_on_board(&mut g, 0, "Mechagnome Interpreter"); // 3/1 at slot 2

    // Play Lullabot (2/2, Magnetic) onto Cord Puller at slot 0:
    // - Lullabot merges (+2/+2, mag = 1).
    // - Interpreter sees the played Magnetic (+3/+1).
    // - Spark Snapper sees the played Magnetic, attaches a 2/3 Satellite (+2/+3, mag = 2),
    //   and improves itself (improvements = 1).
    // - Interpreter ALSO sees the directly attached Satellite (+3/+1), while Spark Snapper
    //   does NOT trigger again on the Satellite.
    g.give(0, "Lullabot").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let host = g.players[0].board[0];
    assert_eq!(host.magnetizations, 2);
    assert_eq!((host.attack, host.health), (1 + 2 + 3 + 2 + 3, 1 + 2 + 1 + 3 + 1));
    assert_eq!(g.players[0].board[1].improvements, 1);

    // Put Resourceful Robot on board at slot 3 and end turn:
    // Resourceful Robot directly attaches a Volumizer to itself (not a play).
    // Interpreter buffs Resourceful Robot (+3/+1), while Spark Snapper does NOT attach a Satellite
    // (so Resourceful Robot has 1 Magnetization, not 2, and Spark Snapper stays at improvements = 1).
    put_on_board(&mut g, 0, "Resourceful Robot");
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].board[3].magnetizations, 1);
    assert_eq!(g.players[0].board[1].improvements, 1);
    g.check_invariants().unwrap();
}

#[test]
fn beetle_bonus_from_forest_rover_and_ravaging_scorpid() {
    let mut g = s2_game(104);
    g.give(0, "Forest Rover").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].beetle_bonus, (2, 1));

    // Check that Buzzing Vermin's Beetle spawns as 4/3 (2/2 + 2/1) in a fight:
    // Without the bonus, Vermin (1/1) + Beetle (2/2) loses to a 2/4 minion;
    // with the +2/+1 bonus, Vermin (1/1) + Beetle (4/3) kills a 2/4 minion and survives at 4/1.
    let db = &g.data.cards;
    let vermin = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
    let mut enemy = db.make(db.by_name("Zoatroid").unwrap(), false);
    enemy.attack = 2;
    enemy.health = 4;
    let out_plain = bgcore::fight_with(
        db,
        &[vermin],
        &[enemy],
        [Default::default(), Default::default()],
        500,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out_plain.winner, Some(1));
    let out_buffed = bgcore::fight_with(
        db,
        &[vermin],
        &[enemy],
        [g.players[0].counters(), Default::default()],
        500,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out_buffed.winner, Some(0));

    // Now sell Forest Rover and put Ravaging Scorpid on player 0's board for a fight.
    act(&mut g, 0, Action::Sell(0));
    put_on_board(&mut g, 0, "Ravaging Scorpid");
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, q as usize, "Buzzing Vermin");
    next_turn(&mut g);
    // Scorpid attacked in the fight, raising player 0's beetle_bonus by +4/+4 for future fights.
    assert!(g.players[0].beetle_bonus.0 >= 6);
    assert!(g.players[0].beetle_bonus.1 >= 5);
}

#[test]
fn lurking_leviathan_improvement_survives_fight_and_goldrinn_buff_does_not() {
    let mut g = s2_game(105);
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    let q = q as usize;

    put_on_board(&mut g, 0, "Buzzing Vermin"); // Taunt, dies first and summons a Beetle
    put_on_board(&mut g, 0, "Goldrinn the Great Wolf");
    put_on_board(&mut g, 0, "Lurking Leviathan");
    // Give Goldrinn Taunt so the enemy kills both Vermin and Goldrinn.
    g.players[0].board[1].keywords.set(Keywords::TAUNT);

    put_on_board(&mut g, q, "Mama Bear");
    g.players[q].board[0].attack = 10;
    g.players[q].board[0].health = 20;

    next_turn(&mut g);

    // After the fight:
    // - Goldrinn's +7/+7 combat buff did NOT stay on the real board.
    // - Lurking Leviathan's permanent improvement DID stay on the real board (improvements == 1).
    let leviathan = g.players[0].board[2];
    assert_eq!((leviathan.attack, leviathan.health), (3, 9));
    assert_eq!(leviathan.improvements, 1);

    // Playing a Beast in the shop now gets +6 Attack from the improved Lurking Leviathan (instead of +3).
    g.give(0, "Flittering Bat").unwrap(); // 1/4 base
    act(&mut g, 0, Action::Play { hand: 0, pos: 3 });
    assert_eq!((g.players[0].board[3].attack, g.players[0].board[3].health), (1 + 6, 4));
    assert_eq!(g.players[0].board[2].improvements, 2);
}

#[test]
fn brann_and_drakkari_multiply_triggers_including_golden() {
    let mut g = s2_game(106);
    put_on_board(&mut g, 0, "Brann Bronzebeard");
    g.give(0, "Southsea Busker").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].gold_next_turn, 2);

    // Golden Brann triggers Battlecries 3 times.
    // Take 2 more Branns from the pool so the golden Brann holds 3 pool copies.
    let brann_id = g.data.cards.by_name("Brann Bronzebeard").unwrap() as usize;
    g.pool[brann_id] -= 2;
    g.players[0].board[0].golden = true;
    g.players[0].board[0].pool_copies = 3;

    g.give(0, "Forest Rover").unwrap(); // +2/+1 per trigger
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    assert_eq!(g.players[0].beetle_bonus, (6, 3));

    // Drakkari triggers end of turn twice, golden three times.
    put_on_board(&mut g, 0, "Lullabot"); // slot 3: 2/2, +1 HP at end of turn
    put_on_board(&mut g, 0, "Drakkari Enchanter"); // slot 4
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].board[3].health, 4); // +2 HP with plain Drakkari

    next_turn(&mut g);
    let drak_id = g.data.cards.by_name("Drakkari Enchanter").unwrap() as usize;
    g.pool[drak_id] -= 2;
    g.players[0].board[4].golden = true;
    g.players[0].board[4].pool_copies = 3;
    let hp_before = g.players[0].board[3].health;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].board[3].health, hp_before + 3); // +3 HP with golden Drakkari
}

#[test]
fn sprightly_scarab_choose_one_and_patient_scout_discover() {
    let mut g = s2_game(107);
    put_on_board(&mut g, 0, "Buzzing Vermin"); // 1/1 at slot 0
    put_on_board(&mut g, 0, "Flittering Bat"); // 1/4 at slot 1
    g.give(0, "Sprightly Scarab").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });

    turn_of(&mut g, 0);
    assert_eq!(legal(&g), [Action::Choose(0), Action::Choose(1)]);
    // Option 1: +4 Attack and Windfury to a Beast
    act(&mut g, 0, Action::Choose(1));
    turn_of(&mut g, 0);
    assert_eq!(legal(&g), [Action::Target(0), Action::Target(1)]);
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].board[1].attack, 5);
    assert!(g.players[0].board[1].keywords.has(Keywords::WINDFURY));

    // Patient Scout improves its tier at the end of turn and discovers on sell.
    put_on_board(&mut g, 0, "Patient Scout"); // slot 3, starts at tier 1
    next_turn(&mut g); // End of turn 1 improves Patient Scout to tier 2
    assert_eq!(g.players[0].board[3].improvements, 1);
    act(&mut g, 0, Action::Sell(3));
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(g.players[0].discover.iter().all(|&c| g.data.cards.get(c).tier == 2));
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].hand.len(), 1);
    g.check_invariants().unwrap();
}

#[test]
fn zoatroid_blue_shell_accord_o_tron_and_auto_reveille() {
    let mut g = s2_game(108);
    // Zoatroid gives a 0/2 Tentacle with Taunt when sold.
    put_on_board(&mut g, 0, "Zoatroid");
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(name(&g, &g.players[0].hand[0]), "Tentacle");
    assert_eq!(g.players[0].hand[0].pool_copies, 0);
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Sell(0)); // selling the generated Tentacle does not change the pool

    // Accord-o-Tron gives +1 Gold at start of turn (even when Magnetized into a host).
    put_on_board(&mut g, 0, "Cord Puller");
    g.give(0, "Accord-o-Tron").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 }); // merges into Cord Puller
    next_turn(&mut g);
    // Turn 2 base gold is 4 + 1 from Accord-o-Tron = 5.
    assert_eq!(g.players[0].gold, 5);

    // Tortollan Blue Shell sells for 1 gold normally, and 5 gold after losing a combat.
    put_on_board(&mut g, 0, "Tortollan Blue Shell");
    g.players[0].lost_last_combat = true;
    let gold_before = g.players[0].gold;
    act(&mut g, 0, Action::Sell(1));
    assert_eq!(g.players[0].gold, gold_before + 5);

    // Auto Reveille triggers after buying 3 cards.
    put_on_board(&mut g, 0, "Auto Reveille");
    g.players[0].gold = 20;
    set_shop(&mut g, 0, &["Buzzing Vermin", "Flittering Bat", "Zoatroid"]);
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].board[1].magnetizations, 0);
    act(&mut g, 0, Action::Buy(0));
    // 3rd buy triggers Auto Reveille: 1 Magnetization on itself + 1 Volumizer copy in hand.
    assert_eq!(g.players[0].board[1].magnetizations, 1);
    assert_eq!(g.players[0].hand.len(), 4);
    g.check_invariants().unwrap();
}

#[test]
fn pool_invariants_hold_through_merges_generated_cards_and_sells() {
    for seed in 0..30 {
        let mut g = s2_game(seed);
        let mut rng = Rng::new(seed ^ 0x123456);
        while !g.over {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
    }
    for seed in 0..20 {
        let mut g = s2_game(seed);
        while !g.over {
            let a = bgcore::bots::rule_action(&g);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
    }
}

