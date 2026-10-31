//! What the player sees: the state screen, open choices and card text.

use std::collections::HashMap;
use std::fmt::Write;

use bgcore::cards::Card;
use bgcore::game::{PendingTarget, TargetKind};
use bgcore::{Action, CardDb, CardId, Game, Keywords, Minion, Tribe, Tribes};

/// Card text from `ref/lobby5.tsv` when that file is there (it is
/// git-ignored), keyed by card name.
#[derive(Default)]
pub struct CardTexts(HashMap<String, String>);

impl CardTexts {
    pub fn load() -> CardTexts {
        let paths = [
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../ref/lobby5.tsv"),
            "ref/lobby5.tsv",
        ];
        for p in paths {
            if let Ok(text) = std::fs::read_to_string(p) {
                return CardTexts::parse(&text);
            }
        }
        CardTexts::default()
    }

    pub fn parse(text: &str) -> CardTexts {
        let mut map = HashMap::new();
        for line in text.lines() {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() >= 8 && (cols[0] == "minion" || cols[0] == "spell") {
                let text: Vec<&str> = cols[7].split_whitespace().collect();
                map.insert(cols[3].to_string(), text.join(" "));
            }
        }
        CardTexts(map)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

pub fn tribe_name(t: Tribe) -> &'static str {
    match t {
        Tribe::Beast => "Beast",
        Tribe::Mech => "Mech",
        Tribe::Dragon => "Dragon",
        Tribe::Murloc => "Murloc",
        Tribe::Demon => "Demon",
        Tribe::Pirate => "Pirate",
        Tribe::Elemental => "Elemental",
        Tribe::Undead => "Undead",
        Tribe::Quilboar => "Quilboar",
        Tribe::Aberration => "Aberration",
    }
}

pub fn parse_tribe(s: &str) -> Option<Tribe> {
    Tribe::EVERY.into_iter().find(|&t| tribe_name(t).eq_ignore_ascii_case(s.trim()))
}

/// A lobby mask as "Beast, Mech, ...".
pub fn lobby_names(t: Tribes) -> String {
    Tribe::EVERY
        .iter()
        .filter(|&&tribe| t.has(tribe))
        .map(|&tribe| tribe_name(tribe))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a `--tribe` value: one or more comma-separated tribes (1 to 5,
/// distinct), the lobby a sandbox plays with.
pub fn parse_lobby(s: &str) -> Option<Tribes> {
    let mut lobby = Tribes::NONE;
    for part in s.split(',') {
        if part.trim().is_empty() {
            return None;
        }
        let t = parse_tribe(part)?;
        if lobby.has(t) {
            return None;
        }
        lobby = lobby.with(t);
    }
    (1..=5).contains(&lobby.len()).then_some(lobby)
}

fn card_tribes(c: &Card) -> String {
    if c.tribes.is_neutral() {
        return "Neutral".into();
    }
    if Tribe::EVERY.iter().all(|&t| c.tribes.has(t)) {
        return "All types".into();
    }
    Tribe::EVERY.iter().filter(|&&t| c.tribes.has(t)).map(|&t| tribe_name(t)).collect::<Vec<_>>().join("/")
}

fn tags(m: &Minion, db: &CardDb) -> String {
    let mut out = String::new();
    let mut add = |s: &str| {
        out.push(' ');
        out.push_str(s);
    };
    for (bit, tag) in [
        (Keywords::TAUNT, "T"),
        (Keywords::DIVINE_SHIELD, "DS"),
        (Keywords::WINDFURY, "W"),
        (Keywords::VENOMOUS, "V"),
        (Keywords::REBORN, "R"),
        (Keywords::STEALTH, "S"),
        (Keywords::MAGNETIC, "M"),
    ] {
        if m.keywords.has(bit) {
            add(tag);
        }
    }
    if m.golden {
        add("G");
    }
    if let Some((cost, true)) = activate_of(m, db).map(|c| (c, !m.activated)) {
        add(&format!("A{cost}"));
    }
    if let Some(n) = m.events_left(db) {
        add(&format!("[{n} left]"));
    }
    if m.locked_turns > 0 {
        add(&format!("L{}", m.locked_turns));
    }
    out
}

fn activate_of(m: &Minion, db: &CardDb) -> Option<u8> {
    m.parts().find_map(|(c, _)| db.get(c).activate.map(|a| a.cost))
}

/// Cost of a Tavern spell as shown, with the player's discount.
fn spell_cost(g: &Game, id: CardId) -> String {
    let c = g.data.cards.get(id);
    if c.pay_health {
        format!("{} health", c.cost)
    } else {
        let d = g.players[0].spell_discount;
        let paid = c.cost.saturating_sub(d);
        if paid < c.cost { format!("{paid}g, was {}g", c.cost) } else { format!("{paid}g") }
    }
}

/// A minion or spell as one short label: name, stats, tags.
pub fn label(g: &Game, m: &Minion) -> String {
    let db = &g.data.cards;
    let c = db.get(m.card);
    if c.is_spell() {
        return format!("{} ({})", c.name, spell_cost(g, m.card));
    }
    format!("{} {}/{}{}", c.name, m.attack, m.health, tags(m, db))
}

fn numbered(g: &Game, list: &[Minion]) -> String {
    if list.is_empty() {
        return "(empty)".into();
    }
    list.iter().enumerate().map(|(i, m)| format!("{} {}", i + 1, label(g, m))).collect::<Vec<_>>().join(" | ")
}

fn counters_line(g: &Game) -> String {
    let pl = &g.players[0];
    let mut parts: Vec<String> = Vec::new();
    let (sa, sh) = g.total_spell_bonus(0);
    if (sa, sh) != (0, 0) {
        parts.push(format!("Spell bonus +{sa}/+{sh}"));
    }
    if pl.improve[0] > 0 {
        parts.push(format!("Lobster count {}", pl.improve[0]));
    }
    if !pl.fodder_refreshes.is_empty() {
        let q: Vec<String> = pl.fodder_refreshes.iter().map(|n| n.to_string()).collect();
        parts.push(format!("Fodder queue [{}]", q.join(",")));
    }
    if pl.beetle_bonus != (0, 0) {
        parts.push(format!("Beetle bonus +{}/+{}", pl.beetle_bonus.0, pl.beetle_bonus.1));
    }
    if pl.beast_bonus != (0, 0) {
        parts.push(format!("Beast bonus +{}/+{} until next turn", pl.beast_bonus.0, pl.beast_bonus.1));
    }
    if pl.volumizer_bonus != (0, 0) {
        parts.push(format!("Volumizer bonus +{}/+{}", pl.volumizer_bonus.0, pl.volumizer_bonus.1));
    }
    if pl.deathrattles_triggered > 0 {
        parts.push(format!("Deathrattles triggered {}", pl.deathrattles_triggered));
    }
    if pl.spell_discount > 0 {
        parts.push(format!("Next spell {} cheaper", pl.spell_discount));
    }
    if pl.boon_of_beetles > 0 {
        parts.push(format!("Boon of Beetles {}", pl.boon_of_beetles));
    }
    if pl.brood_of_nozdormu > 0 {
        parts.push(format!("Brood of Nozdormu {}", pl.brood_of_nozdormu));
    }
    if pl.upper_hand > 0 {
        parts.push(format!("Upper Hand {}", pl.upper_hand));
    }
    if pl.sharing_is_caring > 0 {
        parts.push(format!("Sharing is Caring {}", pl.sharing_is_caring));
    }
    if pl.gold_next_turn > 0 {
        parts.push(format!("Gold next turn +{}", pl.gold_next_turn));
    }
    if pl.extra_max_gold > 0 {
        parts.push(format!("Max gold +{}", pl.extra_max_gold));
    }
    if pl.gem_attack != 1 || pl.gem_health != 1 {
        parts.push(format!("Blood Gems +{}/+{}", pl.gem_attack, pl.gem_health));
    }
    if pl.barrage != (0, 0) {
        parts.push(format!("Barrage +{}/+{} per Refresh", pl.barrage.0, pl.barrage.1));
    }
    for b in &pl.tavern_buffs {
        let who = b.tribe.map_or("all", tribe_name);
        let how = if b.this_turn { "this turn" } else { "this game" };
        parts.push(format!("Tavern {who} +{}/+{} {how}", b.attack, b.health));
    }
    parts.join("  ")
}

/// The whole screen: header, counters, Tavern, hand, board, and any open choice.
pub fn state(g: &Game, texts: &CardTexts, ctx: Option<&str>) -> String {
    let pl = &g.players[0];
    let db = &g.data.cards;
    let mut out = String::new();
    let max_gold = g.data.rules.gold(g.turn).saturating_add(pl.extra_max_gold);
    let up = if pl.tier < bgcore::MAX_TIER { format!(" (up {}g)", pl.upgrade_cost) } else { String::new() };
    let _ = write!(
        out,
        "Turn {}  Tier {}{}  Gold {}/{}  Health {}  Armor {}  Free rolls {}",
        g.turn, pl.tier, up, pl.gold, max_gold, pl.health, pl.armor, pl.free_rolls
    );
    if pl.frozen {
        out.push_str("  [Frozen]");
    }
    out.push('\n');
    let heroes = &g.data.heroes;
    let hero_line = match pl.hero {
        Some(h) => {
            let hero = heroes.get(h);
            let power = match &pl.power {
                Some(st) => {
                    let pw = &heroes.get(st.of).power;
                    let state = if pw.passive {
                        String::new()
                    } else {
                        format!(
                            " ({}, {}/{} uses{}{})",
                            if pw.cost == 0 { "free".into() } else { format!("{}g", pw.cost) },
                            st.uses_left,
                            pw.uses_per_turn,
                            if st.unlocked { "" } else { ", locked" },
                            if pw.once_per_game && st.used_game > 0 { ", used" } else { "" },
                        )
                    };
                    format!("{}: {}{}{}", hero.name, pw.name, if state.is_empty() { "" } else { " " }, state)
                }
                None => String::new(),
            };
            format!("Hero: {} (armor {}) {}", hero.name, pl.armor, power)
        }
        None => String::new(),
    };
    if !hero_line.is_empty() {
        let _ = writeln!(out, "{hero_line}");
    }
    let _ = writeln!(out, "Lobby: {}", lobby_names(g.lobby));
    let c = counters_line(g);
    if !c.is_empty() {
        let _ = writeln!(out, "{c}");
    }
    let mut tavern = numbered(g, &pl.shop);
    if pl.shop.is_empty() {
        tavern = "(empty)".into();
    }
    match pl.shop_spell {
        Some(id) => {
            let _ = write!(out, "Tavern: {tavern} | S {} ({})", db.get(id).name, spell_cost(g, id));
        }
        None => {
            let _ = write!(out, "Tavern: {tavern} | S (no spell)");
        }
    }
    out.push('\n');
    let _ = writeln!(out, "Hand:   {}", numbered(g, &pl.hand));
    let _ = write!(out, "Board:  {}", numbered(g, &pl.board));
    if g.over {
        out.push_str("\n\nGAME OVER: the hero was knocked out (or the turn limit was reached). undo takes back the last action, quit saves and exits.");
    } else if let Some(p) = prompt(g, texts, ctx) {
        out.push_str("\n\n");
        out.push_str(&p);
    }
    out
}

/// What the open choice is and how to answer it, None when nothing is open.
pub fn prompt(g: &Game, texts: &CardTexts, ctx: Option<&str>) -> Option<String> {
    let pl = &g.players[0];
    let db = &g.data.cards;
    let from = ctx.map(|c| format!(" for {c}")).unwrap_or_default();
    let text = ctx.and_then(|c| texts.get(c)).map(|t| format!("\n  Card text: {t}")).unwrap_or_default();
    if let Some(pt) = pl.pending_target {
        let boards: Vec<String> = (0..pl.board.len())
            .filter(|&s| g.is_legal(Action::Target(s as u8)))
            .map(|s| format!("{} {}", s + 1, label(g, &pl.board[s])))
            .collect();
        let mut out = format!(
            "CHOICE: pick a target{from} ({}). Type a board number.{text}\n  Board targets: {}",
            describe_target(&pt),
            if boards.is_empty() { "none".into() } else { boards.join(" | ") }
        );
        if pt.kind.takes_shop() {
            let shop: Vec<String> = (0..pl.shop.len())
                .filter(|&s| g.is_legal(Action::TargetShop(s as u8)))
                .map(|s| format!("t{} {}", s + 1, label(g, &pl.shop[s])))
                .collect();
            let _ = write!(out, "\n  Tavern targets (type tN): {}", if shop.is_empty() { "none".into() } else { shop.join(" | ") });
        }
        return Some(out);
    }
    if pl.pending_shop_target.is_some() {
        let shop: Vec<String> =
            (0..pl.shop.len()).map(|s| format!("{} {}", s + 1, label(g, &pl.shop[s]))).collect();
        return Some(format!(
            "CHOICE: pick a Tavern minion{from}. Type its slot number (or tN).{text}\n  Tavern: {}",
            shop.join(" | ")
        ));
    }
    if pl.pending_discard.is_some() {
        let hand: Vec<String> = (0..pl.hand.len())
            .filter(|&s| g.is_legal(Action::Discard(s as u8)))
            .map(|s| format!("{} {}", s + 1, label(g, &pl.hand[s])))
            .collect();
        return Some(format!(
            "CHOICE: discard a card{from}. Type a hand number (cN).\n  Hand: {}",
            if hand.is_empty() { "none".into() } else { hand.join(" | ") }
        ));
    }
    if let Some(pco) = pl.pending_choose_one {
        let opts: Vec<String> = (0..2)
            .map(|k| {
                let ok = g.is_legal(Action::Choose(k as u8));
                format!(
                    "{} {}{}",
                    k + 1,
                    describe_option(&pco.options[k]),
                    if ok { "" } else { " (not available)" }
                )
            })
            .collect();
        return Some(format!("CHOICE: Choose One{from}. Type 1 or 2.{text}\n  {}", opts.join("\n  ")));
    }
    // The hero pick before turn 1.
    if g.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX) {
        let heroes = &g.data.heroes;
        let opts: Vec<String> = pl
            .hero_offer
            .iter()
            .filter(|&&h| h != u8::MAX)
            .map(|&h| {
                let hero = heroes.get(h);
                format!(
                    "{} (armor {}, {}, {})",
                    hero.name,
                    hero.armor,
                    power_text(heroes, h),
                    if hero.tribes.is_neutral() {
                        "any lobby".to_string()
                    } else {
                        format!("needs {}", lobby_names(hero.tribes))
                    },
                )
            })
            .collect();
        return Some(format!(
            "CHOICE: pick your hero. Type its number.\n  {}",
            opts.join("\n  ")
        ));
    }
    // A Hero Power Discover (Unmasked Identity, Finley's Adventure!).
    if !pl.power_offer.is_empty() {
        let heroes = &g.data.heroes;
        let opts: Vec<String> = pl
            .power_offer
            .iter()
            .map(|&h| format!("{} {}", heroes.get(h).name, power_text(heroes, h)))
            .collect();
        return Some(format!(
            "CHOICE: pick a Hero Power. It replaces your current one.\n  {}",
            opts.join("\n  ")
        ));
    }
    if !pl.discover.is_empty() {
        let opts: Vec<String> = pl
            .discover
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                let c = db.get(id);
                if c.is_spell() {
                    format!("{} {} (spell, tier {}, {})", i + 1, c.name, c.tier, spell_cost(g, id))
                } else {
                    format!("{} {} {}/{} (tier {}, {})", i + 1, c.name, c.attack, c.health, c.tier, card_tribes(c))
                }
            })
            .collect();
        // a golden minion played from hand also offers a Discover, and the
        // card that was just played says nothing about it
        let text = match ctx.and_then(|c| texts.get(c)) {
            Some(t) if t.contains("iscover") => text,
            Some(_) => "\n  (a golden minion's reward)".to_string(),
            None => text,
        };
        return Some(format!(
            "CHOICE: Discover{from}. Type 1 to {} (?d1 shows a card's text).{text}\n  {}",
            pl.discover.len(),
            opts.join("\n  ")
        ));
    }
    None
}

/// A hero power as one short line: name, button cost and limits.
fn power_text(heroes: &bgcore::heroes::HeroDb, h: u8) -> String {
    let pw = &heroes.get(h).power;
    let mut out = String::new();
    if !pw.passive {
        out.push_str(&format!(
            "power {}g{}",
            pw.cost,
            if pw.uses_per_turn > 1 { format!(", {}/turn", pw.uses_per_turn) } else { String::new() },
        ));
        if pw.unlock_tier > 0 {
            out.push_str(&format!(", unlocks at Tier {}", pw.unlock_tier));
        }
        if pw.once_per_game {
            out.push_str(", once per game");
        }
    } else {
        out.push_str("passive");
    }
    out
}

fn describe_target(pt: &PendingTarget) -> String {
    match pt.kind {
        TargetKind::MinionBuff { tribe, attack, health, keyword, .. } => {
            let who = tribe.map_or("a minion", |t| tribe_name(t));
            format!("{who} gets +{attack}/+{health}{}", kw_suffix(keyword))
        }
        TargetKind::AnyBuff { attack, health } => format!("a minion gets +{attack}/+{health}"),
        TargetKind::AnySetStats { attack, health } => format!("a minion's stats become {attack}/{health}"),
        TargetKind::MakeGolden { tribe, .. } => {
            format!("{} becomes Golden", tribe.map_or("a minion", |t| tribe_name(t)))
        }
        TargetKind::TriggerRally => "trigger a Rally".into(),
        TargetKind::TriggerBattlecry => "trigger a Battlecry".into(),
        TargetKind::Consume { .. } => "it consumes Tavern minions".into(),
        TargetKind::DestroyDiscover { tribe, .. } => {
            format!("destroy a {}, then Discover one", tribe_name(tribe))
        }
        TargetKind::DestroyCopy { tribe, copies } => {
            format!("destroy a {}, then get {copies} plain copies of it", tribe_name(tribe))
        }
        TargetKind::RebornDestroy { tribe, attack, health } => {
            format!("a {} gets Reborn and is destroyed, you gain +{attack}/+{health}", tribe_name(tribe))
        }
        TargetKind::StealShopCard { damage } => {
            format!("steal a Tavern card (any card, the spell slot too), take {damage} damage")
        }
        TargetKind::BecomeCopy => "become a copy of a minion".into(),
        TargetKind::Spell(st) => format!("{st:?}"),
    }
}

fn kw_suffix(k: u8) -> String {
    let names: Vec<&str> = Keywords::ALL.iter().filter(|(b, _)| k & b != 0).map(|(_, n)| *n).collect();
    if names.is_empty() { String::new() } else { format!(" and {}", names.join(", ")) }
}

fn describe_option(o: &bgcore::cards::ChooseOption) -> String {
    use bgcore::cards::ChooseOption as C;
    match o {
        C::MinionTarget { tribe, attack, health, keyword } => format!(
            "give {} +{attack}/+{health}{}",
            tribe.map_or("a minion", |t| tribe_name(t)),
            kw_suffix(*keyword)
        ),
        C::SpellBonus { attack, health } => format!("Tavern spells give +{attack}/+{health} more this game"),
        C::SpellTarget(bgcore::cards::SpellTarget::Buff { attack, health, keyword, times, .. }) => {
            let again = if *times > 1 { format!(", {times} times") } else { String::new() };
            format!("give a minion +{attack}/+{health}{}{again}", kw_suffix(*keyword))
        }
        C::SpellEffect(bgcore::cards::SpellEffect::BuffBoard { attack, health, times, .. }) => {
            let again = if *times > 1 { format!(", {times} times") } else { String::new() };
            format!("give your minions +{attack}/+{health}{again}")
        }
        _ => "(see the card text above)".to_string(),
    }
}

/// Full text of one minion or spell for `?`.
pub fn card_info(g: &Game, texts: &CardTexts, m: &Minion) -> String {
    let db = &g.data.cards;
    let c = db.get(m.card);
    let mut out = String::new();
    if c.is_spell() {
        let _ = writeln!(out, "{}  (Tavern spell, tier {}, costs {})", c.name, c.tier, spell_cost(g, m.card));
    } else {
        let _ = writeln!(
            out,
            "{}{}  (tier {}, {})  {}/{}",
            if m.golden { "Golden " } else { "" },
            c.name,
            c.tier,
            card_tribes(c),
            m.attack,
            m.health
        );
        let kws: Vec<&str> = Keywords::ALL.iter().filter(|(b, _)| m.keywords.has(*b)).map(|(_, n)| *n).collect();
        if !kws.is_empty() {
            let _ = writeln!(out, "Keywords: {}", kws.join(", "));
        }
    }
    out.push_str(&card_text(texts, c));
    if m.golden {
        out.push_str("\nGolden: the numbers in the text are doubled.");
    }
    let mut state = Vec::new();
    if m.improvements > 0 {
        state.push(format!("improvements {}", m.improvements));
    }
    if m.magnetizations > 0 {
        state.push(format!("magnetizations {}", m.magnetizations));
    }
    if m.winners_bread > 0 {
        state.push(format!("Winner's Bread x{}", m.winners_bread));
    }
    if m.activated {
        state.push("Activate used this turn".into());
    }
    if m.locked_turns > 0 {
        state.push(format!("locked in hand for {} more turns", m.locked_turns));
    }
    if let Some(n) = m.events_left(db) {
        state.push(format!("{n} left"));
    }
    if m.n_attached > 0 {
        let names: Vec<String> = m
            .attached_slice()
            .iter()
            .map(|a| format!("{}{}", if a.golden { "golden " } else { "" }, db.get(a.card).name))
            .collect();
        state.push(format!("attached: {}", names.join(", ")));
    }
    if let Some(sp) = m.taught_spell {
        state.push(format!("taught {}", db.get(sp).name));
    }
    if !state.is_empty() {
        let _ = write!(out, "\nState: {}", state.join("; "));
    }
    out
}

/// A card's rules text: from the tsv when there, else its data fields.
pub fn card_text(texts: &CardTexts, c: &Card) -> String {
    if let Some(t) = texts.get(&c.name) {
        return format!("Text: {t}");
    }
    let mut parts: Vec<String> = Vec::new();
    macro_rules! field {
        ($($f:ident),*) => {$(
            if let Some(v) = &c.$f { parts.push(format!("{}: {:?}", stringify!($f), v)); }
        )*};
    }
    field!(battlecry, choose_one, deathrattle, start_of_turn, end_of_turn, start_of_combat, rally,
        avenge, activate, on_sell, aura, spell_target, spell_effect, on_event);
    if parts.is_empty() {
        "Text: (no card text available, ref/lobby5.tsv is missing)".into()
    } else {
        format!("Text (from the card data, ref/lobby5.tsv is missing):\n  {}", parts.join("\n  "))
    }
}

/// Plain card text for a card id, used for discover options.
pub fn card_info_by_id(g: &Game, texts: &CardTexts, id: CardId) -> String {
    let m = g.players[0].make_card(&g.data.cards, id, false, 0);
    card_info(g, texts, &m)
}
