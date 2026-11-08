//! The comp library: end boards, early cards, signals, give-up rules and
//! level curves as data (`data/comps/comps.toml`), loaded at start.
//!
//! The planner bot (`crate::planner`) reads these entries; nothing here is a
//! game rule. A card, hero or trinket name is checked against the loaded data
//! when the library is read, so a typo fails at load with the name in it
//! rather than quietly doing nothing in a game.

use serde::Deserialize;

use crate::cards::{CardDb, Tribe, Tribes, tribe};
use crate::minion::{CardId, Keywords, Minion};
use crate::{Data, MAX_TIER};

/// The abort state is one bit per comp, so the library is capped at 64.
pub const MAX_COMPS: usize = 64;

/// The player-wide counter that makes a comp grow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scales {
    /// No player-wide scaling in the progress score.
    #[default]
    None,
    /// The Beetle bonus every summoned Beetle carries (`Player.beetle_bonus`).
    Beetles,
    /// The comp grows through its own minions' improvements.
    Stats,
}

/// One card of the end board.
#[derive(Clone, Debug)]
pub struct CoreCard {
    pub name: String,
    pub id: CardId,
    pub weight: f32,
    /// The card's own growth (improvements, Magnetizations, Golden) is worth
    /// extra progress.
    pub scale: bool,
}

/// What keeps a comp alive before its core is there.
#[derive(Clone, Debug, Default)]
pub struct Early {
    pub cards: Vec<CardId>,
    /// Any minion with stats at least twice its Tier is a body worth having
    /// before turn 8.
    pub strong_stats: bool,
}

/// One way to make a comp ACTIVE. Every group is needed (one card from each
/// group); a group with several cards is an either/or. A comp with several
/// `Enabler` entries needs any one of them.
#[derive(Clone, Debug, Default)]
pub struct Enabler {
    pub all: Vec<Vec<CardId>>,
}

/// When a comp's enabler no longer activates it. The thresholds are one-way:
/// once the turn or Tier has passed with no enabler held, the comp is off for
/// the rest of the game.
#[derive(Clone, Debug, Default)]
pub struct Abort {
    /// From after this turn (`turn > after_turn`) with no enabler held.
    pub after_turn: Option<u32>,
    /// At or above this Tier with no enabler held.
    pub min_tier: Option<u8>,
}

/// One reason to go for a comp.
#[derive(Clone, Debug)]
pub enum SignalKind {
    /// One point per distinct card seen on the board, in hand or in the Tavern.
    Cards(Vec<CardId>),
    /// A tribe on the board or in hand, worth its weight once `min_count`
    /// cards are there.
    Tribe(Tribe),
    /// The player's hero, by `HeroDb` index.
    Hero(u8),
    /// A trinket the player holds, by `TrinketDb` index.
    Trinket(u8),
    /// One point per minion on the board or in hand with the keyword.
    Keyword(u8),
}

#[derive(Clone, Debug)]
pub struct Signal {
    pub kind: SignalKind,
    pub weight: f32,
    pub min_count: u32,
}

/// When to drop a comp and what to play instead.
#[derive(Clone, Debug)]
pub struct GiveUp {
    /// The first turn the rule may fire.
    pub after_turn: u32,
    /// Fire only while the comp progress is below this number.
    pub progress_below: Option<f32>,
    /// The comp to switch to, by name.
    pub to: String,
    /// The resolved index of `to`.
    pub to_index: usize,
}

#[derive(Clone, Debug)]
pub struct Comp {
    pub name: String,
    /// The card tribes the comp plays (for target and Discover preference).
    pub tribes: Vec<Tribe>,
    /// Every tribe the comp needs in the lobby; empty for a comp that works
    /// in any lobby (Menagerie).
    pub lobby: Vec<Tribe>,
    pub draft: bool,
    /// The planner may pick this comp late with nothing else to play.
    pub fallback: bool,
    pub commit_turn: Option<(u32, u32)>,
    /// Tier target for turn 1, 2, ...; None for a comp with no level curve.
    pub level: Option<Vec<u8>>,
    pub scales: Scales,
    pub core: Vec<CoreCard>,
    /// Ways to make the comp ACTIVE; any one of them is enough.
    pub enabler: Vec<Enabler>,
    /// Cards that matter when the comp is active.
    pub payoff: Vec<CardId>,
    /// Cards bought only to feed the comp.
    pub filler: Vec<CardId>,
    pub early: Early,
    pub signals: Vec<Signal>,
    pub abort: Option<Abort>,
    pub give_up: Option<GiveUp>,
    pub not_in_comp: Vec<CardId>,
}

impl Comp {
    /// The core weight of a card, or None when it is not core.
    pub fn core_weight(&self, id: CardId) -> Option<f32> {
        self.core.iter().find(|c| c.id == id).map(|c| c.weight)
    }

    pub fn is_core(&self, id: CardId) -> bool {
        self.core.iter().any(|c| c.id == id)
    }

    pub fn is_not_in_comp(&self, id: CardId) -> bool {
        self.not_in_comp.contains(&id)
    }

    pub fn early_has(&self, id: CardId) -> bool {
        self.early.cards.contains(&id)
    }

    pub fn is_enabler(&self, id: CardId) -> bool {
        self.enabler.iter().any(|e| e.all.iter().any(|g| g.contains(&id)))
    }

    /// Whether any one of the comp's enabler entries is complete: every group
    /// has one of its cards on the board or in the hand.
    pub fn enabler_held(&self, board: &[Minion], hand: &[Minion]) -> bool {
        let held = |id: &CardId| {
            board.iter().chain(hand).any(|m| m.card == *id)
        };
        self.enabler.iter().any(|e| {
            !e.all.is_empty() && e.all.iter().all(|g| g.iter().any(held))
        })
    }

    /// Whether the comp's abort condition holds now: a threshold turn or Tier
    /// is here and no enabler is held. One-way state is kept by the caller
    /// (`OverlayState` and `PlannerState`), because the threshold passing
    /// without an enabler stays true for the rest of the game.
    pub fn abort_condition(&self, turn: u32, tier: u8, board: &[Minion], hand: &[Minion]) -> bool {
        let Some(abort) = self.abort.as_ref() else { return false };
        if self.enabler_held(board, hand) {
            return false;
        }
        abort.after_turn.is_some_and(|t| turn > t)
            || abort.min_tier.is_some_and(|t| tier >= t)
    }

    /// Every tribe in `lobby` is in the lobby (an empty list is always fine).
    pub fn lobby_ok(&self, lobby: &Tribes) -> bool {
        self.lobby.iter().all(|t| lobby.has(*t))
    }

    pub fn is_payoff(&self, id: CardId) -> bool {
        self.payoff.contains(&id)
    }

    pub fn is_filler(&self, id: CardId) -> bool {
        self.filler.contains(&id)
    }

    /// A card that belongs to the comp: enabler, payoff, filler or an old
    /// planner core card. The full-board sell treats all of them as "in the
    /// comp"; only enabler and payoff are actually unsellable.
    pub fn in_comp(&self, id: CardId) -> bool {
        self.is_enabler(id)
            || self.is_payoff(id)
            || self.is_filler(id)
            || self.is_core(id)
    }

    pub fn is_enabler_or_payoff(&self, id: CardId) -> bool {
        self.is_enabler(id) || self.is_payoff(id)
    }

    /// A card that can make the comp active.
    pub fn has_enabler(&self) -> bool {
        !self.enabler.is_empty()
    }

    /// The tier target for a turn: the comp's own curve, else the library
    /// default. `level = []` is a comp with no curve, which returns None.
    pub fn level_at(&self, turn: u32, default: &[u8]) -> Option<u8> {
        let levels = match self.level.as_ref() {
            Some(own) => {
                if own.is_empty() {
                    return None;
                }
                own.as_slice()
            }
            None => default,
        };
        if levels.is_empty() {
            return None;
        }
        let index = (turn.saturating_sub(1) as usize).min(levels.len() - 1);
        Some(levels[index])
    }
}

/// The library defaults a comp inherits. A comp's own `level` overrides the
/// level list; `level = []` means the comp has no level curve at all.
#[derive(Clone, Debug)]
pub struct Defaults {
    /// The tier target for turn 1, 2, ...
    pub level: Vec<u8>,
    /// Health at or below which the planner stops leveling.
    pub dying_health: i32,
    /// The overlay's bonus for any action that costs Health on the player's
    /// own turn while no Soul Rewinder or Ashen Corruptor is on the board.
    pub health_loss_penalty: f32,
}

pub const FALLBACK_LEVEL: [u8; 12] = [1, 2, 2, 2, 3, 4, 4, 5, 5, 5, 5, 6];
pub const FALLBACK_DYING_HEALTH: i32 = 10;
pub const FALLBACK_HEALTH_LOSS_PENALTY: f32 = -10.0;

impl Default for Defaults {
    fn default() -> Defaults {
        Defaults {
            level: FALLBACK_LEVEL.to_vec(),
            dying_health: FALLBACK_DYING_HEALTH,
            health_loss_penalty: FALLBACK_HEALTH_LOSS_PENALTY,
        }
    }
}

/// The loaded comp library.
#[derive(Clone, Debug, Default)]
pub struct Comps {
    pub comps: Vec<Comp>,
    pub defaults: Defaults,
}impl Comps {
    /// Read the library, resolving every name against `data`. The first
    /// unknown name is the error, with the comp and the name in it.
    pub fn from_toml(text: &str, data: &Data) -> Result<Comps, String> {
        let raw: RawFile = toml::from_str(text).map_err(|e| format!("comps: {e}"))?;
        if raw.comp.is_empty() {
            return Err("comps: the file has no [[comp]] entry".to_string());
        }
        if raw.comp.len() > MAX_COMPS {
            return Err(format!("comps: the file has more than {MAX_COMPS} entries"));
        }
        let mut defaults = Defaults::default();
        if let Some(d) = raw.defaults {
            if let Some(level) = d.level {
                if let Some(&t) = level.iter().find(|&&t| t == 0 || t > MAX_TIER) {
                    return Err(format!("defaults: level target {t} is not a tier"));
                }
                if !level.is_empty() {
                    defaults.level = level;
                }
            }
            if let Some(h) = d.dying_health {
                defaults.dying_health = h;
            }
            if let Some(p) = d.health_loss_penalty {
                defaults.health_loss_penalty = p;
            }
        }
        let mut comps: Vec<Comp> = Vec::with_capacity(raw.comp.len());
        for r in raw.comp {
            let name = r.name.clone();
            let fail = |what: String| format!("comp {name:?}: {what}");
            if comps.iter().any(|c| c.name == name) {
                return Err(fail(format!("duplicate comp name {name:?}")));
            }
            if let Some([lo, hi]) = r.commit_turn
                && lo > hi
            {
                return Err(fail(format!("commit_turn {lo}..{hi} is backwards")));
            }
            if let Some(levels) = &r.level
                && let Some(&t) = levels.iter().find(|&&t| t == 0 || t > MAX_TIER)
            {
                return Err(fail(format!("level target {t} is not a tier")));
            }
            let scales = match r.scales.as_deref().unwrap_or("none") {
                "none" => Scales::None,
                "beetles" => Scales::Beetles,
                "stats" => Scales::Stats,
                other => return Err(fail(format!("unknown scales {other:?}"))),
            };
            let mut core = Vec::with_capacity(r.core.len());
            let mut seen = Vec::new();
            for c in r.core {
                let id = card(&data.cards, &c.name)
                    .ok_or_else(|| fail(format!("unknown card {:?}", c.name)))?;
                if seen.contains(&id) {
                    return Err(fail(format!("card {:?} is in core twice", c.name)));
                }
                seen.push(id);
                core.push(CoreCard { name: c.name, id, weight: c.weight, scale: c.scale });
            }
            if core.is_empty() {
                return Err(fail("core is empty".to_string()));
            }
            let enabler = enabler_list(&data.cards, r.enabler).map_err(fail)?;
            let payoff = card_list(&data.cards, &r.payoff).map_err(fail)?;
            let filler = card_list(&data.cards, &r.filler).map_err(fail)?;
            let early = r.early.unwrap_or_default();
            let mut early_cards = Vec::with_capacity(early.cards.len());
            for c in &early.cards {
                early_cards.push(
                    card(&data.cards, c).ok_or_else(|| fail(format!("unknown card {c:?}")))?,
                );
            }
            let mut signals = Vec::with_capacity(r.signals.len());
            for s in r.signals {
                signals.push(
                    signal(s, data)
                        .map_err(fail)?,
                );
            }
            let mut not_in_comp = Vec::with_capacity(r.not_in_comp.len());
            for c in &r.not_in_comp {
                not_in_comp.push(
                    card(&data.cards, c).ok_or_else(|| fail(format!("unknown card {c:?}")))?,
                );
            }
            let mut tribes = Vec::with_capacity(r.tribes.len());
            for t in &r.tribes {
                tribes.push(tribe(t).map_err(fail)?);
            }
            let mut lobby = Vec::with_capacity(r.lobby.len());
            for t in &r.lobby {
                lobby.push(tribe(t).map_err(fail)?);
            }
            let abort = r.abort.map(|a| Abort { after_turn: a.after_turn, min_tier: a.min_tier });
            if let Some(a) = &abort
                && a.after_turn.is_none()
                && a.min_tier.is_none()
            {
                return Err(fail("abort has neither after_turn nor min_tier".to_string()));
            }
            comps.push(Comp {
                name,
                tribes,
                lobby,
                draft: r.draft,
                fallback: r.fallback,
                commit_turn: r.commit_turn.map(|[lo, hi]| (lo, hi)),
                level: r.level,
                scales,
                core,
                enabler,
                payoff,
                filler,
                early: Early { cards: early_cards, strong_stats: early.strong_stats },
                signals,
                abort,
                give_up: r.give_up.map(|g| GiveUp {
                    after_turn: g.after_turn,
                    progress_below: g.progress_below,
                    to: g.to.clone(),
                    to_index: usize::MAX,
                }),
                not_in_comp,
            });
        }
        // Resolve the give-up targets now that every name exists.
        for i in 0..comps.len() {
            let Some(g) = comps[i].give_up.as_ref() else { continue };
            let name = comps[i].name.clone();
            let to = g.to.clone();
            let Some(j) = comps.iter().position(|c| c.name == to) else {
                return Err(format!(
                    "comp {name:?}: give_up.to names unknown comp {to:?}"
                ));
            };
            comps[i].give_up.as_mut().expect("just borrowed").to_index = j;
        }
        Ok(Comps { comps, defaults })
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.comps.iter().position(|c| c.name == name)
    }

    pub fn fallback(&self) -> Option<usize> {
        self.comps.iter().position(|c| c.fallback)
    }
}

fn card(cards: &CardDb, name: &str) -> Option<CardId> {
    cards.by_name(name)
}

fn card_list(cards: &CardDb, names: &[String]) -> Result<Vec<CardId>, String> {
    let mut ids = Vec::with_capacity(names.len());
    for name in names {
        ids.push(card(cards, name).ok_or_else(|| format!("unknown card {name:?}"))?);
    }
    Ok(ids)
}

/// Resolve every enabler entry's groups. An entry with no group, or a group
/// with no card, would be held by everyone, so both fail the load.
fn enabler_list(cards: &CardDb, raw: Vec<RawEnabler>) -> Result<Vec<Enabler>, String> {
    let mut out = Vec::with_capacity(raw.len());
    for e in raw {
        if e.all.is_empty() {
            return Err("an enabler entry needs at least one group".to_string());
        }
        let mut all = Vec::with_capacity(e.all.len());
        for group in e.all {
            if group.is_empty() {
                return Err("an enabler group needs at least one card".to_string());
            }
            let mut ids = Vec::with_capacity(group.len());
            for name in group {
                ids.push(card(cards, &name).ok_or_else(|| format!("unknown card {name:?}"))?);
            }
            all.push(ids);
        }
        out.push(Enabler { all });
    }
    Ok(out)
}

fn signal(raw: RawSignal, data: &Data) -> Result<Signal, String> {
    let kinds = !raw.cards.is_empty() as u8
        + raw.tribe.is_some() as u8
        + raw.hero.is_some() as u8
        + raw.trinket.is_some() as u8
        + raw.keyword.is_some() as u8;
    if kinds != 1 {
        return Err(format!(
            "a signal needs exactly one of cards, tribe, hero, trinket or keyword (it has {kinds})"
        ));
    }
    let kind = if !raw.cards.is_empty() {
        let mut ids = Vec::with_capacity(raw.cards.len());
        for c in &raw.cards {
            ids.push(card(&data.cards, c).ok_or_else(|| format!("unknown card {c:?}"))?);
        }
        SignalKind::Cards(ids)
    } else if let Some(t) = &raw.tribe {
        SignalKind::Tribe(tribe(t)?)
    } else if let Some(h) = &raw.hero {
        let id = data
            .heroes
            .by_name(h)
            .ok_or_else(|| format!("unknown hero {h:?}"))?;
        SignalKind::Hero(id)
    } else if let Some(t) = &raw.trinket {
        let id = data
            .trinkets
            .by_name(t)
            .ok_or_else(|| format!("unknown trinket {t:?}"))?;
        SignalKind::Trinket(id)
    } else {
        let name = raw.keyword.as_deref().unwrap_or("");
        let keyword = Keywords::ALL
            .iter()
            .find(|&&(_, n)| n == name)
            .map(|&(k, _)| k)
            .ok_or_else(|| format!("unknown keyword {name:?}"))?;
        SignalKind::Keyword(keyword)
    };
    Ok(Signal { kind, weight: raw.weight, min_count: raw.min_count.unwrap_or(1).max(1) })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    defaults: Option<RawDefaults>,
    comp: Vec<RawComp>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawDefaults {
    #[serde(default)]
    level: Option<Vec<u8>>,
    #[serde(default)]
    dying_health: Option<i32>,
    #[serde(default)]
    health_loss_penalty: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawComp {
    name: String,
    #[serde(default)]
    tribes: Vec<String>,
    #[serde(default)]
    lobby: Vec<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    fallback: bool,
    #[serde(default)]
    commit_turn: Option<[u32; 2]>,
    #[serde(default)]
    level: Option<Vec<u8>>,
    #[serde(default)]
    scales: Option<String>,
    #[serde(default)]
    core: Vec<RawCore>,
    #[serde(default)]
    enabler: Vec<RawEnabler>,
    #[serde(default)]
    payoff: Vec<String>,
    #[serde(default)]
    filler: Vec<String>,
    #[serde(default)]
    early: Option<RawEarly>,
    #[serde(default)]
    signals: Vec<RawSignal>,
    #[serde(default)]
    abort: Option<RawAbort>,
    #[serde(default)]
    give_up: Option<RawGiveUp>,
    #[serde(default)]
    not_in_comp: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnabler {
    all: Vec<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAbort {
    #[serde(default)]
    after_turn: Option<u32>,
    #[serde(default)]
    min_tier: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCore {
    name: String,
    weight: f32,
    #[serde(default)]
    scale: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawEarly {
    #[serde(default)]
    cards: Vec<String>,
    #[serde(default)]
    strong_stats: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignal {
    #[serde(default)]
    cards: Vec<String>,
    #[serde(default)]
    tribe: Option<String>,
    #[serde(default)]
    hero: Option<String>,
    #[serde(default)]
    trinket: Option<String>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default)]
    min_count: Option<u32>,
    weight: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGiveUp {
    #[serde(default)]
    after_turn: u32,
    #[serde(default)]
    progress_below: Option<f32>,
    to: String,
}
