//! `bgbench tracker`: the Battlegrounds views a Power.log builds.
//!
//! Runs the tracker over a log and prints, game by game, the lobby seats and
//! every fight: both boards, the boards at the first attack, who swung first,
//! the result, and the hero powers, trinkets, rewards, counters, Deities and
//! hands of both sides. The same dump comes out of the older Python tracker
//! (`battlegrounds.py`), so the two can be diffed line for line.
//!
//!   bgbench tracker FILE [--out FILE] [--cards FILE] [--time]
//!
//! `--cards` is the card name data, `ref/hsjson-cards.json` by default.
//! `--time` parses only and prints the time taken.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::exit;
use std::time::Instant;

use bglog::{
    Cards, Deity, Fight, HeroPower, Minion, Reward, TagValue, Tracker, Trinket,
};

use crate::powerlog::{quote, value_text};

pub fn run(args: &[String]) {
    let Some(path) = args.get(2).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: bgbench tracker FILE [--out FILE] [--cards FILE] [--time]");
        exit(2);
    };
    let flag = |name: &str, default: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|k| args.get(k + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let cards = flag("--cards", "ref/hsjson-cards.json");
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|k| args.get(k + 1))
        .cloned();
    let time_only = args.iter().any(|a| a == "--time");
    if let Err(e) = dump(path, out.as_deref(), &cards, time_only) {
        eprintln!("{e}");
        exit(2);
    }
}

fn dump(path: &str, out: Option<&str>, cards: &str, time_only: bool) -> Result<(), String> {
    let cards = Cards::load(cards)?;
    let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut tracker = Tracker::new(cards);
    let mut line = String::new();
    let started = Instant::now();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).map_err(|e| format!("{path}: {e}"))?;
        if read == 0 {
            break;
        }
        tracker.feed(&line);
    }
    let elapsed = started.elapsed();
    let games = tracker.games();
    let fights: usize = games.iter().map(|(_, fights)| fights.len()).sum();
    let seats: usize = games.iter().map(|(seats, _)| seats.len()).sum();
    if time_only {
        eprintln!(
            "{path}: {fights} fights and {seats} seats in {} games, read in {:.3} s",
            games.len(),
            elapsed.as_secs_f64()
        );
        return Ok(());
    }
    let mut writer: Box<dyn Write> = match out {
        Some(path) => Box::new(BufWriter::with_capacity(
            1 << 20,
            File::create(path).map_err(|e| format!("{path}: {e}"))?,
        )),
        None => Box::new(BufWriter::with_capacity(1 << 20, io::stdout())),
    };
    for (index, (seats, fights)) in games.iter().enumerate() {
        write_game(&mut writer, index + 1, seats, fights).map_err(|e| format!("{path}: {e}"))?;
    }
    writeln!(writer, "version={}", tracker.version()).map_err(|e| format!("{path}: {e}"))?;
    writer.flush().map_err(|e| format!("{path}: {e}"))?;
    eprintln!(
        "{path}: {fights} fights and {seats} seats in {} games, read and dumped in {:.3} s",
        games.len(),
        elapsed.as_secs_f64()
    );
    Ok(())
}

fn write_game(
    out: &mut dyn Write,
    index: usize,
    seats: &std::collections::BTreeMap<i32, bglog::Seat>,
    fights: &[Fight],
) -> io::Result<()> {
    writeln!(out, "game {index}")?;
    writeln!(out, "  seats n={}", seats.len())?;
    for seat in seats.values() {
        let last_fight = match seat.last_fight {
            Some(index) => index.to_string(),
            None => "none".to_string(),
        };
        writeln!(
            out,
            "  seat {} hero={} hero_card={} health={} armor={} tier={} place={} name={} \
             last_fight={last_fight} dead={}",
            seat.player_id,
            quote(&seat.hero),
            quote(&seat.hero_card),
            seat.health,
            seat.armor,
            seat.tier,
            seat.place,
            quote(&seat.name),
            seat.dead() as u8,
        )?;
    }
    writeln!(out, "  fights n={}", fights.len())?;
    for (index, fight) in fights.iter().enumerate() {
        write_fight(out, index, fight)?;
    }
    Ok(())
}

fn write_fight(out: &mut dyn Write, index: usize, fight: &Fight) -> io::Result<()> {
    writeln!(out, "  fight {index} turn={} opponent={}", fight.turn, fight.opponent)?;
    writeln!(
        out,
        "    at_attack={}",
        if fight.at_attack.is_some() {
            "present"
        } else {
            "none"
        }
    )?;
    writeln!(
        out,
        "    first={} result={} gems=({},{})({},{}) hidden=({},{}) waits=({},{}) opening={}",
        quote(&fight.first),
        quote(&fight.result),
        fight.gems.0.0,
        fight.gems.0.1,
        fight.gems.1.0,
        fight.gems.1.1,
        fight.hidden.0,
        fight.hidden.1,
        fight.waits.0,
        fight.waits.1,
        match fight.opening {
            Some((side, index)) => format!("({side},{index})"),
            None => "none".to_string(),
        },
    )?;
    write_board(out, "mine", &fight.mine)?;
    write_board(out, "theirs", &fight.theirs)?;
    if let Some((mine, theirs)) = &fight.at_attack {
        write_board(out, "at_attack.mine", mine)?;
        write_board(out, "at_attack.theirs", theirs)?;
    }
    write_optional_board(out, "hand.mine", fight.hand.as_ref())?;
    write_optional_board(out, "hand.theirs", fight.their_hand.as_ref())?;
    write_optional(out, "hero_power.mine", fight.hero_powers.0.as_ref(), hero_power_text)?;
    write_optional(
        out,
        "hero_power.theirs",
        fight.hero_powers.1.as_ref(),
        hero_power_text,
    )?;
    write_trinkets(out, "trinkets.mine", &fight.trinkets.0)?;
    write_trinkets(out, "trinkets.theirs", &fight.trinkets.1)?;
    write_rewards(out, "rewards.mine", &fight.rewards.0)?;
    write_rewards(out, "rewards.theirs", &fight.rewards.1)?;
    write_counters(out, "counters.mine", &fight.counters.0)?;
    write_counters(out, "counters.theirs", &fight.counters.1)?;
    write_optional(out, "deities.mine", fight.deities.0.as_ref(), deity_text)?;
    write_optional(out, "deities.theirs", fight.deities.1.as_ref(), deity_text)?;
    Ok(())
}

fn write_board(out: &mut dyn Write, name: &str, board: &[Minion]) -> io::Result<()> {
    writeln!(out, "    {name} n={}", board.len())?;
    for (index, minion) in board.iter().enumerate() {
        writeln!(out, "    {name} {index} {}", minion_text(minion))?;
    }
    Ok(())
}

fn write_optional_board(out: &mut dyn Write, name: &str, board: Option<&Vec<Minion>>) -> io::Result<()> {
    match board {
        Some(board) => write_board(out, name, board),
        None => writeln!(out, "    {name} uncaptured"),
    }
}

fn write_trinkets(out: &mut dyn Write, name: &str, trinkets: &[Trinket]) -> io::Result<()> {
    writeln!(out, "    {name} n={}", trinkets.len())?;
    for (index, trinket) in trinkets.iter().enumerate() {
        writeln!(out, "    {name} {index} {}", trinket_text(trinket))?;
    }
    Ok(())
}

fn write_rewards(out: &mut dyn Write, name: &str, rewards: &[Reward]) -> io::Result<()> {
    writeln!(out, "    {name} n={}", rewards.len())?;
    for (index, reward) in rewards.iter().enumerate() {
        writeln!(out, "    {name} {index} {}", reward_text(reward))?;
    }
    Ok(())
}

fn write_counters(
    out: &mut dyn Write,
    name: &str,
    counters: &std::collections::BTreeMap<&'static str, i64>,
) -> io::Result<()> {
    let text: Vec<String> = counters
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    writeln!(out, "    {name} [{}]", text.join(","))
}

fn write_optional<T>(
    out: &mut dyn Write,
    name: &str,
    value: Option<&T>,
    text: fn(&T) -> String,
) -> io::Result<()> {
    match value {
        Some(value) => writeln!(out, "    {name} {}", text(value)),
        None => writeln!(out, "    {name} none"),
    }
}

fn minion_text(minion: &Minion) -> String {
    let gifts: Vec<String> = minion.dark_gifts.iter().map(|g| quote(g)).collect();
    let enchants: Vec<String> = minion.hp_enchants.iter().map(|g| quote(g)).collect();
    format!(
        "minion(card_id={} name={} atk={} hp={} tier={} golden={} kw=[{}] gifts=[{}] hpe=[{}] \
         tags=[{}])",
        quote(&minion.card_id),
        quote(&minion.name),
        minion.attack,
        minion.health,
        minion.tier,
        minion.golden as u8,
        minion.keywords.join(","),
        gifts.join(","),
        enchants.join(","),
        tags_text(&minion.tags),
    )
}

fn tags_text(tags: &HashMap<String, TagValue>) -> String {
    let mut sorted: Vec<(&str, &TagValue)> = tags.iter().map(|(k, v)| (k.as_str(), v)).collect();
    sorted.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let text: Vec<String> = sorted
        .iter()
        .map(|(tag, value)| format!("{tag}={}", value_text(value)))
        .collect();
    text.join(",")
}

fn numbers_text(numbers: [i64; 4]) -> String {
    format!(
        "({},{},{},{})",
        numbers[0], numbers[1], numbers[2], numbers[3]
    )
}

fn hero_power_text(power: &HeroPower) -> String {
    let waiting = match &power.waiting {
        Some(minion) => minion_text(minion),
        None => "none".to_string(),
    };
    format!(
        "hero_power(card_id={} name={} turn={} numbers={} waiting={waiting})",
        quote(&power.card_id),
        quote(&power.name),
        power.turn,
        numbers_text(power.numbers),
    )
}

fn trinket_text(trinket: &Trinket) -> String {
    format!(
        "trinket(card_id={} name={} greater={} numbers={})",
        quote(&trinket.card_id),
        quote(&trinket.name),
        trinket.greater as u8,
        numbers_text(trinket.numbers),
    )
}

fn reward_text(reward: &Reward) -> String {
    format!(
        "reward(card_id={} name={} numbers={})",
        quote(&reward.card_id),
        quote(&reward.name),
        numbers_text(reward.numbers),
    )
}

fn deity_text(deity: &Deity) -> String {
    format!(
        "deity(card_id={} name={} atk={} hp={} left={})",
        quote(&deity.card_id),
        quote(&deity.name),
        deity.attack,
        deity.health,
        deity.left,
    )
}
