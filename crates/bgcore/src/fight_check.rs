//! The fight check: play Era's real fights many times in the engine and say
//! which real results the engine finds unlikely (docs/real-boards.md "The
//! fight check"). It finds bugs; it fixes nothing.
//!
//! A fight is one of Era's boards and the opponent's board on the same turn of
//! the same game (`pair_fights`). `prepare` seats both sides exactly as they
//! went in, through the same `RealBoard::build_one` and `Player` a real seat
//! uses, with the hero power and the trinkets the file carries; what the
//! engine does not build is listed and the fight is `incomplete`.
//! `check_fight` then plays it `runs` times and counts who won and how much
//! damage the loser took. The rendering below is pure text: `bgbench
//! fightcheck` writes the three files.

use arrayvec::ArrayVec;

use crate::cards::Tribes;
use crate::fight::{Counters, Outcome, fight_with_hands};
use crate::game::Player;
use crate::heroes::PowerEffect;
use crate::minion::Minion;
use crate::real_boards::{
    RealBoard, RealBoardSpec, RealFightSpec, RealPowerSpec, RealResult, RealTrinketSpec, STAND_IN,
    is_other_build,
};
use crate::rng::Rng;
use crate::trinkets::{FightAct, Trinket};
use crate::{BOARD, Data, HAND, PLAYERS};

/// The cut a complete fight is flagged at when nothing else is asked for: our
/// engine gives the real result less than this percent of the time.
pub const FLAG_BELOW: f64 = 2.0;

/// The fight check's settings: how many times each fight is played, the flag
/// cut, and the seed the runs start from (each fight gets its own, so a run is
/// repeatable and two fights never share a draw).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckOptions {
    pub runs: u32,
    pub flag_below: f64,
    pub seed: u64,
    /// The card data's `game_build`. A fight from a file whose `build` differs
    /// is `old-patch` and not judged.
    pub game_build: Option<i64>,
    /// Judge every build too (`bgbench fightcheck --all-builds`).
    pub all_builds: bool,
}

impl Default for CheckOptions {
    fn default() -> CheckOptions {
        CheckOptions {
            runs: 1000,
            flag_below: FLAG_BELOW,
            seed: 0x7265_616c_6669_6768,
            game_build: None,
            all_builds: false,
        }
    }
}

/// One side of a real fight, ready to fight: everything `fight_with_hands`
/// reads off a side.
#[derive(Clone, Debug, Default)]
pub struct PreparedSide {
    pub board: ArrayVec<Minion, BOARD>,
    pub hand: ArrayVec<Minion, HAND>,
    pub counters: Counters,
}

/// A real fight seated in the engine, before it is played. Built once and
/// fought `runs` times: `fight_with_hands` copies both sides, so one
/// preparation is enough for every run.
#[derive(Clone, Debug, Default)]
pub struct PreparedFight {
    /// Era's side is 0 (the file's `me`), the opponent's is 1.
    pub sides: [PreparedSide; 2],
    /// What the engine does not build for each side, in the order found. A
    /// fight with any of these is incomplete and is not flagged.
    pub missing: [Vec<String>; 2],
}

/// The kind of a missing piece, without the card's name: `minion not built`,
/// `hero power not built`, and so on. The tally in report.md counts by this.
pub fn missing_kind(line: &str) -> &str {
    line.split(':').next().unwrap_or(line)
}

impl PreparedFight {
    pub fn is_complete(&self) -> bool {
        self.missing[0].is_empty() && self.missing[1].is_empty()
    }

    /// Every missing piece of both sides, one line each.
    pub fn missing_lines(&self) -> Vec<String> {
        self.missing[0].iter().chain(&self.missing[1]).cloned().collect()
    }
}

/// The numbers a hero power's text carries that a fight reads. One power's
/// fight effect scales with a number the log shows: Onyxia's Broodmother,
/// whose {0} is the bonus its whelps get, which the fight reads off the
/// power's `progress`. Every other fight power's magnitudes are fixed in
/// heroes.toml (`SocDamageEnemies`, `SocDoubleLeftAttack`, the keyword
/// grants).
fn power_progress(effect: PowerEffect, numbers: &[i32]) -> u32 {
    match effect {
        PowerEffect::Broodmother { .. } => numbers.first().copied().unwrap_or(0).max(0) as u32,
        _ => 0,
    }
}

/// Ozumat's Tentacle, from the numbers the power's file entry shows. The
/// number is the Tentacle's Attack as it stands (checked against the Tentacle
/// the logged fights summon: 2 gives a 2/2, 3 a 3/3), and the engine's
/// Tentacle is the card's own stats plus one more of each for every minion
/// sold this game (`Counters.minions_sold`). So what to seat is the printed
/// Attack minus the card's printed Attack. None when the file shows no number.
fn tentacle_sold(cards: &crate::CardDb, card: crate::CardId, numbers: &[i32]) -> Option<u32> {
    let printed = numbers.first().copied()?;
    let base = cards.make(card, false).attack;
    Some((printed - base).max(0) as u32)
}

/// The kind a Secret card id stands for, in `crate::secrets::CARD_IDS` order.
fn secret_kind(card_id: &str) -> Option<u8> {
    crate::secrets::CARD_IDS.iter().position(|c| *c == card_id).map(|k| k as u8)
}

/// A trinket's "improve this" number from the numbers its file shows: a
/// fight effect that grows its stats (Fang Anklet, All-Purpose Kibble) prints
/// the stats it gives now, so the count is (printed - base) / step. Every
/// other trinket's fight effect is fixed in trinkets.toml and reads nothing.
fn trinket_value(t: &Trinket, numbers: &[i32]) -> u8 {
    for e in &t.fight {
        if let FightAct::Stats { attack, grow, .. } = e.act
            && grow != 0
        {
            let now = numbers.first().copied().unwrap_or(attack);
            return ((now - attack) / grow).clamp(0, u8::MAX as i32) as u8;
        }
    }
    0
}

/// How a side's hero power is seated, for the two checks that load a real
/// state: the fight check plays a power whose fight effect only runs when it
/// was pressed as pressed, and a card that lives on the player's state (the
/// hero power's own numbers) comes from the file. The shop replay reads the
/// log's uses left on the power instead, and presses nothing.
pub struct SeatMode {
    /// The turn the state belongs to (a power unlocks by turn and tier).
    pub turn: u32,
    /// The presses the log says are left this turn, when it says.
    pub uses_left: Option<u8>,
    /// Whether a power that only fires when pressed counts as pressed.
    pub armed: bool,
}

impl SeatMode {
    /// The fight check's: the power is played as pressed.
    pub fn fight(turn: u32) -> SeatMode {
        SeatMode { turn, uses_left: None, armed: true }
    }
}

/// Give the player the hero power the file names, with its numbers, the
/// minion it still owes the board and Ozumat's Tentacle size. What the engine
/// does not build comes back as a line saying so.
pub fn seat_power(
    data: &Data,
    pl: &mut Player,
    spec_power: &RealPowerSpec,
    secrets: Option<&Vec<String>>,
    mode: &SeatMode,
) -> Vec<String> {
    let mut missing = Vec::new();
    let Some(i) = data.heroes.power_by_card_id(&spec_power.card_id) else {
        missing.push(format!(
            "hero power not built: {} ({})",
            spec_power.name, spec_power.card_id
        ));
        return missing;
    };
    let effect = data.heroes.power(i).effect;
    let progress = power_progress(effect, &spec_power.numbers);
    match mode.uses_left {
        Some(left) => pl.hold_power_uses(data, i, progress, mode.turn, left, mode.armed),
        None => pl.hold_power(data, i, progress, mode.turn),
    }
    // A Secret is in the player's Secret zone, not on a board, and the power
    // that fills it is the only reason a check needs the zone. A file without
    // the key cannot say what was up; an empty list says there was nothing.
    if effect == PowerEffect::DiscoverSecret && secrets.is_none() {
        missing.push("Secrets not in the file: Prestidigitation (the Secret zone)".to_string());
    }
    // Ozumat's Tentacles carry the game's sales in the power's own number, and
    // the fight (and the shop's sale) adds them to the Tentacle it summons.
    if let PowerEffect::Tentacular { card } = effect {
        match tentacle_sold(&data.cards, card, &spec_power.numbers) {
            Some(sold) => pl.counts.minions_sold = sold,
            None => missing.push(
                "the hero power's numbers not in the file: Ozumat's Tentacle size".to_string(),
            ),
        }
    }
    // Lock and Load's minion comes into the fight as soon as there is space,
    // and attacks at once; in the shop phase the power keeps it until then.
    if effect == PowerEffect::LockAndLoad {
        match &spec_power.waiting {
            Some(waiting_spec) => {
                let (m, _) = RealBoard::build_minion(
                    waiting_spec,
                    mode.turn,
                    &data.cards,
                    &data.gifts,
                );
                pl.locked_and_loaded = Some(m);
            }
            None if !spec_power.waiting_known => missing.push(
                "the minion locked in hand not in the file: Tavish's Lock and Load adds nothing"
                    .to_string(),
            ),
            // Known and empty: the power owes nothing, so it adds nothing,
            // exactly as in the game.
            None => {}
        }
    }
    missing
}

/// The Secret zone: the engine reads it off the player, in the order the
/// Secrets were got, which is the order they fire in. A file without the key
/// says nothing and adds nothing here (`seat_power` reports it).
pub fn seat_secrets(pl: &mut Player, secrets: Option<&Vec<String>>) -> Vec<String> {
    let mut missing = Vec::new();
    let Some(secret_cards) = secrets else { return missing };
    for card_id in secret_cards {
        match secret_kind(card_id) {
            Some(kind) => {
                pl.secrets.add(kind);
            }
            None => missing.push(format!("Secret not modelled: {card_id}")),
        }
    }
    missing
}

/// The trinkets a side's file carries, in the slots it marks. The engine has
/// three: lesser, greater, and the slot a hero power turned into a trinket
/// fills. A board with two lesser trinkets (a game that handed out an extra
/// one) puts the second in that third slot, which no fight effect can tell
/// apart.
pub fn seat_trinkets(data: &Data, pl: &mut Player, trinkets: &[RealTrinketSpec]) -> Vec<String> {
    let mut missing = Vec::new();
    let mut slot = 0usize;
    for spec_trinket in trinkets {
        let want = if slot == 0 && !spec_trinket.greater {
            0
        } else if slot <= 1 && spec_trinket.greater {
            1
        } else {
            2
        };
        slot = want + 1;
        match data.trinkets.by_card_id(&spec_trinket.card_id) {
            Some(i) => {
                pl.hold_trinket(
                    data,
                    want,
                    i,
                    trinket_value(data.trinkets.get(i), &spec_trinket.numbers),
                );
                // A trinket that reads the player's type at the pick (a
                // Compass) writes the type it locked in its own first number
                // as the game's race code (43 for a Quilboar: the tag the card
                // export calls `RACES`), which is the only trace of a pick the
                // loader never ran. A number that names no tribe leaves it
                // unread, as before.
                let locks = data
                    .trinkets
                    .get(i)
                    .on_pick
                    .iter()
                    .any(|a| matches!(a, crate::trinkets::Act::LockType));
                if locks {
                    pl.trinkets[want].tribe =
                        race_code_tribe(spec_trinket.numbers.first().copied());
                }
            }
            None => missing.push(format!(
                "trinket not built: {} ({})",
                spec_trinket.name, spec_trinket.card_id
            )),
        }
    }
    missing
}

/// The minion type a Compass locked in, from the game's race code: the numbers
/// `ref/carddefs/CardDefs.xml` writes as `CARDRACE` and `tools/export_cards.py`
/// names (20 a Beast, 43 a Quilboar). A code with no tribe in this card set
/// (`ALL`, or one the engine has no name for) reads as none.
fn race_code_tribe(code: Option<i32>) -> Option<crate::cards::Tribe> {
    use crate::cards::Tribe;
    match code? {
        11 => Some(Tribe::Undead),
        14 => Some(Tribe::Murloc),
        15 => Some(Tribe::Demon),
        17 => Some(Tribe::Mech),
        18 => Some(Tribe::Elemental),
        20 => Some(Tribe::Beast),
        23 => Some(Tribe::Pirate),
        24 => Some(Tribe::Dragon),
        43 => Some(Tribe::Quilboar),
        126 => Some(Tribe::Aberration),
        _ => None,
    }
}

/// Seat one side of a real fight: `RealBoard::build_one` makes the board, a
/// `Player` takes it as a real seat does, and this adds the hero power, the
/// trinkets and the quest rewards, which need the hero and trinket tables.
/// Whatever the engine does not build goes into `missing`.
fn prepare_side(data: &Data, spec: &RealBoardSpec, side: usize, p: &mut PreparedFight) {
    let (built, unmatched) = RealBoard::build_one(spec, &data.cards, &data.gifts);
    let mut pl = Player::from_real_board(data, &built);
    // A minion the card set does not build keeps its stats, keywords and
    // numbers as a Stand-in (Era's rule for the loader), so the fight is
    // played with the right bodies and the wrong text.
    for name in &unmatched {
        p.missing[side].push(format!("minion not built: {name} (a Stand-in with its stats)"));
    }
    let stand_ins = spec
        .minions
        .iter()
        .chain(&spec.hand)
        .filter(|m| m.name == STAND_IN)
        .count();
    if stand_ins > 0 {
        p.missing[side].push(format!(
            "minion not built: {stand_ins} card id the ref card file does not know (Stand-ins)"
        ));
    }
    // The Dark Gifts the loader did not find: it leaves them off silently, so
    // the check says so.
    for m in spec.minions.iter().chain(&spec.hand) {
        for gift in &m.dark_gifts {
            if !data.gifts.gifts.iter().any(|g| &g.id == gift) {
                p.missing[side].push(format!("Dark Gift not built: {gift}"));
            }
        }
    }
    if let Some(spec_power) = &spec.power {
        let hit = seat_power(data, &mut pl, spec_power, spec.secrets.as_ref(), &SeatMode::fight(spec.turn));
        p.missing[side].extend(hit);
    }
    p.missing[side].extend(seat_secrets(&mut pl, spec.secrets.as_ref()));
    p.missing[side].extend(seat_trinkets(data, &mut pl, &spec.trinkets));
    for reward in &spec.rewards {
        p.missing[side].push(format!(
            "quest reward not modelled: {} ({})",
            reward.name, reward.card_id
        ));
    }
    if spec.hand_hidden > 0 {
        p.missing[side].push(format!(
            "{} card(s) in hand the log does not name",
            spec.hand_hidden
        ));
    }
    // The lobby a real game had is not in the file: a card that draws a random
    // card inside the fight (Ghastcoiler, a random spell) may draw one that
    // game did not have. `Tribes::ALL` is the fight's "unknown lobby".
    let mut counters = pl.counters(data, Tribes::ALL);
    counters.waits = spec.waits.min(u8::MAX as u32) as u8;
    p.sides[side] = PreparedSide {
        board: pl.board.clone(),
        hand: pl.hand.clone(),
        counters,
    };
}

/// Seat both sides of a real fight in the engine.
pub fn prepare(data: &Data, fight: &RealFightSpec) -> PreparedFight {
    let mut p = PreparedFight {
        sides: [
            PreparedSide {
                board: ArrayVec::new(),
                hand: ArrayVec::new(),
                counters: Counters::default(),
            },
            PreparedSide {
                board: ArrayVec::new(),
                hand: ArrayVec::new(),
                counters: Counters::default(),
            },
        ],
        missing: [Vec::new(), Vec::new()],
    };
    prepare_side(data, &fight.me, 0, &mut p);
    prepare_side(data, &fight.opp, 1, &mut p);
    p
}

/// The damage the losing hero takes from a fight: the winner's Tavern tier
/// plus its surviving minions' tiers, capped by the rules of the turn (the
/// same sum `Game::combat` applies). `winner_tier` is the side that won's.
fn damage_taken(data: &Data, turn: u32, winner_tier: u8, out: &Outcome) -> i32 {
    let raw = winner_tier as i32 + out.survivor_tiers;
    data.rules.damage_cap(turn, PLAYERS).map_or(raw, |c| raw.min(c))
}

/// What the engine made of one real fight over `runs` plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FightCheck {
    pub runs: u32,
    /// How the fight ended for Era's side.
    pub wins: u32,
    pub ties: u32,
    pub losses: u32,
    /// The damage Era's hero took in each run it lost (the p10, median and
    /// p90 of our spread; the mean is 0 for a fight Era never loses).
    pub damage_mean: f64,
    pub damage_p10: i32,
    pub damage_p50: i32,
    pub damage_p90: i32,
    /// Runs the fight reached `rules.max_attacks` without a side emptying:
    /// the winner there is arbitrary, so a tie (or a narrow result) may be the
    /// attack cap and not the boards.
    pub capped: u32,
    pub complete: bool,
    /// The game was played on another client build than the card data models:
    /// the fight is not judged.
    pub old_patch: bool,
    pub missing: Vec<String>,
}

impl FightCheck {
    pub fn win_pct(&self) -> f64 {
        100.0 * self.wins as f64 / self.runs.max(1) as f64
    }

    pub fn tie_pct(&self) -> f64 {
        100.0 * self.ties as f64 / self.runs.max(1) as f64
    }

    pub fn loss_pct(&self) -> f64 {
        100.0 * self.losses as f64 / self.runs.max(1) as f64
    }

    /// The chance the engine gives the result the game recorded, in percent.
    pub fn real_result_pct(&self, result: RealResult) -> Option<f64> {
        let n = match result {
            RealResult::Win => self.wins,
            RealResult::Lose => self.losses,
            RealResult::Tie => self.ties,
            RealResult::Unknown => return None,
        };
        Some(100.0 * n as f64 / self.runs.max(1) as f64)
    }

    /// A complete fight with a real result the engine barely ever produces.
    pub fn flagged(&self, result: RealResult, below: f64) -> bool {
        !self.old_patch
            && self.complete
            && self.real_result_pct(result).is_some_and(|p| p < below)
    }
}

/// One paired fight and what the engine did with it.
#[derive(Clone, Debug)]
pub struct FightEntry {
    pub fight: RealFightSpec,
    pub prepared: PreparedFight,
    pub check: FightCheck,
}

impl FightEntry {
    /// A fight from another client build: not prepared, not played, not judged.
    pub fn old_patch(fight: RealFightSpec) -> FightEntry {
        FightEntry {
            fight,
            prepared: PreparedFight::default(),
            check: FightCheck { old_patch: true, ..FightCheck::default() },
        }
    }
}

/// Play one real fight `runs` times on one core, when the build is not judged
/// against a card data build. `seed` seeds the runs: the same fight and seed
/// give the same numbers.
pub fn check_fight(data: &Data, fight: &RealFightSpec, runs: u32, seed: u64) -> FightEntry {
    let opts = CheckOptions { runs, ..CheckOptions::default() };
    check_fight_with(data, fight, &opts, seed)
}

/// The same with the build check: a fight from another build than the card
/// data's `game_build` is `old-patch`, not played, unless `all_builds` is set.
pub fn check_fight_with(
    data: &Data,
    fight: &RealFightSpec,
    opts: &CheckOptions,
    seed: u64,
) -> FightEntry {
    if !opts.all_builds && is_other_build(fight.build(), opts.game_build) {
        return FightEntry::old_patch(fight.clone());
    }
    let runs = opts.runs;
    let prepared = prepare(data, fight);
    let [me, opp] = &prepared.sides;
    let mut rng = Rng::new(seed);
    let mut check = FightCheck {
        runs,
        complete: prepared.is_complete(),
        missing: prepared.missing_lines(),
        ..FightCheck::default()
    };
    let mut damage: Vec<i32> = Vec::new();
    for _ in 0..runs {
        let out = fight_with_hands(
            &data.cards,
            &me.board,
            &opp.board,
            [&me.hand, &opp.hand],
            [me.counters, opp.counters],
            data.rules.max_attacks,
            &mut rng,
            &mut (),
        );
        if out.attacks >= data.rules.max_attacks {
            check.capped += 1;
        }
        match out.winner {
            Some(0) => check.wins += 1,
            None => check.ties += 1,
            Some(_) => {
                check.losses += 1;
                damage.push(damage_taken(data, fight.turn(), opp.counters.tier, &out));
            }
        }
    }
    if !damage.is_empty() {
        damage.sort_unstable();
        let n = damage.len();
        check.damage_mean = damage.iter().sum::<i32>() as f64 / n as f64;
        let at = |q: f64| damage[(((n as f64 - 1.0) * q).round() as usize).min(n - 1)];
        check.damage_p10 = at(0.1);
        check.damage_p50 = at(0.5);
        check.damage_p90 = at(0.9);
    }
    FightEntry { fight: fight.clone(), prepared, check }
}

/// Play every fight once each, one after another, with seeds counted on from
/// `opts.seed`. `bgbench fightcheck` runs the same work on every core.
pub fn check_all(data: &Data, fights: &[RealFightSpec], opts: &CheckOptions) -> CheckReport {
    let entries = fights
        .iter()
        .enumerate()
        .map(|(i, f)| check_fight_with(data, f, opts, opts.seed.wrapping_add(i as u64)))
        .collect();
    CheckReport::new(data, entries, *opts)
}

// ---- what the files share: the tallies behind report.md and cards.tsv ----

/// One kind of thing a fight can contain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PieceKind {
    Minion,
    Power,
    Trinket,
    Gift,
    Reward,
}

impl PieceKind {
    pub fn name(self) -> &'static str {
        match self {
            PieceKind::Minion => "minion",
            PieceKind::Power => "hero power",
            PieceKind::Trinket => "trinket",
            PieceKind::Gift => "dark gift",
            PieceKind::Reward => "quest reward",
        }
    }
}

/// How one card (or power, trinket or gift) did across the fights it was in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PieceTally {
    pub kind: Option<PieceKind>,
    pub name: String,
    /// The fights it was in.
    pub fights: u32,
    /// Of those, the ones the engine could be played exactly (not incomplete).
    pub complete: u32,
    /// Of those, the ones where the real result was unlikely.
    pub flagged: u32,
}

impl PieceTally {
    /// Flagged out of the complete fights it was in, in percent. 0 for a card
    /// that only ever sat in incomplete fights: it was never judged.
    pub fn ratio(&self) -> f64 {
        if self.complete == 0 { 0.0 } else { 100.0 * self.flagged as f64 / self.complete as f64 }
    }
}

/// One predicted-win-percent bucket of the calibration table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CalibrationBucket {
    /// The bucket covers `low` to `low + 10` percent.
    pub low: u32,
    pub fights: u32,
    /// How many of them the game recorded as a win for Era.
    pub real_wins: u32,
    /// Our own mean win percent over the bucket.
    pub our_win: f64,
}

/// Thousands of fights, added up: the three output files are written from
/// this and nothing else.
#[derive(Clone, Debug)]
pub struct CheckReport {
    pub options: CheckOptions,
    pub entries: Vec<FightEntry>,
    /// Boards with no partner board (see `pair_fights`).
    pub unpaired: Vec<String>,
    /// Minions whose card id no source knew, by the name their file shows.
    pub unknown_minions: Vec<String>,
    /// One row per card, power, trinket, gift and reward, most suspicious
    /// first.
    pub pieces: Vec<PieceTally>,
    pub calibration: Vec<CalibrationBucket>,
}

impl CheckReport {
    pub fn new(data: &Data, entries: Vec<FightEntry>, options: CheckOptions) -> CheckReport {
        let mut report = CheckReport {
            options,
            entries,
            unpaired: Vec::new(),
            unknown_minions: Vec::new(),
            pieces: Vec::new(),
            calibration: Vec::new(),
        };
        report.tally_pieces(data);
        report.calibrate();
        report
    }

    pub fn complete(&self) -> usize {
        self.entries.iter().filter(|e| !e.check.old_patch && e.check.complete).count()
    }

    pub fn old_patch(&self) -> usize {
        self.entries.iter().filter(|e| e.check.old_patch).count()
    }

    pub fn incomplete(&self) -> usize {
        self.entries.iter().filter(|e| !e.check.old_patch && !e.check.complete).count()
    }

    pub fn flagged(&self) -> usize {
        self.entries.iter().filter(|e| e.check.flagged(e.fight.result(), self.options.flag_below)).count()
    }

    /// The old-patch fights by their game's build (`0` for a session that had
    /// no `BuildNumber` line).
    pub fn old_patch_builds(&self) -> Vec<(i64, usize)> {
        let mut out: Vec<(i64, usize)> = Vec::new();
        for e in &self.entries {
            if !e.check.old_patch {
                continue;
            }
            let build = e.fight.build().unwrap_or(0);
            match out.iter_mut().find(|(b, _)| *b == build) {
                Some((_, n)) => *n += 1,
                None => out.push((build, 1)),
            }
        }
        out.sort_by_key(|(build, _)| *build);
        out
    }

    /// Every name a fight holds, once each, with what kind of thing it is.
    fn fight_pieces(data: &Data, e: &FightEntry) -> Vec<(PieceKind, String)> {
        let mut out: Vec<(PieceKind, String)> = Vec::new();
        let mut add = |kind: PieceKind, name: String| {
            if !out.iter().any(|(k, n)| *k == kind && *n == name) {
                out.push((kind, name));
            }
        };
        for side in 0..2 {
            let spec = if side == 0 { &e.fight.me } else { &e.fight.opp };
            for m in &e.prepared.sides[side].board {
                add(PieceKind::Minion, data.cards.get(m.card).name.clone());
            }
            for m in &e.prepared.sides[side].hand {
                add(PieceKind::Minion, data.cards.get(m.card).name.clone());
            }
            for m in spec.minions.iter().chain(&spec.hand) {
                for gift in &m.dark_gifts {
                    let name = data
                        .gifts
                        .gifts
                        .iter()
                        .find(|g| &g.id == gift)
                        .map_or_else(|| gift.clone(), |g| g.name.clone());
                    add(PieceKind::Gift, name);
                }
            }
            if let Some(p) = &spec.power {
                add(PieceKind::Power, p.name.clone());
            }
            for t in &spec.trinkets {
                add(PieceKind::Trinket, t.name.clone());
            }
            for r in &spec.rewards {
                add(PieceKind::Reward, r.name.clone());
            }
        }
        out
    }

    fn tally_pieces(&mut self, data: &Data) {
        let below = self.options.flag_below;
        let mut tallies: Vec<PieceTally> = Vec::new();
        for e in &self.entries {
            if e.check.old_patch {
                continue;
            }
            let flagged = e.check.flagged(e.fight.result(), below);
            for (kind, name) in Self::fight_pieces(data, e) {
                let i = match tallies.iter().position(|t| t.kind == Some(kind) && t.name == name) {
                    Some(i) => i,
                    None => {
                        tallies.push(PieceTally {
                            kind: Some(kind),
                            name,
                            ..PieceTally::default()
                        });
                        tallies.len() - 1
                    }
                };
                tallies[i].fights += 1;
                if e.check.complete {
                    tallies[i].complete += 1;
                    if flagged {
                        tallies[i].flagged += 1;
                    }
                }
            }
        }
        // Most suspicious first: the highest share of flagged fights, then
        // the most flags, then the most fights.
        tallies.sort_by(|a, b| {
            b.ratio()
                .partial_cmp(&a.ratio())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.flagged.cmp(&a.flagged))
                .then(b.complete.cmp(&a.complete))
                .then(a.name.cmp(&b.name))
        });
        self.pieces = tallies;
    }

    fn calibrate(&mut self) {
        let mut buckets: Vec<CalibrationBucket> =
            (0..10).map(|i| CalibrationBucket { low: i * 10, ..CalibrationBucket::default() }).collect();
        for e in &self.entries {
            if !e.check.complete || e.fight.result() == RealResult::Unknown {
                continue;
            }
            let w = e.check.win_pct();
            let i = ((w / 10.0) as usize).min(9);
            buckets[i].fights += 1;
            buckets[i].our_win += w;
            if e.fight.result() == RealResult::Win {
                buckets[i].real_wins += 1;
            }
        }
        for b in &mut buckets {
            if b.fights > 0 {
                b.our_win /= b.fights as f64;
            }
        }
        self.calibration = buckets;
    }

    /// The most surprising complete fights: the smallest chance for the real
    /// result, the worst first. Only fights with a recorded result and a
    /// chance below the flag cut are "surprising", but the list is filled to
    /// `n` anyway so a report with few flags still shows its worst fights.
    pub fn surprise_fights(&self, n: usize) -> Vec<&FightEntry> {
        let mut v: Vec<&FightEntry> = self
            .entries
            .iter()
            .filter(|e| e.check.complete && e.check.real_result_pct(e.fight.result()).is_some())
            .collect();
        v.sort_by(|a, b| {
            let ra = a.check.real_result_pct(a.fight.result()).unwrap_or(100.0);
            let rb = b.check.real_result_pct(b.fight.result()).unwrap_or(100.0);
            ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal).then(
                a.fight.file().cmp(&b.fight.file()).then(a.fight.turn().cmp(&b.fight.turn())),
            )
        });
        v.truncate(n);
        v
    }
}

// ---- the three files (`bgbench fightcheck` writes them) ----

/// A result as the report writes it.
fn result_name(r: RealResult) -> &'static str {
    match r {
        RealResult::Win => "win",
        RealResult::Lose => "lose",
        RealResult::Tie => "tie",
        RealResult::Unknown => "unknown",
    }
}

fn keywords_text(m: &Minion) -> String {
    let mut out = Vec::new();
    for (bit, name) in crate::minion::Keywords::ALL {
        if m.keywords.has(bit) {
            out.push(name.to_string());
        }
    }
    out.join(" ")
}

/// One minion as a report line: its stats, golden flag, keywords, its Dark
/// Gifts and the scaling count the engine keeps for it.
fn minion_text(data: &Data, m: &Minion) -> String {
    let mut s = format!("{}/{} {}", m.attack, m.health, data.cards.get(m.card).name);
    if m.golden {
        s.push_str(" (golden)");
    }
    let kw = keywords_text(m);
    if !kw.is_empty() {
        s.push_str(&format!(" [{kw}]"));
    }
    if m.n_gifts > 0 {
        let gifts: Vec<String> = m
            .gift_slice()
            .iter()
            .map(|g| {
                data.cards
                    .get(g.card())
                    .name
                    .clone()
            })
            .collect();
        s.push_str(&format!(" gifts: {}", gifts.join(", ")));
    }
    if m.improvements > 0 {
        s.push_str(&format!(" (improvements {})", m.improvements));
    }
    s
}

impl CheckReport {
    /// `fights.tsv`: one row per paired fight, the real result against what
    /// the engine makes of it.
    pub fn fights_tsv(&self) -> String {
        let mut out = String::from(
            "file\tgame\tturn\treal\tour_win\tour_tie\tour_loss\treal_result_pct\t\
             complete\tmissing\tdamage_mean\tdamage_p10\tdamage_p50\tdamage_p90\treal_damage\t\
             attack_cap_pct\tverdict\n",
        );
        for e in &self.entries {
            let c = &e.check;
            let pct = if c.old_patch {
                "-".to_string()
            } else {
                c.real_result_pct(e.fight.result()).map_or("-".to_string(), |p| format!("{p:.1}"))
            };
            let real_damage = e
                .fight
                .real_damage()
                .map_or("-".to_string(), |d| d.to_string());
            let missing = if c.missing.is_empty() {
                String::new()
            } else {
                c.missing.join("; ")
            };
            let verdict = if c.old_patch {
                "old-patch"
            } else if !c.complete {
                "incomplete"
            } else if c.flagged(e.fight.result(), self.options.flag_below) {
                "flagged"
            } else {
                "ok"
            };
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{}\t{}\t{}\t{:.2}\t{}\t{}\t{}\t{}\t{:.1}\t{}\n",
                e.fight.file(),
                e.fight.me.game,
                e.fight.turn(),
                result_name(e.fight.result()),
                c.win_pct(),
                c.tie_pct(),
                c.loss_pct(),
                pct,
                if c.complete { "yes" } else { "no" },
                missing,
                c.damage_mean,
                c.damage_p10,
                c.damage_p50,
                c.damage_p90,
                real_damage,
                100.0 * c.capped as f64 / c.runs.max(1) as f64,
                verdict,
            ));
        }
        out
    }

    /// `cards.tsv`: every card, power, trinket, Dark Gift and quest reward,
    /// with the share of its fights that came out unlikely.
    pub fn cards_tsv(&self) -> String {
        let mut out = String::from("kind\tname\tfights\tcomplete\tflagged\tflag_pct\n");
        for t in &self.pieces {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{:.1}\n",
                t.kind.map_or("?", PieceKind::name),
                t.name,
                t.fights,
                t.complete,
                t.flagged,
                t.ratio()
            ));
        }
        out
    }

    /// `report.md`: the summary, the calibration table, the damage comparison
    /// and the most surprising fights with both boards written out.
    pub fn report_md(&self, data: &Data) -> String {
        let mut out = String::new();
        let n = self.entries.len();
        let complete = self.complete();
        let flagged = self.flagged();
        let old = self.old_patch();
        let judged = n - old;
        out.push_str("# The fight check\n\n");
        out.push_str(&format!(
            "Every paired fight of Era's real games (a `me` and an `opp` board of the same \
             turn of the same game), played {} times each in this engine. A fight is \
             **flagged** when it is complete and we give the real result less than {:.1}%.\n\n",
            self.options.runs, self.options.flag_below
        ));
        out.push_str(&format!(
            "- fights: **{n}** (complete **{complete}**, incomplete **{}**, old-patch **{old}**)\n\
             - flagged: **{flagged}** ({:.1}% of the complete fights)\n",
            self.incomplete(),
            if complete == 0 { 0.0 } else { 100.0 * flagged as f64 / complete as f64 },
        ));
        out.push_str(
            "\nA file with no `build` key (collected before the key existed) is judged \
             as today: it cannot say which build its game was played on.\n\n",
        );
        let current = self
            .options
            .game_build
            .map_or("not in the card data".to_string(), |b| format!("build {b}"));
        if old == 0 {
            out.push_str(&format!(
                "The card data's `game_build` is {current}; no fight came from another build.\n\n"
            ));
        } else {
            let builds: Vec<String> = self
                .old_patch_builds()
                .into_iter()
                .map(|(build, n)| format!("build {build} ({n} fights)"))
                .collect();
            out.push_str(&format!(
                "The card data's `game_build` is {current}. **{old} fights are `old-patch`** \
                 (another build, not judged): {}.\n\n",
                builds.join(", ")
            ));
        }
        let mut real = [0u32; 3];
        let mut ours = [0f64; 3];
        for e in &self.entries {
            if e.check.old_patch {
                continue;
            }
            match e.fight.result() {
                RealResult::Win => real[0] += 1,
                RealResult::Tie => real[1] += 1,
                RealResult::Lose => real[2] += 1,
                RealResult::Unknown => {}
            }
            ours[0] += e.check.win_pct();
            ours[1] += e.check.tie_pct();
            ours[2] += e.check.loss_pct();
        }
        out.push_str(&format!(
            "- the real results: {} win, {} tie, {} lose\n",
            real[0], real[1], real[2]
        ));
        if judged > 0 {
            out.push_str(&format!(
                "- the engine's mean per fight: {:.1}% win, {:.1}% tie, {:.1}% lose\n",
                ours[0] / judged as f64,
                ours[1] / judged as f64,
                ours[2] / judged as f64
            ));
        }
        // A fight that reaches the attack cap has no winner to trust.
        let runs: u64 = self.entries.iter().map(|e| e.check.runs as u64).sum();
        let capped: u64 = self.entries.iter().map(|e| e.check.capped as u64).sum();
        if runs > 0 {
            out.push_str(&format!(
                "- runs that hit the {} attack cap (no side emptied, so a tie there is the \
                 engine's, not the boards'): {} ({:.2}%)\n",
                data.rules.max_attacks,
                capped,
                100.0 * capped as f64 / runs as f64
            ));
        }
        if !self.unpaired.is_empty() {
            out.push_str(&format!("\nBoards with no opponent board: {}\n", self.unpaired.len()));
        }

        // Why fights were incomplete, most common first.
        let mut reasons: Vec<(String, u32)> = Vec::new();
        for e in &self.entries {
            for line in &e.check.missing {
                let head = missing_kind(line).to_string();
                match reasons.iter_mut().find(|(r, _)| *r == head) {
                    Some((_, c)) => *c += 1,
                    None => reasons.push((head, 1)),
                }
            }
        }
        reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if !reasons.is_empty() {
            out.push_str("\n## Why fights were incomplete\n\n| missing piece | times |\n|---|---|\n");
            for (name, count) in &reasons {
                out.push_str(&format!("| {name} | {count} |\n"));
            }
        }

        // Calibration.
        out.push_str(
            "\n## Calibration\n\nComplete fights by our predicted win %, with the share the game \
             actually recorded as a win. A good engine's real rate tracks its own.\n\n\
             | our win % | fights | real win % | our mean win % |\n|---|---|---|---|\n",
        );
        for b in &self.calibration {
            let real = if b.fights == 0 {
                "-".to_string()
            } else {
                format!("{:.1}", 100.0 * b.real_wins as f64 / b.fights as f64)
            };
            let ours = if b.fights == 0 { "-".to_string() } else { format!("{:.1}", b.our_win) };
            out.push_str(&format!(
                "| {} to {}% | {} | {} | {} |\n",
                b.low,
                b.low + 10,
                b.fights,
                real,
                ours
            ));
        }

        // Damage, where Era lost and the next board is known.
        let mut with_damage: Vec<(i32, &FightCheck)> = Vec::new();
        for e in &self.entries {
            if e.fight.result() != RealResult::Lose || !e.check.complete {
                continue;
            }
            if let Some(d) = e.fight.real_damage() {
                with_damage.push((d, &e.check));
            }
        }
        if !with_damage.is_empty() {
            let n = with_damage.len() as f64;
            let real_mean = with_damage.iter().map(|(d, _)| *d as f64).sum::<f64>() / n;
            // Our damage for the same fight is what it costs Era on average
            // over every run: a loss's damage times how often we lose.
            let our_mean = with_damage
                .iter()
                .map(|(_, c)| c.damage_mean * c.losses as f64 / c.runs.max(1) as f64)
                .sum::<f64>()
                / n;
            let within = with_damage
                .iter()
                .filter(|(d, c)| *d >= c.damage_p10 && *d <= c.damage_p90)
                .count();
            out.push_str(&format!(
                "\n## Damage (the {} complete fights Era lost, where the next turn's board is \
                 known)\n\n\
                 The damage Era's hero took is the drop in Health plus armor from this turn's \
                 board to the next one, so it also holds what the next shop phase took (a Health \
                 price, an Ice Block that came off). It is reported, never flagged.\n\n\
                 - real damage per fight: **{real_mean:.2}** on average\n\
                 - our damage, per run (a loss's damage times how often we lose): \
                 **{our_mean:.2}** on average\n\
                 - fights whose real damage is inside our p10 to p90: **{within} of {}** \
                 ({:.0}%)\n\n| our damage when losing (p50) | fights | mean real damage |\n\
                 |---|---|---|\n",
                with_damage.len(),
                with_damage.len(),
                100.0 * within as f64 / n,
            ));
            let mut rows: Vec<(i32, [f64; 2])> = Vec::new();
            for (d, c) in &with_damage {
                match rows.iter_mut().find(|(p, _)| p == &c.damage_p50) {
                    Some((_, acc)) => {
                        acc[0] += 1.0;
                        acc[1] += *d as f64;
                    }
                    None => rows.push((c.damage_p50, [1.0, *d as f64])),
                }
            }
            rows.sort_by_key(|(p, _)| *p);
            for (p, [count, sum]) in &rows {
                out.push_str(&format!(
                    "| {} | {} | {:.2} |\n",
                    p,
                    *count as u32,
                    sum / count
                ));
            }
        }

        // The most surprising fights.
        out.push_str("\n## The 30 most surprising complete fights\n\n");
        for (i, e) in self.surprise_fights(30).into_iter().enumerate() {
            let c = &e.check;
            out.push_str(&format!(
                "### {}. `{}` turn {}, real **{}**, we give it {:.1}% ({} win / {} tie / {} loss)\n\n",
                i + 1,
                e.fight.file(),
                e.fight.turn(),
                result_name(e.fight.result()),
                c.real_result_pct(e.fight.result()).unwrap_or(0.0),
                c.wins,
                c.ties,
                c.losses
            ));
            for side in 0..2 {
                let spec = if side == 0 { &e.fight.me } else { &e.fight.opp };
                let label = if side == 0 { "Era" } else { "Opponent" };
                let power = spec
                    .power
                    .as_ref()
                    .map_or("-".to_string(), |p| p.name.clone());
                let trinkets = if spec.trinkets.is_empty() {
                    "-".to_string()
                } else {
                    spec.trinkets
                        .iter()
                        .map(|t| format!("{} ({})", t.name, if t.greater { "greater" } else { "lesser" }))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                out.push_str(&format!(
                    "**{label}**: tier {}, {} Health, {} armor, power {}, trinkets {}\n\n",
                    spec.tier, spec.health, spec.armor, power, trinkets
                ));
                let board = &e.prepared.sides[side].board;
                if board.is_empty() {
                    out.push_str("  board: (empty)\n\n");
                } else {
                    out.push_str("  board: ");
                    out.push_str(
                        &board
                            .iter()
                            .map(|m| minion_text(data, m))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    out.push_str("\n\n");
                }
                let hand = &e.prepared.sides[side].hand;
                if !hand.is_empty() {
                    out.push_str("  hand: ");
                    out.push_str(
                        &hand
                            .iter()
                            .map(|m| minion_text(data, m))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    out.push_str("\n\n");
                }
            }
        }
        out
    }
}
