//! One sandbox game: the engine state, the action history that rebuilds it
//! (for undo), and the recording of every fought turn.

use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;

use bgcore::record::{self, GameRecord, TurnRecord};
use bgcore::{Action, Data, Game, SandboxFight, Tribes};

use crate::parse::{Arg, Cmd, Look, parse};
use crate::screen::{self, CardTexts};

pub struct Config {
    /// The lobby the sandbox plays with (the --tribe tribes); None rolls the
    /// normal 5 of 10.
    pub lobby: Option<Tribes>,
    /// The hero forced with --hero (its name); None offers a pick from the
    /// hero table instead.
    pub hero: Option<String>,
    /// The build played. None until the player names it (at the end, or
    /// with the label command), recorded as "unlabeled" meanwhile.
    pub label: Option<String>,
    pub seed: u64,
    pub free_rolls: u8,
    pub boards_dir: PathBuf,
}

/// What a command printed, and whether the program should exit.
pub struct Reply {
    pub text: String,
    pub quit: bool,
    /// The game is over (hero knocked out or turn limit) after this command.
    pub over: bool,
}

pub const UNLABELED: &str = "unlabeled";

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
    label: String,
    label_given: bool,
    /// The end-of-game label prompt was already shown.
    label_asked: bool,
    /// The file the recording is in right now: a temporary
    /// `game-<seed>.toml` until `finish` renames it after the label.
    current: Option<PathBuf>,
}

const HELP: &str = "\
How to play (numbers are what the screen shows, counting from 1):
  b2        buy Tavern slot 2            bs       buy the spell slot
  p1        play hand card 1 (right end) p1 3     play hand card 1 to board position 3
  c1 4      cast spell in hand 1 on board minion 4, c1 t4 on Tavern minion 4
            (c1 alone opens the target prompt; p and c are the same command)
  s4        sell board minion 4          m2 5     move board minion 2 to position 5
  a3        Activate board minion 3 (a target prompt follows if it needs one)
  1 2 3     answer an open choice: a Discover or Choose One option, a board
            target, the hand card to discard, or a hero on offer; t3 answers
            with Tavern minion 3 (the spell slot is one past the last card)
  r         Refresh (free ones first)    f        freeze / unfreeze the Tavern
  u         upgrade the Tavern           e        end turn
  hp        press your hero power (a target prompt follows if it needs one)
  ? 3       full card text: Tavern 3     ?s spell slot   ?h1 hand 1   ?b2 board 2   ?d1 Discover option 1
  undo      take back the last action (works across turns, even an e)
  label X   set or change the label of this game (the build you play, e.g. lobster)
  quit      save and exit                help     this list
Tags: T taunt, DS divine shield, W windfury, V venomous, R reborn, S stealth, G golden,
M magnetic, A1 Activate ready (cost 1), L2 locked in hand 2 more turns.
A Magnetic minion merges into the Mech at the position you give; to put it on the board
unmerged, play it at the right end (plain p1).
The pool is this game's 5-tribe lobby (with --tribe it is the tribes you named). Every turn
ends with a fight against a copy of your own board: it never hurts your hero, only shop
damage does.
Every end-of-turn board is saved to boards/game-<seed>.toml while you play. At the end (quit,
hero death) you are asked to label the build and the file is renamed to
boards/<label>-<seed>.toml (<tribe>-<label>-<seed> with --tribe). An existing file is never
overwritten, a suffix -2, -3 is added.";

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
        Session::new_with_data(cfg, Arc::new(Data::bundled()))
    }

    /// A session over a shared card set (the binary checks --hero against
    /// the same one before starting).
    pub fn new_with_data(cfg: Config, data: Arc<Data>) -> Session {
        let (game, turns, ctx) = Session::build(&cfg, &data, &[]);
        let label_given = cfg.label.as_ref().is_some_and(|l| !l.trim().is_empty());
        let label = if label_given { cfg.label.clone().unwrap().trim().to_string() } else { UNLABELED.to_string() };
        Session {
            cfg,
            data,
            texts: CardTexts::load(),
            game,
            history: Vec::new(),
            turns,
            ctx,
            label,
            label_given,
            label_asked: false,
            current: None,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Set or change the label (an empty text changes nothing).
    pub fn set_label(&mut self, text: &str) {
        let t = text.trim();
        if !t.is_empty() {
            self.label = t.to_string();
            self.label_given = true;
        }
    }

    /// Whether the end-of-game label question should be asked now.
    pub fn needs_label(&self) -> bool {
        !self.label_given && !self.label_asked && !self.turns.is_empty()
    }

    /// Take the answer to the label question; blank keeps "unlabeled".
    pub fn answer_label(&mut self, text: &str) {
        self.label_asked = true;
        self.set_label(text);
    }

    /// The file name the finished game gets, before any suffix.
    fn final_name(&self) -> String {
        let label = sanitize(&self.label);
        match self.cfg.lobby {
            Some(t) => format!(
                "{}-{label}-{}",
                crate::screen::lobby_names(t).to_lowercase().replace(", ", "-"),
                self.cfg.seed
            ),
            None => format!("{label}-{}", self.cfg.seed),
        }
    }

    /// The file the recording is in now, None before the first fought turn.
    pub fn file_path(&self) -> Option<PathBuf> {
        self.current.clone()
    }

    /// A path in the boards folder for `stem` that no file has yet.
    fn free_path(&self, stem: &str, keep: Option<&PathBuf>) -> PathBuf {
        let mut n = 1;
        loop {
            let name = if n == 1 { format!("{stem}.toml") } else { format!("{stem}-{n}.toml") };
            let p = self.cfg.boards_dir.join(name);
            if !p.exists() || keep == Some(&p) {
                return p;
            }
            n += 1;
        }
    }

    /// Save, then rename the file to its final name from the label. Safe to
    /// call again: with the same label nothing moves.
    pub fn finish(&mut self) -> Result<Option<PathBuf>, String> {
        let Some(path) = self.save()? else { return Ok(None) };
        let target = self.free_path(&self.final_name(), Some(&path));
        if target != path {
            std::fs::rename(&path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
            self.current = Some(target.clone());
            return Ok(Some(target));
        }
        Ok(Some(path))
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
        let mut game = match &cfg.hero {
            Some(name) => {
                let h = data
                    .heroes
                    .by_name(name)
                    .unwrap_or_else(|| panic!("no built hero named {name:?}"));
                Game::new_sandbox_with_hero(data.clone(), cfg.seed, cfg.lobby, cfg.free_rolls, h)
            }
            None => Game::new_sandbox_heroes(data.clone(), cfg.seed, cfg.lobby, cfg.free_rolls),
        };
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
            && p.pending_discard.is_none()
            && p.discover.is_empty()
            && p.power_offer.is_empty()
            && (game.turn != 0 || p.hero_offer.iter().all(|&h| h == u8::MAX))
        {
            *ctx = None;
        }
        Ok(())
    }

    fn record(&self) -> GameRecord {
        GameRecord {
            tribe: self.cfg.lobby.map(crate::screen::lobby_names),
            label: self.label.clone(),
            seed: self.cfg.seed,
            free_rolls: self.cfg.free_rolls,
            date: record::today(),
            turns: self.turns.clone(),
        }
    }

    /// Write the recording from the history. With no fought turn there is
    /// nothing to keep, so a file left by an undone turn is removed.
    pub fn save(&mut self) -> Result<Option<PathBuf>, String> {
        if self.turns.is_empty() {
            if let Some(path) = self.current.take() {
                match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(format!("{}: {e}", path.display()));
                    }
                    _ => {}
                }
            }
            return Ok(None);
        }
        let text = record::to_toml(&self.data.cards, &self.record())?;
        std::fs::create_dir_all(&self.cfg.boards_dir).map_err(|e| format!("{}: {e}", self.cfg.boards_dir.display()))?;
        let path = match &self.current {
            Some(p) => p.clone(),
            None => {
                let p = self.free_path(&format!("game-{}", self.cfg.seed), None);
                self.current = Some(p.clone());
                p
            }
        };
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
        let rig = match self.cfg.lobby {
            Some(t) => format!("{} lobby", crate::screen::lobby_names(t)),
            None => "a rolled 5-tribe lobby".to_string(),
        };
        let label = if self.label_given { format!("label {:?}", self.label) } else { "label asked at the end".to_string() };
        format!(
            "bgsandbox: {rig}, {label}, seed {} (replay with --seed {}), {} free Refreshes a turn.\nRecording to {}/game-{}.toml, renamed after the label at the end.\nType help for the commands.\n\n{}",
            self.cfg.seed,
            self.cfg.seed,
            self.cfg.free_rolls,
            self.cfg.boards_dir.display(),
            self.cfg.seed,
            self.screen()
        )
    }

    /// Run one typed line.
    pub fn exec(&mut self, line: &str) -> Reply {
        let (text, quit) = self.exec_inner(line);
        Reply { text, quit, over: self.game.over }
    }

    fn exec_inner(&mut self, line: &str) -> (String, bool) {
        let say = |text: String| (text, false);
        let cmd = match parse(line) {
            Ok(c) => c,
            Err(e) => return say(e),
        };
        match cmd {
            Cmd::Help => return say(HELP.to_string()),
            Cmd::Info(l) => return say(self.info(l)),
            Cmd::Quit => {
                let text = match self.finish() {
                    Ok(Some(p)) => format!("Saved {} turns to {}", self.turns.len(), p.display()),
                    Ok(None) => "No turn was played to the end, so no file was written.".to_string(),
                    Err(e) => format!("could not save: {e}"),
                };
                return (text, true);
            }
            Cmd::Label(text) => {
                self.set_label(&text);
                let mut out = format!("Label is now {:?}.", self.label);
                if self.current.is_some()
                    && let Err(e) = self.save()
                {
                    out.push_str(&format!(" COULD NOT UPDATE THE FILE: {e}"));
                }
                return say(out);
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
            || pl.pending_discard.is_some()
            || !pl.discover.is_empty()
            || !pl.power_offer.is_empty()
            || (g.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX));
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
            Cmd::Power => one(Action::HeroPower),
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
            Cmd::Help | Cmd::Info(_) | Cmd::Undo | Cmd::Quit | Cmd::Label(_) => unreachable!(),
        }
    }

    fn plan_pick(&self, arg: Arg) -> Result<Vec<Action>, String> {
        let pl = &self.game.players[0];
        let a = if pl.pending_target.is_some() {
            match arg {
                Arg::Board(n) => Action::Target(n.saturating_sub(1).min(255) as u8),
                Arg::Tavern(n) => Action::TargetShop(n.saturating_sub(1).min(255) as u8),
            }
        } else if pl.pending_discard.is_some() {
            match arg {
                Arg::Board(n) => Action::Discard(n.saturating_sub(1).min(255) as u8),
                Arg::Tavern(_) => {
                    return Err("this choice takes a hand card number, not a Tavern slot".into())
                }
            }
        } else if pl.pending_shop_target.is_some() {
            match arg {
                Arg::Board(n) | Arg::Tavern(n) => Action::TargetShop(n.saturating_sub(1).min(255) as u8),
            }
        } else if !pl.trinket_offer.is_empty() {
            // A trinket offer: `choose 1` .. `choose 4`. A Discover a trinket
            // opened at the start of the turn is answered first (is_legal
            // gives it priority), so try that too.
            let n = match arg {
                Arg::Board(n) => n.saturating_sub(1).min(255) as u8,
                Arg::Tavern(_) => return Err("a trinket offer takes a plain number".into()),
            };
            if self.game.is_legal(Action::Choose(n)) {
                Action::Choose(n)
            } else {
                Action::ChooseTrinket(n)
            }
        } else if pl.pending_choose_one.is_some()
            || !pl.discover.is_empty()
            || !pl.power_offer.is_empty()
            || (self.game.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX))
        {
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
            || pl.pending_discard.is_some()
            || !pl.discover.is_empty()
            || !pl.power_offer.is_empty()
            || (g.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX))
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
            Action::HeroPower => {
                let (Some(st), pl) = (pl.power, pl) else {
                    return "you have no hero power".into();
                };
                let power = &g.data.heroes.get(st.of).power;
                if power.passive {
                    format!("{} is passive: it has no button", power.name)
                } else if !st.unlocked {
                    format!("{} unlocks at Tier {}", power.name, power.unlock_tier)
                } else if st.uses_left == 0 {
                    "no uses of the power left this turn".into()
                } else if power.once_per_game && st.used_game > 0 {
                    "the power was already used this game".into()
                } else if pl.gold < power.cost {
                    gold(power.cost)
                } else {
                    "there is nothing for the power to target right now".into()
                }
            }
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
