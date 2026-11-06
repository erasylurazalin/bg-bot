//! What the observation holds for each opponent since the 2026-11-06 run:
//! the leaderboard's tribe line and last three fights (docs/rules/design.md
//! "The leaderboard"), and opponents' old boards going dark behind
//! `obs::OldBoards`.

use std::sync::Arc;

use bgcore::obs::{self, OldBoards, OPP_BLOCK, OPP_TOP_TRIBE, OWN_END};
use bgcore::{Action, Data, FightResult, Game, Leaderboard, Minion, Opponent, PLAYERS, Rng, Tribe};

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(
        Arc::new(Data::bundled()),
        seed,
        lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Aberration, Tribe::Murloc, Tribe::Demon]),
    )
}

/// Answer whatever the current player has open (a Discover, a target, a
/// discard, a trinket offer), so a helper that only wants the next turn never
/// stalls on a prompt. False when nothing is open.
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

/// Everyone ends their turn until the next turn starts (the combat between
/// them is done).
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
    // A generated copy (no pool copies): the card is put on a board from
    // nowhere, so no pool count moves and nothing in the game can be asked
    // to give a copy back.
    g.players[0].make_card(&g.data.cards, c, false, 0)
}

/// Put a card on a board without touching the pool.
fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let m = take(g, name);
    g.players[p].board.push(m);
}

/// The observation block of opponent `q` in `p`'s view, as its offset in the
/// flat array (the blocks are the seats after `p`, in seat order).
fn block_of(p: usize, q: usize) -> usize {
    OWN_END + ((q + PLAYERS - p - 1) % PLAYERS) * OPP_BLOCK
}

fn tribe_slot(t: Tribe) -> usize {
    Tribe::EVERY.iter().position(|&x| x == t).unwrap()
}

// ---- The leaderboard's tribe line ----

#[test]
fn the_tribe_line_is_the_top_tribe_and_its_count() {
    let mut g = game(3);
    let cards = &g.data.cards;
    assert_eq!(Leaderboard::of(cards, &g.players[0].board), Leaderboard::default());
    for name in ["Joyous", "Joyous", "Zoatroid", "Zoatroid", "Brain Rotter"] {
        put_on_board(&mut g, 0, name);
    }
    let line = Leaderboard::of(&g.data.cards, &g.players[0].board);
    assert_eq!(line.top, Some(Tribe::Aberration));
    assert_eq!(line.count, 5);
    assert!(!line.mixed);
    // An all-types minion counts for every tribe: 3 Beasts and 2 Amalgams
    // read "Beasts 5" (Era).
    let mut g = game(4);
    for _ in 0..3 {
        put_on_board(&mut g, 0, "Buzzing Vermin");
    }
    for _ in 0..2 {
        put_on_board(&mut g, 0, "Amalgam");
    }
    let line = Leaderboard::of(&g.data.cards, &g.players[0].board);
    assert_eq!(line.top, Some(Tribe::Beast));
    assert_eq!(line.count, 5);
    assert!(!line.mixed);
    // A two-tribe minion counts for both: 2 Beasts and 2 Beasts-and-Pirates
    // is four Beasts, and the Pirates tie nothing.
    let mut g = game(5);
    for _ in 0..2 {
        put_on_board(&mut g, 0, "Buzzing Vermin");
    }
    for _ in 0..2 {
        put_on_board(&mut g, 0, "Treasure Parrot");
    }
    let line = Leaderboard::of(&g.data.cards, &g.players[0].board);
    assert_eq!(line.top, Some(Tribe::Beast));
    assert_eq!(line.count, 4);
    assert!(!line.mixed);
    // Two tribes with the same top count show as mixed, with that count.
    let mut g = game(6);
    for _ in 0..2 {
        put_on_board(&mut g, 0, "Buzzing Vermin");
    }
    for _ in 0..2 {
        put_on_board(&mut g, 0, "Bubble Gunner");
    }
    let line = Leaderboard::of(&g.data.cards, &g.players[0].board);
    assert_eq!(line.top, None);
    assert!(line.mixed);
    assert_eq!(line.count, 2);
    // A neutral board shows nothing.
    let mut g = game(7);
    for _ in 0..3 {
        put_on_board(&mut g, 0, "Suspicious Prisonguard");
    }
    assert_eq!(Leaderboard::of(&g.data.cards, &g.players[0].board), Leaderboard::default());
}

#[test]
fn the_tribe_line_is_snapshotted_at_the_fight_and_held() {
    let mut g = game(11);
    // Player 3 builds five Aberrations (no three of a kind, which would
    // triple) and then changes nothing.
    for name in ["Joyous", "Joyous", "Zoatroid", "Zoatroid", "Brain Rotter"] {
        put_on_board(&mut g, 3, name);
    }
    let mut out = vec![0.0f32; obs::OBS_SIZE];
    // Before the fight nothing is snapshotted yet.
    obs::write(&g, 0, &mut out);
    let at = block_of(0, 3);
    assert_eq!(out[at + OPP_TOP_TRIBE + tribe_slot(Tribe::Aberration)], 0.0);
    assert_eq!(out[at + obs::OPP_TRIBE_COUNT], 0.0);
    next_turn(&mut g);
    obs::write(&g, 0, &mut out);
    assert_eq!(out[at + OPP_TOP_TRIBE + tribe_slot(Tribe::Aberration)], 1.0);
    assert_eq!(out[at + obs::OPP_TRIBE_COUNT], 5.0);
    assert_eq!(out[at + obs::OPP_TRIBE_MIXED], 0.0);
    // Their live board changes in the shop; the snapshot does not.
    for _ in 0..3 {
        g.players[3].board.pop();
    }
    put_on_board(&mut g, 3, "Buzzing Vermin");
    put_on_board(&mut g, 3, "Flittering Bat");
    obs::write(&g, 0, &mut out);
    assert_eq!(out[at + OPP_TOP_TRIBE + tribe_slot(Tribe::Aberration)], 1.0);
    assert_eq!(out[at + obs::OPP_TRIBE_COUNT], 5.0);
    // The next fight's start takes a new snapshot: two Aberrations and two
    // Beasts tie, which shows as mixed with that count.
    next_turn(&mut g);
    obs::write(&g, 0, &mut out);
    assert_eq!(out[at + OPP_TOP_TRIBE], 0.0, "a mixed board shows no tribe");
    assert_eq!(out[at + obs::OPP_TRIBE_MIXED], 1.0);
    assert_eq!(out[at + obs::OPP_TRIBE_COUNT], 2.0);
}

// ---- The last three fights ----

#[test]
fn the_last_three_fights_roll_over() {
    let mut g = game(21);
    let mut rng = Rng::new(21);
    let mut fights_seen = 0;
    while g.turn < 7 && !g.over {
        let before: Vec<[FightResult; bgcore::FIGHTS_SHOWN]> =
            g.players.iter().map(|p| p.fights).collect();
        let t = g.turn;
        while g.turn == t && !g.over {
            let a = bgcore::bots::random_action(&g, &mut rng);
            g.step(a).unwrap();
        }
        for (p, old) in before.iter().enumerate() {
            if !g.players[p].alive {
                continue;
            }
            let now = g.players[p].fights;
            // This turn's fight is the newest, and the rest slid down one.
            assert!(now[0].happened, "player {p} fought and its line is empty");
            assert_eq!(now[1], old[0], "player {p}: the second line");
            assert_eq!(now[2], old[1], "player {p}: the third line");
            // The outcome agrees with the flags the own scalars read, and the
            // damage's sign follows it.
            let pl = &g.players[p];
            let want = if pl.won_last_combat {
                1
            } else if pl.lost_last_combat {
                -1
            } else {
                0
            };
            assert_eq!(now[0].outcome, want, "player {p}");
            match want {
                1 => assert!(now[0].damage >= 0, "player {p} won and took damage"),
                -1 => assert!(now[0].damage <= 0, "player {p} lost and dealt damage"),
                _ => assert_eq!(now[0].damage, 0, "player {p} tied with damage"),
            }
        }
        // With every seat alive each fight is one player against another, so
        // what one side dealt is exactly what the other took.
        if g.players.iter().all(|p| p.alive) {
            let sum: i32 = g.players.iter().map(|p| p.fights[0].damage).sum();
            assert_eq!(sum, 0, "damage dealt and taken differ");
        }
        fights_seen += 1;
    }
    assert!(fights_seen >= 6, "only {fights_seen} combats");
    // Something was won and something lost along the way, or this proves
    // little.
    let wins = g.players.iter().filter(|p| p.fights.iter().any(|f| f.outcome == 1)).count();
    let losses = g.players.iter().filter(|p| p.fights.iter().any(|f| f.outcome == -1)).count();
    assert!(wins > 0 && losses > 0, "{wins} winners and {losses} losers");
}

#[test]
fn the_fight_lines_are_in_the_observation() {
    let mut g = game(23);
    let mut rng = Rng::new(23);
    while g.turn < 3 && !g.over {
        let t = g.turn;
        while g.turn == t && !g.over {
            let a = bgcore::bots::random_action(&g, &mut rng);
            g.step(a).unwrap();
        }
    }
    let mut out = vec![0.0f32; obs::OBS_SIZE];
    for p in 0..PLAYERS {
        if !g.players[p].alive {
            continue;
        }
        obs::write(&g, p, &mut out);
        for q in 0..PLAYERS {
            if q == p {
                continue;
            }
            let at = block_of(p, q);
            for f in 0..bgcore::FIGHTS_SHOWN {
                let line = g.players[q].fights[f];
                let base = at + obs::OPP_FIGHTS + f * obs::FIGHT_FEATURES;
                assert_eq!(out[base], line.happened as u8 as f32);
                assert_eq!(out[base + 1], line.outcome as f32);
                assert_eq!(out[base + 2], line.damage as f32);
            }
        }
    }
}

// ---- Old boards go dark ----

#[test]
fn drop_writes_only_the_fresh_boards() {
    let mut g = game(31);
    // Turn 5, in the shop phase: q1 is the last opponent, q2 the one this
    // turn, and q3 and q4 are old news.
    while g.turn < 5 {
        next_turn(&mut g);
    }
    for q in 1..5 {
        put_on_board(&mut g, q, "Joyous");
        g.players[0].seen[q] = Default::default();
        g.players[0].seen[q].board.push(g.players[q].board[0]);
    }
    g.players[0].last_opponent = Some(Opponent::Player(1));
    g.players[0].seen[1].turn = g.turn - 1;
    g.players[0].opponent = Some(Opponent::Player(2));
    g.players[0].seen[2].turn = g.turn - 2;
    g.players[0].seen[3].turn = g.turn - 3;
    g.players[0].seen[4].turn = g.turn - 4;
    let mut kept = vec![0.0f32; obs::OBS_SIZE];
    let mut gone = vec![0.0f32; obs::OBS_SIZE];
    obs::write_with(&g, 0, &mut kept, OldBoards::Keep);
    obs::write_with(&g, 0, &mut gone, OldBoards::Drop);
    let slots = |out: &[f32], q: usize| {
        let at = block_of(0, q) + obs::OPP_SCALARS;
        out[at..at + bgcore::BOARD * obs::SLOT_FEATURES].iter().filter(|v| **v != 0.0).count()
    };
    for q in 1..5 {
        assert!(slots(&kept, q) > 0, "keep: player {q}'s board is empty");
    }
    // The one fought last turn, and the one fought this turn two turns ago,
    // keep their board; three and four turns ago do not.
    assert!(slots(&gone, 1) > 0, "the last opponent's board is gone");
    assert!(slots(&gone, 2) > 0, "the opponent two turns ago is gone");
    assert_eq!(slots(&gone, 3), 0, "an opponent three turns ago kept its board");
    assert_eq!(slots(&gone, 4), 0, "an opponent four turns ago kept its board");
    // Their scalars are untouched: health, tier and the new lines stay.
    for q in 1..5 {
        let at = block_of(0, q);
        assert_eq!(gone[at + obs::OPP_HEALTH], kept[at + obs::OPP_HEALTH]);
        assert_eq!(gone[at + obs::OPP_TIER], kept[at + obs::OPP_TIER]);
        assert_eq!(gone[at + obs::OPP_TRIBE_COUNT], kept[at + obs::OPP_TRIBE_COUNT]);
        assert_eq!(gone[at + obs::OPP_FIGHT_HAPPENED], kept[at + obs::OPP_FIGHT_HAPPENED]);
    }
    // The opponent fought this turn but three turns ago is too old to write.
    g.players[0].last_opponent = None;
    g.players[0].opponent = Some(Opponent::Player(4));
    obs::write_with(&g, 0, &mut gone, OldBoards::Drop);
    assert_eq!(slots(&gone, 4), 0, "a stale board of the next opponent is written");
}

#[test]
fn drop_keeps_the_boards_the_flags_point_at() {
    // Over real games: with drop, a block keeps a board exactly when it is
    // the last opponent, or the next one seen within OLD_BOARD_AGE turns.
    let mut seen_kept = 0;
    let mut seen_dropped = 0;
    for seed in 0..6 {
        let mut g = game(100 + seed);
        let mut rng = Rng::new(seed);
        while g.turn < 9 && !g.over {
            let t = g.turn;
            while g.turn == t && !g.over {
                let a = bgcore::bots::random_action(&g, &mut rng);
                g.step(a).unwrap();
            }
            if g.over {
                break;
            }
            let mut kept = vec![0.0f32; obs::OBS_SIZE];
            let mut gone = vec![0.0f32; obs::OBS_SIZE];
            for p in 0..PLAYERS {
                if !g.players[p].alive {
                    continue;
                }
                obs::write_with(&g, p, &mut kept, OldBoards::Keep);
                obs::write_with(&g, p, &mut gone, OldBoards::Drop);
                for q in 0..PLAYERS {
                    if q == p {
                        continue;
                    }
                    let at = block_of(p, q);
                    let n = bgcore::BOARD * obs::SLOT_FEATURES;
                    let kept_any = kept[at + obs::OPP_SCALARS..at + obs::OPP_SCALARS + n]
                        .iter()
                        .any(|v| *v != 0.0);
                    let gone_any = gone[at + obs::OPP_SCALARS..at + obs::OPP_SCALARS + n]
                        .iter()
                        .any(|v| *v != 0.0);
                    if kept_any {
                        let seen = &g.players[p].seen[q];
                        let age = g.turn.saturating_sub(seen.turn);
                        let want = gone[at + obs::OPP_LAST] == 1.0
                            || (gone[at + obs::OPP_NEXT] == 1.0 && age <= obs::OLD_BOARD_AGE);
                        assert_eq!(
                            gone_any, want,
                            "seed {seed} turn {}: seat {p} vs {q}, age {age}, next {}, last {}",
                            g.turn,
                            gone[at + obs::OPP_NEXT],
                            gone[at + obs::OPP_LAST]
                        );
                        if gone_any {
                            seen_kept += 1;
                        } else {
                            seen_dropped += 1;
                        }
                    } else {
                        assert!(!gone_any);
                    }
                }
            }
        }
    }
    assert!(seen_kept > 0 && seen_dropped > 0, "{seen_kept} kept, {seen_dropped} dropped");
}
