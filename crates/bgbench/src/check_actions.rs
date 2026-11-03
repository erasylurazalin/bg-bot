//! `bgbench check-actions`: how many turns of the action files add up.
//!
//! A port of `~/projects/bg-bot/tools/check_actions.py`. Each file holds one
//! game and, per turn, the state I started with, every step I took and the state
//! I ended with (`docs/real-boards.md` "The action file"). Two things are
//! checked for every turn, with nothing from our engine:
//!
//! - **The Gold.** What every step took (`paid.gold`, plus any Gold the game
//!   took with no step of mine, `late_gold`) and gave back, against the Gold the
//!   turn started and ended with. A turn with Health paid for a card is checked
//!   the same way against Health and Armor together.
//! - **The cards.** My hand and board at the start, plus everything a step says
//!   it moved, brought in (`entered`, a Discover's pick, a buy, a steal out of
//!   the Tavern) or took away (`left`, a sell, a play), against my hand and board
//!   at the end. Cards are followed by the game's own entity id.
//!
//!   bgbench check-actions [--verbose] [PATTERN] [--actions DIR]
//!
//! `PATTERN` picks the files (the Python's `boards/actions.glob`, so `*` and `?`
//! match within a file name; `*-g*.json` by default) and `--actions DIR` is the
//! folder they are in (`boards/actions` by default). `--verbose` lists every
//! turn that does not balance with all its reasons.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use serde_json::Value;

use crate::collect_boards::flag;

/// The steps the client puts a Gold price on: the card's or button's COST tag.
const PRICED: [&str; 5] = ["buy", "roll", "level", "hero_power", "dark_discovery"];

pub fn run(args: &[String]) {
    let out = flag(args, "--actions", "boards/actions");
    let verbose = args.iter().any(|a| a == "--verbose");
    // The Python takes the first argument that is not a flag for its pattern;
    // the folder ours names is not one, so it is stepped over.
    let mut pattern = "*-g*.json".to_string();
    let mut found = false;
    let mut skip = false;
    for arg in args.iter().skip(2) {
        if skip {
            skip = false;
            continue;
        }
        if arg == "--actions" {
            skip = true;
        } else if !arg.starts_with("--") && !found {
            pattern = arg.clone();
            found = true;
        }
    }
    if let Err(e) = check_all(Path::new(&out), &pattern, verbose) {
        eprintln!("{e}");
        exit(2);
    }
}

fn check_all(out: &Path, pattern: &str, verbose: bool) -> Result<(), String> {
    let files = files(out, pattern)?;
    let mut turns = 0;
    let mut ok = 0;
    let mut counts = Counter::new();
    let mut named = Counter::new();
    let mut sources = PairCounter::new();
    let mut unnamed: Vec<(String, i64, String)> = Vec::new();
    let mut bad_turns: Vec<(String, i64, Vec<String>)> = Vec::new();
    for path in &files {
        let (good, bad, c, n, s, u) = check(path)?;
        turns += c.get("turns");
        ok += good;
        counts.add_others(&c);
        named.add_all(&n);
        sources.add_all(&s);
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let file = file.unwrap_or_default();
        for (turn, why) in u {
            unnamed.push((file.clone(), turn, why));
        }
        let mut line = format!(
            "{file}: {good}/{} turns balance",
            c.get("turns")
        );
        if !bad.is_empty() {
            let which = if bad.len() == 1 { "does" } else { "do" };
            let turns: Vec<String> = bad.iter().map(|(t, _)| format!("turn {t}")).collect();
            line.push_str(&format!(", {which} not: {}", turns.join(", ")));
        }
        println!("{line}");
        if verbose {
            for (turn, reasons) in &bad {
                println!("   turn {turn}:");
                for reason in reasons {
                    println!("      {reason}");
                }
            }
            for (_, turn, why) in unnamed.iter().filter(|(f, _, _)| *f == file) {
                println!("   turn {turn}: unnamed: {why}");
            }
        }
        for (turn, reasons) in bad {
            bad_turns.push((file.clone(), turn, reasons));
        }
    }
    println!();
    let mut summary = format!("{} files, {turns} turns, {ok} balance", files.len());
    if turns > 0 {
        summary.push_str(&format!(
            " ({:.1}%)",
            ok as f64 * 100.0 / turns as f64
        ));
    }
    println!("{summary}");
    let counts_text = if counts.is_empty() {
        "nothing".to_string()
    } else {
        counts.render()
    };
    println!("what did not balance: {counts_text}");
    println!("what the steps name instead of a price or a creator:");
    for (kind, count) in named.most_common() {
        println!("  {kind}: {count}");
        let rows: Vec<(i64, String)> = sources
            .most_common()
            .into_iter()
            .filter(|((k, _), _)| *k == kind)
            .map(|((_, by), n)| (n, by))
            .collect();
        for (n, by) in rows.iter().take(8) {
            println!("      {n:5}  {by}");
        }
        if rows.len() > 8 {
            println!("      and {} more kinds of source", rows.len() - 8);
        }
    }
    println!("left unnamed: {}", unnamed.len());
    for (file, turn, why) in &unnamed {
        println!("  {file} turn {turn}: {why}");
    }
    if !bad_turns.is_empty() && !verbose {
        println!("\nthe turns that do not balance:");
        for (file, turn, reasons) in &bad_turns {
            println!(
                "  {file} turn {turn}: {}",
                reasons.iter().take(3).cloned().collect::<Vec<String>>().join("; ")
            );
        }
    }
    Ok(())
}

/// The action files of `dir` whose name matches `pattern`, sorted.
fn files(dir: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .map(|name| matches(&name.to_string_lossy(), pattern))
                .unwrap_or(false)
        })
        .collect();
    out.sort();
    Ok(out)
}

/// A shell pattern against a file name, as `Path.glob` matches one: `*` is any
/// run of characters (a `/` included, which a file name cannot hold anyway),
/// `?` is one, and everything else is itself.
fn matches(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut n, mut p) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            n += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// A count of something, in the order it was first named.
struct Counter(Vec<(String, i64)>);

impl Counter {
    fn new() -> Counter {
        Counter(Vec::new())
    }

    fn add(&mut self, key: &str, n: i64) {
        match self.0.iter_mut().find(|(have, _)| have == key) {
            Some(slot) => slot.1 += n,
            None => self.0.push((key.to_string(), n)),
        }
    }

    fn get(&self, key: &str) -> i64 {
        self.0
            .iter()
            .find(|(have, _)| have == key)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    fn add_all(&mut self, other: &Counter) {
        for (key, n) in &other.0 {
            self.add(key, *n);
        }
    }

    /// Every count of `other` but its `turns`, which the run adds up itself.
    fn add_others(&mut self, other: &Counter) {
        for (key, n) in other.0.iter().filter(|(key, _)| key != "turns") {
            self.add(key, *n);
        }
    }

    fn is_empty(&self) -> bool {
        self.0.iter().all(|(_, n)| *n == 0)
    }

    /// `dict(counter)`: `{'Gold': 2, 'cards': 5}`.
    fn render(&self) -> String {
        let items: Vec<String> = self
            .0
            .iter()
            .map(|(key, n)| format!("'{key}': {n}"))
            .collect();
        format!("{{{}}}", items.join(", "))
    }

    /// `Counter.most_common`: by count, highest first, and in the order they
    /// were first named when two are equal.
    fn most_common(&self) -> Vec<(String, i64)> {
        let mut out = self.0.clone();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }
}

/// A count keyed by a pair: the kind of a named thing and the card or slot it
/// says.
struct PairCounter(Vec<((String, String), i64)>);

impl PairCounter {
    fn new() -> PairCounter {
        PairCounter(Vec::new())
    }

    fn add(&mut self, kind: &str, by: &str) {
        let key = (kind.to_string(), by.to_string());
        match self.0.iter_mut().find(|(have, _)| *have == key) {
            Some(slot) => slot.1 += 1,
            None => self.0.push((key, 1)),
        }
    }

    fn add_all(&mut self, other: &PairCounter) {
        for ((kind, by), n) in &other.0 {
            let key = (kind.clone(), by.clone());
            match self.0.iter_mut().find(|(have, _)| *have == key) {
                Some(slot) => slot.1 += n,
                None => self.0.push((key, *n)),
            }
        }
    }

    fn most_common(&self) -> Vec<((String, String), i64)> {
        let mut out = self.0.clone();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }
}

/// A card as the checker follows it: its card id, and whether it was on my
/// board (`position`) rather than in my hand (`index`).
#[derive(Clone)]
struct Card {
    card_id: Option<String>,
    on_board: bool,
}

/// A hand and a board as the cards in them, by the game's own entity id, and
/// the number of hand cards the log does not name.
fn cards(state: &Value) -> (Vec<(i64, Card)>, i64) {
    let mut out: Vec<(i64, Card)> = Vec::new();
    for part in ["hand", "board"] {
        for card in array(state, part) {
            let id = card.get("id").and_then(Value::as_i64).unwrap_or(0);
            let card = Card {
                card_id: card
                    .get("card_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                on_board: card.get("position").is_some(),
            };
            match out.iter_mut().find(|(have, _)| *have == id) {
                Some(slot) => slot.1 = card,
                None => out.push((id, card)),
            }
        }
    }
    let hidden = state.get("hand_hidden").and_then(Value::as_i64).unwrap_or(0);
    (out, hidden)
}

/// The cards of a state part.
fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|v| !v.is_null())
}

/// The Python's truthiness: null, `false`, zero, an empty string, list or
/// object are false, and everything else is true.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|n| n != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `value[key]` where the Python expects a number.
fn number(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// The entity a step acted on: the card it moved, played, took or was aimed
/// at.
fn id_of(step: &Value) -> Option<i64> {
    for (part, key) in [("moved", "id"), ("card", "id"), ("target", "id"), ("picked", "id")] {
        if let Some(id) = field(step, part)
            .and_then(|part| part.get(key))
            .and_then(Value::as_i64)
            && id != 0
        {
            return Some(id);
        }
    }
    None
}

/// The first of `card`, `target` and `picked` that the step has.
fn card_of(step: &Value) -> Option<&Value> {
    for part in ["card", "target", "picked"] {
        if let Some(value) = field(step, part)
            && truthy(Some(value))
        {
            return Some(value);
        }
    }
    None
}

/// A card of a step as `card id (name)`.
fn name(card: Option<&Value>) -> String {
    let Some(card) = card else {
        return "no card".to_string();
    };
    let Some(card_id) = card.get("card_id").filter(|v| !v.is_null()) else {
        return "no card".to_string();
    };
    let card_id = card_id.as_str().unwrap_or("None");
    let name = match card.get("name") {
        Some(Value::String(name)) => name.clone(),
        _ => "None".to_string(),
    };
    format!("{card_id} ({name})")
}

/// What a card of a state id prints as.
fn card_id_text(card: &Card) -> String {
    card.card_id.clone().unwrap_or_else(|| "None".to_string())
}

/// What a turn's steps say about my hand and board: the cards they account for,
/// the hidden hand count, and the cards that came in with nothing named behind
/// them (the step's number, its kind, and the card).
type Walked = (Vec<(i64, Card)>, i64, Vec<(usize, String, String)>);

/// The cards in my hand and board that the turn's steps account for, the hidden
/// hand count, and the ones that came in with nothing named behind them.
fn walk(turn: &Value) -> Walked {
    let (mut have, mut hidden) = cards(turn.get("start").unwrap_or(&Value::Null));
    let mut unnamed: Vec<(usize, String, String)> = Vec::new();
    for (i, step) in array(turn, "actions").iter().enumerate() {
        if let Some(moved) = field(step, "moved")
            && let Some(id) = id_of(step)
        {
            let to = field(moved, "to");
            let place = to.and_then(|to| to.get("where")).and_then(Value::as_str);
            if let Some(to) = to
                && matches!(place, Some("hand") | Some("board"))
            {
                // a pick the game landed as a new entity says which one it made
                let id = to.get("id").and_then(Value::as_i64).unwrap_or(id);
                let card = card_of(step);
                let entry = Card {
                    card_id: card
                        .and_then(|c| c.get("card_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    on_board: to.get("position").is_some(),
                };
                if !have.iter().any(|(have, _)| *have == id) {
                    have.push((id, entry));
                }
            } else if to.is_none() {
                // off my hand and board both: the step took it away
                have.retain(|(have, _)| *have != id);
            } else {
                have.retain(|(have, _)| *have != id);
            }
        }
        for card in array(step, "entered") {
            let id = number(card, "id");
            let entry = Card {
                card_id: card
                    .get("card_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                on_board: card.get("position").is_some(),
            };
            match have.iter_mut().find(|(have, _)| *have == id) {
                Some(slot) => slot.1 = entry.clone(),
                None => have.push((id, entry.clone())),
            }
            if !truthy(card.get("from")) && !truthy(card.get("stolen")) {
                unnamed.push((
                    i,
                    step.get("step").and_then(Value::as_str).unwrap_or("").to_string(),
                    card_id_text(&entry),
                ));
            }
        }
        for card in array(step, "left") {
            let id = number(card, "id");
            have.retain(|(have, _)| *have != id);
        }
        if let Some(count) = step.get("hand_hidden").and_then(Value::as_i64) {
            hidden = count;
        }
    }
    (have, hidden, unnamed)
}

/// What the turn's steps took, in Gold and in Health, and what the client's own
/// numbers say the turn took.
fn ledger(turn: &Value) -> (i64, i64, i64, i64) {
    let actions = array(turn, "actions");
    let mut spent = 0;
    let mut health = 0;
    for step in actions {
        spent += number_or(step.get("paid"), "gold") + number_or(step.get("late_gold"), "gold");
        health += number_or(step.get("paid"), "health");
    }
    let start = turn.get("start").unwrap_or(&Value::Null);
    let end = turn.get("end").unwrap_or(&Value::Null);
    // a Health price is paid out of Armor first, so the two go together
    (
        spent,
        number(start, "gold") - number(end, "gold"),
        health,
        number(start, "health") + number(start, "armor")
            - number(end, "health")
            - number(end, "armor"),
    )
}

/// `value[key]` where `value` may be missing.
fn number_or(value: Option<&Value>, key: &str) -> i64 {
    match value {
        Some(value) => number(value, key),
        None => 0,
    }
}

/// Everything the turn's steps name: what a price was paid in, what gave Gold
/// or Health back, where a free reroll came from, what the Tavern gave up. Each
/// as (what it is, the card or slot it says).
fn features(turn: &Value) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for step in array(turn, "actions") {
        if let Some(gain) = field(step, "gold_gain")
            && truthy(gain.get("gold"))
        {
            out.push(("a Gold gain inside a step", name(field(gain, "by"))));
        }
        if let Some(rewound) = field(step, "rewound") {
            out.push((
                "a price paid in Health, given back",
                name(field(rewound, "by")),
            ));
        } else if truthy(field(step, "cost")) && number_or(step.get("paid"), "health") != 0 {
            out.push((
                "a price paid in Health",
                name(card_of(step)),
            ));
        }
        if let Some(from) = field(step, "free_from") {
            let names: Vec<String> = from
                .as_array()
                .map(|cards| cards.iter().map(|c| name(Some(c))).collect())
                .unwrap_or_default();
            out.push((
                "a free reroll from free Refreshes",
                names.join(", "),
            ));
        }
        for card in array(step, "entered") {
            if truthy(card.get("stolen")) {
                out.push((
                    "a card stolen out of the Tavern",
                    name(field(step, "card")),
                ));
            }
        }
    }
    out
}

/// The steps whose price is not what the client printed and that the file does
/// not explain, and the cards that came in with nothing behind them.
fn unexplained(turn: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for step in array(turn, "actions") {
        let cost = field(step, "cost");
        let paid = step.get("paid").unwrap_or(&Value::Null);
        let gold = number(paid, "gold");
        let health = number(paid, "health");
        let note = format!(
            "{} priced {}, paid {gold} Gold and {health} Health",
            step.get("step").and_then(Value::as_str).unwrap_or(""),
            match cost {
                Some(Value::Number(n)) => n.to_string(),
                _ => "None".to_string(),
            }
        );
        if step.get("step").and_then(Value::as_str) == Some("sell") {
            if gold >= 0 {
                out.push(note);
            }
            continue;
        }
        let kind = step.get("step").and_then(Value::as_str).unwrap_or("");
        let Some(cost) = cost.and_then(Value::as_i64) else {
            continue;
        };
        if !PRICED.contains(&kind) || cost == 0 || cost == gold {
            continue;
        }
        let gain = field(step, "gold_gain")
            .map(|gain| number(gain, "gold"))
            .unwrap_or(0);
        if cost == gold + gain
            || truthy(field(step, "rewound"))
            || health != 0
        {
            continue;
        }
        if truthy(field(step, "free_from")) || truthy(field(step, "free_refreshes")) {
            continue;
        }
        out.push(note);
    }
    out
}

/// One file: the turns that balance, the ones that do not, and how the steps
/// name what their price tags do not.
type Checked = (
    i64,
    Vec<(i64, Vec<String>)>,
    Counter,
    Counter,
    PairCounter,
    Vec<(i64, String)>,
);

fn check(path: &Path) -> Result<Checked, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let rec: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut ok = 0;
    let mut bad: Vec<(i64, Vec<String>)> = Vec::new();
    let mut counts = Counter::new();
    let mut named = Counter::new();
    let mut sources = PairCounter::new();
    let mut unnamed: Vec<(i64, String)> = Vec::new();
    for turn in array(&rec, "turns") {
        counts.add("turns", 1);
        let mut reasons: Vec<String> = Vec::new();
        if turn.get("end").map(|end| end.is_null()).unwrap_or(true) {
            reasons.push("the log ends before the fight".to_string());
            counts.add("no end", 1);
        } else {
            let (spent, delta, health, hurt) = ledger(turn);
            if spent != delta {
                reasons.push(format!(
                    "Gold: the steps took {spent}, the turn lost {delta}"
                ));
                counts.add("Gold", 1);
            }
            if health != hurt {
                reasons.push(format!(
                    "Health: the steps took {health}, the turn lost {hurt}"
                ));
                counts.add("Health", 1);
            }
            for (kind, by) in features(turn) {
                named.add(kind, 1);
                if !by.is_empty() {
                    sources.add(kind, &by);
                }
            }
            for note in unexplained(turn) {
                unnamed.push((number(turn, "turn"), note));
                counts.add("a price with nothing behind it", 1);
            }
        }
        let (have, hidden, no_card) = walk(turn);
        for (_, step, card_id) in no_card {
            unnamed.push((
                number(turn, "turn"),
                format!("{step} brought in {card_id} with nothing behind it"),
            ));
            counts.add("a card with nothing behind it", 1);
        }
        if !turn.get("end").map(|end| end.is_null()).unwrap_or(true) {
            let (end, end_hidden) = cards(turn.get("end").unwrap_or(&Value::Null));
            let mut notes: Vec<String> = Vec::new();
            for (id, card) in &end {
                if !have.iter().any(|(have, _)| have == id) {
                    let place = if card.on_board { "board" } else { "hand" };
                    notes.push(format!(
                        "{} in my {place} at the end that no step brought in",
                        card_id_text(card)
                    ));
                }
            }
            for (id, card) in &have {
                if !end.iter().any(|(end, _)| end == id) {
                    let place = if card.on_board { "board" } else { "hand" };
                    notes.push(format!(
                        "{} gone from my {place} that no step took",
                        card_id_text(card)
                    ));
                }
            }
            if hidden != end_hidden {
                notes.push(format!("{:+} hidden hand cards", end_hidden - hidden));
            }
            if !notes.is_empty() {
                reasons.push(format!("cards: {}", notes.join("; ")));
                counts.add("cards", 1);
            }
        }
        if reasons.is_empty() {
            ok += 1;
        } else {
            bad.push((number(turn, "turn"), reasons));
        }
    }
    Ok((ok, bad, counts, named, sources, unnamed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_matches_like_the_python_glob() {
        assert!(matches("Hearthstone_x-g1.json", "*-g*.json"));
        assert!(matches("Hearthstone_x-g12.json", "*-g??.json"));
        assert!(!matches("Hearthstone_x-g12.json", "*-g?.json"));
        assert!(matches("a.json", "a.json"));
        assert!(!matches("b.json", "a.json"));
        assert!(matches("anything", "*"));
    }

    /// One invented turn: a hand card bought for 3 Gold, which lands in the
    /// hand as a new entity. No log, no player, nothing of Era's.
    fn a_turn() -> Value {
        serde_json::json!({
            "turn": 3,
            "start": {
                "board": [],
                "hand": [{"id": 1, "index": 1, "card_id": "CARD_A", "name": "A"}],
                "tavern": [{"id": 2, "slot": 1, "card_id": "CARD_B", "name": "B"}],
                "hand_hidden": 0,
                "gold": 10, "health": 30, "armor": 5,
            },
            "actions": [{
                "step": "buy", "option": 3, "target": {"id": 2, "kind": "slot", "slot": 1},
                "paid": {"gold": 3, "health": 0},
                "moved": {"id": 2, "from": {"where": "tavern", "slot": 1},
                          "to": {"where": "hand", "index": 2}},
                "entered": [{"id": 2, "where": "hand", "card_id": "CARD_B", "name": "B",
                             "index": 2, "from": {"card_id": "CARD_C", "name": "C"}}],
                "after": {},
            }],
            "end": {
                "board": [],
                "hand": [{"id": 1, "index": 1, "card_id": "CARD_A", "name": "A"},
                         {"id": 2, "index": 2, "card_id": "CARD_B", "name": "B"}],
                "tavern": [],
                "hand_hidden": 0,
                "gold": 7, "health": 30, "armor": 5,
            },
        })
    }

    #[test]
    fn a_turn_that_balances_adds_up() {
        let turn = a_turn();
        assert_eq!(ledger(&turn), (3, 3, 0, 0));
        let (have, hidden, unnamed) = walk(&turn);
        assert_eq!(hidden, 0);
        assert!(unnamed.is_empty());
        assert_eq!(have.len(), 2);
        assert!(have.iter().any(|(id, card)| *id == 2 && !card.on_board));
        assert!(features(&turn).is_empty());
        assert!(unexplained(&turn).is_empty());
    }

    #[test]
    fn a_price_the_client_printed_and_nothing_explains_is_named() {
        let mut turn = a_turn();
        turn["actions"][0]["cost"] = serde_json::json!(5);
        assert_eq!(
            unexplained(&turn),
            vec![
                "buy priced 5, paid 3 Gold and 0 Health".to_string()
            ]
        );
    }

    #[test]
    fn a_counter_prints_as_the_python_dict() {
        let mut counts = Counter::new();
        counts.add("Gold", 2);
        counts.add("cards", 5);
        assert_eq!(counts.render(), "{'Gold': 2, 'cards': 5}");
        assert!(Counter::new().is_empty());
    }
}
