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

fn session(tribe: Tribe, seed: u64, dir: &Path) -> Session {
    Session::new(Config { tribe, label: "test".into(), seed, free_rolls: 3, boards_dir: dir.to_path_buf() })
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
    let file = s.file_path();
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
    for (i, tribe) in Tribe::ALL.into_iter().enumerate() {
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
        assert!(accepted > 100, "{tribe:?}: only {accepted} commands accepted");
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
    let args = ["--tribe", "murloc", "--label", "Test Run", "--seed", "42", "--boards", d];
    let (status, out) = run_binary(&args, script);
    assert!(status.success(), "{out}");
    assert!(out.contains("seed 42"), "{out}");
    assert!(out.contains("mirror fight"), "{out}");
    assert!(out.contains("Undone."), "{out}");
    let file = dir.join("murloc-Test-Run-42.toml");
    assert!(file.exists(), "no file in {dir:?}: {out}");
    let rec = record::load_file(&Data::bundled().cards, &file).unwrap();
    assert_eq!((rec.tribe.as_str(), rec.label.as_str(), rec.seed, rec.free_rolls), ("Murloc", "Test Run", 42, 3));
    // four turns ended, then one undone
    assert_eq!(rec.turns.len(), 3, "{out}");
    assert_eq!(rec.turns.iter().map(|t| t.turn).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(rec.turns.iter().all(|t| t.health == 30));

    // the same script again gives the same file: seed and actions replay
    let first = std::fs::read_to_string(&file).unwrap();
    let dir2 = tmp("binary2");
    let args2 = ["--tribe", "murloc", "--label", "Test Run", "--seed", "42", "--boards", dir2.to_str().unwrap()];
    let (status, _) = run_binary(&args2, script);
    assert!(status.success());
    let second = std::fs::read_to_string(dir2.join("murloc-Test-Run-42.toml")).unwrap();
    assert_eq!(first, second);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn the_binary_asks_when_given_no_flags_and_exits_cleanly_at_eof() {
    let dir = tmp("ask");
    let d = dir.to_str().unwrap();
    let (status, out) = run_binary(&["--boards", d], "dragon\nfodder run\n77\ne\n");
    assert!(status.success(), "{out}");
    assert!(out.contains("seed 77"), "{out}");
    assert!(dir.join("dragon-fodder-run-77.toml").exists(), "{out}");
    let (status, out) = run_binary(&["--tribe", "nope"], "");
    assert!(!status.success());
    assert!(out.contains("unknown tribe"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
