//! Card ids to card names.
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

/// Card ids to names, with the card id itself for an id no data has.
#[derive(Clone, Debug, Default)]
pub struct Cards {
    names: HashMap<String, String>,
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
        }
        let raw: Vec<RawCard> = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut names = HashMap::with_capacity(raw.len());
        for card in raw {
            if let Some(name) = card.name {
                names.insert(card.id, name);
            }
        }
        Ok(Cards { names })
    }

    /// Build the table from pairs, for tests.
    pub fn from_pairs<I: IntoIterator<Item = (String, String)>>(pairs: I) -> Cards {
        Cards {
            names: pairs.into_iter().collect(),
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
}
