// Combo completions for `deck suggest`: rank cards that complete near-miss
// combo variants in the store. This is the answer a bare
// `stm deck suggest <name>` gives (no query, no role). Kept as its own
// module; it shares the deck/identity helpers via `super::suggest`.

use rusqlite::Connection;

use super::legal::infer_format;
use super::store::load_deck;
use crate::db::CardRow;

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
/// # Errors
/// Propagates SQLite failures.
pub fn run_combo_suggest(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    bracket: Option<u8>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    let combos_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM combos", [], |r| r.get(0))
        .unwrap_or(0);
    if combos_count == 0 {
        out.error("no combo data in the store");
        out.hint("run 'stm sync' online to pull combo variants from Commander Spellbook");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let (_path, deck) = load_deck(paths, deck_name)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck);
    let identity = super::suggest::commander_identity(&deck, &cards_by_name);
    let is_commander = matches!(infer_format(&deck), super::legal::InferredFormat::Commander);

    // The deck's own names drive the candidate join; commander decks keep
    // their sideboard wishlist out of the join.
    let deck_names: std::collections::HashSet<String> = deck
        .sections
        .iter()
        .filter(|(section, _)| !(is_commander && section.eq_ignore_ascii_case("SIDEBOARD")))
        .flat_map(|(_, entries)| entries.iter().map(|e| e.name.clone()))
        .collect();

    let variants = crate::spellbook::load_variants_for(conn, &deck_names)?;
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
        let held: std::collections::HashSet<&str> = pieces
            .iter()
            .filter(|p| deck_names.contains(&p.name))
            .map(|p| p.name.as_str())
            .collect();
        if pieces.len() < 2 || held.len() + 1 < pieces.len() {
            continue;
        }
        let missing: Vec<&str> = pieces
            .iter()
            .map(|p| p.name.as_str())
            .filter(|name| !held.contains(name))
            .collect();
        if missing.len() == 1 {
            by_missing
                .entry(missing[0].to_string())
                .or_default()
                .push(((*variant).clone(), pieces.clone()));
        }
    }

    let mut completions: Vec<Completion> = by_missing
        .iter()
        .filter_map(|(name, hits)| completion(conn, name, hits, is_commander, &identity, bracket))
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

/// Build one completion when the card passes the identity, legality, and
/// bracket filters.
fn completion(
    conn: &Connection,
    name: &str,
    hits: &[(
        crate::spellbook::ComboVariant,
        Vec<crate::spellbook::ComboPieceRow>,
    )],
    is_commander: bool,
    identity: &str,
    bracket: Option<u8>,
) -> Option<Completion> {
    let card = crate::db::get_card(conn, name).ok().flatten()?;
    if is_commander && !super::suggest::identity_ok(&card, identity) {
        return None;
    }
    if is_commander && !super::suggest::card_is_commander_legal(&card) {
        return None;
    }
    if !super::suggest::bracket_allows(bracket, &card) {
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
    })
}

/// JSON rows: one object per completion, `combo` naming the best example.
fn print_json(conn: &Connection, completions: &[Completion]) -> anyhow::Result<()> {
    let owned = crate::collection::owned_names_all(conn)?;
    let items: Vec<serde_json::Value> = completions
        .iter()
        .map(|c| {
            let identity: serde_json::Value =
                serde_json::from_str(&c.card.color_identity).unwrap_or_default();
            serde_json::json!({
                "name": c.card.name,
                "mana_cost": c.card.mana_cost,
                "cmc": c.card.cmc,
                "type_line": c.card.type_line,
                "edhrec_rank": c.card.edhrec_rank,
                "game_changer": c.card.game_changer,
                "owned": owned.contains(&c.card.name),
                "price_usd": crate::prints::price_range(conn, &c.card.name)
                    .ok()
                    .and_then(|r| r.cheapest.and_then(|p| p.usd)),
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
    let owned = crate::collection::owned_names_all(conn)?;
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Combo completions"),
        styles.dim(&format!(
            "one card away for {deck_name:?} (ranked by variants completed)"
        ))
    );
    for (i, c) in completions.iter().enumerate() {
        let own_note = if owned.contains(&c.card.name) {
            styles.success("own")
        } else {
            match crate::prints::price_range(conn, &c.card.name)
                .ok()
                .and_then(|r| r.cheapest.and_then(|p| p.usd))
            {
                Some(p) => format!("buy ${p:.2}"),
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
