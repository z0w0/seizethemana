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
    let text = match (file, url) {
        (Some(file), _) => {
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?
        }
        (None, Some(url)) => {
            if crate::offline_requested() {
                out.error("--url needs the network; --offline skips all fetches");
                out.hint("export a txt from the deck site and import the file instead");
                return Ok(crate::cli::codes::USAGE);
            }
            out.status("Fetching", url);
            super::url_fetch::fetch(url)?.to_text()
        }
        (None, None) => {
            out.error("nothing to import: pass a file path or --url <URL>");
            out.hint("e.g. stm deck import Froggy ~/Downloads/Froggy.txt");
            return Ok(crate::cli::codes::USAGE);
        }
    };
    let source_label = match url {
        Some(url) => url.to_string(),
        None => file.map(|f| f.display().to_string()).unwrap_or_default(),
    };
    let mut deck = import_deck_text(out, &text, format, &source_label)?;

    // Commander prompt: only when the decklist carries no real COMMANDER
    // section, stdout is a TTY, and JSON output is off (agents never hang).
    let cards_by_name = super::stats::lookup_names(conn, &deck)?;
    let mut commander_note = String::new();
    if !has_commander_section(&deck) && !has_commander_line(&deck, &cards_by_name) {
        let candidates = commander_candidates(&cards_by_name);
        if candidates.is_empty() {
            commander_note = "no commander candidate found in the decklist; set one with \
                 stm deck update <name> --add commander:1 <card>"
                .to_string();
        } else if !json && std::io::stdout().is_terminal() {
            match prompt_commander(&candidates)? {
                Some(pick) => match crate::db::resolve_name(conn, &pick)? {
                    crate::db::NameMatch::Found(card) => {
                        deck.section_entries_mut("COMMANDER").push(DeckEntry {
                            quantity: 1,
                            name: card.name.clone(),
                            set_code: Some(card.set_code.clone()).filter(|s| !s.is_empty()),
                            collector_number: Some(card.collector_number.clone())
                                .filter(|c| !c.is_empty()),
                            foil: false,
                        });
                        commander_note = format!("commander set: {}", card.name);
                    }
                    _ => {
                        commander_note = format!(
                            "commander {pick:?} not in the oracle; skipped — \
                                 set it with stm deck update <name> --add commander:1 <card>"
                        );
                    }
                },
                None => {
                    commander_note = format!(
                        "no commander set; candidates: {}; \
                         set one with stm deck update <name> --add commander:1 <card>",
                        candidates.join(", ")
                    );
                }
            }
        } else {
            commander_note = format!(
                "no commander section; candidates: {} (set with \
                 stm deck update <name> --add commander:1 <card>)",
                candidates.join(", ")
            );
        }
    }

    ensure_deck_file(paths, name)?;
    save_deck(paths, name, &deck)?;
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
        out.print_note(&commander_note);
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
    Ok(crate::cli::codes::OK)
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
        .filter(|(s, _)| !s.eq_ignore_ascii_case("SIDEBOARD"))
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
mod tests {
    use super::*;
    use crate::deck::store::primer_file;
    use crate::output::Output;

    fn setup() -> (tempfile::TempDir, crate::paths::Paths, rusqlite::Connection) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        (tmp, paths, conn)
    }

    /// Seed oracle rows so name resolution and commander detection work.
    fn seed_card(conn: &rusqlite::Connection, name: &str, type_line: &str, rank: Option<i64>) {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, type_line, edhrec_rank, set_code, collector_number)
             VALUES (?1, 'oid', ?2, ?3, 'tst', '1')",
            rusqlite::params![name, type_line, rank],
        )
        .unwrap();
    }

    const DECK_TXT: &str =
        "// COMMANDER\n1 Breya, Etherium Shaper (MH3) 372 *F*\n\n// DECK\n2 Island (SOS) 274\n";

    #[test]
    fn import_upserts_by_name() {
        let (_tmp, paths, conn) = setup();
        seed_card(
            &conn,
            "Breya, Etherium Shaper",
            "Legendary Creature — Human",
            Some(10),
        );
        seed_card(&conn, "Island", "Basic Land — Island", None);
        let src = paths.root().join("source.txt");
        std::fs::write(&src, DECK_TXT).unwrap();
        let mut out = Output::new(true, false, false);

        import(ImportSource {
            paths: &paths,
            conn: &conn,
            out: &mut out,
            json: true,
            name: "Round",
            file: Some(&src),
            url: None,
            format: None,
        })
        .unwrap();
        // Upsert: importing again succeeds and overwrites by name.
        let code = import(ImportSource {
            paths: &paths,
            conn: &conn,
            out: &mut out,
            json: true,
            name: "Round",
            file: Some(&src),
            url: None,
            format: None,
        })
        .unwrap();
        assert_eq!(code, crate::cli::codes::OK);
        // Contents on disk match the source grammar.
        let stored = std::fs::read_to_string(paths.deck_file("Round")).unwrap();
        assert_eq!(stored, DECK_TXT);
        // Primer stub exists.
        assert!(primer_file(&paths, "Round").exists());

        let dest = paths.root().join("out.txt");
        export(&paths, &mut out, "Round", &dest, false, "manabox").unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), DECK_TXT);
        // Export refuses to overwrite without --force.
        let code = export(&paths, &mut out, "Round", &dest, false, "manabox").unwrap();
        assert_eq!(code, crate::cli::codes::ERROR);
        let code = export(&paths, &mut out, "Round", &dest, true, "manabox").unwrap();
        assert_eq!(code, crate::cli::codes::OK);
    }

    #[test]
    fn manabox_quirk_renames_oversized_commander_section() {
        let (_tmp, paths, conn) = setup();
        seed_card(&conn, "Bolt", "Instant", None);
        let src = paths.root().join("manabox.txt");
        // ManaBox's export shape: the whole deck under // COMMANDER.
        std::fs::write(
            &src,
            "// COMMANDER\n2 Bolt\n1 Bolt\n1 Fog\n1 Giant Growth\n1 Lightning Helix\n1 Shock\n",
        )
        .unwrap();
        let mut out = Output::new(true, false, false);
        import(ImportSource {
            paths: &paths,
            conn: &conn,
            out: &mut out,
            json: true,
            name: "Quirk",
            file: Some(&src),
            url: None,
            format: None,
        })
        .unwrap();
        let deck =
            Deck::parse(&std::fs::read_to_string(paths.deck_file("Quirk")).unwrap()).unwrap();
        assert!(deck.section_index("COMMANDER").is_none(), "merged to DECK");
        assert_eq!(deck.total(), 7);
    }

    #[test]
    fn real_commander_section_survives() {
        let (_tmp, paths, conn) = setup();
        seed_card(
            &conn,
            "Breya, Etherium Shaper",
            "Legendary Creature — Human",
            Some(10),
        );
        seed_card(&conn, "Island", "Basic Land — Island", None);
        let src = paths.root().join("ok.txt");
        std::fs::write(&src, DECK_TXT).unwrap();
        let mut out = Output::new(true, false, false);
        import(ImportSource {
            paths: &paths,
            conn: &conn,
            out: &mut out,
            json: true,
            name: "Ok",
            file: Some(&src),
            url: None,
            format: None,
        })
        .unwrap();
        let deck = Deck::parse(&std::fs::read_to_string(paths.deck_file("Ok")).unwrap()).unwrap();
        assert_eq!(deck.section_index("COMMANDER"), Some(0));
        assert_eq!(deck.total(), 3);
    }

    #[test]
    fn import_rejects_bad_grammar() {
        let (_tmp, paths, conn) = setup();
        let src = paths.root().join("bad.txt");
        std::fs::write(&src, "not a deck line").unwrap();
        let mut out = Output::new(true, false, false);
        assert!(
            import(ImportSource {
                paths: &paths,
                conn: &conn,
                out: &mut out,
                json: true,
                name: "Bad",
                file: Some(&src),
                url: None,
                format: None
            })
            .is_err()
        );
    }

    #[test]
    fn export_names_strips_print_and_foil_decorations() {
        let (_tmp, paths, conn) = setup();
        seed_card(&conn, "Bolt", "Instant", None);
        let deck_path = paths.deck_file("Names");
        if let Some(dir) = deck_path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(
            deck_path,
            "// COMMANDER\n1 Breya\n// DECK\n2 Bolt (SOS) 100 *F*\n1 Fog\n",
        )
        .unwrap();
        let dest = _tmp.path().join("names.txt");
        let mut out = Output::new(true, false, false);
        let code = export(&paths, &mut out, "Names", &dest, false, "names").unwrap();
        assert_eq!(code, crate::cli::codes::OK);
        let text = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(text, "// COMMANDER\n1 Breya\n\n// DECK\n2 Bolt\n1 Fog\n");
    }

    #[test]
    fn primer_create_read_update() {
        let (_tmp, paths, _conn) = setup();
        std::fs::create_dir_all(paths.decks_dir()).unwrap();
        std::fs::write(paths.deck_file("P"), "// DECK\n1 Bolt\n").unwrap();
        let mut out = Output::new(false, true, false);

        // No primer yet: reading creates an empty stub.
        let code = primer(&paths, &mut out, "P", None).unwrap();
        assert_eq!(code, crate::cli::codes::OK);
        assert_eq!(
            std::fs::read_to_string(primer_file(&paths, "P")).unwrap(),
            ""
        );
        let _ = out; // empty-primer note checked visually; output is mode-safe

        // --set writes contents from a markdown file.
        let src = paths.root().join("primer.md");
        std::fs::write(&src, "# My deck\nPlan: win.\n").unwrap();
        let code = primer(&paths, &mut out, "P", Some(&src)).unwrap();
        assert_eq!(code, crate::cli::codes::OK);
        assert_eq!(
            std::fs::read_to_string(primer_file(&paths, "P")).unwrap(),
            "# My deck\nPlan: win.\n"
        );
    }

    #[test]
    fn primer_requires_existing_deck() {
        let (_tmp, paths, _conn) = setup();
        let mut out = Output::new(true, false, false);
        let code = primer(&paths, &mut out, "Ghost", None);
        assert!(code.is_err());
    }

    #[test]
    fn import_validates_deck_names() {
        let (_tmp, paths, conn) = setup();
        let src = paths.root().join("deck.txt");
        std::fs::write(&src, DECK_TXT).unwrap();
        let mut out = Output::new(true, false, false);
        // Invalid names error before any file is touched.
        assert!(
            import(ImportSource {
                paths: &paths,
                conn: &conn,
                out: &mut out,
                json: true,
                name: "../escape",
                file: Some(&src),
                url: None,
                format: None
            })
            .is_err()
        );
        assert!(!paths.deck_file("../escape").exists());
    }
}

#[cfg(test)]
#[path = "tests/prompt_tests.rs"]
mod prompt_tests;
