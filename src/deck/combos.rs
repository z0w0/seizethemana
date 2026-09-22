// `stm deck combos`: bracket-aware combo audit over a deck, split by
// section.
//
// A static join against the Spellbook store (`load_variants_for` +
// `combos::candidates`); no simulation runs. Complete combos and one-card-
// away near misses report per section (COMMANDER / DECK / SIDEBOARD), with
// near-miss rows flagging whether the deck holds the piece and whether the
// completing card is owned (buy vs pull from the binder). `--bracket`
// flags combos — complete or near miss — whose Spellbook bracket tag
// breaks the target bracket (an "S"-tagged infinite-turn combo in a
// bracket-3 main deck is the Stationz case).

/// One combo audit row for a deck section.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComboRow {
    /// Piece names joined with " + ".
    pub combo: String,
    /// Spellbook variant id.
    pub id: String,
    /// Feature names the combo produces.
    pub produces: Vec<String>,
    /// Spellbook bracket tag ("S", "K", …), when classified.
    pub bracket_tag: Option<String>,
    /// EDHREC popularity rank of the combo (lower = more popular), when
    /// ranked.
    pub popularity: Option<i64>,
    /// True when the deck section holds every piece.
    pub complete: bool,
    /// Names of the missing pieces (near misses).
    pub missing: Vec<String>,
    /// True when the deck owns the completing piece (in any deck or
    /// binder) — the near-miss upgrade path is a swap, not a buy.
    pub owned: Option<bool>,
    /// True when every held piece resolves from this section (a main-deck
    /// and a sideboard piece do not combine inside one section).
    pub self_contained: bool,
    /// True when a commander-only piece is required (its resolution
    /// depends on the commander staying in the command zone).
    pub requires_commander: bool,
}

/// One section's audit result.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SectionCombos {
    pub section: String,
    pub complete: Vec<ComboRow>,
    pub near_misses: Vec<ComboRow>,
    /// Combos whose bracket tag exceeds the target bracket.
    pub bracket_breaks: Vec<String>,
}

/// Resolve a deck section's combo candidates against the store.
///
/// `names` is the section's card-name set; commander-required pieces
/// resolve through the deck's commander names so they never report as
/// missing. `owned` is the collection-wide owned-counts map for the
/// near-miss `owned` flag.
pub fn section_combos(
    conn: &rusqlite::Connection,
    section: &str,
    names: &std::collections::HashSet<String>,
    commander_names: &std::collections::HashSet<String>,
    format_key: &str,
    owned_counts: &std::collections::HashMap<String, i64>,
) -> anyhow::Result<SectionCombos> {
    let variants = crate::combos::load_variants_for(conn, names)?;
    let mut out = SectionCombos {
        section: section.to_string(),
        ..Default::default()
    };
    for (variant, pieces) in variants {
        if !variant.legalities.get(format_key).copied().unwrap_or(false) {
            continue;
        }
        // One slot per ordinal, preferring faces the section holds.
        let mut by_ordinal: std::collections::BTreeMap<i64, &crate::spellbook::ComboPieceRow> =
            std::collections::BTreeMap::new();
        let mut requires_commander = false;
        for row in &pieces {
            if row.must_be_commander {
                requires_commander = true;
            }
            let held = names.contains(&row.name);
            by_ordinal
                .entry(row.ordinal)
                .and_modify(|slot| {
                    if !names.contains(&slot.name) && held {
                        *slot = row;
                    }
                })
                .or_insert(row);
        }
        let mut missing = Vec::new();
        let mut self_contained = true;
        for row in by_ordinal.values() {
            if row.must_be_commander || commander_names.contains(&row.name) {
                continue;
            }
            if !names.contains(&row.name) {
                missing.push(row.name.clone());
                self_contained = false;
            }
        }
        // The completing piece's ownership: a near miss names exactly the
        // pieces outside this section; owned means any copy anywhere.
        let owned = missing.first().map(|name| owned_counts.contains_key(name));
        let row = ComboRow {
            combo: by_ordinal
                .values()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(" + "),
            id: variant.id.clone(),
            produces: variant.produces.clone(),
            bracket_tag: variant.bracket_tag.clone(),
            popularity: variant.popularity,
            complete: missing.is_empty(),
            missing: missing.clone(),
            owned,
            self_contained,
            requires_commander,
        };
        if row.complete {
            out.complete.push(row);
        } else {
            out.near_misses.push(row);
        }
    }
    Ok(out)
}

/// True when a combo's Spellbook bracket tag breaks the target bracket.
///
/// Only the two hard tags count: "S" (cEDH-grade infinite-turn loops) and
/// "K" (infinite combo with few or no alternative pieces). Both break
/// brackets 1-4; a tagged card never breaks bracket 5, and an untagged
/// combo breaks nothing here.
pub fn breaks_bracket(tag: Option<&str>, bracket: u8) -> bool {
    let Some(tag) = tag else { return false };
    matches!(tag, "S" | "K") && bracket <= 4
}

/// Entry point for `stm deck combos <name>`.
pub fn combos(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    format: Option<&str>,
    bracket: Option<u8>,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = super::store::load_deck(paths, name)?;
    if !super::simulator::store_has_combos_pub(conn) {
        out.error("no combo data in the store");
        out.hint("run 'stm setup' or 'stm sync' to refresh combo data");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    // Format key: explicit flag, else the deck's inferred format.
    let format_key = format
        .map(str::to_string)
        .unwrap_or_else(|| super::simulator::deck::infer_format_key(&deck));

    // Commander names: the COMMANDER section's entries.
    let commander_names: std::collections::HashSet<String> = deck
        .sections
        .iter()
        .filter(|(s, _)| s.eq_ignore_ascii_case("COMMANDER"))
        .flat_map(|(_, e)| e.iter().map(|e| e.name.clone()))
        .collect();

    let mut sections = Vec::new();
    let owned_counts = crate::collection::owned_counts_all(conn)?;
    for (section, entries) in &deck.sections {
        let names: std::collections::HashSet<String> =
            entries.iter().map(|e| e.name.clone()).collect();
        match section_combos(
            conn,
            section,
            &names,
            &commander_names,
            &format_key,
            &owned_counts,
        ) {
            Ok(mut c) => {
                if let Some(target) = bracket {
                    c.bracket_breaks = c
                        .complete
                        .iter()
                        .chain(c.near_misses.iter())
                        .filter(|r| breaks_bracket(r.bracket_tag.as_deref(), target))
                        .map(|r| r.combo.clone())
                        .collect();
                }
                sections.push(c);
            }
            Err(err) => out.warning(&format!("combo lookup failed for {section}: {err:#}")),
        }
    }

    if json {
        let payload: Vec<&SectionCombos> = sections.iter().collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(crate::cli::codes::OK);
    }

    let styles = out.styles();
    let mut any = false;
    for section in &sections {
        if section.complete.is_empty() && section.near_misses.is_empty() {
            continue;
        }
        any = true;
        println!();
        println!("{}", styles.header(&format!("// {}", section.section)));
        if !section.complete.is_empty() {
            println!("  {}:", styles.dim("Complete"));
            for row in &section.complete {
                let bracket = row
                    .bracket_tag
                    .as_deref()
                    .map(|b| format!(" [{b}]"))
                    .unwrap_or_default();
                println!(
                    "    {}  {}{}",
                    styles.card_name(&row.combo),
                    styles.dim(&row.produces.join(", ")),
                    styles.dim(&bracket)
                );
            }
        }
        if !section.near_misses.is_empty() {
            println!("  {}:", styles.dim("One card away"));
            for row in &section.near_misses {
                let owned = match row.owned {
                    Some(true) => styles.glyph("✓ owned", crate::output::GlyphKind::Good),
                    Some(false) => styles.dim("to buy"),
                    None => styles.dim(""),
                };
                println!(
                    "    {}  {}  {}  {}",
                    styles.card_name(row.missing.first().map(String::as_str).unwrap_or("?")),
                    styles.dim(&format!("completes {}", row.combo)),
                    styles.dim(if row.self_contained {
                        "maindeck-only"
                    } else {
                        "needs another section"
                    }),
                    owned
                );
            }
        }
        if !section.bracket_breaks.is_empty() {
            println!("  {}:", styles.dim("Breaks the bracket"));
            for combo in &section.bracket_breaks {
                println!("    {}", styles.error(combo));
            }
        }
    }
    if !any {
        println!(
            "{}",
            styles.success("no combos found in the store for this deck")
        );
        return Ok(crate::cli::codes::OK);
    }
    Ok(crate::cli::codes::OK)
}

#[cfg(test)]
#[path = "tests/combos_tests.rs"]
mod combos_tests;
