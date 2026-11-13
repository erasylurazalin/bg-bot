//! The fight-state export: a `FightSnapshot` as the board-file JSON
//! `bgpy::BoardFight` and `real_boards` parse.
//!
//! Everything the shape can carry is written: minions with stats, keywords,
//! golden, Dark Gifts, `numbers` and the marks beside them; the hand; the
//! side's counters; the hero power with its numbers and waiting minion; the
//! trinkets with their values and slots; a Deity; the Secret zone. What the
//! shape has no key for is left out (the collector's report names it):
//! Dark Gift countdowns, a second hero power, a power's pressed state and
//! its presses left, the Beast/Boon/Sharing is Caring counters, a warband a
//! Magnanimoose chose, and a power trinket's own engine slot.

use crate::cards::Counter;
use crate::heroes::PowerEffect;
use crate::real_boards::CardIdMap;
use crate::trinkets::{FightAct, Slot};
use crate::{
    Data, FightSideSnapshot, FightSnapshot, Keywords, Minion, PowerState, Tribe, TrinketState,
};
use serde_json::{json, Map, Value};

/// The fight as two board-file objects, in `BoardFight`'s constructor order:
/// `me`'s side, the opponent's side, and the paired seat (None for a ghost).
pub fn pair_json(
    snapshot: &FightSnapshot,
    data: &Data,
    ids: &CardIdMap,
) -> (String, String, Option<usize>) {
    let me = board_json(&snapshot.me, snapshot.turn, data, ids);
    let opp = board_json(&snapshot.opp_side, snapshot.turn, data, ids);
    (me.to_string(), opp.to_string(), snapshot.opp.map(usize::from))
}

fn board_json(
    side: &FightSideSnapshot,
    turn: u32,
    data: &Data,
    ids: &CardIdMap,
) -> Value {
    let mut board = Map::new();
    board.insert("turn".to_string(), json!(turn));
    board.insert("tier".to_string(), json!(side.tier));
    board.insert("health".to_string(), json!(side.health));
    board.insert("armor".to_string(), json!(side.armor));
    board.insert(
        "minions".to_string(),
        Value::Array(side.board.iter().map(|m| minion_json(m, data, ids)).collect()),
    );
    board.insert(
        "hand".to_string(),
        Value::Array(side.hand.iter().map(|m| minion_json(m, data, ids)).collect()),
    );
    board.insert("counters".to_string(), counters_json(side));
    board.insert("rewards".to_string(), Value::Array(Vec::new()));
    board.insert(
        "secrets".to_string(),
        Value::Array(
            side.secrets
                .iter()
                .filter_map(|k| crate::secrets::CARD_IDS.get(k as usize))
                .map(|id| json!(id))
                .collect(),
        ),
    );
    board.insert("hand_hidden".to_string(), json!(0));
    if let Some(power) = side.power.or(side.power2) {
        board.insert("hero_power".to_string(), power_json(&power, side, data, ids));
    }
    let trinkets: Vec<Value> = side
        .trinkets
        .iter()
        .filter(|t| t.of != 0)
        .map(|t| trinket_json(t, data))
        .collect();
    if !trinkets.is_empty() {
        board.insert("trinkets".to_string(), Value::Array(trinkets));
    }
    if let Some(d) = side.deity {
        let card = data.cards.get(d.card);
        let card_id = ids
            .id_for_name(&card.name, d.golden)
            .map(str::to_string)
            .unwrap_or_else(|| format!("UNKNOWN:{}", card.name));
        board.insert(
            "deity".to_string(),
            json!({
                "card_id": card_id,
                "name": card.name,
                "attack": d.attack,
                "health": d.health,
            }),
        );
    }
    Value::Object(board)
}

fn minion_json(m: &Minion, data: &Data, ids: &CardIdMap) -> Value {
    let card = data.cards.get(m.card);
    let card_id = ids
        .id_for_name(&card.name, false)
        .or_else(|| ids.id_for_name(&card.name, true))
        .map(str::to_string)
        .unwrap_or_else(|| format!("UNKNOWN:{}", card.name));
    let keywords: Vec<&str> = Keywords::ALL
        .iter()
        .filter(|(k, _)| m.keywords.has(*k))
        .map(|(_, name)| *name)
        .collect();
    let gifts: Vec<&str> = (0..m.n_gifts as usize)
        .filter_map(|i| {
            let gift = m.gifts[i].card();
            data.gifts.gifts.iter().find(|g| g.card == gift).map(|g| g.id.as_str())
        })
        .collect();
    json!({
        "card_id": card_id,
        "name": card.name,
        "attack": m.attack,
        "health": m.health,
        "golden": m.golden,
        "keywords": keywords,
        "numbers": numbers(m, &card.name, data),
        "dark_gifts": gifts,
        "vanguard_aura": [m.vanguard_aura[0], m.vanguard_aura[1]],
        "reanimate": m.reanimate,
        "no_reward": m.no_reward,
    })
}

/// The per-minion numbers the board loader reads back: the counts `improvements`
/// holds (Lurking Leviathan, Spark Snapper, Ultraviolet Ascendant, Patient
/// Scout, Sacrificial Wrathguard, Lockbox), and the uses left of a card with
/// limited uses. A card the loader does not map keeps no numbers, so its count
/// is lost with them.
fn numbers(m: &Minion, name: &str, data: &Data) -> Vec<i32> {
    let mul = if m.golden { 2 } else { 1 };
    let imp = m.improvements as i32;
    let mut out: Vec<i32> = Vec::new();
    match name {
        "Lurking Leviathan" if imp > 0 => out = vec![imp * 3 * mul],
        "Spark Snapper" if imp > 0 => out = vec![imp * 2 * mul],
        "Ultraviolet Ascendant" if imp > 0 => out = vec![0, 0, imp],
        "Patient Scout" if imp > 0 => out = vec![imp + 1],
        "Sacrificial Wrathguard" if imp > 0 => out = vec![0, 0, imp * 2 * mul],
        "Lockbox" if imp > 0 => out = vec![imp],
        _ => {}
    }
    if m.turn_uses(&data.cards) > 0 {
        if out.is_empty() {
            out.push(0);
        }
        out[0] = m.uses_left as i32;
    }
    out
}

fn counters_json(side: &FightSideSnapshot) -> Value {
    let c = &side.real;
    let mut out = Map::new();
    let pairs: [(&str, i32); 23] = [
        ("gem_attack", c.gem_attack),
        ("gem_health", c.gem_health),
        ("beetle_attack", c.beetle_attack),
        ("beetle_health", c.beetle_health),
        ("undead_attack", c.undead_attack),
        ("undead_health", c.undead_health),
        ("spell_attack", c.spell_attack),
        ("spell_health", c.spell_health),
        ("spells_cast", c.spells_cast as i32),
        ("knight_deaths", c.knight_deaths as i32),
        ("golden_played", c.golden_played as i32),
        ("gold_spent", c.gold_spent as i32),
        ("tavern_attack", c.tavern_attack),
        ("tavern_health", c.tavern_health),
        ("tavern_turn_attack", c.tavern_turn_attack),
        ("tavern_turn_health", c.tavern_turn_health),
        ("tavern_elemental_attack", c.tavern_elemental_attack),
        ("tavern_elemental_health", c.tavern_elemental_health),
        ("upper_hand", c.upper_hand as i32),
        ("refresh_attack", c.refresh_attack),
        ("refresh_health", c.refresh_health),
        ("barrage_attack", c.barrage_attack),
        ("barrage_health", c.barrage_health),
    ];
    for (key, value) in pairs {
        out.insert(key.to_string(), json!(value));
    }
    // The two "improve your future" counters are the only ones there are.
    out.insert(
        "tasty_lobster".to_string(),
        json!(c.improve[Counter::TastyLobster.index()]),
    );
    out.insert("baller".to_string(), json!(c.improve[Counter::Baller.index()]));
    out.insert(
        "volumizer_attack".to_string(),
        json!(c.volumizer_attack),
    );
    out.insert(
        "volumizer_health".to_string(),
        json!(c.volumizer_health),
    );
    Value::Object(out)
}

fn power_json(
    power: &PowerState,
    side: &FightSideSnapshot,
    data: &Data,
    ids: &CardIdMap,
) -> Value {
    let entry = data.heroes.power(power.of);
    let mut out = Map::new();
    out.insert("card_id".to_string(), json!(entry.card_id));
    out.insert("name".to_string(), json!(entry.name));
    let numbers: Vec<i32> = match entry.effect {
        // The only two powers whose fight magnitude is a number: Broodmother's
        // whelp bonus and Ozumat's Tentacle Attack (the engine's Tentacle is
        // the card plus one of each per minion sold this game).
        PowerEffect::Broodmother { .. } => vec![power.progress as i32],
        PowerEffect::Tentacular { card } => {
            let base = data.cards.get(card).attack;
            vec![base + side.counters.minions_sold as i32]
        }
        _ => vec![power.progress as i32],
    };
    out.insert("numbers".to_string(), json!(numbers));
    if matches!(entry.effect, PowerEffect::LockAndLoad) {
        let waiting = side
            .waiting
            .map(|m| minion_json(&m, data, ids))
            .unwrap_or(Value::Null);
        out.insert("waiting".to_string(), waiting);
    }
    Value::Object(out)
}

fn trinket_json(t: &TrinketState, data: &Data) -> Value {
    let entry = data.trinkets.get(t.of - 1);
    let mut numbers: Vec<i32> = Vec::new();
    for e in &entry.fight {
        if let FightAct::Stats { attack, grow, .. } = e.act
            && grow != 0
        {
            numbers.push(attack + grow * t.value as i32);
            break;
        }
    }
    if numbers.is_empty()
        && let Some(tribe) = t.tribe
    {
        numbers.push(race_code(tribe));
    }
    json!({
        "card_id": entry.card_id,
        "name": entry.name,
        "greater": entry.slot == Slot::Greater,
        "numbers": numbers,
    })
}

/// The game's race code for a tribe, the number a Compass writes at its pick
/// (`fight_check::race_code_tribe` reads it back).
fn race_code(tribe: Tribe) -> i32 {
    match tribe {
        Tribe::Undead => 11,
        Tribe::Murloc => 14,
        Tribe::Demon => 15,
        Tribe::Mech => 17,
        Tribe::Elemental => 18,
        Tribe::Beast => 20,
        Tribe::Pirate => 23,
        Tribe::Dragon => 24,
        Tribe::Quilboar => 43,
        Tribe::Aberration => 126,
    }
}

/// What the export cannot carry, one line each, for the collector's report.
pub fn limitations() -> Vec<&'static str> {
    vec![
        "Dark Gift countdowns: the board shape has no dark_gift_states key",
        "a second hero power: the board shape carries one hero_power",
        "a hero power's pressed state and presses left: the shape has no key",
        "Beast bonus, Boon of Beetles, Brood of Nozdormu and Sharing is Caring counters: no board key",
        "the warband a Magnanimoose chose: no board key",
        "a power trinket's own engine slot when the shape's slot order puts it elsewhere",
        "a minion's scaling count for cards the board loader does not map (Timewarped Painter and kin)",
        "minion marks the shape has no key for: temporary Divine Shield/Reborn, extra tribes from a portrait, attached parts and Magnetizations",
        "a hand card the ref card file has no id for (the Blood Gem tokens): it loads as a Stand-in",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::fight_with_hands;
    use crate::fight_check::prepare;
    use crate::real_boards::{parse_board_files, RealBoardSpec, RealFightSpec};
    use crate::{Action, Counters, Game, Opponent, PLAYERS, Rng, Tribes};
    use std::sync::Arc;

    fn card_ids() -> CardIdMap {
        let path = format!("{}/../../ref/bg-cards.json", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).expect("the ref card file");
        CardIdMap::parse(&text).expect("the ref card file parses")
    }

    fn parse_one(text: &str, ids: &CardIdMap) -> RealBoardSpec {
        let wrapped = format!("{{\"source\":\"board\",\"game\":1,\"boards\":[{text}]}}");
        let (mut specs, _) =
            parse_board_files(&[("board".to_string(), wrapped)], ids).expect("a board parses");
        specs.remove(0)
    }

    /// Wins, ties, losses, capped and uncapped dealt and taken, as
    /// `BoardFight::fight` tallies them.
    #[allow(clippy::too_many_arguments)]
    fn tally(
        data: &Data,
        turn: u32,
        me_tier: u8,
        opp_tier: u8,
        me: &[Minion],
        opp: &[Minion],
        hands: [&[Minion]; 2],
        counters: [Counters; 2],
        n: u32,
        seed: u64,
    ) -> (u32, u32, u32, i64, i64, i64, i64) {
        let mut rng = Rng::new(seed);
        let mut wins = 0;
        let mut ties = 0;
        let mut losses = 0;
        let mut dealt = 0;
        let mut taken = 0;
        let mut dealt_uncapped = 0;
        let mut taken_uncapped = 0;
        for _ in 0..n {
            let out = fight_with_hands(
                &data.cards,
                me,
                opp,
                hands,
                counters,
                data.rules.max_attacks,
                &mut rng,
                &mut (),
            );
            let cap = |raw: i32| data.rules.damage_cap(turn, PLAYERS).map_or(raw, |c| raw.min(c));
            match out.winner {
                Some(0) => {
                    let raw = me_tier as i32 + out.survivor_tiers;
                    wins += 1;
                    dealt += cap(raw) as i64;
                    dealt_uncapped += raw as i64;
                }
                None => ties += 1,
                Some(_) => {
                    let raw = opp_tier as i32 + out.survivor_tiers;
                    losses += 1;
                    taken += cap(raw) as i64;
                    taken_uncapped += raw as i64;
                }
            }
        }
        (wins, ties, losses, dealt, dealt_uncapped, taken, taken_uncapped)
    }

    #[test]
    fn an_exported_fight_matches_the_engines_own_fight() {
        // A game played to a fight: player 0 has two 5/5 Campers, the opponent
        // one 1/1 Bat. The board is deterministic (no rolls of its own), so
        // every run must be the same win.
        let mut game =
            Game::new_with_lobby(Arc::new(Data::bundled()), 7, Tribes::ALL);
        let camper = game.data.cards.by_name("Tusked Camper").expect("a Camper");
        let bat = game.data.cards.by_name("Flittering Bat").expect("a Bat");
        let mut camper = game.data.cards.make(camper, false);
        camper.attack = 5;
        camper.health = 5;
        let mut bat = game.data.cards.make(bat, false);
        bat.attack = 1;
        bat.health = 1;
        let Some(Opponent::Player(q)) = game.players[0].opponent else {
            panic!("player 0 was not paired with a player");
        };
        game.players[0].board.clear();
        game.players[0].board.push(camper);
        game.players[0].board.push(camper);
        game.players[q as usize].board.clear();
        game.players[q as usize].board.push(bat);
        game.record_fights(true);
        for _ in 0..PLAYERS {
            game.step(Action::EndTurn).expect("End Turn is legal");
        }
        let snapshot = game
            .fight_snapshots
            .as_ref()
            .and_then(|s| s[0].as_ref())
            .expect("player 0 fought in the round");
        let ids = card_ids();
        let (me_json, opp_json, opp_seat) = pair_json(snapshot, &game.data, &ids);
        assert_eq!(opp_seat, Some(q as usize));

        // The same pair through the board-file parser and `prepare`, which is
        // exactly what `BoardFight` does.
        let me_spec = parse_one(&me_json, &ids);
        let opp_spec = parse_one(&opp_json, &ids);
        let turn = me_spec.turn;
        let (me_tier, opp_tier) = (me_spec.tier, opp_spec.tier);
        let prepared = prepare(
            &game.data,
            &RealFightSpec { me: me_spec, opp: opp_spec, next: None },
        );
        assert!(prepared.is_complete(), "{:?}", prepared.missing_lines());
        let [me, opp] = &prepared.sides;

        let engine = tally(
            &game.data,
            snapshot.turn,
            snapshot.me.tier,
            snapshot.opp_side.tier,
            &snapshot.me.board,
            &snapshot.opp_side.board,
            [&snapshot.me.hand, &snapshot.opp_side.hand],
            [snapshot.me.counters, snapshot.opp_side.counters],
            200,
            42,
        );
        let exported = tally(
            &game.data,
            turn,
            me_tier,
            opp_tier,
            &me.board,
            &opp.board,
            [&me.hand, &opp.hand],
            [me.counters, opp.counters],
            200,
            42,
        );
        assert_eq!(exported, engine);
        assert_eq!(exported, (200, 0, 0, 600, 600, 0, 0));
    }

    #[test]
    fn a_real_fight_records_the_uncapped_damage_the_export_tallies() {
        // Six 5/5 Campers beat a 1/4 Bat on turn 1. The raw win is the
        // winner's tier plus its surviving minions' tiers: 1 + 6 = 7, and the
        // turn 1 cap is 5, so capped and uncapped differ.
        let mut game =
            Game::new_with_lobby(Arc::new(Data::bundled()), 7, Tribes::ALL);
        let camper = game.data.cards.by_name("Tusked Camper").expect("a Camper");
        let bat = game.data.cards.by_name("Flittering Bat").expect("a Bat");
        let mut camper = game.data.cards.make(camper, false);
        camper.attack = 5;
        camper.health = 5;
        let mut bat = game.data.cards.make(bat, false);
        bat.attack = 1;
        bat.health = 4;
        let Some(Opponent::Player(q)) = game.players[0].opponent else {
            panic!("player 0 was not paired with a player");
        };
        game.players[0].board.clear();
        for _ in 0..6 {
            game.players[0].board.push(camper);
        }
        game.players[q as usize].board.clear();
        game.players[q as usize].board.push(bat);
        let before = game.players[q as usize].health;
        game.record_fights(true);
        for _ in 0..PLAYERS {
            game.step(Action::EndTurn).expect("End Turn is legal");
        }

        assert!(game.fight_recorded);
        assert_eq!(game.fight_dealt_uncapped[0], 7);
        assert_eq!(game.fight_taken_uncapped[0], 0);
        assert_eq!(game.fight_dealt_uncapped[q as usize], 0);
        assert_eq!(game.fight_taken_uncapped[q as usize], 7);
        assert_eq!(before - game.players[q as usize].health, 5, "the cap applied");

        // The same fight through the export's own tally: the uncapped dealt is
        // the raw 7, the capped dealt 5.
        let snapshot = game
            .fight_snapshots
            .as_ref()
            .and_then(|s| s[0].as_ref())
            .expect("player 0 fought in the round");
        let ids = card_ids();
        let (me_json, opp_json, opp_seat) = pair_json(snapshot, &game.data, &ids);
        assert_eq!(opp_seat, Some(q as usize));
        let me_spec = parse_one(&me_json, &ids);
        let opp_spec = parse_one(&opp_json, &ids);
        let prepared = prepare(
            &game.data,
            &RealFightSpec { me: me_spec, opp: opp_spec, next: None },
        );
        assert!(prepared.is_complete(), "{:?}", prepared.missing_lines());
        let (wins, ties, losses, dealt, dealt_uncapped, taken, taken_uncapped) = tally(
            &game.data,
            snapshot.turn,
            snapshot.me.tier,
            snapshot.opp_side.tier,
            &snapshot.me.board,
            &snapshot.opp_side.board,
            [&snapshot.me.hand, &snapshot.opp_side.hand],
            [snapshot.me.counters, snapshot.opp_side.counters],
            1,
            42,
        );
        assert_eq!((wins, ties, losses), (1, 0, 0));
        assert_eq!((dealt, dealt_uncapped), (5, 7));
        assert_eq!((taken, taken_uncapped), (0, 0));
    }
}
