//! The drill catalogue: what each difficulty draws from. Data only.

use crate::tutor::fingers::{Band, Reach, Spot};

/// A conjunctive predicate over key positions. `Group { band: Home, reach: Normal }` is the eight
/// resting keys — G, H and the modifier columns are excluded rather than argued about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub band: Option<Band>,
    pub reach: Option<Reach>,
}

impl Group {
    const fn at(band: Band, reach: Reach) -> Group {
        Group { band: Some(band), reach: Some(reach) }
    }

    const fn reaching(reach: Reach) -> Group {
        Group { band: None, reach: Some(reach) }
    }

    pub fn matches(self, spot: &Spot) -> bool {
        self.band.is_none_or(|b| b == spot.band) && self.reach.is_none_or(|r| r == spot.reach)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    Never,
    Allowed,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Words,
    Syllables,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Position,
    Programmer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Characters come from the keys these groups select, resolved through the live keymap. The
    /// alphabet is everything `include` selects; an item is kept only if it uses something from
    /// `focus`, so a drill mixes new keys with familiar ones yet always exercises the new ones.
    Keys { include: &'static [Group], focus: Option<Group> },
    /// Fixed snippets, filtered to the ones this keymap can type.
    Tokens(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drill {
    pub name: &'static str,
    pub kind: Kind,
    pub source: Source,
    pub shift: Shift,
    /// Ignored for `Source::Tokens`, which always emits whole snippets.
    pub style: Style,
}

/// An index into `DRILLS`.
pub type DrillId = usize;

pub fn drill(id: DrillId) -> &'static Drill {
    &DRILLS[id]
}

pub fn ids_of(kind: Kind) -> impl Iterator<Item = DrillId> {
    (0..DRILLS.len()).filter(move |&id| DRILLS[id].kind == kind)
}

const HOME: Group = Group::at(Band::Home, Reach::Normal);
const TOP: Group = Group::at(Band::Top, Reach::Normal);
const BOTTOM: Group = Group::at(Band::Bottom, Reach::Normal);
const NUMBER: Group = Group::at(Band::Number, Reach::Normal);
const OUTWARD: Group = Group::reaching(Reach::Outward);
const INWARD: Group = Group::reaching(Reach::Inward);

pub const DRILLS: &[Drill] = &[
    Drill {
        name: "Home keys",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME], focus: Some(HOME) },
        shift: Shift::Never,
        style: Style::Syllables,
    },
    Drill {
        name: "Stretch up",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, TOP], focus: Some(TOP) },
        shift: Shift::Never,
        style: Style::Words,
    },
    // Letter groups, not words: the bottom row is z x c v m , . / and English barely uses
    // those letters together — the word list yields six. The Words fallback exists for
    // surprises, not for a standard drill that would apologise on every batch.
    Drill {
        name: "Stretch down",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, BOTTOM], focus: Some(BOTTOM) },
        shift: Shift::Never,
        style: Style::Syllables,
    },
    Drill {
        name: "Number row",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, NUMBER], focus: Some(NUMBER) },
        shift: Shift::Never,
        style: Style::Syllables,
    },
    // Right-pinky only on the reference board, and permanently so: the left outer column is
    // Esc/Tab/Ctrl/Shift, which type nothing. Still position-driven, so it would pick up a
    // character if one ever appeared there.
    Drill {
        name: "Outer column",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, OUTWARD], focus: Some(OUTWARD) },
        shift: Shift::Allowed,
        style: Style::Syllables,
    },
    Drill {
        name: "Index reach",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, INWARD], focus: Some(INWARD) },
        shift: Shift::Never,
        style: Style::Words,
    },
    // No focus group: `Shift::Required` already guarantees every item uses Shift, which is the
    // whole point of the drill.
    Drill {
        name: "Shift combinations",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, TOP, BOTTOM, NUMBER], focus: None },
        shift: Shift::Required,
        style: Style::Words,
    },
    Drill { name: "Markdown", kind: Kind::Programmer, source: Source::Tokens(MARKDOWN), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "HTML", kind: Kind::Programmer, source: Source::Tokens(HTML), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "Rust", kind: Kind::Programmer, source: Source::Tokens(RUST), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "TypeScript", kind: Kind::Programmer, source: Source::Tokens(TYPESCRIPT), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "Elixir", kind: Kind::Programmer, source: Source::Tokens(ELIXIR), shift: Shift::Allowed, style: Style::Words },
];

const MARKDOWN: &[&str] = &[
    "# Heading", "## Section", "### Detail", "**bold**", "_italic_", "`code`", "[text](url)",
    "![alt](img.png)", "- [ ]", "- [x]", "> quote", "---", "|---|---|", "1. first", "2. second",
    "~~struck~~", "&nbsp;", "```rust", "<!-- note -->", "* bullet",
];

const HTML: &[&str] = &[
    "<div>", "</div>", "<span>", "</span>", "<p>", "</p>", "<ul>", "<li>", "<br />", "<hr />",
    r##"<a href="#">"##, r#"class="row""#, r#"id="main""#, r#"<input type="text">"#,
    "<!-- note -->", "&amp;", "&lt;", "&gt;", "</html>", "<h1>",
];

const RUST: &[&str] = &[
    "fn main()", "let mut x = 0;", "-> Result<(), E>", "&mut self", "Vec<u8>", "Option<&str>",
    "impl Trait", "match x {", "=> {}", "#[derive(Debug)]", "|x| x + 1", "0..=9", "self.field",
    "::<T>", "?;", "&[u8]", "if let Some(v)", "pub(crate)", "'static", "format!(\"{x}\")",
];

const TYPESCRIPT: &[&str] = &[
    "const x = 1;", "=> {}", "?.", "??", ": string", "<T>", "${value}", "async () =>", "await fn()",
    "...rest", "interface X {", "export default", r#"import { a } from "b";"#, "as const",
    "!== null", "Array<number>", "type Id = string;", "readonly", "#private", "obj?.key",
];

const ELIXIR: &[&str] = &[
    "|>", "->", "defmodule X do", "def run(x) do", "end", "%{key: 1}", ":atom", "<<1, 2>>",
    "fn x -> x end", "case x do", "{:ok, v}", "{:error, e}", "=~", "@moduledoc", "&1",
    "Enum.map(list)", "|> Enum.filter()", "do:", "..", "when is_map(x)",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostlayout::HostLayout;
    use crate::tutor::fingers;
    use crate::tutor::fixture::reference_keymap;
    use crate::tutor::hint;

    #[test]
    fn the_catalogue_is_the_twelve_drills_the_spec_lists() {
        assert_eq!(DRILLS.len(), 12);
        assert_eq!(ids_of(Kind::Position).count(), 7);
        assert_eq!(ids_of(Kind::Programmer).count(), 5);
        assert_eq!(drill(0).name, "Home keys");
        let names: Vec<&str> = DRILLS.iter().map(|d| d.name).collect();
        for expected in ["Stretch up", "Stretch down", "Number row", "Outer column", "Index reach",
                         "Shift combinations", "Markdown", "HTML", "Rust", "TypeScript", "Elixir"] {
            assert!(names.contains(&expected), "{expected} is missing");
        }
    }

    /// An item has to be buildable: every key the focus selects must also be in `include`, or the
    /// drill could never satisfy its own rule.
    #[test]
    fn every_focus_group_is_inside_its_include_list() {
        for d in DRILLS {
            let Source::Keys { include, focus: Some(focus) } = d.source else { continue };
            for row in 0..fingers::ROWS {
                for col in 0..fingers::COLS {
                    let Some(spot) = fingers::spot(row, col) else { continue };
                    if focus.matches(&spot) {
                        assert!(include.iter().any(|g| g.matches(&spot)), "{}: ({row}, {col}) is focused but not included", d.name);
                    }
                }
            }
        }
    }

    /// The shipped snippets have to be typeable, or the drill quietly shrinks.
    #[test]
    fn every_token_can_be_typed_on_the_reference_keymap() {
        let km = reference_keymap();
        for d in DRILLS {
            let Source::Tokens(tokens) = d.source else { continue };
            assert!(!tokens.is_empty(), "{} has no tokens", d.name);
            for token in tokens {
                for c in token.chars() {
                    assert!(hint::resolve(&km, HostLayout::Gb, c).is_some(), "{}: {token:?} needs {c:?}", d.name);
                }
            }
        }
    }

    #[test]
    fn home_keys_selects_the_eight_resting_keys() {
        let Source::Keys { focus: Some(home), .. } = drill(0).source else { panic!("Home keys uses key groups") };
        let selected: Vec<(u8, u8)> = (0..fingers::ROWS)
            .flat_map(|r| (0..fingers::COLS).map(move |c| (r, c)))
            .filter(|&(r, c)| fingers::spot(r, c).is_some_and(|s| home.matches(&s)))
            .collect();
        assert_eq!(selected.len(), 8, "four fingers on each hand, no stretches: {selected:?}");
    }
}
