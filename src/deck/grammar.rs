// ManaBox deck txt grammar.
//
// Sections start with `// NAME`; entries are `qty Name (SET) cn [*F*]` —
// the set/cn part and the foil marker are optional (plain deck lists omit
// them). Blank lines are separators and are not preserved — except inside
// `// COMMANDER`, where ManaBox exports one blank line between the
// commander(s) and the rest of the deck; that blank splits the section
// into `COMMANDER` and `DECK`. Parse → serialize is round-trip stable for
// entries ManaBox itself writes.

use anyhow::Context;

/// One deck entry: `qty Name (SET) cn [*F*]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckEntry {
    pub quantity: i64,
    pub name: String,
    /// Set code, when the line carried one.
    pub set_code: Option<String>,
    /// Collector number, when the line carried one.
    pub collector_number: Option<String>,
    /// `*F*` foil marker (ManaBox writes it for foil printings).
    pub foil: bool,
}

impl DeckEntry {
    /// Key for update math: name + set + cn + foil (print identity).
    ///
    /// Set codes are lowercased: ManaBox exports write them uppercase while
    /// the oracle snapshot stores them lowercase.
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.name,
            self.set_code.as_deref().unwrap_or("").to_ascii_lowercase(),
            self.collector_number.as_deref().unwrap_or(""),
            if self.foil { "F" } else { "" },
        )
    }

    /// The ManaBox txt line for this entry: `qty Name (SET) cn [*F*]`.
    pub fn to_line(&self) -> String {
        let mut line = format!("{} {}", self.quantity, self.name);
        if let Some(set) = &self.set_code {
            line.push_str(&format!(" ({set})"));
            if let Some(cn) = &self.collector_number {
                line.push_str(&format!(" {cn}"));
            }
        }
        if self.foil {
            line.push_str(" *F*");
        }
        line
    }
}

/// A parsed deck: ordered sections, each with ordered entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deck {
    /// `(section, entries)` in file order; entries within a section keep
    /// their line order.
    pub sections: Vec<(String, Vec<DeckEntry>)>,
}

impl Deck {
    /// Parse a ManaBox txt deck.
    ///
    /// Lines before the first `// SECTION` land in section `DECK` (ManaBox's
    /// own default). Blank lines are skipped — except inside `// COMMANDER`,
    /// where ManaBox exports separate the commander(s) from the rest of the
    /// deck with one blank line: the section splits into `COMMANDER` +
    /// `DECK` there.
    ///
    /// # Errors
    /// Fails on malformed quantity or entry syntax, with the line content.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let mut sections: Vec<(String, Vec<DeckEntry>)> = Vec::new();
        // Set by a blank line inside COMMANDER; ManaBox exports put one
        // between the commander(s) and the rest of the deck. The DECK
        // section materializes only when a card line follows — a blank
        // before the next `// HEADER` is just spacing.
        let mut split_pending = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                if sections
                    .last()
                    .is_some_and(|(s, e)| s.eq_ignore_ascii_case("COMMANDER") && !e.is_empty())
                {
                    split_pending = true;
                }
                continue;
            }
            if let Some(section) = line.strip_prefix("//") {
                let section = section.trim().to_string();
                anyhow::ensure!(!section.is_empty(), "empty section header: {line:?}");
                split_pending = false;
                sections.push((section, Vec::new()));
                continue;
            }
            let entry = parse_entry(line)?;
            if split_pending {
                split_pending = false;
                sections.push(("DECK".to_string(), Vec::new()));
            }
            match sections.last_mut() {
                Some((_, entries)) => entries.push(entry),
                // Entries before any section header go to ManaBox's default.
                None => sections.push(("DECK".to_string(), vec![entry])),
            }
        }
        Ok(Self { sections })
    }

    /// Find the index of a section (ASCII case-insensitive), if present.
    pub fn section_index(&self, name: &str) -> Option<usize> {
        self.sections
            .iter()
            .position(|(s, _)| s.eq_ignore_ascii_case(name))
    }

    /// Entries in a named section, creating an empty trailing section when
    /// absent (append semantics for update ops).
    pub fn section_entries_mut(&mut self, name: &str) -> &mut Vec<DeckEntry> {
        match self.section_index(name) {
            Some(i) => &mut self.sections[i].1,
            None => {
                self.sections.push((name.to_string(), Vec::new()));
                let last = self.sections.len() - 1;
                &mut self.sections[last].1
            }
        }
    }

    /// All entries across sections (in file order).
    pub fn entries(&self) -> impl Iterator<Item = &DeckEntry> {
        self.sections.iter().flat_map(|(_, e)| e.iter())
    }

    /// Total card count.
    pub fn total(&self) -> i64 {
        self.entries().map(|e| e.quantity).sum()
    }

    /// Card count excluding SIDEBOARD sections.
    ///
    /// For commander decks this is the legal deck (100 cards); the sideboard
    /// is a wishlist, not a legal zone. For 60-card formats this is the
    /// maindeck; the sideboard is subtracted from the deck size there too.
    pub fn maindeck_total(&self) -> i64 {
        self.sections
            .iter()
            .filter(|(s, _)| !s.eq_ignore_ascii_case("SIDEBOARD"))
            .flat_map(|(_, e)| e.iter())
            .map(|e| e.quantity)
            .sum()
    }

    /// Card count across SIDEBOARD sections only.
    pub fn sideboard_total(&self) -> i64 {
        self.sections
            .iter()
            .filter(|(s, _)| s.eq_ignore_ascii_case("SIDEBOARD"))
            .flat_map(|(_, e)| e.iter())
            .map(|e| e.quantity)
            .sum()
    }

    /// Serialize back to ManaBox txt.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, (section, entries)) in self.sections.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("// {section}\n"));
            for entry in entries {
                out.push_str(&entry.to_line());
                out.push('\n');
            }
        }
        out
    }
}

/// Parse one entry line: `qty Name [(SET) [cn]] [*F*]`.
///
/// The set/cn part and the foil marker are optional; the name is everything
/// between the quantity and the `(SET)` / `*F*` suffix.
///
/// # Errors
/// Fails on a missing/invalid quantity or an empty name.
pub fn parse_entry(line: &str) -> anyhow::Result<DeckEntry> {
    let line = line.trim();
    let (qty_str, rest) = line
        .split_once(' ')
        .with_context(|| format!("entry is missing a quantity: {line:?}"))?;
    let quantity: i64 = qty_str
        .trim()
        .parse()
        .with_context(|| format!("invalid quantity in entry: {line:?}"))?;
    anyhow::ensure!(quantity > 0, "non-positive quantity in entry: {line:?}");

    // Trailing foil marker.
    let (rest, foil) = match rest.trim().strip_suffix("*F*") {
        Some(rest) => (rest.trim_end(), true),
        None => (rest, false),
    };
    // Optional `(SET) cn` suffix (see [`split_set_suffix`] for the shape).
    let (name, set_code, collector_number) = split_set_suffix(rest);
    anyhow::ensure!(!name.is_empty(), "entry is missing a card name: {line:?}");
    Ok(DeckEntry {
        quantity,
        name,
        set_code,
        collector_number,
        foil,
    })
}

/// Split an optional `(SET) [cn]` suffix off the entry tail.
///
/// Card names can contain parentheses (e.g. `Crocodile (Piranha) Plant`), so
/// only ManaBox's exact suffix shape is treated as set info: `(CODE) CN`
/// where CODE is a short uppercase/digit token and CN is `372`, `28p`, or
/// `CON-31`. Anything else stays part of the name.
fn split_set_suffix(rest: &str) -> (String, Option<String>, Option<String>) {
    let rest = rest.trim_end();
    // Try `(SET) CN`, bare `(SET)`, then a set token glued to nothing.
    for words in [3usize, 2, 1] {
        if let Some((name, set, cn)) = strip_tail(rest, words) {
            return (name, Some(set), cn);
        }
    }
    (rest.trim().to_string(), None, None)
}

/// Match a tail of `words` whitespace-separated tokens; the name is the
/// prefix. The tail must be `(SET) [CN]`.
fn strip_tail(rest: &str, words: usize) -> Option<(String, String, Option<String>)> {
    // The candidate suffix is the last `words` tokens.
    let tail_start = rest.rmatch_indices(' ').nth(words.saturating_sub(1))?;
    let tail = &rest[tail_start.0 + 1..];
    let name = rest[..tail_start.0].trim_end();
    if name.is_empty() {
        return None;
    }
    let mut parts = tail.split_whitespace();
    let set_token = parts.next()?;
    let cn = match parts.next() {
        Some(cn) => {
            if parts.next().is_some() {
                return None;
            }
            Some(cn)
        }
        None => None,
    };
    let set = set_token.trim_matches(|c| c == '(' || c == ')');
    if !set_token.starts_with('(') || !set_token.ends_with(')') || !valid_set(set) {
        return None;
    }
    if let Some(cn) = &cn
        && !valid_collector_number(cn)
    {
        return None;
    }
    Some((name.to_string(), set.to_string(), cn.map(|c| c.to_string())))
}

/// Set codes are short uppercase/digit tokens (MH3, 2XM, PLST).
fn valid_set(set: &str) -> bool {
    (1..=6).contains(&set.len())
        && set
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !c.is_ascii_lowercase())
}

/// Collector numbers: `372`, `28p`, `CON-31`.
fn valid_collector_number(cn: &str) -> bool {
    (1..=8).contains(&cn.len()) && cn.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grammar_shapes() {
        let full = parse_entry("1 Breya, Etherium Shaper (MH3) 372 *F*").unwrap();
        assert_eq!(full.quantity, 1);
        assert_eq!(full.name, "Breya, Etherium Shaper");
        assert_eq!(full.set_code.as_deref(), Some("MH3"));
        assert_eq!(full.collector_number.as_deref(), Some("372"));
        assert!(full.foil);

        let no_foil = parse_entry("2 Island (SOS) 274").unwrap();
        assert_eq!(no_foil.quantity, 2);
        assert_eq!(no_foil.name, "Island");
        assert_eq!(no_foil.set_code.as_deref(), Some("SOS"));
        assert_eq!(no_foil.collector_number.as_deref(), Some("274"));
        assert!(!no_foil.foil);

        let set_only = parse_entry("1 Bolt (TST)").unwrap();
        assert_eq!(set_only.set_code.as_deref(), Some("TST"));
        assert_eq!(set_only.collector_number, None);

        let bare = parse_entry("3 Sol Ring").unwrap();
        assert_eq!(bare.name, "Sol Ring");
        assert_eq!(bare.set_code, None);
        assert!(!bare.foil);
    }

    #[test]
    fn parses_the_list_style_collector_numbers() {
        // Real ManaBox exports use hyphenated and promo numbers.
        for line in [
            "1 Master Transmuter (PLST) CON-31",
            "1 Oswald Fiddlebender (PAFR) 28p",
            "1 Time Sieve (PLST) ARB-31",
            "1 Fabricate (SLD) 7124",
        ] {
            let e = parse_entry(line).unwrap();
            assert!(e.set_code.is_some(), "{line}");
            assert!(e.collector_number.is_some(), "{line}");
        }
    }

    #[test]
    fn names_with_parens_stay_names() {
        // ManaBox card names can contain parentheses; a non-set-shaped tail
        // must not be eaten.
        let e = parse_entry("1 Crocodile (Piranha) Plant").unwrap();
        assert_eq!(e.name, "Crocodile (Piranha) Plant");
        assert_eq!(e.set_code, None);
        // Lowercase token is not a set code either.
        let e = parse_entry("1 Thing (abc) 1").unwrap();
        assert_eq!(e.name, "Thing (abc) 1");
        assert_eq!(e.set_code, None);
    }

    #[test]
    fn rejects_bad_entries() {
        for line in ["Bolt", "x2 Bolt", "0 Bolt", "-1 Bolt", "1 "] {
            assert!(parse_entry(line).is_err(), "{line:?} should fail");
        }
    }

    #[test]
    fn deck_sections_round_trip() {
        let text = "// COMMANDER\n1 Glarb, Calamity's Augur (BLB) 331\n\n// DECK\n2 Island (SOS) 274\n1 Bare Card\n";
        let deck = Deck::parse(text).unwrap();
        assert_eq!(deck.sections.len(), 2);
        assert_eq!(deck.sections[0].0, "COMMANDER");
        assert_eq!(deck.sections[0].1.len(), 1);
        assert_eq!(deck.sections[1].0, "DECK");
        assert_eq!(deck.sections[1].1.len(), 2);
        assert_eq!(deck.total(), 4);
        assert_eq!(
            deck.to_text(),
            "// COMMANDER\n1 Glarb, Calamity's Augur (BLB) 331\n\n// DECK\n2 Island (SOS) 274\n1 Bare Card\n"
        );
    }

    #[test]
    fn entries_before_first_section_get_default() {
        let deck = Deck::parse("1 Sol Ring\n2 Bolt").unwrap();
        assert_eq!(deck.sections.len(), 1);
        assert_eq!(deck.sections[0].0, "DECK");
        assert_eq!(deck.sections[0].1.len(), 2);
    }

    #[test]
    fn commander_blank_line_splits_into_deck_section() {
        // ManaBox export shape: commander(s), blank line, then the 99.
        let deck = Deck::parse("// COMMANDER\n1 Breya\n\n1 Island\n2 Bolt\n").unwrap();
        assert_eq!(deck.sections.len(), 2);
        assert_eq!(deck.sections[0].0, "COMMANDER");
        assert_eq!(deck.sections[0].1.len(), 1);
        assert_eq!(deck.sections[1].0, "DECK");
        assert_eq!(deck.sections[1].1.len(), 2);
        assert_eq!(deck.total(), 4);
    }

    #[test]
    fn blank_line_before_next_header_makes_no_empty_deck() {
        // A blank line right before another header is spacing, not a split.
        let deck = Deck::parse("// COMMANDER\n1 Breya\n\n// SIDEBOARD\n1 Bolt\n").unwrap();
        assert_eq!(deck.sections.len(), 2);
        assert_eq!(deck.sections[0].0, "COMMANDER");
        assert_eq!(deck.sections[0].1.len(), 1);
        assert_eq!(deck.sections[1].0, "SIDEBOARD");
        assert_eq!(deck.sections[1].1.len(), 1);
    }

    #[test]
    fn trailing_blank_line_makes_no_empty_deck() {
        // Blank lines around the whole list are spacing, not a split.
        let deck = Deck::parse("// COMMANDER\n1 Breya\n\n").unwrap();
        assert_eq!(deck.sections.len(), 1);
        assert_eq!(deck.sections[0].0, "COMMANDER");
        assert_eq!(deck.sections[0].1.len(), 1);
    }

    #[test]
    fn section_lookup_is_case_insensitive_and_append_creates() {
        let mut deck = Deck::parse("// COMMANDER\n1 Breya\n").unwrap();
        assert_eq!(deck.section_index("commander"), Some(0));
        deck.section_entries_mut("SIDEBOARD").push(DeckEntry {
            quantity: 3,
            name: "Bolt".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
        assert_eq!(deck.section_index("sideboard"), Some(1));
        assert_eq!(deck.total(), 4);
    }

    #[test]
    fn round_trip_preserves_all_fields() {
        let deck = Deck::parse(
            "// COMMANDER\n1 Breya, Etherium Shaper (MH3) 372 *F*\n// SIDEBOARD\n1 Whir of Invention (PLST) AER-49\n2 Bare Card\n",
        )
        .unwrap();
        let text = deck.to_text();
        let reparsed = Deck::parse(&text).unwrap();
        assert_eq!(deck, reparsed);
    }
}
