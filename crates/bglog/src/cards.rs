//! Card ids to card names and tribes.
//!
//! The tracker's `card_name()` asks python-hearthstone's `cardxml` for the name
//! of every card it prints, and falls back to the card id for an id no data has
//! (`bg.powerlog` / `bg.battlegrounds`).
//!
//! Ours reads `ref/hsjson-cards.json`, HearthstoneJSON's whole CardDefs
//! (`docs/card-data.md`), which holds every card the tracker names, hero skins
//! included: `ref/bg-cards.json` and `ref/bg-meta.json` together are missing
//! 125 of the 1,313 ids the Battlegrounds views ask about in Era's 20 logs.

use std::collections::HashMap;

/// The ten tribes, named the way the engine spells them, from the card data's
/// race names (the tracker's `TRIBE_NAMES`).
const TRIBES: [(&str, &str); 10] = [
    ("beast", "beast"),
    ("dragon", "dragon"),
    ("murloc", "murloc"),
    ("demon", "demon"),
    ("pirate", "pirate"),
    ("elemental", "elemental"),
    ("undead", "undead"),
    ("quilboar", "quilboar"),
    ("aberration", "aberration"),
    // The data spells it `MECHANICAL` and the engine `mech`.
    ("mechanical", "mech"),
];

/// Card ids to names and tribes, with the card id itself for a name no data
/// has.
#[derive(Clone, Debug, Default)]
pub struct Cards {
    names: HashMap<String, String>,
    tribes: HashMap<String, Vec<String>>,
}

impl Cards {
    /// Read a HearthstoneJSON `cards.json`: an array of objects with `id` and
    /// `name` (the file is `ref/hsjson-cards.json`, git-ignored).
    pub fn load(path: &str) -> Result<Cards, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        Cards::parse(&text).map_err(|e| format!("{path}: {e}"))
    }

    /// The same, from the text of the file.
    pub fn parse(json: &str) -> Result<Cards, String> {
        #[derive(serde::Deserialize)]
        struct RawCard {
            id: String,
            #[serde(default)]
            name: Option<String>,
            #[serde(default)]
            race: Option<String>,
            #[serde(default)]
            races: Option<Vec<String>>,
        }
        let raw: Vec<RawCard> = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut names = HashMap::with_capacity(raw.len());
        let mut tribes = HashMap::new();
        for card in raw {
            if let Some(name) = card.name {
                names.insert(card.id.clone(), name);
            }
            // `races` holds every tribe of a card and `race` the first, so the
            // array is the one to read when the data has it.
            let races = card.races.or(card.race.map(|race| vec![race]));
            let tribes_of: Vec<String> = races
                .unwrap_or_default()
                .iter()
                .filter_map(|race| tribe(race))
                .map(str::to_string)
                .collect();
            if !tribes_of.is_empty() {
                tribes.insert(card.id, tribes_of);
            }
        }
        Ok(Cards { names, tribes })
    }

    /// Build the table from pairs, for tests.
    pub fn from_pairs<I: IntoIterator<Item = (String, String)>>(pairs: I) -> Cards {
        Cards {
            names: pairs.into_iter().collect(),
            tribes: HashMap::new(),
        }
    }

    /// The card's tribes, as the engine names them, in the order the data
    /// lists them, or nothing when the card has no tribe the engine knows
    /// (a spell, a hero, or an id no data has).
    pub fn tribes(&self, card_id: &str) -> &[String] {
        match self.tribes.get(card_id) {
            Some(tribes) => tribes,
            None => &[],
        }
    }

    /// The card's name, or the card id when no data names it (a placeholder
    /// card, or a tag the data cannot know).
    pub fn name<'a>(&'a self, card_id: &'a str) -> &'a str {
        match self.names.get(card_id) {
            Some(name) => name,
            None => card_id,
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// One race name from the card data, in the engine's spelling.
fn tribe(race: &str) -> Option<&'static str> {
    let race = race.to_ascii_lowercase();
    TRIBES
        .iter()
        .find(|(data, _)| *data == race)
        .map(|(_, tribe)| *tribe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fall_back_to_the_card_id() {
        let cards = Cards::parse(
            r#"[{"id": "BG36_345", "name": "Suspicious Prisonguard"},
                {"id": "BG30_Trinket_1st", "name": "Lesser Trinket"},
                {"id": "BG_NONAME"}]"#,
        )
        .unwrap();
        assert_eq!(cards.name("BG36_345"), "Suspicious Prisonguard");
        assert_eq!(cards.name("BG30_Trinket_1st"), "Lesser Trinket");
        // No name in the data: the id, as the Python does.
        assert_eq!(cards.name("BG_NONAME"), "BG_NONAME");
        assert_eq!(cards.name("BG27_556_Gpe"), "BG27_556_Gpe");
    }

    #[test]
    fn tribes_come_from_the_races_the_data_lists() {
        let cards = Cards::parse(
            r#"[{"id": "BG31_808", "name": "Runed Progenitor", "race": "BEAST", "races": ["BEAST"]},
                {"id": "BG_MULTI", "name": "Two Tribes", "race": "BEAST", "races": ["BEAST", "MURLOC"]},
                {"id": "BG_OLD", "name": "Race Only", "race": "MECHANICAL"},
                {"id": "BG_AMALGAM", "name": "All of Them", "races": ["ALL"]},
                {"id": "BG_SPELL", "name": "A Spell", "type": "SPELL"}]"#,
        )
        .unwrap();
        assert_eq!(cards.tribes("BG31_808"), ["beast"]);
        assert_eq!(cards.tribes("BG_MULTI"), ["beast", "murloc"]);
        // The engine's word for the data's `MECHANICAL`.
        assert_eq!(cards.tribes("BG_OLD"), ["mech"]);
        // A race the engine has no tribe for, a spell, and an id no data has.
        assert!(cards.tribes("BG_AMALGAM").is_empty());
        assert!(cards.tribes("BG_SPELL").is_empty());
        assert!(cards.tribes("BG_NOWHERE").is_empty());
    }
}
