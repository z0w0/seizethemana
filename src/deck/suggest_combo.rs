// Combo completions for `deck suggest`: rank cards that complete near-miss
// combo variants in the store. This is the answer a bare
// `stm deck suggest <name>` gives (no query, no role). Kept as its own
// module; it shares the deck/identity helpers via `super::suggest`.

use rusqlite::Connection;

use super::store::load_deck;
use crate::db::CardRow;
use crate::deck::grammar::is_bench_section;

/// One ranked completion: the missing card, how many variants it
/// completes, and the best example.
pub struct Completion {
    card: CardRow,
    /// Near-miss variants this card completes.
    variants_completed: usize,
    /// (piece names joined, Spellbook bracket tag) of the best example.
    best: BestExample,
    /// True when any completed variant wins the game.
    win_game: bool,
    /// Highest Spellbook popularity among the completed variants.
    popularity: i64,
    /// Ranking score in `[0, 1]`: variant count blended with popularity,
    /// each normalized by the row-set maximum.
    score: f32,
}

/// The best example combo a missing card completes: (pieces, bracket tag).
type BestExample = (String, Option<String>);

/// Rank cards that complete one-card-away combo variants for the deck.
///
/// Every near-miss variant (the deck holds all pieces but one) contributes
/// a completion for its missing card. Ranking: variants completed, the
/// "win the game" feature, Spellbook popularity, EDHREC rank. An empty
/// combo store or no near-misses yields NO_RESULTS with a note, never an
/// error.
///
/// `format` pins the combo legality filter (a commander-shaped deck
/// defaults to commander). Commander-required variants and pieces are
/// excluded for 60-card formats.
///
/// # Errors
/// Propagates SQLite failures.
#[allow(clippy::too_many_arguments)]
pub fn run_combo_suggest(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    format: Option<&str>,
    bracket: Option<u8>,
    max_price: Option<f64>,
    limit: u32,
    exclude: &[String],
    json: bool,
) -> anyhow::Result<i32> {
    let excluded = super::suggest::resolve_exclusions(exclude)?;
    let excluded_set: std::collections::HashSet<&str> =
        excluded.iter().map(String::as_str).collect();
    let combos_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM combos", [], |r| r.get(0))
        .unwrap_or(0);
    if combos_count == 0 {
        out.error("no combo data in the store");
        out.hint("run 'stm sync' online to pull combo variants from Commander Spellbook");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let (_path, deck) = load_deck(paths, deck_name)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck)?;
    let identity = super::suggest::commander_identity(&deck, &cards_by_name);
    let is_commander = super::legal::is_commander(&deck, None);
    // No pinned format: commander-shaped decks play commander; everything
    // else takes the combos its cards appear in, minus commander-required
    // variants.
    let combo_format = match format {
        Some(format) => Some(format),
        None if is_commander => Some("commander"),
        None => None,
    };

    // The deck's own names drive the candidate join; commander decks keep
    // their bench sections (sideboard/maybeboard) out of the join.
    let deck_names: std::collections::HashSet<String> = deck
        .sections
        .iter()
        .filter(|(section, _)| !(is_commander && is_bench_section(section)))
        .flat_map(|(_, entries)| entries.iter().map(|e| e.name.clone()))
        .collect();

    let variants = crate::combos::load_variants_for(conn, &deck_names)?;
    let variants = match combo_format {
        Some(format) => crate::combos::filter_for_format(variants, format),
        None => variants
            .into_iter()
            .filter(|(_, pieces)| !crate::combos::requires_commander(pieces))
            .collect(),
    };
    if variants.is_empty() {
        out.error("no combo variants touch this deck's cards");
        out.hint("add cards that participate in known combo variants");
        return Ok(crate::cli::codes::NO_RESULTS);
    }

    // Group near-miss variants by their one missing piece.
    let mut by_missing: std::collections::HashMap<
        String,
        Vec<(
            crate::spellbook::ComboVariant,
            Vec<crate::spellbook::ComboPieceRow>,
        )>,
    > = std::collections::HashMap::new();
    for (variant, pieces) in &variants {
        // One slot per ordinal: a face split produces several rows with
        // the same ordinal (the simulator's combos.rs does the same).
        // Prefer a face the deck holds; otherwise keep the first-seen row.
        let mut by_ordinal: std::collections::BTreeMap<i64, &crate::spellbook::ComboPieceRow> =
            std::collections::BTreeMap::new();
        for piece in pieces {
            by_ordinal
                .entry(piece.ordinal)
                .and_modify(|slot| {
                    let slot_held = deck_names.contains(&slot.name);
                    let held = deck_names.contains(&piece.name);
                    if !slot_held && held {
                        *slot = piece;
                    }
                })
                .or_insert(piece);
        }
        let slots: Vec<&crate::spellbook::ComboPieceRow> = by_ordinal.values().copied().collect();
        let held: std::collections::HashSet<&str> = slots
            .iter()
            .filter(|p| deck_names.contains(&p.name))
            .map(|p| p.name.as_str())
            .collect();
        if slots.len() < 2 || held.len() + 1 < slots.len() {
            continue;
        }
        let missing: Vec<&str> = slots
            .iter()
            .map(|p| p.name.as_str())
            .filter(|name| !held.contains(name))
            .collect();
        if missing.len() == 1 && !excluded_set.contains(missing[0]) {
            by_missing
                .entry(missing[0].to_string())
                .or_default()
                .push(((*variant).clone(), pieces.clone()));
        }
    }

    let mut completions: Vec<Completion> = by_missing
        .iter()
        .filter_map(|(name, hits)| {
            completion(
                conn,
                name,
                hits,
                is_commander,
                &identity,
                combo_format,
                bracket,
            )
        })
        .collect();
    completions.sort_by(|a, b| {
        b.variants_completed
            .cmp(&a.variants_completed)
            .then_with(|| b.win_game.cmp(&a.win_game))
            .then_with(|| b.popularity.cmp(&a.popularity))
            .then_with(|| {
                a.card
                    .edhrec_rank
                    .unwrap_or(i64::MAX)
                    .cmp(&b.card.edhrec_rank.unwrap_or(i64::MAX))
            })
            .then_with(|| a.card.name.cmp(&b.card.name))
    });
    // Ranking score for JSON rows: variants completed and popularity, each
    // normalized by the row-set maximum, blended 60/40. Computed after the
    // sort so the score follows the same ranking the table shows.
    let max_variants = completions
        .iter()
        .map(|c| c.variants_completed)
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let max_popularity = completions
        .iter()
        .map(|c| c.popularity)
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    for c in &mut completions {
        let variants = c.variants_completed as f32 / max_variants;
        let pop = c.popularity as f32 / max_popularity;
        c.score = 0.6 * variants + 0.4 * pop;
    }
    // --max-price: the budget cap applies before the limit cut; unpriced
    // candidates are excluded.
    if let Some(max_price) = max_price {
        let (kept, hidden) =
            crate::prints::retain_by_price(conn, completions, |c| &c.card.name, max_price)?;
        completions = kept;
        if !json && hidden > 0 {
            println!(
                "{}",
                out.styles()
                    .note(&crate::prints::price_cap_note(max_price, hidden))
            );
        }
    }
    completions.truncate(limit as usize);
    if completions.is_empty() {
        out.error("no one-card-away combos found");
        out.hint("add a second combo piece or widen the identity filter");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        print_json(conn, &completions)?;
    } else {
        print_text(conn, out, deck_name, &completions)?;
    }
    Ok(crate::cli::codes::OK)
}

#[cfg(test)]
#[path = "tests/suggest_combo_tests.rs"]
mod suggest_combo_tests;

/// Build one completion when the card passes the identity, format, and
/// bracket filters.
#[allow(clippy::too_many_arguments)]
fn completion(
    conn: &Connection,
    name: &str,
    hits: &[(
        crate::spellbook::ComboVariant,
        Vec<crate::spellbook::ComboPieceRow>,
    )],
    is_commander: bool,
    identity: &str,
    format: Option<&str>,
    bracket: Option<u8>,
) -> Option<Completion> {
    let card = crate::db::get_card(conn, name).ok().flatten()?;
    if is_commander && !super::suggest::identity_ok(&card, identity) {
        return None;
    }
    if is_commander && !super::suggest::card_is_commander_legal(&card) {
        return None;
    }
    // Legality gate: for 60-card decks with no format pinned, a card must
    // be legal in at least one 60-card format (the same gate `gather_hits`
    // uses); `format` pinned wins when given.
    if !super::suggest::card_legal_in(&card, format)
        && (format.is_some() || !super::suggest::card_legal_in_any_60(&card))
    {
        return None;
    }
    if !super::suggest::bracket_allows(bracket, &card, 0) {
        return None;
    }
    let win_game = hits.iter().any(|(v, _)| {
        v.produces
            .iter()
            .any(|p| p.eq_ignore_ascii_case("Win the game"))
    });
    let popularity = hits
        .iter()
        .filter_map(|(v, _)| v.popularity)
        .max()
        .unwrap_or(0);
    let best = hits
        .iter()
        .max_by_key(|(v, _)| v.popularity.unwrap_or(0))
        .map(|(v, pieces)| {
            let combo = pieces
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(" + ");
            (combo, v.bracket_tag.clone())
        })
        .unwrap_or_default();
    Some(Completion {
        card,
        variants_completed: hits.len(),
        best,
        win_game,
        popularity,
        score: 0.0,
    })
}

/// JSON rows: one object per completion, `combo` naming the best example.
fn print_json(conn: &Connection, completions: &[Completion]) -> anyhow::Result<()> {
    let owned = crate::collection::owned_counts_all(conn)?;
    let names: Vec<String> = completions.iter().map(|c| c.card.name.clone()).collect();
    // One batched query per finish kind instead of four per card name.
    let ranges = crate::prints::price_ranges(conn, &names)?;
    let items: Vec<serde_json::Value> = completions
        .iter()
        .map(|c| {
            let identity: serde_json::Value =
                serde_json::from_str(&c.card.color_identity).unwrap_or_default();
            serde_json::json!({
                "name": c.card.name,
                "oracle_id": c.card.oracle_id,
                "mana_cost": c.card.mana_cost,
                "cmc": c.card.cmc,
                "type_line": c.card.type_line,
                "edhrec_rank": c.card.edhrec_rank,
                "game_changer": c.card.game_changer,
                "owned": owned.get(&c.card.name).copied().unwrap_or(0),
                "score": (c.score * 10_000.0).round() / 10_000.0,
                "price": ranges
                    .get(&c.card.name)
                    .and_then(|r| r.cheapest.as_ref())
                    .and_then(|p| p.usd),
                "combo": {
                    "pieces": c.best.0,
                    "bracket_tag": c.best.1,
                    "produces": if c.win_game { vec!["Win the game"] } else { vec![] },
                    "variants_completed": c.variants_completed,
                },
                "oracle_text": c.card.oracle_text,
                "color_identity": identity,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Human table: rank, name, cost, ownership, and the combo each card
/// completes.
fn print_text(
    conn: &Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    completions: &[Completion],
) -> anyhow::Result<()> {
    let owned = crate::collection::owned_counts_all(conn)?;
    // Batched prices for the ownership note below.
    let names: Vec<String> = completions.iter().map(|c| c.card.name.clone()).collect();
    let ranges = crate::prints::price_ranges(conn, &names)?;
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Combo completions"),
        styles.dim(&format!(
            "one card away for {deck_name:?} (ranked by variants completed)"
        ))
    );
    for (i, c) in completions.iter().enumerate() {
        let copies = owned.get(&c.card.name).copied().unwrap_or(0);
        let own_note = if copies > 0 {
            styles.success(&format!("own {}", styles.thousands(copies)))
        } else {
            match ranges
                .get(&c.card.name)
                .and_then(|r| r.cheapest.as_ref())
                .and_then(|p| p.usd)
            {
                Some(p) => format!("buy {}", styles.money(p)),
                None => "unpriced".to_string(),
            }
        };
        let gc = if c.card.game_changer == Some(true) {
            " [GC]"
        } else {
            ""
        };
        let count_note = styles.dim(&format!("completes {}", c.variants_completed));
        let example = styles.dim(&format!(
            " {}{}",
            c.best.0,
            c.best
                .1
                .as_deref()
                .map(|b| format!(" [{b}]"))
                .unwrap_or_default()
        ));
        let gc_note = format!("{gc}{}", example);
        println!(
            "{:>2}. {} {} {} {} {}",
            i + 1,
            styles.card_name(&c.card.name),
            styles.mana_pips(&c.card.mana_cost),
            count_note,
            styles.dim(&own_note),
            styles.dim(&gc_note)
        );
    }
    Ok(())
}
