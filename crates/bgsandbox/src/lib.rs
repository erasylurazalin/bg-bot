//! A one-player terminal sandbox over bgcore: play shop phases (the normal
//! pool, or a Tavern rigged to one tribe), fight a copy of your own board
//! each turn, and record every end-of-turn board to `boards/`. See `Session`
//! and `play`.

pub mod parse;
pub mod screen;
pub mod session;

use std::io::{BufRead, Write};

pub use session::{Config, Reply, Session, UNLABELED};

const LABEL_QUESTION: &str = "Label this game (the build you played, e.g. lobster, fodder, magnetic; blank keeps 'unlabeled'): ";

/// Ask for the label when none was given yet, then save the file under its
/// final name. With no answer (end of input) the label stays 'unlabeled'.
fn end_game(session: &mut Session, input: &mut dyn BufRead, out: &mut dyn Write, interactive: bool) {
    if interactive && session.needs_label() {
        let _ = write!(out, "\n{LABEL_QUESTION}");
        let _ = out.flush();
        let mut answer = String::new();
        let _ = input.read_line(&mut answer);
        session.answer_label(&answer);
    }
    match session.finish() {
        Ok(Some(p)) => {
            let _ = writeln!(out, "Saved {} turns to {} (label {:?})", session.turns().len(), p.display(), session.label());
        }
        Ok(None) => {
            let _ = writeln!(out, "No turn was played to the end, so no file was written.");
        }
        Err(e) => {
            let _ = writeln!(out, "could not save: {e}");
        }
    }
}

/// The command loop: read lines until quit or end of input. `interactive`
/// (a terminal) prints a prompt and asks the label question at the end; a
/// pipe never blocks on it, the label stays whatever `--label` or the
/// `label` command made it.
pub fn play(session: &mut Session, input: &mut dyn BufRead, out: &mut dyn Write, interactive: bool) {
    let _ = writeln!(out, "{}", session.banner());
    let mut line = String::new();
    let mut ended = false;
    loop {
        if interactive {
            let _ = write!(out, "\n> ");
            let _ = out.flush();
        }
        line.clear();
        if !matches!(input.read_line(&mut line), Ok(n) if n > 0) {
            end_game(session, input, out, interactive);
            return;
        }
        if interactive {
            let _ = writeln!(out);
        } else {
            let _ = writeln!(out, "\n> {}", line.trim());
        }
        if parse::parse(&line) == Ok(parse::Cmd::Quit) {
            end_game(session, input, out, interactive);
            return;
        }
        let r = session.exec(&line);
        let _ = writeln!(out, "{}", r.text);
        if r.over && !ended {
            ended = true;
            end_game(session, input, out, interactive);
        } else if !r.over {
            ended = false;
        }
    }
}
