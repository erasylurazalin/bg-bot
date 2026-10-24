use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use bgsandbox::screen::parse_tribe;
use bgsandbox::{Config, Session, play};

const USAGE: &str = "\
usage: bgsandbox [--tribe beast|mech|dragon|murloc|demon|pirate|elemental|undead|quilboar|aberration] [--label TEXT] [--seed N]
                 [--free-rolls N] [--boards DIR]

A one-player Battlegrounds sandbox: a normal game with the full pool, no opponents.
Every turn ends with a fight against a copy of your own board (it never hurts your hero),
and the end-of-turn boards are saved to boards/game-<seed>.toml.
--tribe X  rig the Tavern and every minion draw to that tribe plus neutral minions.
--label    the build you play. Without it you are asked at the end (or use the label
           command in the game). The file is renamed to boards/<label>-<seed>.toml,
           or <tribe>-<label>-<seed> when rigged; an existing file is never overwritten.
--seed     defaults to a random one that is printed.
--free-rolls  free Refreshes each turn, default 0.
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

fn fail(msg: &str) -> ExitCode {
    eprintln!("bgsandbox: {msg}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut tribe = None;
    let mut label = None;
    let mut seed = None;
    let mut free_rolls = 0u8;
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
    let seed = seed.unwrap_or_else(random_seed);
    let mut session = Session::new(Config { tribe, label, seed, free_rolls, boards_dir: boards });
    let tty = std::io::stdin().is_terminal();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    play(&mut session, &mut input, &mut out, tty);
    ExitCode::SUCCESS
}
