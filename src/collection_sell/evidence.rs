//! Deck coverage and canonical Spellbook evidence for sell plans.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::Context;
use rusqlite::Connection;

use super::{Candidate, SellOptions, binder_excluded};

/// Demand signals for one canonical card, separate from sale quantities.
#[derive(Debug, Default, Clone)]
pub(super) struct ComboEvidence {
    pub variants: usize,
    pub piece_sets: usize,
    pub owned_options: usize,
    pub examples: Vec<String>,
    pub interested: bool,
    pub deck_completion: bool,
}

/// Batched evidence and explicit coverage warnings.
#[derive(Default)]
pub(super) struct Evidence {
    pub cards: BTreeMap<String, ComboEvidence>,
    pub deck_needed: BTreeMap<String, i64>,
    pub combos_known: bool,
    pub decks_known: bool,
    pub warnings: Vec<String>,
}

/// Canonical names and color identities for face-safe combo joins.
#[derive(Default)]
struct Identities {
    names: HashMap<String, String>,
    colors: HashMap<String, BTreeSet<String>>,
}

/// A deck's playable cards, commanders, and color identity.
#[derive(Default)]
struct DeckEvidence {
    name: String,
    counts: BTreeMap<String, i64>,
    commanders: BTreeSet<String>,
    colors: BTreeSet<String>,
    identity_known: bool,
}

/// Load canonical names without guessing that similar cards are interchangeable.
fn identities(conn: &Connection) -> anyhow::Result<Identities> {
    let mut stmt = conn.prepare("SELECT name, color_identity FROM cards")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    let mut result = Identities::default();
    for row in rows {
        let (name, colors) = row?;
        if let Some(colors) = colors {
            result.colors.insert(
                name.clone(),
                serde_json::from_str(&colors)
                    .with_context(|| format!("reading color identity for {name}"))?,
            );
        }
        result.names.insert(name.clone(), name.clone());
        for face in name.split(" // ") {
            result.names.insert(face.to_string(), name.clone());
        }
    }
    Ok(result)
}

/// Read decklists once, retaining failures as report-level uncertainty.
fn decks(
    paths: &crate::paths::Paths,
    ids: &Identities,
    evidence: &mut Evidence,
) -> anyhow::Result<Vec<DeckEvidence>> {
    evidence.decks_known = true;
    let entries = match std::fs::read_dir(paths.decks_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading deck directory for sell protection"),
    };
    let mut paths = entries
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut result = Vec::new();
    for path in paths {
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let parsed = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|text| crate::deck::grammar::Deck::parse(&text));
        let deck = match parsed {
            Ok(deck) => deck,
            Err(error) => {
                evidence.decks_known = false;
                evidence.warnings.push(format!(
                    "Could not check {}: {error}. Sale quantities require review.",
                    path.display()
                ));
                continue;
            }
        };
        let mut row = DeckEvidence {
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .context("deck file has no UTF-8 name")?
                .to_string(),
            identity_known: true,
            ..DeckEvidence::default()
        };
        for (section, cards) in deck.sections {
            for card in cards {
                let name = ids.names.get(&card.name).unwrap_or(&card.name).clone();
                if crate::deck::grammar::is_maybeboard_section(&section) {
                    evidence.cards.entry(name).or_default().interested = true;
                    continue;
                }
                *row.counts.entry(name.clone()).or_default() += card.quantity;
                if section.eq_ignore_ascii_case("COMMANDER") {
                    row.commanders.insert(name.clone());
                    if let Some(colors) = ids.colors.get(&name) {
                        row.colors.extend(colors.iter().cloned());
                    } else {
                        row.identity_known = false;
                    }
                }
            }
        }
        result.push(row);
    }
    Ok(result)
}

/// Protect each deck's shortage using assignments to that specific deck.
fn deck_needs(
    conn: &Connection,
    decks: &[DeckEvidence],
    ids: &Identities,
) -> anyhow::Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT binder, name, SUM(quantity) FROM collection WHERE binder_type = 'deck' GROUP BY binder, name")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut assigned = HashMap::new();
    for row in rows {
        let (deck, name, quantity) = row?;
        let name = ids.names.get(&name).unwrap_or(&name).clone();
        *assigned.entry((deck, name)).or_insert(0) += quantity;
    }
    let mut needed = BTreeMap::new();
    for deck in decks {
        for (name, demand) in &deck.counts {
            let assigned = assigned
                .get(&(deck.name.clone(), name.clone()))
                .copied()
                .unwrap_or(0);
            *needed.entry(name.clone()).or_insert(0) += (demand - assigned).max(0);
        }
    }
    Ok(needed)
}

/// Check physical ownership and a compatible known commander when required.
fn owned_option(
    pieces: &BTreeMap<String, bool>,
    colors: &BTreeSet<String>,
    colors_known: bool,
    owned: &HashMap<String, i64>,
    decks: &[DeckEvidence],
    commander_format: bool,
) -> bool {
    if !pieces.keys().all(|n| owned.get(n).is_some_and(|q| *q > 0)) {
        return false;
    }
    if !commander_format {
        return !pieces.values().any(|v| *v);
    }
    colors_known
        && decks.iter().any(|deck| {
            !deck.commanders.is_empty()
                && deck.identity_known
                && colors.is_subset(&deck.colors)
                && pieces
                    .iter()
                    .all(|(n, required)| !required || deck.commanders.contains(n))
        })
}

/// Check whether a loose piece completes a combo in an existing deck.
fn completes_deck(
    candidate: &str,
    pieces: &BTreeMap<String, bool>,
    colors: &BTreeSet<String>,
    colors_known: bool,
    decks: &[DeckEvidence],
    commander_format: bool,
) -> bool {
    decks.iter().any(|deck| {
        if commander_format
            && (deck.commanders.is_empty()
                || !deck.identity_known
                || !colors_known
                || !colors.is_subset(&deck.colors))
        {
            return false;
        }
        if !commander_format && !deck.commanders.is_empty() {
            return false;
        }
        pieces.iter().all(|(name, required)| {
            if *required {
                deck.commanders.contains(name)
            } else {
                name == candidate || deck.counts.contains_key(name)
            }
        })
    })
}

/// Count variants and canonical piece sets with bounded personal examples.
fn combo_evidence(
    conn: &Connection,
    candidates: &[Candidate],
    ids: &Identities,
    decks: &[DeckEvidence],
    options: &SellOptions<'_>,
    evidence: &mut Evidence,
) -> anyhow::Result<()> {
    let format = options.format.unwrap_or("commander").to_ascii_lowercase();
    let names = candidates
        .iter()
        .flat_map(|c| {
            std::iter::once(c.name.clone()).chain(c.name.split(" // ").map(str::to_string))
        })
        .collect::<HashSet<_>>();
    let variants =
        crate::combos::filter_for_format(crate::combos::load_variants_for(conn, &names)?, &format);
    let raw_owned = owned_counts(conn, options.exclude_binders)?;
    let mut owned = HashMap::new();
    for (name, q) in raw_owned {
        *owned
            .entry(ids.names.get(&name).unwrap_or(&name).clone())
            .or_insert(0) += q;
    }
    let commander_format = matches!(format.as_str(), "commander" | "brawl" | "oathbreaker");
    let candidate_names = candidates
        .iter()
        .map(|c| c.name.as_str())
        .collect::<HashSet<_>>();
    let mut sets: HashMap<String, BTreeSet<Vec<(String, bool)>>> = HashMap::new();
    for (variant, raw_pieces) in variants {
        let mut pieces = BTreeMap::new();
        for piece in raw_pieces {
            let name = ids.names.get(&piece.name).unwrap_or(&piece.name).clone();
            *pieces.entry(name).or_insert(false) |= piece.must_be_commander;
        }
        let mut colors = BTreeSet::new();
        let mut colors_known = true;
        for name in pieces.keys() {
            if let Some(identity) = ids.colors.get(name) {
                colors.extend(identity.iter().cloned());
            } else {
                colors_known = false;
            }
        }
        let owned_option = owned_option(
            &pieces,
            &colors,
            colors_known,
            &owned,
            decks,
            commander_format,
        );
        for name in pieces
            .keys()
            .filter(|n| candidate_names.contains(n.as_str()))
        {
            let row = evidence.cards.entry(name.clone()).or_default();
            row.variants += 1;
            let unique = sets.entry(name.clone()).or_default().insert(
                pieces
                    .iter()
                    .map(|(n, required)| (n.clone(), *required))
                    .collect(),
            );
            row.piece_sets += usize::from(unique);
            row.owned_options += usize::from(unique && owned_option);
            row.deck_completion |= completes_deck(
                name,
                &pieces,
                &colors,
                colors_known,
                decks,
                commander_format,
            );
            if (owned_option || row.deck_completion) && row.examples.len() < 3 {
                row.examples.push(variant.id.clone());
            }
        }
    }
    Ok(())
}

/// Determine whether absence of a matching combo is meaningful in this snapshot.
fn combo_coverage(conn: &Connection, evidence: &mut Evidence) -> anyhow::Result<()> {
    let (count, updated): (i64, Option<String>) =
        conn.query_row("SELECT COUNT(*), MAX(updated_at) FROM combos", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    evidence.combos_known = count > 0
        && updated.as_deref().is_some_and(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .is_ok_and(|date| chrono::Utc::now().signed_duration_since(date).num_days() <= 7)
        });
    if !evidence.combos_known {
        evidence.warnings.push("Spellbook data is missing, undated, or over seven days old. No matches do not establish zero combo demand.".to_string());
    }
    Ok(())
}

/// Assemble quantity protection and usefulness evidence without writing data.
pub(super) fn load(
    paths: &crate::paths::Paths,
    conn: &Connection,
    candidates: &[Candidate],
    options: &SellOptions<'_>,
) -> anyhow::Result<Evidence> {
    let ids = identities(conn)?;
    let mut evidence = Evidence::default();
    let decks = decks(paths, &ids, &mut evidence)?;
    warn_unlisted_decks(conn, &decks, &mut evidence)?;
    evidence.deck_needed = deck_needs(conn, &decks, &ids)?;
    combo_coverage(conn, &mut evidence)?;
    combo_evidence(conn, candidates, &ids, &decks, options, &mut evidence)?;
    Ok(evidence)
}

/// Count combo pieces without treating collector binders as deckbuilding supply.
fn owned_counts(conn: &Connection, excluded: &[String]) -> anyhow::Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT name, binder, binder_type, SUM(quantity) FROM collection GROUP BY name, binder, binder_type")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut counts = HashMap::new();
    for row in rows {
        let (name, binder, kind, quantity) = row?;
        if kind == "binder" && binder_excluded(&binder, excluded) {
            continue;
        }
        *counts.entry(name).or_insert(0) += quantity;
    }
    Ok(counts)
}

/// Identify assignments whose decklists are unavailable without moving assigned copies.
fn warn_unlisted_decks(
    conn: &Connection,
    decks: &[DeckEvidence],
    evidence: &mut Evidence,
) -> anyhow::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT binder FROM collection WHERE binder_type = 'deck' ORDER BY binder",
    )?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let unlisted = names
        .into_iter()
        .filter(|name| !decks.iter().any(|d| &d.name == name))
        .collect::<Vec<_>>();
    if !unlisted.is_empty() {
        evidence.warnings.push(format!(
            "Assignments are protected, but these decks have no checked decklist: {}",
            unlisted.join(", ")
        ));
    }
    Ok(())
}
