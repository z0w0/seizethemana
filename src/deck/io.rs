// Deck file I/O: decklist import (upsert), export txt, primer read/write.

use std::io::IsTerminal;

use super::grammar::{Deck, DeckEntry};
use super::store::{ensure_deck_file, ensure_primer_file, ensure_valid_name, load_deck, save_deck};
use anyhow::Context;

/// ManaBox's txt export labels a commander-format deck's whole list
/// `// COMMANDER` (verified: 100-card decks exported with one section).
/// A real commander section holds 1-2 commanders (or up to 4 with a
/// "Choose a Background" pair); larger COMMANDER sections merge into the
/// existing DECK section (or become DECK when none exists).
fn normalize_sections(deck: &mut Deck, out: &mut crate::output::Output) {
    let oversized = deck
        .sections
        .iter()
        .find(|(name, entries)| name.eq_ignore_ascii_case("COMMANDER") && entries.len() > 4)
        .is_some();
    if !oversized {
        return;
    }
    let commander = deck
        .sections
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case("COMMANDER"))
        .map(|(_, e)| std::mem::take(e))
        .unwrap_or_default();
    deck.sections
        .retain(|(name, _)| !name.eq_ignore_ascii_case("COMMANDER"));
    if let Some((_, deck_entries)) = deck
        .sections
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case("DECK"))
    {
        deck_entries.extend(commander);
    } else {
        deck.sections.push(("DECK".to_string(), commander));
    }
    out.status(
        "Note",
        "ManaBox exports label the whole deck '// COMMANDER'; treated as the main deck",
    );
}

/// Commander candidates found in the decklist: cards that can lead a
/// commander deck (the `legal::is_commander_type` rules, incl. legendary
/// Vehicles/Spacecraft with a P/T box), best EDHREC rank first.
fn commander_candidates(
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
) -> Vec<String> {
    let mut candidates: Vec<&crate::db::CardRow> = cards_by_name
        .values()
        .filter(|c| super::legal::is_commander_type(c))
        .collect();
    candidates.sort_by_key(|c| (c.edhrec_rank.unwrap_or(i64::MAX), c.name.clone()));
    candidates.into_iter().map(|c| c.name.clone()).collect()
}

/// Menu rows for the commander prompt: candidates plus Skip (second to
/// last) and Other (last). Split out so index mapping stays testable.
fn prompt_items(candidates: &[String]) -> Vec<String> {
    let mut items: Vec<String> = candidates.to_vec();
    items.push("Skip (choose later: stm deck update <name> --add commander:1 <card>)".into());
    items.push("Other (type the name)".into());
    items
}

/// Result of picking a row in the commander prompt.
#[derive(Debug, PartialEq)]
enum CommanderPick {
    /// A named candidate (or typed name).
    Name(String),
    /// Skip: choose a commander later.
    Skip,
}

/// Whether a selected row is the "Other (type the name)" entry.
fn is_other_row(i: usize, items: &[String]) -> bool {
    i + 1 == items.len()
}

/// Map a selected row index to a pick. Esc (`None`) and the Skip row
/// (second to last) skip; the Other row (last) and candidates name a
/// commander. The caller prompts for the typed name on the Other row.
fn map_pick(pick: Option<usize>, items: &[String], other_name: &str) -> CommanderPick {
    match pick {
        None => CommanderPick::Skip,
        Some(i) if i + 2 == items.len() => CommanderPick::Skip,
        Some(i) if is_other_row(i, items) => CommanderPick::Name(other_name.to_string()),
        Some(i) => CommanderPick::Name(items[i].clone()),
    }
}

/// Interactive commander pick. Returns None when the user skips.
///
/// TTY-only: callers never reach this when piped or in JSON mode.
fn prompt_commander(candidates: &[String]) -> anyhow::Result<Option<String>> {
    use dialoguer::FuzzySelect;
    let items = prompt_items(candidates);
    let pick = FuzzySelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Who commands this deck?")
        .items(&items)
        .default(0)
        .interact_opt()?;
    match map_pick(pick, &items, "Other (type the name)") {
        // The Other row: ask for the name.
        CommanderPick::Name(name) if name == "Other (type the name)" => {
            let name: String = dialoguer::Input::new()
                .with_prompt("Commander name")
                .interact_text()?;
            Ok(Some(name))
        }
        CommanderPick::Name(name) => Ok(Some(name)),
        CommanderPick::Skip => Ok(None),
    }
}

/// Import source: a file path or a hosted deck URL.
///
/// Grouped so `import` stays under the argument-count lint.
pub struct ImportSource<'a> {
    pub paths: &'a crate::paths::Paths,
    pub conn: &'a rusqlite::Connection,
    pub out: &'a mut crate::output::Output,
    pub json: bool,
    pub name: &'a str,
    pub file: Option<&'a std::path::Path>,
    pub url: Option<&'a str>,
    pub format: Option<&'a str>,
}

/// Entry point for `stm deck import <name> <file>` (or `--url <URL>`).
///
/// Upserts the decklist by name. The source text comes from a file (any
/// supported format, auto-detected unless `--format` pins one) or from a
/// hosted deck URL (Scryfall, Moxfield, Archidekt). Ownership comes from
/// the collection (deck-assignment rows), never from this import.
pub fn import(source: ImportSource<'_>) -> anyhow::Result<i32> {
    let ImportSource {
        paths,
        conn,
        out,
        json,
        name,
        file,
        url,
        format,
    } = source;
    ensure_valid_name(out, name)?;
    if file.is_some() && url.is_some() {
        out.error("pass either a file path or --url, not both");
        out.hint("import a file: stm deck import <name> <file>");
        return Ok(crate::cli::codes::USAGE);
    }
    let (text, source_label) = match read_source(out, file, url)? {
        Some(pair) => pair,
        None => return Ok(crate::cli::codes::USAGE),
    };
    let mut deck = import_deck_text(out, &text, format, &source_label)?;
    let commander_note = settle_commander(conn, out, &mut deck, json)?;
    save_imported(
        paths,
        conn,
        out,
        name,
        &deck,
        &source_label,
        &commander_note,
    )?;
    Ok(crate::cli::codes::OK)
}

/// Read the decklist text from the file or URL source. `None` means the
/// usage error was already printed and the caller should return.
fn read_source<'a>(
    out: &mut crate::output::Output,
    file: Option<&'a std::path::Path>,
    url: Option<&'a str>,
) -> anyhow::Result<Option<(String, String)>> {
    let text = match (file, url) {
        (Some(file), _) => (
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?,
            file.display().to_string(),
        ),
        (None, Some(url)) => {
            if crate::offline_requested() {
                out.error("--url needs the network; --offline skips all fetches");
                out.hint("export a txt from the deck site and import the file instead");
                return Ok(None);
            }
            out.status("Fetching", url);
            (super::url_fetch::fetch(url)?.to_text(), url.to_string())
        }
        (None, None) => {
            out.error("nothing to import: pass a file path or --url <URL>");
            out.hint("e.g. stm deck import Froggy ~/Downloads/Froggy.txt");
            return Ok(None);
        }
    };
    Ok(Some(text))
}

/// Commander resolution on import: only when the decklist carries no real
/// COMMANDER section, stdout is a TTY, and JSON output is off (agents
/// never hang). Returns the note printed with the ownership summary.
fn settle_commander(
    conn: &rusqlite::Connection,
    _out: &crate::output::Output,
    deck: &mut super::grammar::Deck,
    json: bool,
) -> anyhow::Result<String> {
    let cards_by_name = super::stats::lookup_names(conn, deck)?;
    if has_commander_section(deck) || has_commander_line(deck, &cards_by_name) {
        return Ok(String::new());
    }
    let candidates = commander_candidates(&cards_by_name);
    if candidates.is_empty() {
        return Ok(
            "no commander candidate found in the decklist; set one with \
             stm deck update <name> --add commander:1 <card>"
                .to_string(),
        );
    }
    if json || !std::io::stdout().is_terminal() {
        return Ok(format!(
            "no commander section; candidates: {} (set with \
             stm deck update <name> --add commander:1 <card>)",
            candidates.join(", ")
        ));
    }
    match prompt_commander(&candidates)? {
        Some(pick) => match crate::db::resolve_name(conn, &pick)? {
            crate::db::NameMatch::Found(card) => {
                deck.section_entries_mut("COMMANDER").push(DeckEntry {
                    quantity: 1,
                    name: card.name.clone(),
                    set_code: Some(card.set_code.clone()).filter(|s| !s.is_empty()),
                    collector_number: Some(card.collector_number.clone()).filter(|c| !c.is_empty()),
                    foil: false,
                });
                Ok(format!("commander set: {}", card.name))
            }
            _ => Ok(format!(
                "commander {pick:?} not in the oracle; skipped — \
                 set it with stm deck update <name> --add commander:1 <card>"
            )),
        },
        None => Ok(format!(
            "no commander set; candidates: {}; \
             set one with stm deck update <name> --add commander:1 <card>",
            candidates.join(", ")
        )),
    }
}

/// Persist the imported deck and print the ownership summary and finish
/// line.
fn save_imported(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    deck: &super::grammar::Deck,
    source_label: &str,
    commander_note: &str,
) -> anyhow::Result<()> {
    ensure_deck_file(paths, name)?;
    save_deck(paths, name, deck)?;
    ensure_primer_file(paths, name)?;

    // Ownership summary: read-only counts from the collection split.
    // Coverage consumes the pool per name: two lines of the same name
    // draw against the same owned copies, like `ownership::slot_map`.
    let (in_deck, in_binders) = super::store::owned_copies(conn, name)?;
    let available = super::ownership::available_map(conn, name)?;
    let mut needed: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for e in deck.entries() {
        *needed.entry(e.name.as_str()).or_insert(0) += e.quantity;
    }
    let covered: i64 = needed
        .iter()
        .map(|(name, qty)| available.get(*name).copied().unwrap_or(0).min(*qty))
        .sum();
    if !commander_note.is_empty() {
        out.print_note(commander_note);
    }
    out.status(
        "Ownership",
        &format!(
            "{}/{} owned ({} deck-assigned, +{} in binders)",
            covered,
            deck.total(),
            in_deck,
            in_binders
        ),
    );
    out.finish(
        "Imported",
        &format!(
            "{} cards into deck {name:?} from {source_label}",
            deck.total(),
        ),
        std::time::Duration::ZERO,
    );
    Ok(())
}

/// Parse imported deck text in any supported format and normalize
/// ManaBox's commander quirk. `format` pins the shape; `None` auto-detects.
fn import_deck_text(
    out: &mut crate::output::Output,
    text: &str,
    format: Option<&str>,
    source_label: &str,
) -> anyhow::Result<Deck> {
    let parsed = match format.and_then(super::io_external::DeckFormat::parse) {
        Some(fmt) => super::io_external::parse_external(text, fmt),
        None => super::io_external::parse_external(text, super::io_external::detect(text)),
    };
    let mut parsed = parsed.with_context(|| format!("parsing {source_label}"))?;
    normalize_sections(&mut parsed, out);
    Ok(parsed)
}

/// True when a COMMANDER section exists with a real commander count:
/// 1-2 entries (solo or Partner), or 3-4 with a "Choose a Background"
/// pair.
fn has_commander_section(deck: &Deck) -> bool {
    deck.section_index("COMMANDER")
        .map(|i| {
            let n = deck.sections[i].1.len();
            (1..=4).contains(&n)
        })
        .unwrap_or(false)
}

/// True when some non-sideboard deck entry is a commander-typed card
/// (covers decks whose commander sits in the main list). Sideboard
/// entries do not count: a sideboarded legendary is not this deck's
/// commander.
fn has_commander_line(
    deck: &Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
) -> bool {
    deck.sections
        .iter()
        .filter(|(s, _)| !super::grammar::is_bench_section(s))
        .flat_map(|(_, es)| es.iter())
        .any(|e| {
            cards_by_name
                .get(&e.name)
                .is_some_and(super::legal::is_commander_type)
        })
}

/// Entry point for `stm deck export <name> <file> [--format manabox|names|moxfield|archidekt|arena]`.
pub fn export(
    paths: &crate::paths::Paths,
    out: &mut crate::output::Output,
    name: &str,
    file: &std::path::Path,
    force: bool,
    format: &str,
) -> anyhow::Result<i32> {
    let Some(fmt) = super::io_external::DeckFormat::parse(format) else {
        out.error(&format!("unknown export format {format:?}"));
        out.hint(&format!(
            "formats: {}",
            super::io_external::DeckFormat::all_keys().join(", ")
        ));
        return Ok(crate::cli::codes::USAGE);
    };
    let (_path, deck) = load_deck(paths, name)?;
    if file.exists() && !force {
        out.error(&format!("{} already exists", file.display()));
        out.hint("pass --force to overwrite the destination file");
        return Ok(crate::cli::codes::ERROR);
    }
    let text = super::io_external::render(&deck, fmt);
    std::fs::write(file, text).with_context(|| format!("writing {}", file.display()))?;
    out.finish(
        "Exported",
        &format!(
            "deck {name:?} ({} cards, {format}) to {}",
            deck.total(),
            file.display()
        ),
        std::time::Duration::ZERO,
    );
    Ok(crate::cli::codes::OK)
}

/// Entry point for `stm deck primer <name> [--set <file>]`.
///
/// Without `--set`, prints the primer (empty output for an untouched stub).
/// With `--set`, replaces the primer's contents from a markdown file (`-`
/// reads stdin); the deckbuilding agent edits the file directly between
/// reads.
pub fn primer(
    paths: &crate::paths::Paths,
    out: &mut crate::output::Output,
    name: &str,
    set: Option<&std::path::Path>,
) -> anyhow::Result<i32> {
    // The deck must exist; the primer may not (older decks) — create it.
    load_deck(paths, name)?;
    let primer = ensure_primer_file(paths, name)?;
    match set {
        None => {
            let text = std::fs::read_to_string(&primer)
                .with_context(|| format!("reading {}", primer.display()))?;
            if text.is_empty() {
                // An untouched stub prints nothing; say how to fill it.
                out.print_note(&format!(
                    "the primer for {name:?} is empty; write it with \
                     stm deck primer {name} --set <file> or edit {}",
                    primer.display()
                ));
                return Ok(crate::cli::codes::OK);
            }
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
        }
        Some(file) => {
            let text = if file.as_os_str() == "-" {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .context("reading primer markdown from stdin")?;
                buf
            } else {
                std::fs::read_to_string(file)
                    .with_context(|| format!("reading {}", file.display()))?
            };
            std::fs::write(&primer, &text)
                .with_context(|| format!("writing {}", primer.display()))?;
            let source = if file.as_os_str() == "-" {
                "stdin".to_string()
            } else {
                file.display().to_string()
            };
            out.finish(
                "Wrote",
                &format!("primer for {name:?} from {source}"),
                std::time::Duration::ZERO,
            );
        }
    }
    Ok(crate::cli::codes::OK)
}

#[cfg(test)]
#[path = "tests/io_tests.rs"]
mod io_tests;

#[cfg(test)]
#[path = "tests/prompt_tests.rs"]
mod prompt_tests;
