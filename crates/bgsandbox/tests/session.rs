//! The sandbox as a player drives it: commands, undo, recording, and a
//! scripted session through the real binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bgcore::record;
use bgcore::{Data, Rng, Tribe};
use bgsandbox::{Config, Session};

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bgsandbox-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// George the Fallen: his power is a button these scripts never press, so
/// the forced hero changes nothing they check.
const SCRIPT_HERO: &str = "George the Fallen";

fn session(tribe: Tribe, seed: u64, dir: &Path) -> Session {
    Session::new(Config {
        lobby: Some(bgcore::Tribes::of(tribe)),
        hero: Some(SCRIPT_HERO.into()),
        label: Some("test".into()),
        seed,
        free_rolls: 3,
        boards_dir: dir.to_path_buf(),
    })
}

fn open_session(seed: u64, label: Option<&str>, dir: &Path) -> Session {
    Session::new(Config {
        lobby: None,
        hero: Some(SCRIPT_HERO.into()),
        label: label.map(String::from),
        seed,
        free_rolls: 0,
        boards_dir: dir.to_path_buf(),
    })
}

/// Run the command loop on a script as if at a terminal, returns the output.
fn play_script(s: &mut Session, script: &str) -> String {
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    bgsandbox::play(s, &mut input, &mut out, true);
    String::from_utf8_lossy(&out).to_string()
}

fn boards_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn illegal_and_unparsable_input_changes_nothing() {
    let dir = tmp("illegal");
    let mut s = session(Tribe::Beast, 3, &dir);
    let before = s.fingerprint();
    let hist = s.history_len();
    for bad in [
        "", "zzz", "b", "b0", "b9", "b1 2", "p1", "p5", "p1 9", "c1", "c1 4", "c1 t2", "s1", "s9", "m1 2", "m1 1",
        "a1", "1", "t1", "2 3", "? 9", "?h1", "?b1", "?d1", "undo undo", "r r", "u 1", "bs 1",
    ] {
        let r = s.exec(bad);
        // "bs 1" and friends are errors; a few of these are real, harmless
        // commands in other states, so only the ones that are refused count
        let _ = r;
        if s.history_len() == hist {
            assert_eq!(s.fingerprint(), before, "{bad:?} changed the state");
        } else {
            panic!("{bad:?} was accepted on a fresh turn 1");
        }
    }
    // gold is 3 at the start: a Refresh is free, an upgrade (5) is not affordable
    let r = s.exec("u");
    assert!(r.text.contains("not enough gold"), "{}", r.text);
    assert_eq!(s.fingerprint(), before);
    assert!(!dir.exists(), "nothing was recorded, no file");
}

#[test]
fn commands_map_to_engine_actions() {
    let dir = tmp("map");
    let mut s = session(Tribe::Mech, 7, &dir);
    // turn 1: 3 gold, buy one minion, play it, move/sell/activate paths
    let r = s.exec("b1");
    assert!(r.text.contains("Gold 0/3"), "{}", r.text);
    assert_eq!(s.game().players[0].hand.len(), 1);
    s.exec("p1");
    assert_eq!(s.game().players[0].board.len(), 1);
    assert!(s.game().players[0].hand.is_empty());
    // free refreshes
    for left in [2, 1, 0] {
        s.exec("r");
        assert_eq!(s.game().players[0].free_rolls, left);
        assert_eq!(s.game().players[0].gold, 0);
    }
    let r = s.exec("r");
    assert!(r.text.contains("not enough gold"), "{}", r.text);
    s.exec("f");
    assert!(s.game().players[0].frozen);
    s.exec("f");
    assert!(!s.game().players[0].frozen);
    s.exec("s1");
    assert!(s.game().players[0].board.is_empty());
    assert_eq!(s.game().players[0].gold, 1);
    // info commands never change anything
    let fp = s.fingerprint();
    for q in ["? 1", "?s", "?h1", "?b1", "help", "?"] {
        s.exec(q);
    }
    assert_eq!(s.fingerprint(), fp);
}

#[test]
fn undo_restores_the_exact_state_across_turns() {
    let dir = tmp("undo");
    let mut s = session(Tribe::Dragon, 11, &dir);
    let mut prints = vec![s.fingerprint()];
    let script = ["b1", "p1", "r", "e", "b1", "e", "r", "e"];
    for cmd in script {
        let before = s.history_len();
        s.exec(cmd);
        if s.history_len() > before {
            prints.push(s.fingerprint());
        }
    }
    assert!(s.turns().len() >= 3);
    let file = s.file_path().unwrap();
    assert!(file.exists());
    // undo one at a time and compare with each earlier state, an `e` among them
    while prints.len() > 1 {
        prints.pop();
        let r = s.exec("undo");
        assert!(r.text.starts_with("Undone."), "{}", r.text);
        assert_eq!(s.fingerprint(), *prints.last().unwrap());
        let on_disk = if file.exists() {
            record::load_file(&Data::bundled().cards, &file).unwrap().turns.len()
        } else {
            0
        };
        assert_eq!(on_disk, s.turns().len(), "the file must match the history after undo");
    }
    assert_eq!(s.exec("undo").text, "Nothing to undo.");
    assert!(!file.exists(), "every turn was undone, so the file is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn random_play_never_panics_and_undoes_cleanly() {
    let cmds: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        for n in 1..=7 {
            v.extend([format!("b{n}"), format!("s{n}"), format!("a{n}"), format!("{n}"), format!("t{n}")]);
            for m in 1..=7 {
                v.push(format!("m{n} {m}"));
            }
        }
        for h in 1..=10 {
            v.push(format!("p{h}"));
            v.push(format!("c{h}"));
            for t in 1..=7 {
                v.push(format!("p{h} {t}"));
                v.push(format!("c{h} {t}"));
                v.push(format!("c{h} t{t}"));
            }
        }
        for _ in 0..6 {
            v.extend(["bs", "r", "r", "u", "f"].map(String::from));
        }
        v.extend(["? 1", "?s", "?h1", "?b1", "?d1", "xx"].map(String::from));
        v
    };
    let mut prompts: Vec<String> = Vec::new();
    for (i, tribe) in Tribe::EVERY.into_iter().enumerate() {
        let dir = tmp(&format!("fuzz{i}"));
        let mut s = session(tribe, 100 + i as u64, &dir);
        let mut rng = Rng::new(i as u64 + 1);
        let mut prints = vec![s.fingerprint()];
        let mut accepted = 0;
        for step in 0..2500 {
            let cmd = if step % 25 == 24 { "e".to_string() } else { cmds[rng.below(cmds.len() as u32) as usize].clone() };
            let before = s.history_len();
            let fp = s.fingerprint();
            let r = s.exec(&cmd);
            if let Some(at) = r.text.find("CHOICE:") {
                let kind: String = r.text[at..].chars().take(24).collect();
                if !prompts.contains(&kind) {
                    prompts.push(kind);
                }
            }
            if s.history_len() > before {
                accepted += 1;
                prints.push(s.fingerprint());
            } else {
                assert_eq!(s.fingerprint(), fp, "{cmd:?} was refused ({}) but changed the state", r.text);
            }
            s.game().check_invariants().unwrap();
            if s.game().over {
                break;
            }
        }
        // How many commands land depends on how long the random player stays
        // alive (it fights a copy of its own board), and trinkets can make
        // those fights end sooner.
        assert!(accepted > 50, "{tribe:?}: only {accepted} commands accepted");
        // undo all the way back, checking each state
        while prints.len() > 1 {
            prints.pop();
            s.exec("undo");
            assert_eq!(s.fingerprint(), *prints.last().unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    println!("prompt kinds seen: {prompts:?}");
    assert!(prompts.len() >= 3, "the fuzz should open several kinds of choice: {prompts:?}");
}

fn run_binary(args: &[&str], script: &str) -> (std::process::ExitStatus, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bgsandbox"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status, text)
}

#[test]
fn a_scripted_session_through_the_binary_writes_the_file() {
    let dir = tmp("binary");
    let d = dir.to_str().unwrap();
    // buy, play, roll, end turn for several turns, undo, quit
    let script = "b1\np1\nr\ne\nb1\np1\nr\ne\nb1\nb2\np1\nu\ne\ne\nundo\n?b1\nzzz\nquit\n";
    let args = ["--tribe", "murloc", "--hero", SCRIPT_HERO, "--label", "Test Run", "--seed", "42", "--free-rolls", "3", "--boards", d];
    let (status, out) = run_binary(&args, script);
    assert!(status.success(), "{out}");
    assert!(out.contains("seed 42"), "{out}");
    assert!(out.contains("mirror fight"), "{out}");
    assert!(out.contains("Undone."), "{out}");
    let file = dir.join("murloc-Test-Run-42.toml");
    assert!(file.exists(), "no file in {dir:?}: {out}");
    let rec = record::load_file(&Data::bundled().cards, &file).unwrap();
    assert_eq!((rec.tribe.as_deref(), rec.label.as_str(), rec.seed, rec.free_rolls), (Some("Murloc"), "Test Run", 42, 3));
    // four turns ended, then one undone
    assert_eq!(rec.turns.len(), 3, "{out}");
    assert_eq!(rec.turns.iter().map(|t| t.turn).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(rec.turns.iter().all(|t| t.health == 30));

    // the same script again gives the same file: seed and actions replay
    let first = std::fs::read_to_string(&file).unwrap();
    let dir2 = tmp("binary2");
    let args2 = ["--tribe", "murloc", "--hero", SCRIPT_HERO, "--label", "Test Run", "--seed", "42", "--free-rolls", "3", "--boards", dir2.to_str().unwrap()];
    let (status, _) = run_binary(&args2, script);
    assert!(status.success());
    let second = std::fs::read_to_string(dir2.join("murloc-Test-Run-42.toml")).unwrap();
    assert_eq!(first, second);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn the_default_game_is_unrigged_with_no_free_rolls() {
    let dir = tmp("default");
    let d = dir.to_str().unwrap();
    let (status, out) = run_binary(&["--seed", "9", "--boards", d], "1
r
e
");
    assert!(status.success(), "{out}");
    assert!(out.contains("a rolled 5-tribe lobby"), "{out}");
    assert!(out.contains("CHOICE: pick your hero"), "{out}");
    // no free Refreshes and no tribe prompt: the first Refresh costs gold
    assert!(out.contains("Free rolls 0"), "{out}");
    assert!(out.contains("Gold 2/3"), "{out}");
    assert_eq!(boards_in(&dir), ["unlabeled-9.toml"], "piped input keeps 'unlabeled'");
    let rec = record::load_file(&Data::bundled().cards, &dir.join("unlabeled-9.toml")).unwrap();
    assert_eq!((rec.tribe, rec.label.as_str(), rec.free_rolls), (None, "unlabeled", 0));
    let text = std::fs::read_to_string(dir.join("unlabeled-9.toml")).unwrap();
    assert!(text.contains("rigged = false") && !text.contains("tribe"), "{text}");
    let s = open_session(9, None, &dir);
    assert_eq!(s.game().lobby.len(), 5);
    let (status, out) = run_binary(&["--tribe", "nope"], "");
    assert!(!status.success());
    assert!(out.contains("--tribe wants 1 to 5 distinct tribes"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rigged_game_records_the_tribe_in_header_and_name() {
    let dir = tmp("rigged");
    let d = dir.to_str().unwrap();
    let (status, out) = run_binary(&["--tribe", "beast", "--label", "lobster", "--seed", "4", "--boards", d], "1
e
");
    assert!(status.success(), "{out}");
    assert_eq!(boards_in(&dir), ["beast-lobster-4.toml"]);
    let text = std::fs::read_to_string(dir.join("beast-lobster-4.toml")).unwrap();
    assert!(text.contains("rigged = true") && text.contains("tribe = \"Beast\""), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_label_is_asked_at_the_end_and_renames_the_file() {
    let dir = tmp("ask-label");
    let mut s = open_session(5, None, &dir);
    let out = play_script(&mut s, "b1
p1
e
e
quit
Fodder Run
");
    assert!(out.contains("Label this game"), "{out}");
    assert_eq!(boards_in(&dir), ["Fodder-Run-5.toml"], "{out}");
    let rec = record::load_file(&Data::bundled().cards, &dir.join("Fodder-Run-5.toml")).unwrap();
    assert_eq!((rec.label.as_str(), rec.turns.len()), ("Fodder Run", 2));
    // during the game the file has the temporary name
    let dir2 = tmp("ask-label2");
    let mut s = open_session(5, None, &dir2);
    s.exec("e");
    assert_eq!(boards_in(&dir2), ["game-5.toml"]);
    // a blank answer keeps unlabeled
    let dir3 = tmp("ask-label3");
    let mut s = open_session(5, None, &dir3);
    play_script(&mut s, "e
quit

");
    assert_eq!(boards_in(&dir3), ["unlabeled-5.toml"]);
    // end of input at the question does not hang or lose the game
    let dir4 = tmp("ask-label4");
    let mut s = open_session(5, None, &dir4);
    play_script(&mut s, "e
");
    assert_eq!(boards_in(&dir4), ["unlabeled-5.toml"]);
    // a label given up front is not asked again
    let dir5 = tmp("ask-label5");
    let mut s = open_session(5, Some("magnetic"), &dir5);
    let out = play_script(&mut s, "e
quit
");
    assert!(!out.contains("Label this game"), "{out}");
    assert_eq!(boards_in(&dir5), ["magnetic-5.toml"]);
    for d in [dir, dir2, dir3, dir4, dir5] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[test]
fn the_label_command_sets_and_changes_it_any_time() {
    let dir = tmp("label-cmd");
    let mut s = open_session(6, None, &dir);
    let fp = s.fingerprint();
    assert!(s.exec("label").text.contains("needs a name"));
    assert_eq!(s.label(), "unlabeled");
    s.exec("label lobster");
    assert_eq!(s.fingerprint(), fp, "a label is not a game action");
    s.exec("e");
    assert!(std::fs::read_to_string(dir.join("game-6.toml")).unwrap().contains("label = \"lobster\""));
    s.exec("label Magnetic Mechs");
    assert!(std::fs::read_to_string(dir.join("game-6.toml")).unwrap().contains("label = \"Magnetic Mechs\""));
    // given by the command, so the end does not ask again
    let out = play_script(&mut s, "quit
");
    assert!(!out.contains("Label this game"), "{out}");
    assert_eq!(boards_in(&dir), ["Magnetic-Mechs-6.toml"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_label_is_asked_when_the_game_ends() {
    // 40 turns is the turn limit: the game ends and asks without a quit
    let dir = tmp("game-end");
    let mut s = open_session(8, None, &dir);
    let script = "e\n".repeat(40) + "grind\nquit\n";
    let out = play_script(&mut s, &script);
    assert!(out.contains("GAME OVER"), "{out}");
    assert!(out.contains("Label this game"), "{out}");
    assert_eq!(boards_in(&dir), ["grind-8.toml"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_hero_pick_and_power_button_work() {
    let dir = tmp("hero-pick");
    // Without --hero the game opens on the pick; hp presses a button power.
    let mut s = Session::new(Config {
        lobby: Some(bgcore::Tribes::of(Tribe::Beast)),
        hero: None,
        label: Some("pick".into()),
        seed: 21,
        free_rolls: 0,
        boards_dir: dir.to_path_buf(),
    });
    let r = s.exec("p1");
    assert!(r.text.contains("answer the open choice"), "{}", r.text);
    let r = s.exec("1");
    assert!(r.text.contains("Hero: "), "{}", r.text);
    assert!(s.game().turn == 1, "the pick is before turn 1");
    assert!(s.game().players[0].hero.is_some());
    // Finley's Discover runs right after his pick, before turn 1.
    let mut finley = Session::new(Config {
        lobby: Some(bgcore::Tribes::of(Tribe::Beast)),
        hero: Some("Sir Finley Mrrgglton".into()),
        label: None,
        seed: 21,
        free_rolls: 0,
        boards_dir: dir.to_path_buf(),
    });
    // His power Discover waits right after the pick, so nothing else works.
    assert!(finley.banner().contains("pick a Hero Power"));
    let r = finley.exec("hp");
    assert!(r.text.contains("answer the open choice"), "{}", r.text);
    let r = finley.exec("1");
    assert!(r.text.contains("Hero: Sir Finley Mrrgglton"), "{}", r.text);
    let power = finley.game().players[0].power.unwrap();
    assert_ne!(
        finley.game().data.heroes.get(power.of).power.card_id,
        "TB_BaconShop_HP_057",
        "Adventure! is replaced by the pick"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_board_file_is_never_overwritten() {
    let dir = tmp("no-overwrite");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lobster-5.toml"), "keep me").unwrap();
    std::fs::write(dir.join("game-5.toml"), "keep me too").unwrap();
    let mut s = open_session(5, Some("lobster"), &dir);
    s.exec("e");
    s.exec("quit");
    assert_eq!(boards_in(&dir), ["game-5.toml", "lobster-5-2.toml", "lobster-5.toml"]);
    assert_eq!(std::fs::read_to_string(dir.join("lobster-5.toml")).unwrap(), "keep me");
    assert_eq!(std::fs::read_to_string(dir.join("game-5.toml")).unwrap(), "keep me too");
    // a second game with the same label and seed gets the next suffix
    let mut s = open_session(5, Some("lobster"), &dir);
    s.exec("e");
    s.exec("quit");
    assert!(dir.join("lobster-5-3.toml").exists(), "{:?}", boards_in(&dir));
    assert_eq!(std::fs::read_to_string(dir.join("lobster-5.toml")).unwrap(), "keep me");
    let _ = std::fs::remove_dir_all(&dir);
}
