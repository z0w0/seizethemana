// `stm deck mana <name>`: the static colored-source audit command
// (`deck mana` runs the Karsten census with no simulation; the same
// block rides along in the simulate report as `colored_sources`).

use rusqlite::Connection;

use super::grammar::Deck;
use super::store::load_deck;

/// The card census and shape the audit reads: `(card, copies)` pairs,
/// the deck's color letters, and whether the deck is commander-shaped.
pub struct AuditInput {
    /// Maindeck (plus commander) card rows with copy counts.
    pub rows: Vec<(crate::db::CardRow, f64)>,
    /// The deck's WUBRG color letters.
    pub letters: String,
    /// True when the deck is commander-shaped.
    pub is_commander: bool,
}

/// Colored-source audit rows for a deck: the card census (maindeck plus
/// commander) as `(card, copies)` pairs, the deck's color letters, and
/// whether the deck is commander-shaped.
fn mana_audit_input(conn: &Connection, deck: &Deck) -> anyhow::Result<AuditInput> {
    let cards_by_name = super::stats::lookup_names(conn, deck)?;
    let mut rows: Vec<(crate::db::CardRow, f64)> = Vec::new();
    for (section, entries) in &deck.sections {
        let in_audit = !super::grammar::is_bench_section(section);
        for entry in entries {
            if !in_audit {
                break;
            }
            if let Some(card) = cards_by_name.get(&entry.name) {
                rows.push((card.clone(), entry.quantity as f64));
            }
        }
    }
    let is_commander = super::legal::is_commander(deck, None);
    // Deck colors: the union of every commander's identity (partner
    // pairs) when one exists, else the union of printed colors across the
    // maindeck.
    let letters = if is_commander {
        let mut letters = String::new();
        for (s, entries) in &deck.sections {
            if !s.eq_ignore_ascii_case("COMMANDER") {
                continue;
            }
            for entry in entries {
                let Some(card) = cards_by_name.get(&entry.name) else {
                    continue;
                };
                let identity: String = serde_json::from_str::<Vec<String>>(&card.color_identity)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|c| c.chars().next())
                    .collect();
                for c in identity.chars() {
                    if !letters.contains(c) {
                        letters.push(c);
                    }
                }
            }
        }
        letters
    } else {
        super::suggest::deck_color_letters(deck, &cards_by_name)
    };
    Ok(AuditInput {
        rows,
        letters,
        is_commander,
    })
}

/// Run the Karsten colored-source audit for a deck.
pub fn mana_audit_for(conn: &Connection, deck: &Deck) -> anyhow::Result<super::mana_audit::Audit> {
    let input = mana_audit_input(conn, deck)?;
    Ok(super::mana_audit::audit(
        &input.rows,
        &input.letters,
        input.is_commander,
    ))
}

/// Human view of the colored-source audit: per-color table, worst
/// deficits, and a category suggestion line.
pub fn print_mana_audit(styles: &crate::output::Styles, audit: &super::mana_audit::Audit) {
    println!();
    println!("{}", styles.header("Colored sources"));
    let letters = ['W', 'U', 'B', 'R', 'G'];
    let total: f64 = audit.sources.iter().sum();
    for (i, letter) in letters.iter().enumerate() {
        if audit.sources[i] <= 0.0 {
            continue;
        }
        println!(
            "  {} {:>5.1} sources · {:>4.1} untapped turn-1",
            styles.color_letters(&letter.to_string()),
            audit.sources[i],
            audit.untapped_t1[i],
        );
    }
    if total <= 0.0 {
        println!("  (no card data for this deck's colors)");
    }
    if audit.tapland_count > 0 {
        println!(
            "  {} tap land{}",
            audit.tapland_count,
            if audit.tapland_count == 1 { "" } else { "s" }
        );
    }
    let worst = super::mana_audit::worst_deficits(audit, 3);
    if worst.is_empty() {
        println!("  {}", styles.success("all pip requirements met"));
    } else {
        for line in &worst {
            println!("  {}", styles.warning(line));
        }
        if let Some(row) = audit.requirements.iter().find(|r| !r.ok) {
            for (letter, d) in &row.deficit {
                if *d > 0.0 {
                    println!(
                        "  suggest: add 2-3 more {} sources",
                        styles.color_letters(&letter.to_string())
                    );
                    break;
                }
            }
        }
    }
}

/// Entry point for `stm deck mana <name>`: the static colored-source
/// audit. Reads the deck census only — no simulation. `format` is
/// accepted for CLI symmetry with `deck simulate`/`deck cuts`; the
/// audit's commander/60-card split infers from the deck shape.
pub fn mana(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    name: &str,
    _format: Option<&str>,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = load_deck(paths, name)?;
    let audit = mana_audit_for(conn, &deck)?;
    if json {
        let v = super::mana_audit::colored_sources_json(&audit);
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(crate::cli::codes::OK);
    }
    let styles = out.styles();
    println!(
        "{}  {}",
        styles.header(name),
        styles.dim(&format!("lands: {}", audit.lands))
    );
    print_mana_audit(&styles, &audit);
    Ok(crate::cli::codes::OK)
}
