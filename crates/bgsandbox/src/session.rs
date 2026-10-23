//! One sandbox game: the engine state, the action history that rebuilds it
//! (for undo), and the recording of every fought turn.

use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;

use bgcore::record::{self, GameRecord, TurnRecord};
use bgcore::{Action, Data, Game, SandboxFight, Tribe};

use crate::parse::{Arg, Cmd, Look, parse};
use crate::screen::{self, CardTexts, tribe_name};

pub struct Config {
    pub tribe: Tribe,
    pub label: String,
    pub seed: u64,
    pub free_rolls: u8,
    pub boards_dir: PathBuf,
}

/// What a command printed, and whether the program should exit.
pub struct Reply {
    pub text: String,
    pub quit: bool,
}

pub struct Session {
    cfg: Config,
    data: Arc<Data>,
    texts: CardTexts,
    game: Game,
    /// One entry per command the player typed, each the engine actions it
    /// made. Undo drops the last entry and rebuilds from the seed.
    history: Vec<Vec<Action>>,
    turns: Vec<TurnRecord>,
    /// The card whose effect opened the choice now waiting, for the prompt.
    ctx: Option<String>,
}

const HELP: &str = "\
How to play (numbers are what the screen shows, counting from 1):
  b2        buy Tavern slot 2            bs       buy the spell slot
  p1        play hand card 1 (right end) p1 3     play hand card 1 to board position 3
  c1 4      cast spell in hand 1 on board minion 4, c1 t4 on Tavern minion 4
            (c1 alone opens the target prompt; p and c are the same command)
  s4        sell board minion 4          m2 5     move board minion 2 to position 5
  a3        Activate board minion 3 (a target prompt follows if it needs one)
  1 2 3     answer an open choice: a Discover or Choose One option, or a board
            target; t3 answers with Tavern minion 3
  r         Refresh (free ones first)    f        freeze / unfreeze the Tavern
  u         upgrade the Tavern           e        end turn
  ? 3       full card text: Tavern 3     ?s spell slot   ?h1 hand 1   ?b2 board 2   ?d1 Discover option 1
  undo      take back the last action (works across turns, even an e)
  quit      save and exit                help     this list
Tags: T taunt, DS divine shield, W windfury, V venomous, R reborn, S stealth, G golden,
M magnetic, A1 Activate ready (cost 1), L2 locked in hand 2 more turns.
A Magnetic minion merges into the Mech at the position you give; to put it on the board
unmerged, play it at the right end (plain p1).
The Tavern only shows your tribe and neutral minions. Every turn ends with a fight against
a copy of your own board: it never hurts your hero, only shop damage does.
Every end-of-turn board is saved to boards/<tribe>-<label>-<seed>.toml.";

pub fn help() -> &'static str {
    HELP
}

fn sanitize(label: &str) -> String {
    let s: String = label
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "game".into() } else { s }
}

/// The player's own name for what a hand card is, for choice prompts.
fn hand_name(g: &Game, hand: usize) -> Option<String> {
    g.players[0].hand.get(hand).map(|m| g.data.cards.get(m.card).name.clone())
}

impl Session {
    pub fn new(cfg: Config) -> Session {
        let data = Arc::new(Data::bundled());
        let (game, turns, ctx) = Session::build(&cfg, &data, &[]);
        Session { cfg, data, texts: CardTexts::load(), game, history: Vec::new(), turns, ctx }
    }

    /// The file this game records to.
    pub fn file_path(&self) -> PathBuf {
        self.cfg.boards_dir.join(format!(
            "{}-{}-{}.toml",
            tribe_name(self.cfg.tribe).to_lowercase(),
            sanitize(&self.cfg.label),
            self.cfg.seed
        ))
    }

    pub fn game(&self) -> &Game {
        &self.game
    }

    /// How many commands can still be undone.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn turns(&self) -> &[TurnRecord] {
        &self.turns
    }

    /// Everything that makes up the state, as text, to compare two states.
    pub fn fingerprint(&self) -> String {
        format!(
            "turn {} over {} rng {:?} player {:?} pool {:?} consumed {:?} ctx {:?} turns {}",
            self.game.turn,
            self.game.over,
            self.game.rng,
            self.game.players[0],
            self.game.pool,
            self.game.consumed,
            self.ctx,
            self.turns.len()
        )
    }

    /// The game and its recording after `history`, played from the seed.
    fn build(cfg: &Config, data: &Arc<Data>, history: &[Vec<Action>]) -> (Game, Vec<TurnRecord>, Option<String>) {
        let mut game = Game::new_sandbox(data.clone(), cfg.seed, cfg.tribe, cfg.free_rolls);
        let mut turns = Vec::new();
        let mut ctx = None;
        for cmd in history {
            for &a in cmd {
                Session::step(&mut game, &mut turns, &mut ctx, a)
                    .expect("a recorded action is no longer legal, the engine is not deterministic");
            }
        }
        (game, turns, ctx)
    }

    fn step(game: &mut Game, turns: &mut Vec<TurnRecord>, ctx: &mut Option<String>, a: Action) -> Result<(), String> {
        match a {
            Action::Play { hand, .. } => *ctx = hand_name(game, hand as usize),
            Action::Activate(s) => {
                *ctx = game.players[0].board.get(s as usize).map(|m| game.data.cards.get(m.card).name.clone());
            }
            _ => {}
        }
        game.step(a).map_err(|_| "not legal now".to_string())?;
        if let Some(f) = game.sandbox.as_ref().and_then(|s| s.last_fight.as_ref())
            && turns.last().is_none_or(|t| t.turn != f.turn)
        {
            turns.push(TurnRecord::from_fight(f));
        }
        let p = &game.players[0];
        if p.pending_target.is_none()
            && p.pending_shop_target.is_none()
            && p.pending_choose_one.is_none()
            && p.discover.is_empty()
        {
            *ctx = None;
        }
        Ok(())
    }

    fn record(&self) -> GameRecord {
        GameRecord {
            tribe: tribe_name(self.cfg.tribe).to_string(),
            label: self.cfg.label.clone(),
            seed: self.cfg.seed,
            free_rolls: self.cfg.free_rolls,
            date: record::today(),
            turns: self.turns.clone(),
        }
    }

    /// Write the recording from the history. With no fought turn there is
    /// nothing to keep, so a file left by an undone turn is removed.
    pub fn save(&self) -> Result<Option<PathBuf>, String> {
        let path = self.file_path();
        if self.turns.is_empty() {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(format!("{}: {e}", path.display()));
                }
                _ => return Ok(None),
            }
        }
        let text = record::to_toml(&self.data.cards, &self.record())?;
        std::fs::create_dir_all(&self.cfg.boards_dir).map_err(|e| format!("{}: {e}", self.cfg.boards_dir.display()))?;
        // write beside the file and rename, so a crash never leaves half a game
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Some(path))
    }

    pub fn screen(&self) -> String {
        screen::state(&self.game, &self.texts, self.ctx.as_deref())
    }

    pub fn banner(&self) -> String {
        format!(
            "bgsandbox: {} Tavern, label {:?}, seed {} (replay with --seed {}), {} free Refreshes a turn.\nRecording to {}\nType help for the commands.\n\n{}",
            tribe_name(self.cfg.tribe),
            self.cfg.label,
            self.cfg.seed,
            self.cfg.seed,
            self.cfg.free_rolls,
            self.file_path().display(),
            self.screen()
        )
    }

    /// Run one typed line.
    pub fn exec(&mut self, line: &str) -> Reply {
        let say = |text: String| Reply { text, quit: false };
        let cmd = match parse(line) {
            Ok(c) => c,
            Err(e) => return say(e),
        };
        match cmd {
            Cmd::Help => return say(HELP.to_string()),
            Cmd::Info(l) => return say(self.info(l)),
            Cmd::Quit => {
                let text = match self.save() {
                    Ok(Some(p)) => format!("Saved {} turns to {}", self.turns.len(), p.display()),
                    Ok(None) => "No turn was played to the end, so no file was written.".to_string(),
                    Err(e) => format!("could not save: {e}"),
                };
                return Reply { text, quit: true };
            }
            Cmd::Undo => return say(self.undo()),
            _ => {}
        }
        if self.game.over {
            return say("The game is over. undo takes back the last action, quit saves and exits.".into());
        }
        let acts = match self.plan(cmd) {
            Ok(a) => a,
            Err(e) => return say(e),
        };
        // run on a copy so a failure halfway changes nothing
        let mut game = self.game.clone();
        let mut turns = self.turns.clone();
        let mut ctx = self.ctx.clone();
        for &a in &acts {
            if let Err(e) = Session::step(&mut game, &mut turns, &mut ctx, a) {
                return say(e);
            }
        }
        let fought = turns.len() > self.turns.len();
        self.game = game;
        self.turns = turns;
        self.ctx = ctx;
        self.history.push(acts);
        let mut out = String::new();
        if fought {
            let f = self.game.sandbox.as_ref().and_then(|s| s.last_fight.clone()).unwrap();
            let _ = writeln!(out, "{}", self.fight_line(&f));
            match self.save() {
                Ok(Some(p)) => {
                    let _ = writeln!(out, "Recorded turn {} in {}", f.turn, p.display());
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = writeln!(out, "COULD NOT SAVE: {e}");
                }
            }
            out.push('\n');
        }
        out.push_str(&self.screen());
        say(out)
    }

    fn undo(&mut self) -> String {
        if self.history.pop().is_none() {
            return "Nothing to undo.".into();
        }
        let before = self.turns.len();
        let (game, turns, ctx) = Session::build(&self.cfg, &self.data, &self.history);
        self.game = game;
        self.turns = turns;
        self.ctx = ctx;
        let mut out = String::from("Undone.");
        if self.turns.len() != before {
            match self.save() {
                Ok(_) => out.push_str(" The recording no longer has that turn."),
                Err(e) => {
                    let _ = write!(out, " COULD NOT UPDATE THE FILE: {e}");
                }
            }
        }
        out.push_str("\n\n");
        out.push_str(&self.screen());
        out
    }

    fn info(&self, look: Look) -> String {
        let g = &self.game;
        let pl = &g.players[0];
        let none = |what: &str, n: usize, len: usize| format!("No {what} {n}: it has {len}.");
        match look {
            Look::Tavern(n) => match pl.shop.get(n - 1) {
                Some(m) => screen::card_info(g, &self.texts, m),
                None => none("Tavern slot", n, pl.shop.len()),
            },
            Look::TavernSpell => match pl.shop_spell {
                Some(id) => screen::card_info_by_id(g, &self.texts, id),
                None => "The spell slot is empty.".into(),
            },
            Look::Hand(n) => match pl.hand.get(n - 1) {
                Some(m) => screen::card_info(g, &self.texts, m),
                None => none("hand card", n, pl.hand.len()),
            },
            Look::Board(n) => match pl.board.get(n - 1) {
                Some(m) => screen::card_info(g, &self.texts, m),
                None => none("board minion", n, pl.board.len()),
            },
            Look::Discover(n) => match pl.discover.get(n - 1) {
                Some(&id) => screen::card_info_by_id(g, &self.texts, id),
                None => "No Discover option with that number.".into(),
            },
        }
    }

    /// The engine actions for a command, or why it can't be done now.
    fn plan(&self, cmd: Cmd) -> Result<Vec<Action>, String> {
        let g = &self.game;
        let pl = &g.players[0];
        let open = pl.pending_target.is_some()
            || pl.pending_shop_target.is_some()
            || pl.pending_choose_one.is_some()
            || !pl.discover.is_empty();
        let idx = |n: usize, len: usize, what: &str| -> Result<u8, String> {
            if n >= 1 && n <= len {
                Ok((n - 1) as u8)
            } else if len == 0 {
                Err(format!("you have no {what}"))
            } else {
                Err(format!("no {what} {n}: pick 1 to {len}"))
            }
        };
        let one = |a: Action| -> Result<Vec<Action>, String> {
            if g.is_legal(a) { Ok(vec![a]) } else { Err(self.why_not(a)) }
        };
        match cmd {
            Cmd::Buy(n) => one(Action::Buy(idx(n, pl.shop.len(), "Tavern slot")?)),
            Cmd::BuySpell => one(Action::BuySpell),
            Cmd::Sell(n) => one(Action::Sell(idx(n, pl.board.len(), "board minion")?)),
            Cmd::Activate(n) => one(Action::Activate(idx(n, pl.board.len(), "board minion")?)),
            Cmd::Move(a, b) => {
                let from = idx(a, pl.board.len(), "board minion")?;
                let to = idx(b, pl.board.len(), "board position")?;
                if from == to {
                    return Err("it is already there".into());
                }
                one(Action::Move { from, to })
            }
            Cmd::Roll => one(Action::Roll),
            Cmd::Freeze => one(Action::Freeze),
            Cmd::Upgrade => one(Action::Level),
            Cmd::End => one(Action::EndTurn),
            Cmd::Pick(arg) => self.plan_pick(arg),
            Cmd::Play { hand, arg } => {
                // "c4" while a target prompt is open is the answer 4
                if open && arg.is_none() && (pl.pending_target.is_some() || pl.pending_shop_target.is_some()) {
                    return self.plan_pick(Arg::Board(hand));
                }
                if open {
                    return Err("answer the open choice first (see the CHOICE below the board)".into());
                }
                let h = idx(hand, pl.hand.len(), "hand card")?;
                let card = g.data.cards.get(pl.hand[h as usize].card);
                if card.is_spell() {
                    self.plan_spell(h, arg)
                } else {
                    let pos = match arg {
                        None => pl.board.len(),
                        Some(Arg::Board(n)) if n >= 1 && n <= pl.board.len() + 1 => n - 1,
                        Some(Arg::Board(n)) => {
                            return Err(format!(
                                "board position {n} is out of range: your board has {} minions, pick 1 to {}",
                                pl.board.len(),
                                pl.board.len() + 1
                            ));
                        }
                        Some(Arg::Tavern(_)) => return Err("a minion goes to a board position, not a Tavern slot".into()),
                    };
                    one(Action::Play { hand: h, pos: pos as u8 })
                }
            }
            Cmd::Help | Cmd::Info(_) | Cmd::Undo | Cmd::Quit => unreachable!(),
        }
    }

    fn plan_pick(&self, arg: Arg) -> Result<Vec<Action>, String> {
        let pl = &self.game.players[0];
        let a = if pl.pending_target.is_some() {
            match arg {
                Arg::Board(n) => Action::Target(n.saturating_sub(1).min(255) as u8),
                Arg::Tavern(n) => Action::TargetShop(n.saturating_sub(1).min(255) as u8),
            }
        } else if pl.pending_shop_target.is_some() {
            match arg {
                Arg::Board(n) | Arg::Tavern(n) => Action::TargetShop(n.saturating_sub(1).min(255) as u8),
            }
        } else if pl.pending_choose_one.is_some() || !pl.discover.is_empty() {
            match arg {
                Arg::Board(n) => Action::Choose(n.saturating_sub(1).min(255) as u8),
                Arg::Tavern(_) => return Err("this choice takes a plain number, not a Tavern slot".into()),
            }
        } else {
            return Err("there is no open choice to answer".into());
        };
        if self.game.is_legal(a) { Ok(vec![a]) } else { Err("that is not a valid answer here, see the options below the board".into()) }
    }

    /// Cast a spell, and answer its target prompt in the same command when a
    /// target was given.
    fn plan_spell(&self, h: u8, arg: Option<Arg>) -> Result<Vec<Action>, String> {
        let play = Action::Play { hand: h, pos: 0 };
        if !self.game.is_legal(play) {
            return Err(self.why_not(play));
        }
        let Some(arg) = arg else { return Ok(vec![play]) };
        let mut g = self.game.clone();
        g.step(play).map_err(|_| "not legal now".to_string())?;
        let pl = &g.players[0];
        let a = if pl.pending_target.is_some() {
            match arg {
                Arg::Board(n) => Action::Target(n.saturating_sub(1).min(255) as u8),
                Arg::Tavern(n) => Action::TargetShop(n.saturating_sub(1).min(255) as u8),
            }
        } else if pl.pending_shop_target.is_some() {
            match arg {
                Arg::Board(n) | Arg::Tavern(n) => Action::TargetShop(n.saturating_sub(1).min(255) as u8),
            }
        } else if pl.pending_choose_one.is_some() {
            return Err("this spell asks you to choose first: cast it with c<N> alone, then answer 1 or 2".into());
        } else {
            return Err("that spell takes no target: cast it with c<N> alone".into());
        };
        if !g.is_legal(a) {
            return Err("that is not a valid target for this spell, cast it with c<N> alone to see the targets".into());
        }
        Ok(vec![play, a])
    }

    /// Why an action is not legal now, in words.
    fn why_not(&self, a: Action) -> String {
        let g = &self.game;
        let pl = &g.players[0];
        let r = &g.data.rules;
        let db = &g.data.cards;
        if pl.pending_target.is_some()
            || pl.pending_shop_target.is_some()
            || pl.pending_choose_one.is_some()
            || !pl.discover.is_empty()
        {
            return "answer the open choice first (see the CHOICE below the board)".into();
        }
        let gold = |need: u8| format!("not enough gold: it costs {need}, you have {}", pl.gold);
        match a {
            Action::Buy(s) => {
                let card = db.get(pl.shop[s as usize].card);
                if pl.hand.is_full() {
                    "your hand is full".into()
                } else if card.is_spell() && card.pay_health {
                    "not enough health to pay for it".into()
                } else if card.is_spell() {
                    gold(card.cost.saturating_sub(pl.spell_discount))
                } else {
                    gold(r.buy_cost)
                }
            }
            Action::BuySpell => match pl.shop_spell {
                None => "there is no spell in the spell slot".into(),
                Some(_) if pl.hand.is_full() => "your hand is full".into(),
                Some(id) => {
                    let c = db.get(id);
                    if c.pay_health { "not enough health to pay for it".into() } else { gold(c.cost.saturating_sub(pl.spell_discount)) }
                }
            },
            Action::Roll => format!("not enough gold: a Refresh costs {}, you have {} and no free Refreshes left", r.roll_cost, pl.gold),
            Action::Level => {
                if pl.tier >= bgcore::MAX_TIER {
                    "the Tavern is already at the top tier".into()
                } else {
                    gold(pl.upgrade_cost)
                }
            }
            Action::Play { hand, .. } => {
                let m = &pl.hand[hand as usize];
                let c = db.get(m.card);
                if m.locked_turns > 0 {
                    format!("{} is locked in your hand for {} more turns", c.name, m.locked_turns)
                } else if c.is_spell() {
                    "that spell has no valid target right now".into()
                } else {
                    "your board is full (7 minions)".into()
                }
            }
            Action::Activate(s) => {
                let m = &pl.board[s as usize];
                match m.parts().find_map(|(c, _)| db.get(c).activate) {
                    None => format!("{} has no Activate ability", db.get(m.card).name),
                    Some(_) if m.activated => "its Activate was already used this turn".into(),
                    Some(act) if pl.gold < act.cost => gold(act.cost),
                    Some(_) => "there is nothing for its Activate to target right now".into(),
                }
            }
            _ => "not legal now".into(),
        }
    }

    fn fight_line(&self, f: &SandboxFight) -> String {
        let db = &self.game.data.cards;
        let result = match f.winner {
            Some(0) => "WIN",
            Some(_) => "LOSS",
            None => "TIE",
        };
        let name = |slot: usize| f.board.get(slot).map_or("?".to_string(), |m| db.get(m.card).name.clone());
        let s = &f.side;
        let mut got: Vec<String> = Vec::new();
        if s.improve_gain[0] > 0 {
            got.push(format!("Lobster count +{}", s.improve_gain[0]));
        }
        if s.spell_bonus_gain != (0, 0) {
            got.push(format!("Spell bonus +{}/+{}", s.spell_bonus_gain.0, s.spell_bonus_gain.1));
        }
        if s.beetle_attack_gain != 0 || s.beetle_health_gain != 0 {
            got.push(format!("Beetle bonus +{}/+{}", s.beetle_attack_gain, s.beetle_health_gain));
        }
        for (slot, &(a, h)) in s.permanent_stats.iter().enumerate() {
            if (a != 0 || h != 0) && slot < f.board.len() {
                got.push(format!("{} +{a}/+{h}", name(slot)));
            }
        }
        for (slot, &kw) in s.permanent_keywords.iter().enumerate() {
            if kw != 0 && slot < f.board.len() {
                let names: Vec<&str> = bgcore::Keywords::ALL.iter().filter(|(b, _)| kw & b != 0).map(|(_, n)| *n).collect();
                got.push(format!("{} gains {}", name(slot), names.join(", ")));
            }
        }
        for (slot, &i) in s.permanent_improvements.iter().enumerate() {
            if i > 0 && slot < f.board.len() {
                got.push(format!("{} improvements +{i}", name(slot)));
            }
        }
        for (slot, &(a, h)) in s.hand_stats.iter().enumerate() {
            if (a != 0 || h != 0) && slot < f.hand.len() {
                got.push(format!("hand {} +{a}/+{h}", db.get(f.hand[slot].card).name));
            }
        }
        if s.tavern_buff_gain != (0, 0) {
            got.push(format!("Tavern buff +{}/+{}", s.tavern_buff_gain.0, s.tavern_buff_gain.1));
        }
        if s.n_fodder_refreshes > 0 {
            let q: Vec<String> = s.fodder_refreshes[..s.n_fodder_refreshes as usize].iter().map(|n| n.to_string()).collect();
            got.push(format!("Fodder queue +[{}]", q.join(",")));
        }
        if s.deathrattles_triggered > 0 {
            got.push(format!("{} Deathrattles triggered", s.deathrattles_triggered));
        }
        if !s.cards_gained().is_empty() {
            let names: Vec<String> = s.cards_gained().iter().map(|&c| db.get(c).name.clone()).collect();
            got.push(format!("got {}", names.join(", ")));
        }
        let carried = if got.is_empty() { "nothing".to_string() } else { got.join(", ") };
        format!(
            "Turn {} mirror fight: {result} (board of {}). Carried over: {carried}.",
            f.turn,
            f.board.len()
        )
    }
}
