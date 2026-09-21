// Deck file storage and read commands (`create`, `list`, `show`).
//
// Deck txt files under `decks/` are the single source of truth for deck
// contents; ownership comes from the collection (`binder_type = 'deck'`
// rows); primers are sibling markdown files the deckbuilding agent edits.

use rusqlite::Connection;

use super::grammar::Deck;
use anyhow::Context;

/// Primer file path for a deck (`decks/<name>.primer.md`).
pub fn primer_file(paths: &crate::paths::Paths, name: &str) -> std::path::PathBuf {
    paths.decks_dir().join(format!("{name}.primer.md"))
}

/// Validate a deck name: a single path component (no separators or dots).
pub fn valid_deck_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '\'' | '&' | '!' | '+' | ',')
        })
        && !name.starts_with('.')
}

/// Ensure a deck file exists (empty when absent); returns its path.
pub(crate) fn ensure_deck_file(
    paths: &crate::paths::Paths,
    name: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let path = paths.deck_file(name);
    if !path.exists() {
        std::fs::create_dir_all(paths.decks_dir()).context("creating decks directory")?;
        std::fs::write(&path, "")
            .with_context(|| format!("creating deck file {}", path.display()))?;
    }
    Ok(path)
}

/// Ensure the primer file exists (empty stub); returns its path.
pub(crate) fn ensure_primer_file(
    paths: &crate::paths::Paths,
    name: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let primer = primer_file(paths, name);
    if !primer.exists() {
        std::fs::create_dir_all(paths.decks_dir()).context("creating decks directory")?;
        std::fs::write(&primer, "")
            .with_context(|| format!("creating primer {}", primer.display()))?;
    }
    Ok(primer)
}

/// Load and parse a deck file.
pub(crate) fn load_deck(
    paths: &crate::paths::Paths,
    name: &str,
) -> anyhow::Result<(std::path::PathBuf, Deck)> {
    let path = paths.deck_file(name);
    if !path.exists() {
        // Distinguish a missing deck so callers can hint consistently.
        return Err(anyhow::anyhow!(DeckNotFound {
            name: name.to_string()
        }));
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let deck = Deck::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok((path, deck))
}

/// A referenced deck file does not exist; rendered as `error:` + `hint:`.
#[derive(Debug, thiserror::Error)]
#[error("deck {name:?} not found")]
pub struct DeckNotFound {
    pub name: String,
}

/// True when an error chain's root is a missing deck file.
pub fn is_deck_not_found(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|c| c.downcast_ref::<DeckNotFound>().is_some())
}

/// Save a deck back to its txt file (parents ensured).
pub(crate) fn save_deck(
    paths: &crate::paths::Paths,
    name: &str,
    deck: &Deck,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(paths.decks_dir()).context("creating decks directory")?;
    std::fs::write(paths.deck_file(name), deck.to_text())
        .with_context(|| format!("writing deck {name:?}"))
}

/// Entry point for `stm deck create <name>`.
pub fn create(
    paths: &crate::paths::Paths,
    out: &mut crate::output::Output,
    name: &str,
) -> anyhow::Result<i32> {
    ensure_valid_name(out, name)?;
    let path = paths.deck_file(name);
    if path.exists() {
        out.error(&format!("deck {name:?} already exists"));
        out.hint(&format!("use 'stm deck show {name}' to view it"));
        return Ok(crate::cli::codes::ERROR);
    }
    ensure_deck_file(paths, name)?;
    let primer = ensure_primer_file(paths, name)?;
    out.finish(
        "Created",
        &format!(
            "deck {name:?} ({}; primer {})",
            path.display(),
            primer.display()
        ),
        std::time::Duration::ZERO,
    );
    Ok(crate::cli::codes::OK)
}

pub(crate) fn ensure_valid_name(out: &mut crate::output::Output, name: &str) -> anyhow::Result<()> {
    if !valid_deck_name(name) {
        out.error(&format!("invalid deck name {name:?}"));
        out.hint("use letters, digits, spaces, or - _ ' & ! + , (no / or ..)");
        anyhow::bail!("invalid deck name");
    }
    Ok(())
}

/// Deck summary for `stm deck list`.
///
/// `cards` is the maindeck count; `sideboard_cards` reports the sideboard
/// separately (a commander wishlist, not a legal zone).
#[derive(Debug, Clone)]
pub struct DeckSummary {
    pub name: String,
    pub cards: i64,
    pub sideboard_cards: i64,
    pub owned: i64,
    pub has_primer: bool,
}

/// Entry point for `stm deck list`.
pub fn list(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    json: bool,
) -> anyhow::Result<i32> {
    let decks = discover_decks(paths, conn)?;
    // Collection decks with no decklist file: the import gap.
    let mut gaps = registered_deck_names(conn)?;
    for d in &decks {
        gaps.retain(|g| !g.eq_ignore_ascii_case(&d.name));
    }
    if decks.is_empty() && gaps.is_empty() {
        if json {
            println!("[]");
            return Ok(crate::cli::codes::OK);
        }
        out.error("no decks found");
        out.hint("create one: stm deck create <name>, or import: stm deck import <name> <file>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        let mut items: Vec<serde_json::Value> = decks
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "has_decklist": true,
                    "cards": d.cards,
                    "sideboard_cards": d.sideboard_cards,
                    "owned": d.owned,
                    "has_primer": d.has_primer,
                })
            })
            .collect();
        for gap in &gaps {
            items.push(serde_json::json!({
                "name": gap,
                "has_decklist": false,
                "cards": 0,
                "sideboard_cards": 0,
                "owned": owned_count_for(conn, gap)?,
                "has_primer": false,
            }));
        }
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(crate::cli::codes::OK);
    }
    let styles = out.styles();
    println!("{}", styles.header("Decks"));
    for d in &decks {
        let primer = if d.has_primer { " primer" } else { "" };
        let count = if d.sideboard_cards > 0 {
            format!("{} cards + {} sideboard", d.cards, d.sideboard_cards)
        } else {
            format!("{} cards", d.cards)
        };
        // Completion coloring: green when fully owned, yellow when partial,
        // red when nothing is owned.
        let owned_display = if d.owned >= d.cards && d.cards > 0 {
            styles.glyph(
                &format!("own {}/{}", d.owned, d.cards),
                crate::output::GlyphKind::Good,
            )
        } else if d.owned > 0 {
            styles.glyph(
                &format!("own {}/{}", d.owned, d.cards),
                crate::output::GlyphKind::Warn,
            )
        } else {
            styles.glyph(
                &format!("own {}/{}", d.owned, d.cards),
                crate::output::GlyphKind::Bad,
            )
        };
        println!(
            "  {} {}  {}{}",
            styles.card_name(&d.name),
            styles.dim(&count),
            owned_display,
            styles.dim(primer),
        );
    }
    if !gaps.is_empty() {
        println!();
        println!("{}", styles.note("no decklist (owned in the collection):"));
        for gap in &gaps {
            let owned = owned_count_for(conn, gap)?;
            println!(
                "  {}  {}  {}",
                styles.card_name(gap),
                styles.dim(&format!("{owned} copies owned")),
                styles.dim(&format!("→ stm deck import \"{gap}\" <manabox-deck.txt>")),
            );
        }
    }
    Ok(crate::cli::codes::OK)
}

/// Deck names registered in the collection (`binder_type = 'deck'`), sorted.
fn registered_deck_names(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT binder FROM collection
         WHERE binder_type = 'deck' ORDER BY binder COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .context("reading deck names")?;
    Ok(rows)
}

/// Entry point for `stm deck delete <name>`.
///
/// Removes the decklist (txt + primer). Ownership rows in the collection
/// are never touched: the decklist and the ownership assignment are two
/// different things.
pub fn delete(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    name: &str,
) -> anyhow::Result<i32> {
    ensure_valid_name(out, name)?;
    let path = paths.deck_file(name);
    if !path.exists() {
        out.error(&format!("no decklist for {name:?}"));
        out.hint("list decklists: stm deck list");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let cards = Deck::parse(&std::fs::read_to_string(&path).unwrap_or_default())
        .map(|d| d.total())
        .unwrap_or(0);
    let primer = primer_file(paths, name);
    std::fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
    if primer.exists() {
        std::fs::remove_file(&primer).with_context(|| format!("deleting {}", primer.display()))?;
    }
    out.finish(
        "Deleted",
        &format!("deck {name:?} ({cards} cards, decklist + primer)"),
        std::time::Duration::ZERO,
    );
    let (in_deck, in_binders) = owned_copies(conn, name)?;
    if in_deck + in_binders > 0 {
        out.print_note(&format!(
            "{in_deck} deck-assigned + {in_binders} binder copies are still tracked for \
             {name:?} in the collection; ownership is unaffected"
        ));
    }
    Ok(crate::cli::codes::OK)
}

/// Deck files on disk (`*.txt`), with card/owned counts, alphabetical.
fn discover_decks(
    paths: &crate::paths::Paths,
    conn: &Connection,
) -> anyhow::Result<Vec<DeckSummary>> {
    let mut decks = Vec::new();
    let entries = match std::fs::read_dir(paths.decks_dir()) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(decks),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let path = entry.with_context(|| "reading decks directory")?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let name = deck_name_of(&path);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let deck = match Deck::parse(&text) {
            Ok(deck) => deck,
            // A half-written deck must not break `deck list`.
            Err(_) => continue,
        };
        decks.push(DeckSummary {
            cards: deck.maindeck_total(),
            sideboard_cards: deck.sideboard_total(),
            owned: owned_count_for(conn, &name)?,
            has_primer: primer_file(paths, &name).exists(),
            name,
        });
    }
    decks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(decks)
}

/// The deck name is the file stem (the same string used for collection rows).
fn deck_name_of(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Owned copies assigned to this deck in the collection.
fn owned_count_for(conn: &Connection, name: &str) -> anyhow::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(quantity), 0) FROM collection
         WHERE binder_type = 'deck' AND binder = ?1",
        [name],
        |r| r.get(0),
    )
    .context("counting owned deck cards")
}

/// `(deck-assigned copies, binder copies)` for one deck name.
///
/// Binder copies can fill deck slots (they are the user's cards); other
/// decks' copies never count. Read-only.
pub fn owned_copies(conn: &Connection, name: &str) -> anyhow::Result<(i64, i64)> {
    let in_deck = owned_count_for(conn, name)?;
    let in_binders: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(quantity), 0) FROM collection
         WHERE binder_type = 'binder'
           AND name IN (SELECT name FROM collection WHERE binder_type = 'deck' AND binder = ?1)",
            [name],
            |r| r.get(0),
        )
        .context("counting binder copies for deck names")?;
    Ok((in_deck, in_binders))
}
#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod store_tests;
