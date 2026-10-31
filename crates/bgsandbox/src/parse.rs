//! Turning a typed line into a command. Numbers are 1-based, the way the
//! screen shows them.

/// A board minion (a bare number) or a Tavern minion (`t3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg {
    Board(usize),
    Tavern(usize),
}

/// What `?` looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Tavern(usize),
    TavernSpell,
    Hand(usize),
    Board(usize),
    Discover(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Buy(usize),
    BuySpell,
    /// Play hand card `hand`. For a minion the argument is the board
    /// position, for a spell its target.
    Play { hand: usize, arg: Option<Arg> },
    Sell(usize),
    Move(usize, usize),
    Activate(usize),
    /// A bare number or `tN`: answers whatever choice is open.
    Pick(Arg),
    Roll,
    Freeze,
    Upgrade,
    /// Press the MAIN hero power button.
    Power,
    /// Press the SECOND hero power button (Genn, Reinvigorating Light).
    Power2,
    End,
    Info(Look),
    Undo,
    Quit,
    Help,
    /// Set or change the label of this game.
    Label(String),
}

fn number(s: &str) -> Option<usize> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok().filter(|&n| n >= 1)
}

fn arg(s: &str) -> Result<Arg, String> {
    if let Some(n) = s.strip_prefix('t') {
        return number(n).map(Arg::Tavern).ok_or_else(|| format!("{s:?} is not a Tavern slot (t1, t2, ...)"));
    }
    number(s).map(Arg::Board).ok_or_else(|| format!("{s:?} is not a number (1, 2, ...) or a Tavern slot (t1, ...)"))
}

fn need(s: &str, what: &str) -> Result<usize, String> {
    number(s).ok_or_else(|| format!("expected {what}, a number from 1 up, not {s:?}"))
}

fn look(rest: &str) -> Result<Look, String> {
    let rest: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    let bad = || "? takes a Tavern slot (? 3), ?s for the spell slot, ?h1 for a hand card, ?b2 for a board minion, ?d1 for a Discover option".to_string();
    if rest == "s" {
        return Ok(Look::TavernSpell);
    }
    if let Some(n) = rest.strip_prefix('h') {
        return number(n).map(Look::Hand).ok_or_else(bad);
    }
    if let Some(n) = rest.strip_prefix('b') {
        return number(n).map(Look::Board).ok_or_else(bad);
    }
    if let Some(n) = rest.strip_prefix('d') {
        return number(n).map(Look::Discover).ok_or_else(bad);
    }
    let n = rest.strip_prefix('t').unwrap_or(&rest);
    number(n).map(Look::Tavern).ok_or_else(bad)
}

/// Parse one line. Err carries the message to show.
pub fn parse(line: &str) -> Result<Cmd, String> {
    let original = line.trim();
    if let Some((word, rest)) = original.split_once(char::is_whitespace)
        && word.eq_ignore_ascii_case("label")
    {
        let text = rest.trim();
        return if text.is_empty() {
            Err("label needs a name, for example: label lobster".into())
        } else {
            Ok(Cmd::Label(text.to_string()))
        };
    }
    let line = original.to_lowercase();
    if line == "label" {
        return Err("label needs a name, for example: label lobster".into());
    }
    if line.is_empty() {
        return Err("type a command, or help".into());
    }
    if line == "?" {
        return Ok(Cmd::Help);
    }
    if let Some(rest) = line.strip_prefix('?') {
        return look(rest).map(Cmd::Info);
    }
    let mut toks: Vec<&str> = line.split_whitespace().collect();
    // "b2" is "b 2": split the letters off the digits of the first word.
    let first = toks.remove(0);
    let split = first.find(|c: char| c.is_ascii_digit()).unwrap_or(first.len());
    let (word, digits) = first.split_at(split);
    let mut args: Vec<&str> = Vec::new();
    if !digits.is_empty() {
        args.push(digits);
    }
    args.extend(toks);
    // a bare number or tN answers an open choice
    if word.is_empty() {
        return match args.as_slice() {
            [a] => arg(a).map(Cmd::Pick),
            _ => Err("a bare number answers an open choice, one at a time".into()),
        };
    }
    if word == "t" && args.len() == 1 {
        return arg(&format!("t{}", args[0])).map(Cmd::Pick);
    }
    let one = |what: &str| match args.as_slice() {
        [a] => need(a, what),
        [] => Err(format!("missing {what}")),
        _ => Err(format!("too many arguments, expected just {what}")),
    };
    let none = |c: Cmd| {
        if args.is_empty() { Ok(c) } else { Err(format!("{word} takes no arguments")) }
    };
    match word {
        "b" | "buy" => one("a Tavern slot").map(Cmd::Buy),
        "bs" => none(Cmd::BuySpell),
        "p" | "play" | "c" | "cast" => {
            let (h, rest) = args.split_first().ok_or("missing the hand card number (p1, c2 ...)")?;
            let hand = need(h, "a hand card")?;
            match rest {
                [] => Ok(Cmd::Play { hand, arg: None }),
                [a] => Ok(Cmd::Play { hand, arg: Some(arg(a)?) }),
                _ => Err("too many arguments: p<hand> [position] or c<hand> [target]".into()),
            }
        }
        "s" | "sell" => one("a board minion").map(Cmd::Sell),
        "m" | "move" => match args.as_slice() {
            [a, b] => Ok(Cmd::Move(need(a, "a board minion")?, need(b, "a board position")?)),
            _ => Err("move needs two numbers: m2 5 moves board minion 2 to position 5".into()),
        },
        "a" | "act" | "activate" => one("a board minion").map(Cmd::Activate),
        "r" | "roll" | "refresh" => none(Cmd::Roll),
        // "hp2" splits as the letters "hp" and the digit 2: the second power.
        "hp" | "power" if args.as_slice() == ["2"] => Ok(Cmd::Power2),
        "hero_power2" => none(Cmd::Power2),
        "hp" | "power" | "hero_power" => none(Cmd::Power),
        "f" | "freeze" | "unfreeze" => none(Cmd::Freeze),
        "u" | "up" | "upgrade" | "level" => none(Cmd::Upgrade),
        "e" | "end" => none(Cmd::End),
        "undo" => none(Cmd::Undo),
        "quit" | "q" | "exit" => none(Cmd::Quit),
        "help" | "h" => none(Cmd::Help),
        _ => Err(format!("unknown command {first:?}, type help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_parses() {
        assert_eq!(parse("b2"), Ok(Cmd::Buy(2)));
        assert_eq!(parse("b 2"), Ok(Cmd::Buy(2)));
        assert_eq!(parse("  B2  "), Ok(Cmd::Buy(2)));
        assert_eq!(parse("bs"), Ok(Cmd::BuySpell));
        assert_eq!(parse("p1"), Ok(Cmd::Play { hand: 1, arg: None }));
        assert_eq!(parse("p1 3"), Ok(Cmd::Play { hand: 1, arg: Some(Arg::Board(3)) }));
        assert_eq!(parse("c1 4"), Ok(Cmd::Play { hand: 1, arg: Some(Arg::Board(4)) }));
        assert_eq!(parse("c1 t4"), Ok(Cmd::Play { hand: 1, arg: Some(Arg::Tavern(4)) }));
        assert_eq!(parse("s4"), Ok(Cmd::Sell(4)));
        assert_eq!(parse("m2 5"), Ok(Cmd::Move(2, 5)));
        assert_eq!(parse("a3"), Ok(Cmd::Activate(3)));
        assert_eq!(parse("1"), Ok(Cmd::Pick(Arg::Board(1))));
        assert_eq!(parse("t4"), Ok(Cmd::Pick(Arg::Tavern(4))));
        assert_eq!(parse("r"), Ok(Cmd::Roll));
        assert_eq!(parse("hp"), Ok(Cmd::Power));
        assert_eq!(parse("hp2"), Ok(Cmd::Power2));
        assert_eq!(parse("f"), Ok(Cmd::Freeze));
        assert_eq!(parse("u"), Ok(Cmd::Upgrade));
        assert_eq!(parse("e"), Ok(Cmd::End));
        assert_eq!(parse("? 3"), Ok(Cmd::Info(Look::Tavern(3))));
        assert_eq!(parse("?3"), Ok(Cmd::Info(Look::Tavern(3))));
        assert_eq!(parse("?h1"), Ok(Cmd::Info(Look::Hand(1))));
        assert_eq!(parse("?b2"), Ok(Cmd::Info(Look::Board(2))));
        assert_eq!(parse("?s"), Ok(Cmd::Info(Look::TavernSpell)));
        assert_eq!(parse("?d2"), Ok(Cmd::Info(Look::Discover(2))));
        assert_eq!(parse("undo"), Ok(Cmd::Undo));
        assert_eq!(parse("quit"), Ok(Cmd::Quit));
        assert_eq!(parse("help"), Ok(Cmd::Help));
        assert_eq!(parse("?"), Ok(Cmd::Help));
        assert_eq!(parse("label Magnetic Mechs"), Ok(Cmd::Label("Magnetic Mechs".into())));
        assert_eq!(parse("  LABEL   lobster "), Ok(Cmd::Label("lobster".into())));
    }

    #[test]
    fn junk_is_an_error() {
        for bad in ["", "   ", "zzz", "b", "b0", "b x", "b1 2", "p", "p1 2 3", "m2", "m2 x", "s", "r 3",
            "e now", "label", "label   ", "?x", "? 0", "undo 2", "1 2", "t", "pp1"]
        {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }
}
