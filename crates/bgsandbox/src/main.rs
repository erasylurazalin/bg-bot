use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use bgsandbox::screen::parse_tribe;
use bgsandbox::{Config, Session};

const USAGE: &str = "\
usage: bgsandbox [--tribe beast|mech|dragon|murloc|demon] [--label TEXT] [--seed N]
                 [--free-rolls N] [--boards DIR]

A one-player Battlegrounds sandbox. The Tavern only shows your tribe and neutral
minions, every turn ends with a fight against a copy of your own board, and the
end-of-turn boards are saved to boards/<tribe>-<label>-<seed>.toml.
Without --tribe or --label it asks. --seed defaults to a random one that is printed.
--free-rolls (default 3) is the number of free Refreshes each turn.
Commands are read from stdin, type help once it runs.";

fn random_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    let mut z = nanos ^ ((std::process::id() as u64) << 32);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)) & 0xFFFF_FFFF
}

fn ask(input: &mut impl BufRead, question: &str) -> Option<String> {
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("bgsandbox: {msg}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut tribe = None;
    let mut label = None;
    let mut seed = None;
    let mut free_rolls = 3u8;
    let mut boards = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../boards"));
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        let r: Result<(), String> = (|| {
            match a.as_str() {
                "--tribe" => {
                    let v = value("--tribe")?;
                    tribe = Some(parse_tribe(&v).ok_or(format!("unknown tribe {v:?}"))?);
                }
                "--label" => label = Some(value("--label")?),
                "--seed" => {
                    let v = value("--seed")?;
                    let n: u64 = v.parse().map_err(|_| format!("bad seed {v:?}"))?;
                    if n > i64::MAX as u64 {
                        return Err("seed too large (the file stores it as a TOML integer)".into());
                    }
                    seed = Some(n);
                }
                "--free-rolls" => {
                    let v = value("--free-rolls")?;
                    free_rolls = v.parse().map_err(|_| format!("bad --free-rolls {v:?}"))?;
                }
                "--boards" => boards = PathBuf::from(value("--boards")?),
                "-h" | "--help" => {
                    println!("{USAGE}");
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
            }
            Ok(())
        })();
        if let Err(e) = r {
            return fail(&e);
        }
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let asked = tribe.is_none() || label.is_none();
    let tribe = match tribe {
        Some(t) => t,
        None => loop {
            let Some(line) = ask(&mut input, "Tribe (beast, mech, dragon, murloc, demon): ") else {
                return fail("no tribe given");
            };
            match parse_tribe(&line) {
                Some(t) => break t,
                None => println!("{line:?} is not one of the five tribes."),
            }
        },
    };
    let label = match label {
        Some(l) => l,
        None => match ask(&mut input, "Strategy label (free text, for example lobster or fodder): ") {
            Some(l) if !l.is_empty() => l,
            Some(_) => "play".into(),
            None => return fail("no label given"),
        },
    };
    let seed = match seed {
        Some(s) => s,
        None if asked => loop {
            match ask(&mut input, "Seed (a number, blank for random): ") {
                Some(s) if s.is_empty() => break random_seed(),
                Some(s) => match s.parse::<u64>() {
                    Ok(n) if n <= i64::MAX as u64 => break n,
                    _ => println!("{s:?} is not a usable seed."),
                },
                None => break random_seed(),
            }
        },
        None => random_seed(),
    };
    let mut session = Session::new(Config { tribe, label, seed, free_rolls, boards_dir: boards });
    println!("{}", session.banner());
    let tty = std::io::stdin().is_terminal();
    let mut line = String::new();
    loop {
        if tty {
            print!("\n> ");
            let _ = std::io::stdout().flush();
        }
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => {
                let r = session.exec("quit");
                println!("\n{}", r.text);
                return ExitCode::SUCCESS;
            }
            Ok(_) => {}
        }
        if !tty {
            println!("\n> {}", line.trim());
        } else {
            println!();
        }
        let r = session.exec(&line);
        println!("{}", r.text);
        if r.quit {
            return ExitCode::SUCCESS;
        }
    }
}
