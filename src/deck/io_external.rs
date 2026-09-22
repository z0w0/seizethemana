// External decklist formats: Moxfield, Archidekt, and Arena txt shapes,
// plus the export renderers. Parsing normalizes every format into the
// internal `Deck` before `normalize_sections` runs, so commander
// inference, primers, and ownership notes work unchanged.

use super::grammar::{Deck, DeckEntry};

/// A decklist format the import accepts or the export emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckFormat {
    /// ManaBox txt (`// SECTION`, `qty Name (SET) cn *F*`).
    ManaBox,
    /// Moxfield txt (`2x Name (set) cn *F*`, `//Deck`-style headers).
    Moxfield,
    /// Archidekt txt (`1x Name [set] cn *F*`).
    Archidekt,
    /// Plain `qty Name` lines (the shim-free diff feed).
    Names,
    /// Arena txt (bare `Deck`/`Commander`/`Sideboard` headers, `2 Name (set) cn`).
    Arena,
}

impl DeckFormat {
    /// The flag string this format accepts on import/export.
    pub fn key(self) -> &'static str {
        match self {
            DeckFormat::ManaBox => "manabox",
            DeckFormat::Moxfield => "moxfield",
            DeckFormat::Archidekt => "archidekt",
            DeckFormat::Names => "names",
            DeckFormat::Arena => "arena",
        }
    }

    /// Every exportable format key, for the error hint.
    pub fn all_keys() -> &'static [&'static str] {
        &["manabox", "names", "moxfield", "archidekt", "arena"]
    }

    /// Resolve a `--format` key.
    pub fn parse(key: &str) -> Option<DeckFormat> {
        match key.to_ascii_lowercase().as_str() {
            "manabox" => Some(DeckFormat::ManaBox),
            "moxfield" => Some(DeckFormat::Moxfield),
            "archidekt" => Some(DeckFormat::Archidekt),
            "names" => Some(DeckFormat::Names),
            "arena" => Some(DeckFormat::Arena),
            _ => None,
        }
    }
}

/// Guess the decklist format from the text: header and line sniffing.
/// ManaBox (the native shape) wins ties; unknown text falls back to
/// ManaBox so error messages stay the familiar grammar ones.
pub fn detect(text: &str) -> DeckFormat {
    let mut moxfield = 0usize;
    let mut archidekt = 0usize;
    let mut arena_headers = 0usize;
    let mut manabox_headers = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if line.starts_with("//") {
            // ManaBox writes `// DECK`-style headers; Moxfield writes
            // `//Deck` (no space). Anything else is ManaBox too.
            let header = lower.trim_start_matches('/').trim();
            if !header.contains(' ')
                && (header.starts_with("deck")
                    || header.starts_with("sideboard")
                    || header.starts_with("commanders"))
            {
                moxfield += 1;
            } else {
                manabox_headers += 1;
            }
            continue;
        }
        // Bare-word section headers (Arena shape).
        if !lower.contains(' ')
            && matches!(
                lower.as_str(),
                "deck" | "mainboard" | "maindeck" | "commander" | "commanders" | "sideboard"
            )
        {
            arena_headers += 1;
            continue;
        }
        // `2x Name` quantity prefixes (Moxfield/Archidekt).
        if let Some((qty, _)) = line.split_once("x ")
            && qty.trim().parse::<i64>().is_ok()
        {
            if lower.contains('[') {
                archidekt += 1;
            } else {
                moxfield += 1;
            }
        }
    }
    if archidekt > moxfield && archidekt > 0 {
        return DeckFormat::Archidekt;
    }
    if moxfield > manabox_headers {
        return DeckFormat::Moxfield;
    }
    if arena_headers > 0 && moxfield == 0 && archidekt == 0 {
        return DeckFormat::Arena;
    }
    DeckFormat::ManaBox
}

/// Parse any supported txt format into the internal `Deck`.
///
/// ManaBox text goes through the native grammar; the external formats
/// normalize first (`Nx` quantities, lowercase set codes, bracket sets,
/// bare-word section headers) then land in the same `Deck` shape.
///
/// # Errors
/// Fails when the text matches no format's line shape.
pub fn parse_external(text: &str, format: DeckFormat) -> anyhow::Result<Deck> {
    match format {
        DeckFormat::ManaBox => Deck::parse(text),
        DeckFormat::Names => parse_names(text),
        DeckFormat::Moxfield => parse_quantified(text, '(', ')'),
        DeckFormat::Archidekt => parse_quantified(text, '[', ']'),
        DeckFormat::Arena => parse_arena(text),
    }
}

/// Export text in the given format.
pub fn render(deck: &Deck, format: DeckFormat) -> String {
    match format {
        DeckFormat::ManaBox => deck.to_text(),
        DeckFormat::Names => render_sections(deck, names_line),
        DeckFormat::Moxfield => render_sections_moxfield(deck),
        DeckFormat::Archidekt => render_archidekt(deck),
        DeckFormat::Arena => render_arena(deck),
    }
}

/// Section-prefixed renderer over a per-entry line function.
fn render_sections(deck: &Deck, line: impl Fn(&DeckEntry) -> String) -> String {
    let mut out = String::new();
    for (i, (section, entries)) in deck.sections.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("// {section}\n"));
        for entry in entries {
            out.push_str(&line(entry));
            out.push('\n');
        }
    }
    out
}

/// Plain `qty Name` lines per section: the `names` export.
fn names_line(e: &DeckEntry) -> String {
    format!("{} {}", e.quantity, e.name)
}

/// Print-info suffix in Moxfield shape: ` (set) cn *F*` (only known parts).
fn print_tail(e: &DeckEntry, open: char, close: char) -> String {
    let mut tail = String::new();
    if let Some(set) = &e.set_code {
        tail.push_str(&format!(" {open}{}{close}", set.to_ascii_lowercase()));
        if let Some(cn) = &e.collector_number {
            tail.push_str(&format!(" {cn}"));
        }
    }
    if e.foil {
        tail.push_str(" *F*");
    }
    tail
}

/// Moxfield txt: `2x Name (set) cn *F*` under `// SECTION` headers.
fn render_sections_moxfield(deck: &Deck) -> String {
    render_sections(deck, |e| {
        format!("{}x {}{}", e.quantity, e.name, print_tail(e, '(', ')'))
    })
}

/// Archidekt txt: `1x Name [set] cn *F*` (set always present; unknown
/// prints render bare). Section headers stay `// SECTION`.
fn render_archidekt(deck: &Deck) -> String {
    render_sections(deck, |e| {
        let mut line = format!("{}x {}", e.quantity, e.name);
        if let Some(set) = &e.set_code {
            line.push_str(&format!(
                " [{}]{}",
                set.to_ascii_lowercase(),
                e.collector_number
                    .as_deref()
                    .map(|c| format!(" {c}"))
                    .unwrap_or_default()
            ));
        }
        if e.foil {
            line.push_str(" *F*");
        }
        line
    })
}

/// Arena txt: bare-word headers (`Commander` / `Deck` / `Sideboard`),
/// lowercase `(set) cn` print info, no foil marker.
fn render_arena(deck: &Deck) -> String {
    let mut out = String::new();
    for (i, (section, entries)) in deck.sections.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let header = match section.to_ascii_lowercase().as_str() {
            "commander" => "Commander",
            "sideboard" => "Sideboard",
            _ => "Deck",
        };
        out.push_str(&format!("{header}\n"));
        for entry in entries {
            out.push_str(&format!(
                "{} {}{}",
                entry.quantity,
                entry.name,
                print_tail(entry, '(', ')')
            ));
            out.push('\n');
        }
    }
    out
}

/// Shared shape for `Nx Name (set) cn *F*` (Moxfield) and
/// `Nx Name [set] cn *F*` (Archidekt). Headers are `//Word`, `#KnownWord`,
/// or bare known words; blank lines and `#` comments skip.
fn parse_quantified(text: &str, open: char, close: char) -> anyhow::Result<Deck> {
    let mut sections: Vec<(String, Vec<DeckEntry>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section) = external_header(line) {
            sections.push((section, Vec::new()));
            continue;
        }
        // A `#` line that is not a known header is a comment.
        if line.starts_with('#') {
            continue;
        }
        // Card line: `2x Name ...`
        let (qty_str, rest) = line
            .split_once("x ")
            .with_context(|| format!("entry is missing a quantity: {line:?}"))?;
        let quantity: i64 = qty_str
            .trim()
            .parse()
            .with_context(|| format!("invalid quantity in entry: {line:?}"))?;
        anyhow::ensure!(quantity > 0, "non-positive quantity in entry: {line:?}");
        let entry = parse_print_tail(rest, open, close, quantity)?;
        match sections.last_mut() {
            Some((_, entries)) => entries.push(entry),
            None => sections.push(("DECK".to_string(), vec![entry])),
        }
    }
    Ok(Deck { sections })
}

/// Bare-header list (Arena export): `Deck` / `Commander` / `Sideboard`
/// headers and `2 Name (set) cn` card lines.
fn parse_arena(text: &str) -> anyhow::Result<Deck> {
    let mut sections: Vec<(String, Vec<DeckEntry>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if !lower.contains(' ')
            && !lower.contains('(')
            && let Some(section) = bare_header(&lower)
        {
            sections.push((section, Vec::new()));
            continue;
        }
        let (qty_str, rest) = line
            .split_once(' ')
            .with_context(|| format!("entry is missing a quantity: {line:?}"))?;
        let quantity: i64 = qty_str
            .trim()
            .parse()
            .with_context(|| format!("invalid quantity in entry: {line:?}"))?;
        anyhow::ensure!(quantity > 0, "non-positive quantity in entry: {line:?}");
        let entry = parse_print_tail(rest, '(', ')', quantity)?;
        match sections.last_mut() {
            Some((_, entries)) => entries.push(entry),
            None => sections.push(("DECK".to_string(), vec![entry])),
        }
    }
    Ok(Deck { sections })
}

/// Plain `qty Name` lines with no section headers; every line lands in
/// DECK. The `names` export's parse counterpart.
fn parse_names(text: &str) -> anyhow::Result<Deck> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let (qty_str, rest) = line
            .split_once(' ')
            .with_context(|| format!("entry is missing a quantity: {line:?}"))?;
        let quantity: i64 = qty_str
            .trim()
            .parse()
            .with_context(|| format!("invalid quantity in entry: {line:?}"))?;
        anyhow::ensure!(quantity > 0, "non-positive quantity in entry: {line:?}");
        let name = rest.trim().to_string();
        anyhow::ensure!(!name.is_empty(), "entry is missing a card name: {line:?}");
        entries.push(DeckEntry {
            quantity,
            name,
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    if entries.is_empty() {
        anyhow::bail!("no card lines found");
    }
    Ok(Deck {
        sections: vec![("DECK".to_string(), entries)],
    })
}

/// Parse one external card line's print tail: `Name (set) [cn] [*F*]`.
///
/// The bracket pair depends on the format (Moxfield `(...)`, Archidekt
/// `[...]`); set codes may be lowercase and normalize to uppercase.
/// Quantity arrives pre-parsed.
fn parse_print_tail(
    rest: &str,
    open: char,
    close: char,
    quantity: i64,
) -> anyhow::Result<DeckEntry> {
    let rest = rest.trim();
    // Trailing foil marker (ManaBox's `*F*`; both external formats accept it).
    let (rest, foil) = match rest.strip_suffix("*F*") {
        Some(rest) => (rest.trim_end(), true),
        None => (rest, false),
    };
    // Trailing collector-number token (`372`, `28p`, `CON-31`) — only when
    // a print suffix precedes it, so a bare name never loses a word.
    let (rest, set_code, collector_number) = split_print(rest, open, close);
    anyhow::ensure!(!rest.trim().is_empty(), "entry is missing a card name");
    Ok(DeckEntry {
        quantity,
        name: rest.trim().to_string(),
        set_code: set_code.map(|s| s.to_ascii_uppercase()),
        collector_number: collector_number.clone(),
        foil,
    })
}

/// Split an optional `(set) [cn]` / `[set] [cn]` suffix off the tail.
///
/// The suffix must sit at the very end (after any collector number), so
/// card names containing brackets stay names. Returns
/// `(name, set_code, collector_number)`.
fn split_print(rest: &str, open: char, close: char) -> (String, Option<String>, Option<String>) {
    let rest = rest.trim_end();
    // Shape A: `... Name (set) cn` — the bracket opens before the last gap.
    if let Some(open_idx) = rest.rfind(open)
        && rest[open_idx..].contains(close)
    {
        let name = rest[..open_idx].trim_end();
        let tail = &rest[open_idx + 1..];
        if let Some(close_idx) = tail.find(close) {
            let set = tail[..close_idx].trim();
            let after = tail[close_idx + 1..].trim();
            let collector = (!after.is_empty()).then(|| after.to_string());
            // An invalid collector number means the parenthesized text is
            // part of the name (grammar.rs behavior): keep the name intact.
            let collector_valid = collector.as_deref().is_none_or(|cn| {
                (1..=8).contains(&cn.len())
                    && cn.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });
            if !name.is_empty() && valid_set(set) && collector_valid {
                return (name.to_string(), Some(set.to_string()), collector);
            }
        }
    }
    (rest.to_string(), None, None)
}

/// Set codes: 1-6 alphanumeric tokens (either case; normalized later).
fn valid_set(set: &str) -> bool {
    (1..=6).contains(&set.len()) && set.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Section header from an external line (`//Word`, `#Word`), ManaBox
/// vocabulary for all formats. `None` when the line is not a header.
fn external_header(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if let Some(header) = line.strip_prefix("//") {
        return Some(bare_header_or_raw(header.trim()));
    }
    if let Some(header) = line.strip_prefix('#') {
        let header = header.trim();
        // A bare `#` line is a comment, not a header. A `#`-header only
        // counts when the word maps to a known section (an unknown `#`
        // line is a comment, not a section name).
        if header.is_empty() {
            return None;
        }
        return bare_header(&header.to_ascii_lowercase());
    }
    if !lower.contains(' ') {
        return bare_header(&lower);
    }
    None
}

/// Bare-word header mapping; unknown words stay raw uppercase (never a
/// card line: those carry quantities).
fn bare_header(lower: &str) -> Option<String> {
    match lower {
        "deck" | "mainboard" | "maindeck" => Some("DECK".to_string()),
        "commander" | "commanders" => Some("COMMANDER".to_string()),
        "sideboard" | "maybeboard" => Some("SIDEBOARD".to_string()),
        _ => None,
    }
}

/// A `//`-header that may not be a known word: map known ones, keep
/// others verbatim (uppercased, ManaBox style).
fn bare_header_or_raw(header: &str) -> String {
    let lower = header.to_ascii_lowercase();
    bare_header(&lower).unwrap_or_else(|| header.to_ascii_uppercase())
}

use anyhow::Context;

#[cfg(test)]
#[path = "tests/io_external_tests.rs"]
mod io_external_tests;
