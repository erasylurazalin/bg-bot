//! `bgbench powerlog`: a Power.log as the reader sees it, game by game.
//!
//! Prints every game in the log, one line per thing, in a stable order, so a
//! dump from this and a dump from the Python tracker
//! (`~/projects/bg/bg/powerlog.py`) can be diffed line for line: the entity
//! table the reader built, exactly as it stood when the game ended.
//!
//!   bgbench powerlog FILE [--out FILE]   write the dump (stdout by default)
//!   bgbench powerlog FILE --time         parse only, print the time taken
//!
//! The format, one block per game, everything sorted:
//!
//! ```text
//! game <n> type=<game type>
//! players <player id>=<entity id> ...
//! real <player id> ...
//! player_names <player id>='<name>' ...
//! names '<name>'=<entity id> ...
//! last_names <entity id>='<name>' ...
//! entity <id> card_id='<card id>' tags <tag>=<value> ...
//! ```
//!
//! A number prints bare and a text value in single quotes, so `0` and `'0'`
//! stay apart.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::process::exit;
use std::time::Instant;

use bglog::{Game, Handler, PowerLog, TagValue};

pub fn run(args: &[String]) {
    let Some(path) = args.get(2).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: bgbench powerlog FILE [--out FILE] [--time]");
        exit(2);
    };
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|k| args.get(k + 1))
        .cloned();
    let time_only = args.iter().any(|a| a == "--time");
    if let Err(e) = dump(path, out.as_deref(), time_only) {
        eprintln!("{e}");
        exit(2);
    }
}

fn dump(path: &str, out: Option<&str>, time_only: bool) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut log = PowerLog::new();
    let started = Instant::now();
    if time_only {
        let mut counter = Counter::default();
        log.feed_reader(&mut reader, &mut counter)
            .map_err(|e| format!("{path}: {e}"))?;
        let elapsed = started.elapsed();
        eprintln!(
            "{path}: {} games parsed in {:.3} s",
            counter.games,
            elapsed.as_secs_f64()
        );
        return Ok(());
    }
    let mut dumper = Dumper {
        out: match out {
            Some(path) => Box::new(BufWriter::with_capacity(
                1 << 20,
                File::create(path).map_err(|e| format!("{path}: {e}"))?,
            )),
            None => Box::new(BufWriter::with_capacity(1 << 20, io::stdout())),
        },
        index: 0,
        err: None,
    };
    log.feed_reader(&mut reader, &mut dumper)
        .map_err(|e| format!("{path}: {e}"))?;
    // The last game has no next one to end it.
    if let Some(game) = log.game() {
        dumper.end_game(game);
    }
    dumper.out.flush().map_err(|e| format!("{path}: {e}"))?;
    let elapsed = started.elapsed();
    if let Some(e) = dumper.err {
        return Err(format!("{path}: writing the dump: {e}"));
    }
    eprintln!(
        "{path}: {} games, read and dumped in {:.3} s",
        dumper.index,
        elapsed.as_secs_f64()
    );
    Ok(())
}

/// Counts the games, for the parse-only mode.
#[derive(Default)]
struct Counter {
    games: usize,
}

impl Handler for Counter {
    fn new_game(&mut self, _game: &mut Game) {
        self.games += 1;
    }
}

/// Writes a game's final entities and tags.
struct Dumper {
    out: Box<dyn Write>,
    index: usize,
    err: Option<io::Error>,
}

impl Handler for Dumper {
    fn end_game(&mut self, game: &Game) {
        self.index += 1;
        if self.err.is_some() {
            return;
        }
        if let Err(e) = write_game(&mut *self.out, self.index, game) {
            self.err = Some(e);
        }
    }
}

fn write_game(out: &mut dyn Write, index: usize, game: &Game) -> io::Result<()> {
    writeln!(out, "game {index} type={}", game.game_type)?;

    let mut players: Vec<(i32, i32)> = game.players.iter().map(|(&p, &e)| (p, e)).collect();
    players.sort_unstable();
    write!(out, "players")?;
    for (player, entity) in players {
        write!(out, " {player}={entity}")?;
    }
    writeln!(out)?;

    let mut real: Vec<i32> = game.real_players.iter().copied().collect();
    real.sort_unstable();
    write!(out, "real")?;
    for player in real {
        write!(out, " {player}")?;
    }
    writeln!(out)?;

    let mut names: Vec<(i32, &str)> = game
        .player_names
        .iter()
        .map(|(player, name)| (*player, name.as_str()))
        .collect();
    names.sort_unstable();
    write!(out, "player_names")?;
    for (player, name) in names {
        write!(out, " {player}={}", quote(name))?;
    }
    writeln!(out)?;

    let mut known: Vec<(&str, i32)> = game.names.iter().map(|(n, &e)| (n.as_str(), e)).collect();
    known.sort_unstable();
    write!(out, "names")?;
    for (name, entity) in known {
        write!(out, " {}={entity}", quote(name))?;
    }
    writeln!(out)?;

    let mut last: Vec<(i32, &str)> = game
        .last_name
        .iter()
        .map(|(&e, n)| (e, n.as_str()))
        .collect();
    last.sort_unstable();
    write!(out, "last_names")?;
    for (entity, name) in last {
        write!(out, " {entity}={}", quote(name))?;
    }
    writeln!(out)?;

    let mut ids: Vec<i32> = game.entities.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let entity = &game.entities[&id];
        write!(out, "entity {id} card_id={} tags", quote(&entity.card_id))?;
        let mut tags: Vec<(&str, &TagValue)> =
            entity.tags.iter().map(|(t, v)| (t.as_str(), v)).collect();
        // Tag names are unique, so the name alone orders them.
        tags.sort_unstable_by(|a, b| a.0.cmp(b.0));
        for (tag, value) in tags {
            write!(out, " {tag}={}", value_text(value))?;
        }
        writeln!(out)?;
    }
    Ok(())
}

/// A number bare, a text value quoted.
fn value_text(value: &TagValue) -> String {
    match value {
        TagValue::Int(n) => n.to_string(),
        TagValue::Text(s) => quote(s),
    }
}

/// In single quotes, with backslashes and quotes escaped, so the Python dump
/// can produce the same bytes.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}
