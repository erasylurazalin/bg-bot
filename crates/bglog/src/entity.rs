//! One entity of a game: its card id and its tags.
//!
//! A port of the `Entity` of the older tracker's `powerlog.py`. Tags are keyed
//! the way the log spells them: names like `ZONE` stay names, and tags the
//! client has no name for keep their number as the key (`"2022"`). Values
//! that parse as a number become [`TagValue::Int`], everything else stays
//! text (`"PLAY"`, `"GT_BATTLEGROUNDS"`).

use std::collections::HashMap;

/// A tag value, as one of the two things the log spells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagValue {
    /// A number, the usual case (`CONTROLLER=1`, `tag=2022`).
    Int(i64),
    /// Anything else (`ZONE=PLAY`, `value=MAIN_START`).
    Text(String),
}

impl TagValue {
    /// The Python `_value`: the number when the text parses as one, the text
    /// otherwise.
    pub fn parse(text: &str) -> TagValue {
        match parse_int(text) {
            Some(n) => TagValue::Int(n),
            None => TagValue::Text(text.to_string()),
        }
    }

    /// The number, or 0 for a text value (the Python compares a text value to
    /// a number as unequal, and 0 never shows up as text).
    pub fn as_int(&self) -> i64 {
        match self {
            TagValue::Int(n) => *n,
            TagValue::Text(_) => 0,
        }
    }

    /// The text, or `None` for a number.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            TagValue::Text(s) => Some(s),
            TagValue::Int(_) => None,
        }
    }
}

impl From<i64> for TagValue {
    fn from(n: i64) -> TagValue {
        TagValue::Int(n)
    }
}

impl From<&str> for TagValue {
    fn from(s: &str) -> TagValue {
        TagValue::Text(s.to_string())
    }
}

/// The whole text as an int, the way Python's `int()` reads the log's numbers:
/// optional sign, then ASCII digits. Underscores, spaces and non-ASCII digits
/// never appear in the values we read.
fn parse_int(text: &str) -> Option<i64> {
    let digits = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// One entity: an id the log names it by, the card it is, and its tags.
///
/// The game creates entities as lines refer to them, so an entity can exist
/// with no tags at all yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entity {
    /// The entity id the log uses (`ID=25`, `id=287`). Never 0 in a real game.
    pub id: i32,
    /// The card id (`BG33_HERO_001`), or empty while the log has not said.
    pub card_id: String,
    /// The tags the log has set on it, the last value winning.
    pub tags: HashMap<String, TagValue>,
}

impl Entity {
    pub fn new(id: i32) -> Entity {
        Entity {
            id,
            card_id: String::new(),
            tags: HashMap::new(),
        }
    }

    /// The tag's value, or `None` when the log never set it.
    pub fn get(&self, tag: &str) -> Option<&TagValue> {
        self.tags.get(tag)
    }

    /// The tag's value, with the Python's default of `0` for a tag the log
    /// never set.
    pub fn value(&self, tag: &str) -> TagValue {
        self.tags.get(tag).cloned().unwrap_or(TagValue::Int(0))
    }

    /// The tag's number, or 0 when it is missing or text (the Python's
    /// `e["TAG"]` for a numeric tag, and for a missing one).
    pub fn int(&self, tag: &str) -> i64 {
        self.tags.get(tag).map(TagValue::as_int).unwrap_or(0)
    }

    /// The tag's text, or "" when it is missing or a number. Use it for the
    /// enum tags: `e.text("ZONE") == "PLAY"`.
    pub fn text(&self, tag: &str) -> &str {
        match self.tags.get(tag) {
            Some(TagValue::Text(s)) => s,
            _ => "",
        }
    }

    /// Set a tag, replacing what was there.
    pub fn set(&mut self, tag: &str, value: TagValue) {
        self.tags.insert(tag.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_text_split_the_way_the_python_does() {
        assert_eq!(TagValue::parse("5"), TagValue::Int(5));
        assert_eq!(TagValue::parse("0"), TagValue::Int(0));
        assert_eq!(TagValue::parse("-3"), TagValue::Int(-3));
        assert_eq!(TagValue::parse("1843925991"), TagValue::Int(1843925991));
        assert_eq!(
            TagValue::parse("PLAY"),
            TagValue::Text("PLAY".to_string())
        );
        // A tag name the client has no name for is a key, its value still a
        // number.
        assert_eq!(TagValue::parse(""), TagValue::Text(String::new()));
        assert_eq!(TagValue::parse("+"), TagValue::Text("+".to_string()));
    }

    #[test]
    fn a_missing_tag_reads_as_zero() {
        let mut e = Entity::new(7);
        assert_eq!(e.get("ZONE"), None);
        assert_eq!(e.value("ZONE"), TagValue::Int(0));
        assert_eq!(e.int("ZONE"), 0);
        assert_eq!(e.text("ZONE"), "");
        e.set("ZONE", TagValue::parse("PLAY"));
        assert_eq!(e.text("ZONE"), "PLAY");
        assert_eq!(e.int("ZONE"), 0);
    }
}
