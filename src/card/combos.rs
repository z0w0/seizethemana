use super::ambiguous_error;
use anyhow::Context;
/// One Spellbook combo variant the card takes part in, pieces joined.
pub struct CardCombo {
    pub variant: crate::spellbook::ComboVariant,
    pub pieces: Vec<crate::spellbook::ComboPieceRow>,
    /// True when any piece must be the commander.
    pub requires_commander: bool,
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
    let seed = match crate::db::resolve_name(conn, typed).context("resolving card name")? {
        crate::db::NameMatch::Found(card) => card,
        crate::db::NameMatch::Ambiguous { candidates, total } => {
            return ambiguous_error(out, typed, &candidates, total);
        }
        crate::db::NameMatch::NotFound => {
            out.error(&format!("no card named {typed:?}"));
            out.hint("names resolve by exact match, case, or unique prefix");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
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
            println!("[]");
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
    sorted.sort_by(|a, b| {
        b.0.popularity
            .unwrap_or(0)
            .cmp(&a.0.popularity.unwrap_or(0))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
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
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|combo| {
            serde_json::json!({
                "id": combo.variant.id,
                "produces": combo.variant.produces,
                "mana_value_needed": combo.variant.mana_value_needed,
                "bracket_tag": combo.variant.bracket_tag,
                "popularity": combo.variant.popularity,
                "legalities": combo.variant.legalities,
                "requires_commander": combo.requires_commander,
                "pieces": combo.pieces.iter().map(|p| serde_json::json!({
                    "name": p.name,
                    "zones": p.zones,
                    "must_be_commander": p.must_be_commander,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
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
        println!(
            "{:>2}. {}{} → {}{}{}{}",
            i + 1,
            styles.card_name(&pieces),
            styles.dim(cmdr),
            styles.dim(&produces),
            styles.dim(&bracket),
            styles.dim(&pop),
            styles.dim(&legal_note),
        );
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
