// Deck file I/O: decklist import (upsert), export txt, primer read/write.

use std::io::IsTerminal;

use super::grammar::{Deck, DeckEntry};
use super::store::{ensure_deck_file, ensure_primer_file, ensure_valid_name, load_deck, save_deck};
use anyhow::Context;

/// ManaBox's txt export labels a commander-format deck's whole list
/// `// COMMANDER` (verified: 100-card decks exported with one section).
/// A real commander section holds 1-2 cards (Partner).
fn normalize_sections(deck: &mut Deck, out: &mut crate::output::Output) {
    let oversized = deck
        .sections
        .iter()
        .find(|(name, entries)| name.eq_ignore_ascii_case("COMMANDER") && entries.len() > 4)
        .is_some();
    if oversized {
        for (name, _) in deck.sections.iter_mut() {
            if name.eq_ignore_ascii_case("COMMANDER") {
                *name = "DECK".to_string();
            }
        }
        out.status(
            "Note",
            "ManaBox exports label the whole deck '// COMMANDER'; treated as the main deck",
        );
    }
}

/// Commander candidates found in the decklist: legendary creatures or
/// planeswalkers, best EDHREC rank first.
fn commander_candidates(
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
) -> Vec<String> {
    let mut candidates: Vec<&crate::db::CardRow> = cards_by_name
        .values()
        .filter(|c| {
            c.type_line.contains("Legendary Creature")
                || c.type_line.contains("Legendary Planeswalker")
        })
        .collect();
    candidates.sort_by_key(|c| (c.edhrec_rank.unwrap_or(i64::MAX), c.name.clone()));
    candidates.into_iter().map(|c| c.name.clone()).collect()
}

/// Interactive commander pick. Returns None when the user skips.
///
/// TTY-only: callers never reach this when piped or in JSON mode.
fn prompt_commander(candidates: &[String]) -> anyhow::Result<Option<String>> {
    use dialoguer::FuzzySelect;
    let mut items: Vec<String> = candidates.to_vec();
    items.push("Skip (choose later: stm deck update <name> --add commander:1 <card>)".into());
    items.push("Other (type the name)".into());
    let pick = FuzzySelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Who commands this deck?")
        .items(&items)
        .default(0)
        .interact_opt()?;
    match pick {
        None => Ok(None), // Esc: treat as skip
        Some(i) if i + 1 == items.len() => Ok(None),
        Some(i) if i + 2 == items.len() => {
            let name: String = dialoguer::Input::new()
                .with_prompt("Commander name")
                .interact_text()?;
            Ok(Some(name))
        }
        Some(i) => Ok(Some(items[i].clone())),
    }
}

/// Entry point for `stm deck import <name> <file>`.
///
/// Upserts the decklist from a ManaBox deck txt export: creates or
/// overwrites `decks/<name>.txt` by name. Ownership comes from the
/// collection (deck-assignment rows), never from this import.
pub fn import(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    json: bool,
    name: &str,
    file: &std::path::Path,
) -> anyhow::Result<i32> {
    ensure_valid_name(out, name)?;
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut deck = Deck::parse(&text).with_context(|| format!("parsing {}", file.display()))?;
    normalize_sections(&mut deck, out);

    // Commander prompt: only when the decklist carries no real COMMANDER
    // section, stdout is a TTY, and JSON output is off (agents never hang).
    let cards_by_name = super::stats::lookup_names(conn, &deck);
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
    let (in_deck, in_binders) = super::store::owned_copies(conn, name)?;
    let covered = deck
        .entries()
        .map(|e| in_deck.min(e.quantity))
        .sum::<i64>()
        .min(deck.total());
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
            "{} cards into deck {name:?} from {}",
            deck.total(),
            file.display()
        ),
        std::time::Duration::ZERO,
    );
    Ok(crate::cli::codes::OK)
}

/// True when a COMMANDER section exists with 1-2 entries (a real one).
fn has_commander_section(deck: &Deck) -> bool {
    deck.section_index("COMMANDER")
        .map(|i| {
            let n = deck.sections[i].1.len();
            (1..=2).contains(&n)
        })
        .unwrap_or(false)
}

/// True when some deck entry is a commander-typed card (covers decks whose
/// commander sits in the main list).
fn has_commander_line(
    deck: &Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
) -> bool {
    deck.entries().any(|e| {
        cards_by_name
            .get(&e.name)
            .is_some_and(super::legal::is_commander_type)
    })
}

/// Entry point for `stm deck export <name> <file> [--force]`.
pub fn export(
    paths: &crate::paths::Paths,
    out: &mut crate::output::Output,
    name: &str,
    file: &std::path::Path,
    force: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = load_deck(paths, name)?;
    if file.exists() && !force {
        out.error(&format!("{} already exists", file.display()));
        out.hint("pass --force to overwrite the destination file");
        return Ok(crate::cli::codes::ERROR);
    }
    std::fs::write(file, deck.to_text()).with_context(|| format!("writing {}", file.display()))?;
    out.finish(
        "Exported",
        &format!(
            "deck {name:?} ({} cards) to {}",
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
/// With `--set`, replaces the primer's contents from a markdown file; the
/// deckbuilding agent edits the file directly between reads.
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
            let text = std::fs::read_to_string(file)
                .with_context(|| format!("reading {}", file.display()))?;
            std::fs::write(&primer, &text)
                .with_context(|| format!("writing {}", primer.display()))?;
            out.finish(
                "Wrote",
                &format!("primer for {name:?} from {}", file.display()),
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

        import(&paths, &conn, &mut out, true, "Round", &src).unwrap();
        // Upsert: importing again succeeds and overwrites by name.
        let code = import(&paths, &conn, &mut out, true, "Round", &src).unwrap();
        assert_eq!(code, crate::cli::codes::OK);
        // Contents on disk match the source grammar.
        let stored = std::fs::read_to_string(paths.deck_file("Round")).unwrap();
        assert_eq!(stored, DECK_TXT);
        // Primer stub exists.
        assert!(primer_file(&paths, "Round").exists());

        let dest = paths.root().join("out.txt");
        export(&paths, &mut out, "Round", &dest, false).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), DECK_TXT);
        // Export refuses to overwrite without --force.
        let code = export(&paths, &mut out, "Round", &dest, false).unwrap();
        assert_eq!(code, crate::cli::codes::ERROR);
        let code = export(&paths, &mut out, "Round", &dest, true).unwrap();
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
        import(&paths, &conn, &mut out, true, "Quirk", &src).unwrap();
        let deck =
            Deck::parse(&std::fs::read_to_string(paths.deck_file("Quirk")).unwrap()).unwrap();
        assert!(deck.section_index("COMMANDER").is_none(), "renamed to DECK");
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
        import(&paths, &conn, &mut out, true, "Ok", &src).unwrap();
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
        assert!(import(&paths, &conn, &mut out, true, "Bad", &src).is_err());
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
        assert!(import(&paths, &conn, &mut out, true, "../escape", &src).is_err());
        assert!(!paths.deck_file("../escape").exists());
    }
}
