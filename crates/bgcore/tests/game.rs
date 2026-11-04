use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::{Tribe, Action, Data, Game, Keywords, Minion, N_ACTIONS, Opponent, Rng};

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]))
}

/// Let the other players toggle freeze until it's `p`'s action.
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

/// Everyone ends their turn until the next turn starts.
fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
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
fn damage_cap_by_turn_and_players_left() {
    let r = Data::bundled().rules;
    let cases = [(1, 8, Some(5)), (4, 8, Some(5)), (5, 8, Some(10)), (7, 5, Some(10)), (8, 8, Some(15)), (30, 5, Some(15)), (30, 4, None), (2, 4, None), (6, 3, None)];
    for (turn, alive, cap) in cases {
        assert_eq!(r.damage_cap(turn, alive), cap, "turn {turn}, {alive} alive");
    }
}

#[test]
fn magnetic_nesting_and_utility_drone() {
    let mut g = game(100);
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
    let mut g = game(101);
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
fn drone_duplicator_doubles_the_played_magnetic_and_its_satellite() {
    // `10_27_04_30_04-g2` turn 10: a Drone Duplicator at 15/17 Activated, then
    // a Red Volumizer (24/20) played from hand with a 26/39 Satellite from
    // Spark Snapper. The log's host ends at 115/135, which is 15/17 plus the
    // whole package (24 + 26 = 50 Attack, 20 + 39 = 59 Health) twice. Here the
    // board starts from the cards' own 5/2.
    let mut g = game(105);
    put_on_board(&mut g, 0, "Drone Duplicator");
    put_on_board(&mut g, 0, "Spark Snapper");
    // The Snapper's Satellite is its own size: 26/39 here.
    g.players[0].board[1].improvements = 12; // 2/3, each step +2/+3
    let mut red = take(&mut g, "Red Volumizer");
    red.attack = 24;
    red.health = 20;
    g.players[0].hand.push(red);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].board[0].extra_magnetize, 1);
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let host = g.players[0].board[0];
    assert_eq!(
        (host.attack, host.health),
        (5 + 2 * (24 + 26), 2 + 2 * (20 + 39)),
        "the played Volumizer and its Satellite both happen twice"
    );
    assert_eq!(g.players[0].board[0].extra_magnetize, 0, "the extra is spent");
    g.check_invariants().unwrap();
}

#[test]
fn every_fresh_volumizer_copy_a_beatboxer_takes_is_bigger_than_the_one_before() {
    // `10_29_22_20_04-g3` turn 14 step 20: a golden Green Volumizer is played
    // onto a golden Drone Duplicator with two Polarizing Beatboxers on the
    // board. Each Beatboxer takes three copies of it and the log's two come
    // out +4140 and +4146 Attack, where three copies of the card as it was
    // played (1374) would be +4122: the copy the second one gets is minted
    // after the first raised "your Volumizers have +A/+H", so it is a Green
    // Volumizer's +1/+1 (+2/+2 when golden) bigger. Here the cards' own
    // numbers.
    let mut g = game(107);
    g.players[0].tier = 2;
    put_on_board(&mut g, 0, "Cord Puller"); // 1/1 Mech, the host
    put_on_board(&mut g, 0, "Polarizing Beatboxer"); // 5/10
    put_on_board(&mut g, 0, "Polarizing Beatboxer"); // 5/10
    g.give(0, "Green Volumizer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let host = g.players[0].board[0];
    assert_eq!(
        (host.attack, host.health),
        (1 + 3, 1 + 3),
        "the host takes the played Volumizer's own stats"
    );
    let first = g.players[0].board[1];
    let second = g.players[0].board[2];
    assert_eq!(
        (first.attack, first.health),
        (5 + 3, 10 + 3),
        "the first Beatboxer's copy is the card as it was played"
    );
    assert_eq!(
        (second.attack, second.health),
        (5 + 4, 10 + 4),
        "the second Beatboxer's copy carries the +1/+1 the first one paid"
    );
    assert_eq!(
        g.players[0].volumizer_bonus,
        (3, 3),
        "the play and both copies count as Magnetized for the first time"
    );
    g.check_invariants().unwrap();
}

#[test]
fn a_volumizers_triple_counts_the_aura_once_and_each_copys_own_buffs() {
    // `10_29_22_20_04-g3` turn 11 (buying a third Blue Volumizer): the log's
    // three copies stand at 69/108 and 71/111 with the +24/+39 "your
    // Volumizers have" pair on them, and 25/42 on the one just bought, and the
    // Golden Blue goes into the hand at 116/180. That is the printed 1/3
    // doubled, plus the three copies' own bonuses (44/66, 46/69, 0/0), plus
    // the pair once: the Golden is one Volumizer itself.
    let mut g = game(106);
    g.players[0].volumizer_bonus = (24, 39);
    for (attack, health) in [(69, 108), (71, 111)] {
        put_on_board(&mut g, 0, "Blue Volumizer");
        let i = g.players[0].board.len() - 1;
        g.players[0].board[i].attack = attack;
        g.players[0].board[i].health = health;
    }
    set_shop(&mut g, 0, &["Blue Volumizer"]);
    g.players[0].shop[0].attack = 25;
    g.players[0].shop[0].health = 42;
    act(&mut g, 0, Action::Buy(0));
    let gold = g.players[0]
        .hand
        .iter()
        .find(|m| g.data.cards.get(m.card).name == "Blue Volumizer")
        .copied()
        .expect("the triple's Golden Blue is in the hand");
    assert!(gold.golden);
    assert_eq!((gold.attack, gold.health), (116, 180));
    assert!(g.players[0].board.is_empty(), "all three copies went in");
    g.check_invariants().unwrap();
}

#[test]
fn volumizer_bonus_reaches_hand_and_shop_but_not_the_ones_inside_a_host() {
    let mut g = game(102);
    g.players[0].tier = 2;
    put_on_board(&mut g, 0, "Cord Puller"); // 1/1 host at slot 0
    // Magnetize 3 Volumizers (Blue, Green, Red) into Cord Puller. Inside a host
    // they stop counting as Volumizers (Era, 2026-11-03).
    for v_name in ["Blue Volumizer", "Green Volumizer", "Red Volumizer"] {
        g.give(0, v_name).unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    }
    assert_eq!(g.players[0].board[0].volumizers, 0);
    // Bonus so far: +3/+0 (Red) + +1/+1 (Green) + +0/+3 (Blue) = +4/+4.
    assert_eq!(g.players[0].volumizer_bonus, (4, 4));

    // Put a Red Volumizer in the shop and a Blue Volumizer in hand, and another Green Volumizer to play.
    set_shop(&mut g, 0, &["Red Volumizer", "Cord Puller", "Cord Puller"]);
    g.give(0, "Blue Volumizer").unwrap();
    g.give(0, "Green Volumizer").unwrap();

    let shop_before = (g.players[0].shop[0].attack, g.players[0].shop[0].health);
    let hand_before = (g.players[0].hand[0].attack, g.players[0].hand[0].health);
    let host_before = (g.players[0].board[0].attack, g.players[0].board[0].health);

    // Play the Green Volumizer (hand slot 1) as a standalone minion at board pos 1.
    // Its +1/+1 bonus reaches the shop Volumizer (+1/+1), the hand Volumizer (+1/+1),
    // but not the host with 3 Volumizers inside.
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
        host_before
    );
    g.check_invariants().unwrap();
}

#[test]
fn spark_snapper_and_mechagnome_interpreter_distinguish_play_from_direct_attach() {
    let mut g = game(103);
    put_on_board(&mut g, 0, "Cord Puller"); // 1/1 at slot 0
    put_on_board(&mut g, 0, "Spark Snapper"); // 6/8 at slot 1
    put_on_board(&mut g, 0, "Mechagnome Interpreter"); // 3/1 at slot 2

    // Play Lullabot (2/2, Magnetic) onto Cord Puller at slot 0:
    // - Spark Snapper sees the played Mech while it is a minion of its own,
    //   attaches a 2/3 Satellite to it (+2/+3) and improves itself (1).
    // - Lullabot and its Satellite then merge into the host together (+4/+5),
    //   one Magnetization: the Satellite measured too, so the host's own count
    //   goes up by one and not two (`10_29_22_20_04-g3` turn 13 step 10).
    // - Interpreter sees the played Magnetic and the Satellite (+3/+1 each).
    g.give(0, "Lullabot").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let host = g.players[0].board[0];
    assert_eq!(host.magnetizations, 1);
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
    let mut g = game(104);
    g.give(0, "Forest Rover").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].beetle_bonus, (2, 1));

    // Check that Buzzing Vermin's Beetle spawns as 4/3 (2/2 + 2/1) in a fight:
    // Without the bonus, Vermin (1/1) + Beetle (2/2) loses to a 2/4 minion;
    // with the +2/+1 bonus, Vermin (1/1) + Beetle (4/3) kills a 2/4 minion and survives at 4/1.
    let db = &g.data.cards;
    let vermin = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
    let mut enemy = db.make(db.by_name("Microbot").unwrap(), false);
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
        [g.players[0].counters(&g.data, g.lobby), Default::default()],
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
    let mut g = game(105);
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    let q = q as usize;

    put_on_board(&mut g, 0, "Buzzing Vermin"); // Taunt, dies first and summons a Beetle
    put_on_board(&mut g, 0, "Goldrinn the Great Wolf");
    put_on_board(&mut g, 0, "Lurking Leviathan");
    // Give Goldrinn Taunt so the enemy kills both Vermin and Goldrinn.
    g.players[0].board[1].keywords.set(Keywords::TAUNT);

    put_on_board(&mut g, q, "Turquoise Skitterer");
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
fn triple_keeps_the_leviathan_improvement_count() {
    let mut g = game(107);
    put_on_board(&mut g, 0, "Lurking Leviathan");
    put_on_board(&mut g, 0, "Lurking Leviathan");
    // Two Leviathans that have already improved twice and three times.
    g.players[0].board[0].improvements = 2;
    g.players[0].board[1].improvements = 3;
    set_shop(&mut g, 0, &["Lurking Leviathan"]);
    g.players[0].gold = 3;
    act(&mut g, 0, Action::Buy(0));
    // The golden one takes the highest of the three counts, not the sum.
    let h = g.players[0]
        .hand
        .iter()
        .position(|m| m.golden)
        .expect("the triple made a golden Leviathan in hand");
    assert_eq!(g.players[0].hand[h].improvements, 3);
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    // Playing a golden minion gives a Triple Reward: take its first option.
    if !g.players[0].discover.is_empty() {
        act(&mut g, 0, Action::Choose(0));
    }
    // A golden Leviathan applies the count at +6 per step: 3 * 2 * (1 + 3).
    g.give(0, "Flittering Bat").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h, pos: 1 });
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (1 + 24, 4));
    assert_eq!(g.players[0].board[0].improvements, 4);
    g.check_invariants().unwrap();
}

#[test]
fn triple_keeps_the_highest_patient_scout_count() {
    let mut g = game(108);
    put_on_board(&mut g, 0, "Patient Scout");
    put_on_board(&mut g, 0, "Patient Scout");
    g.players[0].board[0].improvements = 2;
    g.players[0].board[1].improvements = 4;
    set_shop(&mut g, 0, &["Patient Scout"]);
    g.players[0].gold = 3;
    act(&mut g, 0, Action::Buy(0));
    let h = g.players[0]
        .hand
        .iter()
        .position(|m| m.golden)
        .expect("the triple made a golden Patient Scout in hand");
    // The highest of 2, 4 and 0, not their sum.
    assert_eq!(g.players[0].hand[h].improvements, 4);
    // Its sale discovers from tier 1 + 4 = 5.
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    if !g.players[0].discover.is_empty() {
        act(&mut g, 0, Action::Choose(0));
    }
    act(&mut g, 0, Action::Sell(0));
    assert!(!g.players[0].discover.is_empty());
    assert!(
        g.players[0]
            .discover
            .iter()
            .all(|&c| g.data.cards.get(c).tier == 5)
    );
    g.check_invariants().unwrap();
}

#[test]
fn brann_and_drakkari_multiply_triggers_including_golden() {
    let mut g = game(106);
    put_on_board(&mut g, 0, "Brann Bronzebeard");
    g.give(0, "Forest Rover").unwrap(); // +2/+1 for the Beetles, twice with Brann
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].beetle_bonus, (4, 2));

    // Golden Brann triggers Battlecries 3 times.
    // Take 2 more Branns from the pool so the golden Brann holds 3 pool copies.
    let brann_id = g.data.cards.by_name("Brann Bronzebeard").unwrap() as usize;
    g.pool[brann_id] -= 2;
    g.players[0].board[0].golden = true;
    g.players[0].board[0].pool_copies = 3;

    g.give(0, "Forest Rover").unwrap(); // +2/+1 per trigger
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    assert_eq!(g.players[0].beetle_bonus, (4 + 6, 2 + 3));

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
fn a_choose_one_spells_cast_triggers_wait_for_the_option_and_the_target() {
    // docs/rules/design.md "Targets in the shop" (Era, 2026-11-02): a Choose One spell is
    // cast, then its option is picked, then its target, and only then do the
    // "after you cast a spell" triggers fire. Living Azerite is one: it gives
    // Elemental Tavern minions +4/+3 after every Tavern spell.
    let mut g = Game::new_with_lobby(
        Arc::new(Data::bundled()),
        143,
        lobby(&[Tribe::Elemental, Tribe::Beast, Tribe::Dragon]),
    );
    put_on_board(&mut g, 0, "Living Azerite");
    set_shop(&mut g, 0, &["Living Azerite"]);
    let before = g.players[0].shop[0].attack;
    let on_board = g.players[0].board[0].attack;
    g.give(0, "Alliance Flag").unwrap();
    let hand = g.players[0].hand.len() - 1;
    act(&mut g, 0, Action::Play { hand: hand as u8, pos: 0 });
    assert_eq!(g.players[0].shop[0].attack, before, "the option is not picked yet");
    assert!(g.players[0].pending_choose_one.is_some(), "the option is asked for");
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].shop[0].attack, before, "the target is not picked yet");
    act(&mut g, 0, Action::Target(0));
    assert_eq!(
        g.players[0].shop[0].attack,
        before + 4,
        "the Tavern spell's own trigger lands after the target"
    );
    assert_eq!(g.players[0].board[0].attack, on_board + 3, "Alliance Flag's own +3 Attack");
    g.check_invariants().unwrap();
}

#[test]
fn a_discover_spells_cast_triggers_wait_for_the_pick() {
    // A spell whose own effect opens a Discover has not resolved until the
    // card is picked, so its "after you cast a spell" triggers wait for that
    // pick, like a target or a Choose One. Logs:
    // Hearthstone_2026_10_24_17_59_26 turn 10 (Fandral's Fortune's Discover
    // picked at line 252294, the Felboar's consume only at 252362) and
    // Hearthstone_2026_10_24_15_25_26 (Hired Headhunter, play at 827899, the
    // pick at 828273, the Felboar at 828321).
    let mut g = game(145);
    put_on_board(&mut g, 0, "Felboar");
    set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller"]);
    // Two spells already counted: this cast is the third.
    g.players[0].board[0].improvements = 2;
    g.give(0, "A New Sprout").unwrap();
    let hand = g.players[0].hand.len() - 1;
    act(&mut g, 0, Action::Play { hand: hand as u8, pos: 0 });
    assert!(!g.players[0].discover.is_empty(), "the Discover is offered");
    assert_eq!(g.players[0].board[0].improvements, 2, "the trigger waits for the pick");
    assert_eq!(g.players[0].shop.len(), 2, "nothing eaten yet");
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].board[0].improvements, 0, "the third spell counted at the pick");
    assert_eq!(g.players[0].shop.len(), 1, "and the Felboar eats once the cast has resolved");
    g.check_invariants().unwrap();
}

#[test]
fn a_triple_rewards_cast_is_a_spell_cast_at_its_pick() {
    // A Triple Reward is "a 0-cost plain spell (not a Tavern spell)"
    // (docs/rules/design.md "Triples"), so its cast counts for "after you cast
    // a spell" (Felboar) and not for the Tavern-spell cards, and the game
    // takes it only once the reward's Discover is answered. Raw log
    // Hearthstone_2026_11_02_00_10_22 turn 8: the reward is played with the
    // Felboar's first number (what is left of "After you cast 3 spells") still
    // at 1, and the pick moves it (0 then 3, a consume) and eats the Hired
    // Mount 3/5 in the same block, the Felboar going 15/22 to 18/27.
    // Hearthstone_2026_11_03_15_22_36 turn 8 is the same pick with a
    // Timecap'n Hooktail on the board, whose Tavern-spell trigger never fires.
    let mut g = game(146);
    put_on_board(&mut g, 0, "Felboar");
    set_shop(&mut g, 0, &["Hired Mount"]);
    // Two spells already counted: the reward's cast is the third.
    g.players[0].board[0].improvements = 2;
    g.give(0, "Triple Reward").unwrap();
    let hand = g.players[0].hand.len() - 1;
    act(&mut g, 0, Action::Play { hand: hand as u8, pos: 0 });
    assert!(!g.players[0].discover.is_empty(), "the reward's Discover is offered");
    assert_eq!(g.players[0].board[0].improvements, 2, "the cast waits for the pick");
    assert_eq!(g.players[0].shop.len(), 1, "nothing eaten yet");
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].board[0].improvements, 0, "the reward's cast counted at the pick");
    assert_eq!(g.players[0].shop.len(), 0, "and the Felboar eats once the cast has resolved");
    g.check_invariants().unwrap();
}

#[test]
fn a_spell_siphon_gift_grows_after_the_pick_not_on_the_play() {
    // The Dark Gift Spell Siphon is "has +2/+2 for each Tavern spell you've
    // cast this game", and the game grows the minion once the spell has
    // resolved. A spell whose own effect opens a Discover has not resolved
    // until the pick, so the growth lands there, not on the play: raw log
    // Hearthstone_2026_10_27_04_30_04 turn 7 (Planar Telescope cast at
    // Power.log 68792, its Discover picked at 69231, the Spell Siphon tag at
    // 69283), and 2026_10_24_17_59_26 turn 10 step 43 to 44 (Fandral's
    // Fortune, Mangled Bandit +2/+2 at the pick).
    let mut g = game(149);
    put_on_board(&mut g, 0, "Wolf Pup");
    set_shop(&mut g, 0, &["Cord Puller", "Cord Puller"]);
    let gift = g.data.cards.by_name("Gift: Spell Siphon strong").unwrap();
    let index = g.data.gifts.gift_of_card(gift).unwrap();
    let mut m = g.players[0].board[0];
    bgcore::game::darkgifts::land_gift(&g.data, &g.players[0], &mut m, index, 1);
    g.players[0].board[0] = m;
    let before = g.players[0].board[0].attack;
    g.give(0, "A New Sprout").unwrap();
    let hand = g.players[0].hand.len() - 1;
    act(&mut g, 0, Action::Play { hand: hand as u8, pos: 0 });
    assert!(!g.players[0].discover.is_empty(), "the Discover is offered");
    assert_eq!(
        g.players[0].board[0].attack, before,
        "the gift's count waits for the pick"
    );
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(
        g.players[0].board[0].attack, before + 2,
        "and it counts the Tavern spell once the cast has resolved"
    );
    g.check_invariants().unwrap();
}

#[test]
fn a_forsaken_weavers_undead_bonus_waits_for_the_pick() {
    // Forsaken Weaver says "After you cast a Tavern spell", so its Undead
    // bonus fires once the spell has resolved, after a pick the spell opened
    // (raw log Hearthstone_2026_10_27_04_30_04 turn 10 step 9 to 10: Tomb
    // Turning's play changes no minion, the +3 Attack lands at the pick).
    // An Undead is not in this file's lobby, so the sandbox (every tribe) is.
    let all = Tribe::EVERY.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t));
    let mut g = Game::new_sandbox(Arc::new(Data::bundled()), 150, Some(all), 0);
    put_on_board(&mut g, 0, "Forsaken Weaver");
    put_on_board(&mut g, 0, "Cadaver Caretaker"); // Undead, 2/4
    let before = g.players[0].board[1].attack;
    g.give(0, "A New Sprout").unwrap();
    let hand = g.players[0].hand.len() - 1;
    act(&mut g, 0, Action::Play { hand: hand as u8, pos: 0 });
    assert!(!g.players[0].discover.is_empty(), "the Discover is offered");
    assert_eq!(
        g.players[0].board[1].attack, before,
        "an \"after you cast\" card waits for the pick"
    );
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(
        g.players[0].board[1].attack, before + 3,
        "and gives its Undead +3 Attack once the cast has resolved"
    );
    g.check_invariants().unwrap();
}

#[test]
fn sprightly_scarab_choose_one_and_patient_scout_discover() {
    let mut g = game(107);
    put_on_board(&mut g, 0, "Buzzing Vermin"); // 1/1 at slot 0
    put_on_board(&mut g, 0, "Flittering Bat"); // 1/4 at slot 1
    g.give(0, "Sprightly Scarab").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });

    turn_of(&mut g, 0);
    assert_eq!(legal(&g), [Action::Choose(0), Action::Choose(1)]);
    // Option 1: +4 Attack and Windfury to a Beast
    act(&mut g, 0, Action::Choose(1));
    turn_of(&mut g, 0);
    // The two board Beasts, and the Tavern's Beasts too (docs/rules/design.md "Targets
    // in the shop").
    let board_targets: Vec<Action> = legal(&g)
        .into_iter()
        .filter(|a| matches!(a, Action::Target(_)))
        .collect();
    assert_eq!(board_targets, [Action::Target(0), Action::Target(1)]);
    assert!(legal(&g).iter().all(|a| matches!(a, Action::Target(_) | Action::TargetShop(_))));
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
fn generated_token_blue_shell_accord_o_tron_and_auto_reveille() {
    let mut g = game(108);
    // A generated token has no pool copy: selling it does not change the pool.
    g.give(0, "Microbot").unwrap();
    assert_eq!(g.players[0].hand[0].pool_copies, 0);
    let pool_before = g.pool.clone();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.pool, pool_before);

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
    set_shop(&mut g, 0, &["Buzzing Vermin", "Flittering Bat", "Cord Puller"]);
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
        let mut g = game(seed);
        let mut rng = Rng::new(seed ^ 0x123456);
        while !g.over {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
    }
    for seed in 0..20 {
        let mut g = game(seed);
        while !g.over {
            let a = bgcore::bots::standard_action(&g);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
    }
}

#[test]
fn spell_shop_slot_buy_hold_cast_freeze_roll_and_discount() {
    let mut g = game(200);
    // Every player starts with a Tier 1 spell in shop_spell.
    let sp0 = g.players[0].shop_spell.unwrap();
    assert!(g.data.cards.get(sp0).is_spell());
    assert_eq!(g.data.cards.get(sp0).tier, 1);

    // Force a 3-cost spell (A New Sprout) into player 0's shop spell slot.
    let sprout = g.data.cards.by_name("A New Sprout").unwrap();
    g.players[0].shop_spell = Some(sprout);
    assert_eq!(g.players[0].spell_cost(&g.data.cards), Some(3));

    // A discount makes the next Tavern spell cost 1 less (3 -> 2).
    g.players[0].spell_discount = 1;
    assert_eq!(g.players[0].spell_cost(&g.data.cards), Some(2));

    // Buy the spell: costs 2 gold (3 -> 1), clears shop_spell and discount, puts spell in hand.
    let pool_before = g.pool.clone();
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].gold, 1);
    assert!(g.players[0].shop_spell.is_none());
    assert_eq!(g.players[0].spell_discount, 0);
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.players[0].hand[0].card, sprout);
    assert_eq!(g.pool, pool_before); // buying a spell never touches the minion pool

    // Roll refreshes the spell slot.
    act(&mut g, 0, Action::Roll);
    let frozen_spell = g.players[0].shop_spell.unwrap();

    // Freeze keeps the spell across the turn boundary.
    act(&mut g, 0, Action::Freeze);
    next_turn(&mut g);
    assert_eq!(g.players[0].shop_spell, Some(frozen_spell));

    // With the board and the Tavern empty, a targeted spell in hand cannot
    // be cast. (A Tavern minion is a legal target for "a minion", per
    // docs/rules/design.md "Who a card can reach".)
    assert!(g.players[0].board.is_empty());
    set_shop(&mut g, 0, &[]);
    g.players[0].hand.clear();
    g.give(0, "Tavern Dish Banana").unwrap();
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Play { hand: 0, pos: 0 }));
    assert_eq!(g.players[0].hand.len(), 1);

    // Holding 3+ copies of a spell never triples it.
    g.give(0, "Tavern Dish Banana").unwrap();
    g.give(0, "Tavern Dish Banana").unwrap();
    assert_eq!(g.players[0].hand.len(), 3);
    assert!(g.players[0].hand.iter().all(|m| !m.golden));
    g.check_invariants().unwrap();
}

#[test]
fn gold_spells_delayed_spells_and_strike_oil_cap() {
    let mut g = game(201);
    // Tavern Coin (+1 gold now), Careful Investment (+2 gold next turn),
    // Leaf Through the Pages (2 free rolls), Strike Oil (+1 max gold).
    let gold0 = g.players[0].gold;
    g.give(0, "Tavern Coin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].gold, gold0 + 1);

    g.give(0, "Leaf Through the Pages").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].free_rolls, 2);
    let gold_before_rolls = g.players[0].gold;
    act(&mut g, 0, Action::Roll);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].gold, gold_before_rolls);
    assert_eq!(g.players[0].free_rolls, 0);

    g.give(0, "Careful Investment").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    g.give(0, "Strike Oil").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });

    // Overconfidence (+3 gold on win, +1 on tie) and Winner's Bread (+2/+3 now, +4/+6 next turn on win).
    put_on_board(&mut g, 0, "Turquoise Skitterer"); // 5/5 at slot 0, guaranteed to win turn 1
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    g.players[q as usize].board.clear();

    g.give(0, "Overconfidence").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    g.give(0, "Winner's Bread").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (5 + 2, 5 + 3));

    next_turn(&mut g);
    // Turn 2 base gold (4) + Strike Oil (1) + Careful Investment (2) + Overconfidence win (3) = 10.
    assert_eq!(g.players[0].gold, 10);
    // Winner's Bread gave +4/+6 at start of turn 2 after winning combat.
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (7 + 4, 8 + 6));

    // Pin both trinket slots with harmless cards (an `on_pick` that never
    // runs, no turn hooks) so the turn 6 and 9 offers do not change the Gold
    // this test is about.
    let lesser = g.data.trinkets.by_name("Explorer's Binoculars").unwrap();
    let greater = g.data.trinkets.by_name("Artisanal Urn").unwrap();
    g.set_trinket(0, 0, Some(lesser));
    g.set_trinket(0, 1, Some(greater));
    // Advance past turn 8 to verify Strike Oil raises the 10-gold cap to 11.
    while g.turn < 9 {
        // Keep other players alive so the game doesn't end early.
        for pl in &mut g.players {
            pl.health = 30;
        }
        next_turn(&mut g);
    }
    assert_eq!(g.players[0].gold, 11);
    g.check_invariants().unwrap();
}

#[test]
fn stat_spells_choose_one_and_discover_get_spells() {
    let mut g = game(202);
    g.players[0].tier = 5;
    put_on_board(&mut g, 0, "Buzzing Vermin"); // Beast 1/1 at slot 0
    put_on_board(&mut g, 0, "Cord Puller"); // Mech 1/1 at slot 1

    // Fortify (+0/+3 and Taunt), Repair Job (+4/+8), Defender's Rites (+7/+7 and Taunt).
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].board[1].keywords.has(Keywords::TAUNT));
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (1, 4));

    g.give(0, "Repair Job").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(1));
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (5, 12));

    g.give(0, "Defender's Rites").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(1));
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (12, 19));

    // Them Apples buffs shop minions (+1/+2).
    set_shop(&mut g, 0, &["Cord Puller"]); // 1/1 in shop
    g.give(0, "Them Apples").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!((g.players[0].shop[0].attack, g.players[0].shop[0].health), (2, 3));

    // Natural Blessing buffs all minions on board that share a tribe with the target (+2/+1).
    put_on_board(&mut g, 0, "Flittering Bat"); // Beast 1/4 at slot 2
    g.give(0, "Natural Blessing").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0)); // target Beast at slot 0
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (3, 2));
    assert_eq!((g.players[0].board[2].attack, g.players[0].board[2].health), (3, 5));
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (12, 19)); // Mech unchanged

    // Eyes of the Earth Mother makes a Tier <= 4 minion Golden;
    // Perfect Vision sets stats to 20/20; Azerite Empowerment buffs board +2/+2 twice.
    g.give(0, "Eyes of the Earth Mother").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert!(g.players[0].board[0].golden);
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (4, 3));

    g.give(0, "Perfect Vision").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(2));
    assert_eq!((g.players[0].board[2].attack, g.players[0].board[2].health), (20, 20));

    g.give(0, "Azerite Empowerment").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!((g.players[0].board[2].attack, g.players[0].board[2].health), (24, 24));

    // Choose One: Alliance Flag, Time Management (delayed option), Boundless Potential (spell discover),
    // Forest's Bounty (targeted option).
    g.give(0, "Alliance Flag").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Choose(0)); // +3/+1
    act(&mut g, 0, Action::Target(2));
    assert_eq!((g.players[0].board[2].attack, g.players[0].board[2].health), (27, 25));

    g.give(0, "Forest's Bounty").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Choose(0)); // +6/+6 twice
    act(&mut g, 0, Action::Target(2));
    assert_eq!((g.players[0].board[2].attack, g.players[0].board[2].health), (39, 37));

    g.give(0, "Boundless Potential").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Choose(1)); // Discover a Tavern spell of your Tier (5)
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(
        g.players[0]
            .discover
            .iter()
            .all(|&c| g.data.cards.get(c).is_spell() && g.data.cards.get(c).tier == 5)
    );
    act(&mut g, 0, Action::Choose(0));
    g.players[0].hand.clear();

    // Discover/Get spells: Recruit a Trainee, A New Sprout, Chef's Choice, Contracted Corpse, Hired Headhunter.
    g.give(0, "Recruit a Trainee").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.data.cards.get(g.players[0].hand[0].card).tier, 1);
    assert_eq!(g.players[0].hand[0].pool_copies, 0);
    g.players[0].hand.clear();

    g.give(0, "Chef's Choice").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(1)); // Cord Puller (Mech)
    assert_eq!(g.players[0].hand.len(), 1);
    let got = g.players[0].hand[0];
    assert!(g.data.cards.get(got.card).tribes.has(bgcore::Tribe::Mech));
    assert_ne!(got.card, g.players[0].board[1].card);
    assert_eq!(got.pool_copies, 0);
    g.players[0].hand.clear();

    g.give(0, "Contracted Corpse").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert!(g.players[0].discover.iter().all(|&c| g.data.cards.get(c).deathrattle.is_some()));
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].hand[0].pool_copies, 1);

    g.give(0, "Hired Headhunter").unwrap();
    act(&mut g, 0, Action::Play { hand: 1, pos: 0 });
    assert!(g.players[0].discover.iter().all(|&c| g.data.cards.get(c).battlecry.is_some()));
    act(&mut g, 0, Action::Choose(0));

    // Time Management option 1: +2/+2 twice at the start of next turn.
    g.give(0, "Time Management").unwrap();
    let h_tm = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h_tm, pos: 0 });
    act(&mut g, 0, Action::Choose(1));
    let before = (g.players[0].board[2].attack, g.players[0].board[2].health);
    next_turn(&mut g);
    assert_eq!(
        (g.players[0].board[2].attack, g.players[0].board[2].health),
        (before.0 + 4, before.1 + 4)
    );
    g.check_invariants().unwrap();
}

/// Making a minion Golden where it stands doubles the stats the card itself
/// scales with ("+3/+3 for each Battlecry you've triggered this game, wherever
/// this is") and keeps every buff, which does not double
/// (`game::shop::golden_up`; the log's own numbers:
/// `Hearthstone_2026_10_24_17_59_26-g1` turn 8 step 21 has a Tavern Dark
/// Paradox go 18/18 to 36/36, and `Hearthstone_2026_11_03_00_29_06-g5` turn 9
/// a Tavern Banana Slamma at 51/51 to 54/57).
#[test]
fn making_a_minion_golden_doubles_its_own_counters_and_keeps_its_buffs() {
    // A seed whose game holds the tier 6 Dark Paradox version: it is the only
    // minion that scales with Battlecries.
    let mut g = game(1);
    // Five Battlecries behind it: 3/3 plus +15/+15.
    g.players[0].battlecries_triggered = 5;
    put_on_board(&mut g, 0, "Dark Paradox t6");
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (18, 18),
        "its own +3/+3 for each Battlecry"
    );
    // A +4/+4 buff on top of that, which is a buff and stays as it is.
    g.players[0].board[0].attack += 4;
    g.players[0].board[0].health += 4;
    g.give(0, "Eyes of the Earth Mother").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert!(g.players[0].board[0].golden, "made Golden");
    // The golden card's 6/6, the Battlecries' +15/+15 a second time, and the
    // +4/+4 the minion carried.
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (40, 40),
        "6/6 printed, twice the Battlecry stats, the buff kept once"
    );

    // A card with nothing of its own to scale: only its printed stats change,
    // the +48/+45 of buffs stay (the Banana Slamma in the log).
    put_on_board(&mut g, 0, "Banana Slamma");
    assert_eq!(
        (g.players[0].board[1].attack, g.players[0].board[1].health),
        (3, 6),
        "3/6 printed"
    );
    g.players[0].board[1].attack += 48;
    g.players[0].board[1].health += 45;
    g.give(0, "Eyes of the Earth Mother").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].board[1].golden, "made Golden");
    assert_eq!(
        (g.players[0].board[1].attack, g.players[0].board[1].health),
        (54, 57),
        "6/12 printed plus the buffs, which do not double"
    );
    g.check_invariants().unwrap();
}

#[test]
fn boon_of_beetles_keeps_unused_charges_across_combats() {
    let mut g = game(203);
    // Give player 0 six minions so there is room for only 1 Beetle at Start of Combat,
    // and the enemy has an empty board so none of player 0's minions die.
    for _ in 0..6 {
        put_on_board(&mut g, 0, "Cord Puller");
    }
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    g.players[q as usize].board.clear();

    g.give(0, "Boon of Beetles").unwrap(); // 2 charges
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].boon_of_beetles, 2);

    // Combat 1: only 1 board slot is open, so 1 Beetle spawns and 1 charge remains!
    next_turn(&mut g);
    assert_eq!(g.players[0].boon_of_beetles, 1);

    // Combat 2: clear enemy board again; the remaining 1 charge is used up in Combat 2.
    let Some(Opponent::Player(q2)) = g.players[0].opponent else { panic!() };
    g.players[q2 as usize].board.clear();
    next_turn(&mut g);
    assert_eq!(g.players[0].boon_of_beetles, 0);
    g.check_invariants().unwrap();
}

#[test]
fn glambot_balinda_banana_interpreter_snapper_and_utility_drone() {
    let mut g = game(204);
    put_on_board(&mut g, 0, "Cord Puller"); // slot 0: 1/1 Mech (target)
    put_on_board(&mut g, 0, "Glambot"); // slot 1: 4/4 Mech
    put_on_board(&mut g, 0, "Balinda Stonehearth"); // slot 2: 6/6 Neutral
    put_on_board(&mut g, 0, "Mechagnome Interpreter"); // slot 3: 3/1 Mech (+3/+1 per Magnetization)
    put_on_board(&mut g, 0, "Spark Snapper"); // slot 4: 6/8 Mech (only triggers on played Mechs)
    put_on_board(&mut g, 0, "Utility Drone"); // slot 5: 4/5 Mech (+4/+5 per Magnetization at end of turn)

    // Cast Tavern Dish Banana (+2/+2) on Cord Puller at slot 0:
    // - Balinda makes it cast twice.
    // - Each cast gives +2/+2 from Banana, attaches a 4/4 Satellite from Glambot (+4/+4, +1 mag),
    //   and triggers Mechagnome Interpreter (+3/+1).
    // - Spark Snapper does NOT trigger on Glambot's Satellites (improvements stays 0, mag is 2 not 4).
    g.give(0, "Tavern Dish Banana").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));

    let host = g.players[0].board[0];
    assert_eq!(host.magnetizations, 2);
    assert_eq!(g.players[0].board[4].improvements, 0);
    // Cord Puller: 1/1 + 2 * (2/2 Banana + 4/4 Satellite + 3/1 Interpreter) = 19/15.
    assert_eq!((host.attack, host.health), (1 + 2 * (2 + 4 + 3), 1 + 2 * (2 + 4 + 1)));

    // End player 0's turn: Utility Drone gives +4/+5 for each of the 2 Magnetizations (+8/+10).
    act(&mut g, 0, Action::EndTurn);
    let host = g.players[0].board[0];
    assert_eq!((host.attack, host.health), (19 + 8, 15 + 10));
    g.check_invariants().unwrap();
}

#[test]
fn spell_stat_bonuses_botanist_humongozz_and_magnetized_enchanted_sentinel() {
    let mut g = game(205);
    put_on_board(&mut g, 0, "Cord Puller"); // slot 0: 1/1 Mech
    put_on_board(&mut g, 0, "Humon'gozz"); // slot 1: +1/+2 spell bonus while on board

    // Play Intrepid Botanist and choose option 0 (+1 Attack this game).
    g.give(0, "Intrepid Botanist").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].spell_bonus, (1, 0));

    // Magnetize Enchanted Sentinel (+1/+1 spell bonus) into Cord Puller at slot 0.
    g.give(0, "Enchanted Sentinel").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // Total spell bonus is now: Botanist (+1/+0) + Humon'gozz (+1/+2) + Magnetized Sentinel (+1/+1) = +3/+3.
    assert_eq!(g.total_spell_bonus(0), (3, 3));

    // Cast Shiny Ring (+1/+1 to board): every minion on board gets +(1+3)/+(1+3) = +4/+4!
    let before = (g.players[0].board[0].attack, g.players[0].board[0].health);
    g.give(0, "Shiny Ring").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (before.0 + 4, before.1 + 4)
    );
    g.check_invariants().unwrap();
}

#[test]
fn czarina_harbinger_groundskeeper_and_rescue_bot() {
    let mut g = game(206);
    put_on_board(&mut g, 0, "Charging Czarina"); // slot 0: 4/1 Divine Shield
    put_on_board(&mut g, 0, "Cord Puller"); // slot 1: 1/1 Divine Shield
    put_on_board(&mut g, 0, "Balinda Stonehearth"); // slot 2: 6/6 (no Divine Shield)
    put_on_board(&mut g, 0, "Cataclysmic Harbinger"); // slot 3: copies last spell at end of turn

    // Cast Fortify on Balinda at slot 2:
    // - Balinda makes it cast twice.
    // - Czarina fires on each of the 2 casts, giving Divine Shield minions (Czarina and Cord Puller) +8 Attack total!
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(2));
    assert_eq!(g.players[0].board[0].attack, 4 + 8);
    assert_eq!(g.players[0].board[1].attack, 1 + 8);
    assert_eq!(g.players[0].board[2].attack, 6); // Balinda has no Divine Shield

    // End player 0's turn: Harbinger gives 1 copy of Fortify.
    act(&mut g, 0, Action::EndTurn);
    let hand_names: Vec<String> = g.players[0].hand.iter().map(|m| name(&g, m)).collect();
    assert_eq!(hand_names, ["Fortify"]);

    // Iron Groundskeeper gives 2 copies of Fortify when played; Rescue Bot gives a Repair Job when it dies in combat.
    let mut g2 = game(207);
    put_on_board(&mut g2, 0, "Rescue Bot");
    g2.give(0, "Iron Groundskeeper").unwrap();
    act(&mut g2, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g2.players[0].hand.len(), 2);
    assert!(g2.players[0].hand.iter().all(|m| name(&g2, m) == "Fortify"));

    let Some(Opponent::Player(q)) = g2.players[0].opponent else { panic!() };
    put_on_board(&mut g2, q as usize, "Turquoise Skitterer");
    next_turn(&mut g2);
    assert!(g2.players[0].hand.iter().any(|m| name(&g2, m) == "Repair Job"));
    g2.check_invariants().unwrap();
}

#[test]
fn stage2_observation_spells_magnetizations_and_scalars() {
    use bgcore::obs::*;
    let mut g = game(300);

    // Check layout sizes and offsets.
    assert_eq!(OWN_SLOTS, bgcore::SHOP + SHOP_SPELL + bgcore::HAND + bgcore::BOARD + bgcore::DISCOVER);
    assert_eq!(OWN_END, SCALARS + OWN_SLOTS * SLOT_FEATURES);
    assert_eq!(OPP_BLOCK, OPP_SCALARS + bgcore::BOARD * SLOT_FEATURES);
    assert_eq!(OBS_SIZE, OWN_END + (bgcore::PLAYERS - 1) * OPP_BLOCK);

    // Put Cord Puller (Mech, Divine Shield), Spark Snapper, and Patient Scout on board.
    put_on_board(&mut g, 0, "Cord Puller");
    put_on_board(&mut g, 0, "Spark Snapper");
    put_on_board(&mut g, 0, "Patient Scout");

    // Magnetize a Red Volumizer onto Cord Puller at slot 0:
    // - Cord Puller gets +3/+1 and 1 Volumizer inside, plus a 2/3 Satellite
    //   from Spark Snapper that joined the Volumizer before it merged: one
    //   Magnetization (`10_29_22_20_04-g3` turn 13 step 10).
    // - Spark Snapper gets improvements = 1.
    // - Player 0 gets volumizer_bonus = (3, 0).
    g.give(0, "Red Volumizer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });

    // Cast Winner's Bread (+2/+3 and 1 winners_bread stack) on Cord Puller at slot 0.
    let bread_id = g.data.cards.by_name("Winner's Bread").unwrap();
    g.give(0, "Winner's Bread").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));

    // Give a spell (Repair Job, tier 3) and a generated token (Microbot) to player 0's hand.
    let repair_id = g.data.cards.by_name("Repair Job").unwrap();
    g.give(0, "Repair Job").unwrap();
    g.give(0, "Microbot").unwrap();

    // Set some additional player state scalars.
    g.players[0].beetle_bonus = (4, 5);
    g.players[0].gold_next_turn = 2;
    g.players[0].extra_max_gold = 1;
    g.players[0].free_rolls = 3;
    g.players[0].spell_discount = 1;
    g.players[0].spell_bonus = (2, 1);
    g.players[0].boon_of_beetles = 2;
    g.players[0].brood_of_nozdormu = 1;
    g.players[0].upper_hand = 1;
    g.players[0].won_last_combat = true;
    g.players[0].board[2].improvements = 3; // Patient Scout at tier 1 + 3 = 4

    let mut out = vec![f32::NAN; OBS_SIZE];
    write(&g, 0, &mut out);
    assert!(out.iter().all(|x| x.is_finite()));

    // Check own scalars.
    assert_eq!(out[SCALAR_VOLUMIZER_ATTACK], 3.0);
    assert_eq!(out[SCALAR_VOLUMIZER_HEALTH], 0.0);
    assert_eq!(out[SCALAR_BEETLE_ATTACK], 4.0);
    assert_eq!(out[SCALAR_BEETLE_HEALTH], 5.0);
    assert_eq!(out[SCALAR_PATIENT_SCOUT_TIER], 4.0);
    assert_eq!(out[SCALAR_GOLD_NEXT_TURN], 2.0);
    assert_eq!(out[SCALAR_EXTRA_MAX_GOLD], 1.0);
    assert_eq!(out[SCALAR_FREE_ROLLS], 3.0);
    assert_eq!(out[SCALAR_SPELL_DISCOUNT], 1.0);
    assert_eq!(out[SCALAR_SPELL_ATTACK], 2.0);
    assert_eq!(out[SCALAR_SPELL_HEALTH], 1.0);
    assert_eq!(out[SCALAR_BOON_OF_BEETLES], 2.0);
    assert_eq!(out[SCALAR_BROOD_OF_NOZDORMU], 1.0);
    assert_eq!(out[SCALAR_UPPER_HAND], 1.0);
    assert_eq!(out[SCALAR_LAST_SPELL], bread_id as f32 + 1.0);
    assert_eq!(out[SCALAR_WON_LAST_COMBAT], 1.0);
    assert_eq!(out[SCALAR_LOST_LAST_COMBAT], 0.0);

    // Check shop spell slot.
    let shop_sp = &out[SHOP_SPELL_START..SHOP_SPELL_START + SLOT_FEATURES];
    assert_eq!(shop_sp[SLOT_PRESENT], 1.0);
    assert_eq!(shop_sp[SLOT_IS_SPELL], 1.0);
    assert_eq!(shop_sp[SLOT_ATTACK], 0.0);
    assert_eq!(shop_sp[SLOT_HEALTH], 0.0);

    // Check spell in hand (hand slot 0 is Repair Job, tier 3): is_spell is 1, stats are 0.
    let hand0 = &out[HAND_START..HAND_START + SLOT_FEATURES];
    assert_eq!(hand0[SLOT_PRESENT], 1.0);
    assert_eq!(hand0[SLOT_CARD_ID], repair_id as f32 + 1.0);
    assert_eq!(hand0[SLOT_IS_SPELL], 1.0);
    assert_eq!(hand0[SLOT_TIER], 3.0);
    for &idx in &[
        SLOT_ATTACK,
        SLOT_HEALTH,
        SLOT_GOLDEN,
        SLOT_BEAST,
        SLOT_MECH,
        SLOT_FIGHT_ATTACK,
        SLOT_FIGHT_HEALTH,
        SLOT_DEMON,
        SLOT_TAUGHT_SPELL,
        SLOT_ACTIVATE_COST,
        SLOT_MAGNETIZATIONS,
        SLOT_VOLUMIZERS,
        SLOT_IMPROVEMENTS,
        SLOT_FROM_POOL,
        SLOT_WINNERS_BREAD,
    ] {
        assert_eq!(hand0[idx], 0.0, "spell stat field {idx} should be 0");
    }

    // Check generated token in hand (hand slot 1 is Microbot: from_pool == 0, a Mech).
    let hand1 = &out[HAND_START + SLOT_FEATURES..HAND_START + 2 * SLOT_FEATURES];
    assert_eq!(hand1[SLOT_PRESENT], 1.0);
    assert_eq!(hand1[SLOT_IS_SPELL], 0.0);
    assert_eq!(hand1[SLOT_MECH], 1.0);
    assert_eq!(hand1[SLOT_BEAST] + hand1[SLOT_DRAGON] + hand1[SLOT_MURLOC] + hand1[SLOT_DEMON], 0.0);
    assert_eq!(hand1[SLOT_FROM_POOL], 0.0);

    // Check board slot 0 (Cord Puller with Red Volumizer + Snapper Satellite + Winner's Bread).
    let b0 = &out[BOARD_START..BOARD_START + SLOT_FEATURES];
    assert_eq!(b0[SLOT_PRESENT], 1.0);
    assert_eq!(b0[SLOT_MECH], 1.0);
    assert_eq!(b0[SLOT_KEYWORDS + 1], 1.0); // divine_shield
    assert_eq!(b0[SLOT_KEYWORDS + 5], 1.0); // magnetic inherited from Red Volumizer
    assert_eq!(b0[SLOT_MAGNETIZATIONS], 1.0);
    // A Volumizer inside a host no longer counts as one (Era, 2026-11-03).
    assert_eq!(b0[SLOT_VOLUMIZERS], 0.0);
    assert_eq!(b0[SLOT_FROM_POOL], 1.0);
    assert_eq!(b0[SLOT_WINNERS_BREAD], 1.0);

    // Check board slot 1 (Spark Snapper: improvements == 1).
    let b1 = &out[BOARD_START + SLOT_FEATURES..BOARD_START + 2 * SLOT_FEATURES];
    assert_eq!(b1[SLOT_IMPROVEMENTS], 1.0);
}

#[test]
fn standard_and_rush_bots_follow_curves_and_play_whole_games() {
    use bgcore::bots::{rush_action, standard_action};

    // 1. Check standard_action reaches tier 2 on turn 2, tier 3 on turn 5,
    //    tier 4 on turn 7, tier 5 on turn 11, tier 6 on turn 12 in a simple game.
    let mut g_std = game(301);
    while g_std.turn <= 12 && !g_std.over {
        for pl in &mut g_std.players {
            pl.health = 30;
        }
        let p = g_std.current_player().unwrap();
        // A trinket can hand the other seats a Discover, so they answer it
        // before passing.
        let a = if p == 0 {
            standard_action(&g_std)
        } else {
            bgcore::bots::answer_pending(&g_std, p, None).unwrap_or(Action::EndTurn)
        };
        g_std.step(a).unwrap_or_else(|_| panic!("turn {} player {p}: {a:?}", g_std.turn));
    }
    assert_eq!(g_std.players[0].tier_turns, [1, 2, 5, 7, 11, 12]);

    // 2. Check rush_action reaches tier 2 on turn 2, tier 3 on turn 4 (spending leftover
    //    gold on a 1-cost spell when offered), tier 4 on turn 5, tier 5 on turn 7, tier 6 on turn 8.
    let mut g_rush = game(302);
    let banana = g_rush.data.cards.by_name("Tavern Dish Banana").unwrap();
    let mut bought_spell_t4 = false;
    while g_rush.turn <= 8 && !g_rush.over {
        for pl in &mut g_rush.players {
            pl.health = 30;
        }
        if g_rush.turn == 4 && g_rush.players[0].tier == 2 {
            // Offer a 1-cost Tavern Dish Banana on turn 4 before player 0 levels.
            g_rush.players[0].shop_spell = Some(banana);
        }
        let p = g_rush.current_player().unwrap();
        let a = if p == 0 {
            rush_action(&g_rush)
        } else {
            bgcore::bots::answer_pending(&g_rush, p, None).unwrap_or(Action::EndTurn)
        };
        if p == 0 && g_rush.turn == 4 && a == Action::BuySpell {
            bought_spell_t4 = true;
        }
        g_rush.step(a).unwrap();
    }
    assert!(bought_spell_t4, "rush bot should spend its leftover gold on turn 4 on a 1-cost spell");
    assert_eq!(g_rush.players[0].tier_turns, [1, 2, 4, 5, 7, 8]);

    // 3. Check curve bots merge Magnetic minions left of a Mech on board.
    let mut g_mag = game(303);
    put_on_board(&mut g_mag, 0, "Buzzing Vermin"); // Beast at slot 0
    put_on_board(&mut g_mag, 0, "Cord Puller"); // Mech at slot 1
    g_mag.give(0, "Lullabot").unwrap(); // Magnetic in hand slot 0
    turn_of(&mut g_mag, 0);
    assert_eq!(standard_action(&g_mag), Action::Play { hand: 0, pos: 1 });
    assert_eq!(rush_action(&g_mag), Action::Play { hand: 0, pos: 1 });

    // 4. Play whole games with both bots and verify no illegal actions and valid finish reports.
    for seed in 0..20 {
        let mut g = game(seed + 400);
        while let Some(p) = g.current_player() {
            let a = if p % 2 == 0 { standard_action(&g) } else { rush_action(&g) };
            g.step(a).unwrap();
            g.check_invariants().unwrap();
        }
        for pl in &g.players {
            assert!((1..=8).contains(&pl.placement));
            assert_eq!(pl.tier_turns[0], 1);
            assert!(pl.final_tribes.iter().all(|&n| n as usize <= bgcore::BOARD));
        }
    }
}


#[test]
fn legal_mask_matches_is_legal_everywhere() {
    for seed in 0..20 {
        let mut g = game(seed);
        let mut rng = Rng::new(seed + 99);
        let mut mask = [false; N_ACTIONS];
        while !g.over {
            g.legal_mask(&mut mask);
            for (i, &m) in mask.iter().enumerate() {
                assert_eq!(m, g.is_legal(Action::from_index(i).unwrap()), "seed {seed}, action {i}");
            }
            g.step(random_action(&g, &mut rng)).unwrap();
        }
    }
}

/// Fight player 0's board against one wall, with the player's own counters, so
/// a fight's grants and what it carries home are the ones a real combat makes.
fn fight_wall(g: &Game, seed: u64, attack: i32, health: i32) -> bgcore::fight::Outcome {
    let board = g.players[0].board.clone();
    let db = &g.data.cards;
    let mut wall = db.make(db.by_name("Lullabot").unwrap(), false);
    wall.attack = attack;
    wall.health = health;
    bgcore::fight_with(
        db,
        &board,
        &[wall],
        [g.players[0].counters(&g.data, g.lobby), Default::default()],
        500,
        &mut Rng::new(seed),
        &mut (),
    )
}

/// Headhunter Gryphon's Rally gets a Buzzing Vermin in a fight while two
/// plain copies stand on the board: nothing triples during a fight, so the
/// three merge at the start of the next turn and the golden lands in the hand
/// then, with the copies' stats (docs/rules/design.md "Nothing triples during
/// combat", THE GOLDEN RULE).
#[test]
fn a_beast_a_fight_grants_triples_at_the_start_of_the_next_turn() {
    let mut g = game(109);
    // One plain copy in hand, two on the board, each buffed so the golden
    // shows which copies went into it.
    let held = take(&mut g, "Buzzing Vermin");
    g.players[0].hand.push(held);
    put_on_board(&mut g, 0, "Headhunter Gryphon");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].board[1].attack = 2;
    g.players[0].board[1].health = 3;
    g.players[0].board[2].attack = 2;
    g.players[0].board[2].health = 3;
    let out = fight_wall(&g, 7, 3, 4);
    let vermin = g.data.cards.by_name("Buzzing Vermin").unwrap();
    assert_eq!(out.sides[0].cards_gained(), &[vermin], "the Rally got one Beast");
    g.apply_side_outcome(0, &out.sides[0], true);
    // The fight is over and the fourth copy waits: no golden yet.
    assert_eq!(g.players[0].hand.len(), 2, "the one in hand and the one the Rally gave");
    assert!(g.players[0].hand.iter().all(|m| m.card == vermin && !m.golden), "nothing triples in combat");
    assert_eq!(g.players[0].board.len(), 3, "both copies still on the board");
    g.check_invariants().unwrap();
    // The next turn's start runs the check the fight could not: three copies
    // merge into a golden in hand and one plain Vermin is left on the board.
    g.dark_gift_start_of_turn(0);
    assert_eq!(g.players[0].hand.len(), 1);
    let gold = g.players[0].hand[0];
    assert_eq!(gold.card, vermin);
    assert!(gold.golden, "the golden Vermin is in the hand");
    assert_eq!((gold.attack, gold.health), (3, 4), "double 1/1 plus the board copy's +1/+2");
    let plain: Vec<(i32, i32)> = g.players[0]
        .board
        .iter()
        .filter(|m| m.card == vermin)
        .map(|m| (m.attack, m.health))
        .collect();
    assert_eq!(plain, vec![(2, 3)], "one plain copy left, and it keeps its buffs");
    g.check_invariants().unwrap();
}

/// The other way a fight fills the hand: a trinket whose counter the fight
/// feeds (Stormcoil Sticker, after six friendly Mech deaths) gets a Mech. It
/// waits to triple like the Rally's Beast does.
#[test]
fn a_mech_a_trinket_gets_after_a_fight_triples_at_the_start_of_the_next_turn() {
    let mut g = game(110);
    let sticker = g.data.trinkets.by_name("Stormcoil Sticker").unwrap();
    g.set_trinket(0, 0, Some(sticker));
    // Six Mechs of a card the counter cannot give (tier 2), at 1 Health.
    for _ in 0..6 {
        put_on_board(&mut g, 0, "Mechagnome Interpreter");
        let last = g.players[0].board.len() - 1;
        g.players[0].board[last].attack = 0;
        g.players[0].board[last].health = 1;
    }
    // Two plain copies of each Tier 1 Mech the trinket can give: whichever one
    // arrives, its third copy is what waits.
    for name in ["Cord Puller", "Lullabot"] {
        for _ in 0..2 {
            let m = take(&mut g, name);
            g.players[0].hand.push(m);
        }
    }
    let out = fight_wall(&g, 1, 100, 500);
    assert_eq!(out.sides[0].friendly_deaths, 6, "all six died");
    g.apply_side_outcome(0, &out.sides[0], true);
    assert_eq!(g.players[0].trinkets[0].count, 0, "the counter fired at six");
    assert_eq!(g.players[0].hand.len(), 5, "the Mech the trinket gave");
    assert!(g.players[0].hand.iter().all(|m| !m.golden), "no golden during the fight");
    let gained = g.players[0].hand[4].card;
    assert!(g.data.cards.get(gained).tribes.has(Tribe::Mech), "a Mech");
    g.check_invariants().unwrap();
    // The next turn's start merges the three copies of the Mech that arrived.
    g.dark_gift_start_of_turn(0);
    let golden: Vec<Minion> = g.players[0].hand.iter().copied().filter(|m| m.golden).collect();
    assert_eq!(golden.len(), 1, "one golden, in the hand");
    assert_eq!(golden[0].card, gained);
    let base = g.data.cards.make(gained, false);
    assert_eq!(
        (golden[0].attack, golden[0].health),
        (2 * base.attack, 2 * base.health),
        "double the printed stats, no buffs on any copy"
    );
    g.check_invariants().unwrap();
}

