//! The client's own words about my decisions.
//!
//! A power log says what happened, not what I was offered. The client prints
//! that separately, in four `GameState` calls the power reader drops:
//! `DebugPrintOptions` (the buttons I could press and what they could be
//! aimed at), `SendOption` (the one I pressed), `DebugPrintEntityChoices` (the
//! cards of a Discover or a Choose One) and `SendChoices` (the card I took).
//!
//! Ported from the reading side of `tools/collect_actions.py`: the same
//! reference grammar and the same field order. What a step *means* is the
//! caller's job; this module only spells out what the client said.

use std::collections::BTreeMap;

/// An entity as the client spells it out:
/// `[entityName=Bob id=1 zone=PLAY zonePos=0 cardId=BG_1 player=6]`.
///
/// The name is dropped: nothing downstream reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    /// `id=`: the entity's number.
    pub id: i32,
    /// `zone=`: where it sat when the line was written, which can lag a step
    /// behind the live table.
    pub zone: String,
    /// `zonePos=`: its position in that zone.
    pub pos: i64,
    /// `cardId=`: its card, empty when the log does not say.
    pub card_id: String,
    /// `player=`: whose it is.
    pub player: i32,
}

impl Ref {
    /// Read the reference in `text`: the leftmost `[entityName=`, then the
    /// last ` id=` that leaves a whole reference behind it (the name is
    /// greedy, as in the Python's `REF.search`). Text before and after the
    /// reference is allowed.
    pub fn parse(text: &str) -> Option<Ref> {
        let at = text.find("[entityName=")?;
        let rest = &text[at + "[entityName=".len()..];
        let mut end = rest.len();
        loop {
            let at = rest[..end].rfind(" id=")?;
            if let Some(r) = ref_tail(&rest[at..]) {
                return Some(r);
            }
            if at == 0 {
                return None;
            }
            end = at;
        }
    }

    /// An entity known only by its number. A dump can name a target it does
    /// not list, and then the number is all there is.
    pub fn only(id: i32) -> Ref {
        Ref {
            id,
            zone: String::new(),
            pos: 0,
            card_id: String::new(),
            player: 0,
        }
    }
}

/// ` id=<id> zone=<zone> zonePos=<pos> cardId=<card> player=<player>]` running
/// to the end of `rest`.
fn ref_tail(rest: &str) -> Option<Ref> {
    let rest = rest.strip_prefix(" id=")?;
    let (id, rest) = number(rest)?;
    let rest = rest.strip_prefix(" zone=")?;
    let (zone, rest) = token(rest);
    if zone.is_empty() {
        return None;
    }
    let rest = rest.strip_prefix(" zonePos=")?;
    let (pos, rest) = number(rest)?;
    let rest = rest.strip_prefix(" cardId=")?;
    let (card_id, rest) = token(rest);
    let rest = rest.strip_prefix(" player=")?;
    let (player, rest) = number(rest)?;
    if !rest.strip_prefix(']')?.is_empty() {
        return None;
    }
    Some(Ref {
        id: i32::try_from(id).ok()?,
        zone: zone.to_string(),
        pos,
        card_id: card_id.to_string(),
        player: i32::try_from(player).ok()?,
    })
}

/// A run of digits at the start of `rest`, and what follows it.
fn number(rest: &str) -> Option<(i64, &str)> {
    let len = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    if len == 0 {
        return None;
    }
    Some((rest[..len].parse().ok()?, &rest[len..]))
}

/// The run of non-space characters at the start of `rest`, and what follows.
/// The run may be empty: `cardId=(\S*)` is allowed to be.
fn token(rest: &str) -> (&str, &str) {
    let len = rest.find(' ').unwrap_or(rest.len());
    (&rest[..len], &rest[len..])
}

/// A run of word characters at the start of `rest`, and what follows.
fn word(rest: &str) -> Option<(&str, &str)> {
    let len = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    if len == 0 {
        return None;
    }
    Some((&rest[..len], &rest[len..]))
}

/// The text of `body` after `marker`, when `marker` sits behind at least
/// `spaces` spaces and nothing else. The client indents an option, and the
/// lines under a choice, by two spaces, and a target of an option by four.
fn indented<'a>(body: &'a str, marker: &str, spaces: usize) -> Option<&'a str> {
    let mut from = 0;
    loop {
        let at = body[from..].find(marker)? + from;
        let before = &body[..at];
        if before.len() >= spaces && before.bytes().all(|b| b == b' ') {
            return Some(&body[at + marker.len()..]);
        }
        from = at + marker.len();
    }
}

/// `id=<n>`: the first line of an options dump, and the model of every other.
fn id_line(body: &str) -> Option<i64> {
    let (id, rest) = number(body.strip_prefix("id=")?)?;
    rest.is_empty().then_some(id)
}

/// One of the things the game offers me, with the entities it can be aimed
/// at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// `option <n>`: the number `SendOption` names it by.
    pub index: i64,
    /// `type=`: `END_TURN`, `POWER`, `CANCEL` and the rest.
    pub kind: String,
    /// `mainEntity=`: what it acts on, when it names one.
    pub entity: Option<Ref>,
    /// `target <n> entity=<ref>`: what it can be aimed at, by number.
    pub targets: BTreeMap<i64, Ref>,
}

/// The options dump: what the game offered me, by the number of each offer.
#[derive(Clone, Debug, Default)]
pub struct Options {
    offers: BTreeMap<i64, Offer>,
}

impl Options {
    /// Read one line of a dump, keeping what it says. Returns whether the line
    /// starts a new dump: the caller closes the step the last one belonged to
    /// and calls [`Options::clear`].
    pub fn read(&mut self, body: &str) -> bool {
        if id_line(body).is_some() {
            return true;
        }
        if let Some(offer) = offer_line(body) {
            self.offers.insert(offer.index, offer);
        } else if let Some((index, entity)) = target_line(body)
            && let Some(entity) = entity
            && let Some((_, last)) = self.offers.iter_mut().next_back()
        {
            // A target belongs to the option listed above it, which is always
            // the last one added.
            last.targets.insert(index, entity);
        }
        false
    }

    /// Forget the dump.
    pub fn clear(&mut self) {
        self.offers.clear();
    }

    /// The offer numbered `index`.
    pub fn get(&self, index: i64) -> Option<&Offer> {
        self.offers.get(&index)
    }

    /// Whether the dump listed anything.
    pub fn is_empty(&self) -> bool {
        self.offers.is_empty()
    }
}

/// `  option <n> type=<type> mainEntity=<ref> error=<e> errorParam=<p>`.
fn offer_line(body: &str) -> Option<Offer> {
    let rest = indented(body, "option ", 2)?;
    let (index, rest) = number(rest)?;
    let rest = rest.strip_prefix(" type=")?;
    let (kind, rest) = word(rest)?;
    let rest = rest.strip_prefix(" mainEntity=")?;
    let entity = up_to_error(rest)?;
    Some(Offer {
        index,
        kind: kind.to_string(),
        entity: Ref::parse(entity),
        targets: BTreeMap::new(),
    })
}

/// `    target <n> entity=<ref> error=<e> errorParam=<p>`.
fn target_line(body: &str) -> Option<(i64, Option<Ref>)> {
    let rest = indented(body, "target ", 4)?;
    let (index, rest) = number(rest)?;
    let rest = rest.strip_prefix(" entity=")?;
    Some((index, Ref::parse(up_to_error(rest)?)))
}

/// The text before the ` error=<word> errorParam=` that ends a dump line: the
/// entity reference, which may be empty, as it is for an `END_TURN`.
fn up_to_error(rest: &str) -> Option<&str> {
    let at = rest.find(" error=")?;
    let (_, tail) = word(&rest[at + " error=".len()..])?;
    tail.strip_prefix(" errorParam=")?;
    Some(&rest[..at])
}

/// `id=<n> Player=<name> TaskList=<tasks> ChoiceType=<type> CountMin=<a>
/// CountMax=<b>`: a new choice. `general` says whether it is a shop choice;
/// the mulligan's hero picks are not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Start {
    /// `id=`: the choice's number.
    pub id: i64,
    /// `ChoiceType=GENERAL`.
    pub general: bool,
}

/// Read the first line of a `DebugPrintEntityChoices` dump.
pub fn choice_start(body: &str) -> Option<Start> {
    let (id, rest) = number(body.strip_prefix("id=")?)?;
    let rest = rest.strip_prefix(" Player=")?;
    let at = rest.rfind(" TaskList=")?;
    let rest = &rest[at + " TaskList=".len()..];
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.strip_prefix(" ChoiceType=")?;
    let (kind, rest) = word(rest)?;
    let rest = rest.strip_prefix(" CountMin=")?;
    let (_, rest) = number(rest)?;
    let rest = rest.strip_prefix(" CountMax=")?;
    let (_, rest) = number(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(Start {
        id,
        general: kind == "GENERAL",
    })
}

/// `selectedOption=<n> selectedSubOption=<n> selectedTarget=<n>
/// selectedPosition=<n>`: the option the client took, the sub option and the
/// target it took it with. The position the click named is not read.
pub fn taken(body: &str) -> Option<(i64, i64, i64)> {
    let rest = body.strip_prefix("selectedOption=")?;
    let (index, rest) = signed(rest)?;
    let (sub, rest) = signed(rest.strip_prefix(" selectedSubOption=")?)?;
    let (target, rest) = signed(rest.strip_prefix(" selectedTarget=")?)?;
    let (_, rest) = signed(rest.strip_prefix(" selectedPosition=")?)?;
    rest.is_empty().then_some((index, sub, target))
}

/// A number, negative ones included: `-1` for a sub option the client did not
/// name.
fn signed(rest: &str) -> Option<(i64, &str)> {
    match rest.strip_prefix('-') {
        Some(rest) => {
            let (n, rest) = number(rest)?;
            Some((-n, rest))
        }
        None => number(rest),
    }
}

/// `id=<n> ChoiceType=<type>`: the pick, which ends the choice.
pub fn pick_start(body: &str) -> Option<(i64, &str)> {
    let (id, rest) = number(body.strip_prefix("id=")?)?;
    let (kind, rest) = word(rest.strip_prefix(" ChoiceType=")?)?;
    rest.is_empty().then_some((id, kind))
}

/// A choice the game offered me: the cards of a Discover, a Choose One, the
/// Dark Discovery, a trinket's two, and the one I picked.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    /// `Source=`: the card that offered the choice, when one did.
    pub source: Option<Ref>,
    /// `Entities[<n>]=`: the cards offered, by number.
    pub entities: BTreeMap<i64, Ref>,
    /// `m_chosenEntities[<n>]=`: the card I took.
    pub pick: Option<Ref>,
}

impl Choice {
    /// Read one of the lines under `DebugPrintEntityChoices`, keeping what it
    /// says. Returns whether it was one of them.
    pub fn read(&mut self, body: &str) -> bool {
        if let Some(text) = indented(body, "Source=", 2) {
            self.source = Ref::parse(text);
        } else if let Some((index, text)) = numbered(body, "Entities[") {
            if let Some(r) = Ref::parse(text) {
                self.entities.insert(index, r);
            }
        } else {
            return false;
        }
        true
    }

    /// Read one of the lines under `SendChoices`, keeping the pick. Returns
    /// whether it was one of them.
    pub fn read_pick(&mut self, body: &str) -> bool {
        let Some((_, text)) = numbered(body, "m_chosenEntities[") else {
            return false;
        };
        if let Some(r) = Ref::parse(text) {
            self.pick = Some(r);
        }
        true
    }
}

/// `  <marker><n>]=<text>`: a numbered line under a choice.
fn numbered<'a>(body: &'a str, marker: &str) -> Option<(i64, &'a str)> {
    let rest = indented(body, marker, 2)?;
    let (index, rest) = number(rest)?;
    Some((index, rest.strip_prefix("]=")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_read_with_and_without_surrounding_text() {
        let r = Ref::parse("[entityName=Bob id=12 zone=PLAY zonePos=3 cardId=BG_1 player=6]")
            .unwrap();
        assert_eq!(r.id, 12);
        assert_eq!(r.zone, "PLAY");
        assert_eq!(r.pos, 3);
        assert_eq!(r.card_id, "BG_1");
        assert_eq!(r.player, 6);
        let r =
            Ref::parse("Entity=[entityName=A id=1 zone=HAND zonePos=1 cardId= player=2]").unwrap();
        assert_eq!(r.card_id, "");
        // The reference ends the text, as in the Python's `$`.
        assert!(Ref::parse("[entityName=A id=1 zone=HAND zonePos=1 cardId=A player=2] tail").is_none());
        assert!(Ref::parse("Entity=Bob").is_none());
        assert!(Ref::parse("").is_none());
    }

    #[test]
    fn a_greedy_name_gives_way_to_the_last_whole_reference() {
        // The name holds a well formed reference, so the first ` id=` does not
        // leave a whole reference behind it and the outer one wins.
        let r = Ref::parse("[entityName=x id=1 zone=PLAY zonePos=0 cardId=A player=2 id=9 zone=HAND zonePos=1 cardId=B player=3]")
            .unwrap();
        assert_eq!(r.id, 9);
        assert_eq!(r.card_id, "B");
    }

    #[test]
    fn an_options_dump_is_read_offer_by_offer() {
        let mut options = Options::default();
        assert!(options.read("id=3"));
        options.clear();
        assert!(options.is_empty());
        assert!(!options.read("  option 0 type=END_TURN mainEntity= error=INVALID errorParam="));
        assert_eq!(options.get(0).unwrap().kind, "END_TURN");
        assert_eq!(options.get(0).unwrap().entity, None);
        assert!(
            !options.read(
                "  option 1 type=POWER mainEntity=[entityName=Bob id=77 zone=PLAY zonePos=0 cardId=BG_1 player=6] error=NONE errorParam="
            )
        );
        assert_eq!(options.get(1).unwrap().entity.as_ref().unwrap().id, 77);
        assert!(
            !options.read(
                "    target 0 entity=[entityName=Bob id=78 zone=PLAY zonePos=1 cardId=BG_2 player=6] error=NONE errorParam="
            )
        );
        assert_eq!(
            options.get(1).unwrap().targets.get(&0).unwrap().card_id,
            "BG_2"
        );
        assert!(options.get(0).unwrap().targets.is_empty());
        // A target with no option above it goes nowhere.
        let mut fresh = Options::default();
        assert!(!fresh.read("    target 0 entity=[entityName=x id=5 zone=PLAY zonePos=0 cardId=A player=1] error=NONE errorParam="));
        assert!(fresh.is_empty());
    }

    #[test]
    fn a_choice_lists_its_source_its_cards_and_the_pick() {
        let start = choice_start(
            "id=2 Player=Someone TaskList= ChoiceType=GENERAL CountMin=1 CountMax=1",
        )
        .unwrap();
        assert!(start.general);
        assert_eq!(start.id, 2);
        let mulligan = choice_start(
            "id=1 Player=Someone TaskList=7 ChoiceType=MULLIGAN CountMin=1 CountMax=1",
        )
        .unwrap();
        assert!(!mulligan.general);
        assert!(choice_start("id=1 Player=Someone TaskList= ChoiceType=GENERAL").is_none());

        let mut choice = Choice::default();
        assert!(choice.read("  Source=[entityName=Bob id=40 zone=PLAY zonePos=0 cardId=BG_9 player=6]"));
        assert_eq!(choice.source.as_ref().unwrap().id, 40);
        assert!(choice.read("  Entities[0]=[entityName=Bob id=41 zone=SETASIDE zonePos=0 cardId=BG_3 player=6]"));
        assert!(choice.read("  Entities[1]=[entityName=Bob id=42 zone=SETASIDE zonePos=0 cardId=BG_4 player=6]"));
        // A card the log scribbles on is not read, and takes no place.
        assert!(choice.read(
            "  Entities[2]=[entityName=Bob id=43 zone=SETASIDE zonePos=0 cardId=BG_5 player=6] errors"
        ));
        assert_eq!(choice.entities.len(), 2);
        assert_eq!(choice.pick, None);

        assert_eq!(pick_start("id=2 ChoiceType=GENERAL"), Some((2, "GENERAL")));
        assert!(pick_start("id=2 ChoiceType=GENERAL extra").is_none());
        assert!(choice.read_pick("  m_chosenEntities[0]=[entityName=Bob id=41 zone=HAND zonePos=1 cardId=BG_3 player=6]"));
        assert_eq!(choice.pick.as_ref().unwrap().id, 41);
        // A pick the log scribbles on leaves the one already read.
        assert!(choice.read_pick(
            "  m_chosenEntities[0]=[entityName=Bob id=41 zone=HAND pointer=1]"
        ));
        assert_eq!(choice.pick.as_ref().unwrap().id, 41);
    }

    #[test]
    fn the_option_taken_is_read_with_its_sub_option_and_target() {
        assert_eq!(
            taken("selectedOption=7 selectedSubOption=-1 selectedTarget=0 selectedPosition=3"),
            Some((7, -1, 0))
        );
        assert_eq!(
            taken("selectedOption=1 selectedSubOption=2 selectedTarget=291 selectedPosition=0"),
            Some((1, 2, 291))
        );
        // A line the log scribbles on is not read, as the Python's `$` refuses it.
        assert!(taken("selectedOption=3 selectedSubOption=-1 selectedTarget=0 selectedPosition=1 tail")
            .is_none());
        assert!(taken("selectedOption=3 selectedSubOption=-1 selectedTarget=0").is_none());
    }
}
