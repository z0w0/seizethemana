//! `stm card combos`: Spellbook combos that include one card.

use super::resolve_card_or_report;

/// One Spellbook combo variant the card takes part in, pieces joined.
pub struct CardCombo {
    /// The Spellbook combo variant.
    pub variant: crate::spellbook::ComboVariant,
    /// Cards that make up the combo.
    pub pieces: Vec<crate::spellbook::ComboPieceRow>,
    /// True when any piece must be the commander.
    pub requires_commander: bool,
}

/// One typed card piece in a Spellbook combo report.
#[derive(Debug, serde::Serialize)]
struct ComboPieceReport {
    /// Card name.
    name: String,
    /// Required zones.
    zones: Vec<String>,
    /// Whether the piece must be the commander.
    must_be_commander: bool,
}

/// One typed Spellbook combo row.
#[derive(Debug, serde::Serialize)]
struct CardComboReport {
    /// Spellbook variant identifier.
    id: String,
    /// Effects the combo produces.
    produces: Vec<String>,
    /// Mana value required by the combo.
    mana_value_needed: i64,
    /// Spellbook bracket tag.
    bracket_tag: Option<String>,
    /// Spellbook popularity.
    popularity: Option<i64>,
    /// Legality by format.
    legalities: std::collections::BTreeMap<String, bool>,
    /// Whether a piece must be the commander.
    requires_commander: bool,
    /// Combo pieces.
    pieces: Vec<ComboPieceReport>,
}

/// Entry point for `stm card combos <name>`.
///
/// Exits 3 when the card is unknown or takes part in no combo (after
/// `--format` filtering). The list is sorted by Spellbook popularity, best
/// first.
#[allow(clippy::too_many_arguments)]
pub fn run_combos(
    paths: &crate::paths::Paths,
    conn: &mut rusqlite::Connection,
    out: &mut crate::output::Output,
    typed: &str,
    format: Option<&str>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let Some(seed) = resolve_card_or_report(out, conn, typed)? else {
        return Ok(crate::cli::codes::NO_RESULTS);
    };
    let mut names = std::collections::HashSet::new();
    names.insert(seed.name.clone());
    let variants = crate::combos::load_variants_for(conn, &names)?;
    let combos = match format {
        Some(format) => crate::combos::filter_for_format(variants, format),
        None => variants,
    };
    if combos.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&Vec::<CardComboReport>::new())?
            );
        } else {
            match format {
                Some(format) => {
                    out.error(&format!(
                        "no combos with {} are legal in {format}",
                        seed.name
                    ));
                    out.hint("drop --format to see every combo the card appears in");
                }
                None => {
                    out.error(&format!("no combos include {}", seed.name));
                    out.hint("run 'stm sync' online first; the combo list needs a sync");
                }
            }
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let mut sorted = combos;
    sort_by_popularity(&mut sorted);
    sorted.truncate(limit as usize);
    let rows: Vec<CardCombo> = sorted
        .into_iter()
        .map(|(variant, pieces)| CardCombo {
            requires_commander: crate::combos::requires_commander(&pieces),
            variant,
            pieces,
        })
        .collect();
    if json {
        print_combos_json(&rows)?;
    } else {
        print_combos_text(out, &seed.name, &rows);
    }
    Ok(crate::cli::codes::OK)
}

/// JSON rows for `card combos`: one object per variant.
fn print_combos_json(rows: &[CardCombo]) -> anyhow::Result<()> {
    let items: Vec<CardComboReport> = rows
        .iter()
        .map(|combo| CardComboReport {
            id: combo.variant.id.clone(),
            produces: combo.variant.produces.clone(),
            mana_value_needed: combo.variant.mana_value_needed,
            bracket_tag: combo.variant.bracket_tag.clone(),
            popularity: combo.variant.popularity,
            legalities: combo
                .variant
                .legalities
                .iter()
                .map(|(format, legal)| (format.clone(), *legal))
                .collect(),
            requires_commander: combo.requires_commander,
            pieces: combo
                .pieces
                .iter()
                .map(|piece| ComboPieceReport {
                    name: piece.name.clone(),
                    zones: piece.zones.clone(),
                    must_be_commander: piece.must_be_commander,
                })
                .collect(),
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Sort combo variants by popularity then id: the most popular first, and
/// ties broken by variant id so the order is stable.
fn sort_by_popularity(
    combos: &mut [(
        crate::spellbook::ComboVariant,
        Vec<crate::spellbook::ComboPieceRow>,
    )],
) {
    combos.sort_by(|a, b| {
        b.0.popularity
            .unwrap_or(0)
            .cmp(&a.0.popularity.unwrap_or(0))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
}

/// One line of the human combo table. Piece names keep the card-name style;
/// the rest is dimmed metadata.
fn combo_line(index: usize, combo: &CardCombo, styles: &crate::output::Styles) -> String {
    let mut names: Vec<String> = combo.pieces.iter().map(|p| p.name.clone()).collect();
    names.sort();
    names.dedup();
    let pieces = names.join(" + ");
    let cmdr = if combo.requires_commander {
        " (commander)"
    } else {
        ""
    };
    let produces = combo.variant.produces.first().cloned().unwrap_or_default();
    let bracket = combo
        .variant
        .bracket_tag
        .as_deref()
        .map(|t| format!(" [{t}]"))
        .unwrap_or_default();
    let pop = match combo.variant.popularity {
        Some(n) => format!(" pop {}", styles.thousands(n)),
        None => String::new(),
    };
    let legal: Vec<&str> = COMBO_NOTE_FORMATS
        .iter()
        .copied()
        .filter(|f| combo.variant.legalities.get(*f).copied().unwrap_or(false))
        .collect();
    let legal_note = if legal.is_empty() {
        String::new()
    } else {
        format!("  legal: {}", legal.join(", "))
    };
    format!(
        "{:>2}. {}{} → {}{}{}{}",
        index + 1,
        styles.card_name(&pieces),
        styles.dim(cmdr),
        styles.dim(&produces),
        styles.dim(&bracket),
        styles.dim(&pop),
        styles.dim(&legal_note),
    )
}

/// Human table for `card combos`, popularity first.
fn print_combos_text(out: &crate::output::Output, seed_name: &str, rows: &[CardCombo]) {
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Combos with"),
        styles.card_name(seed_name)
    );
    for (i, combo) in rows.iter().enumerate() {
        println!("{}", combo_line(i, combo, &styles));
    }
}

/// Formats shown in the human `legal:` note, most-played first.
const COMBO_NOTE_FORMATS: &[&str] = &[
    "standard",
    "pioneer",
    "modern",
    "legacy",
    "vintage",
    "pauper",
    "commander",
    "brawl",
    "oathbreaker",
    "premodern",
    "alchemy",
    "predh",
];

#[cfg(test)]
#[path = "tests/combos_tests.rs"]
mod combos_tests;
