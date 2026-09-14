// `stm deck suggest`: role/theme cards for a deck, ranked by semantic fit
// and cost, with ownership and Game Changer context per hit.
//
// Sources, in priority order: owned cards first (collection), then the
// whole oracle. Candidates come from up to three legs —
// 1. a semantic query (free text; required unless `--role` alone is used),
// 2. role keyword shapes scanned over the oracle,
// 3. Scryfall Tagger labels ("removal", "card draw", "typal frog", ...)
//    matched against the role or the query.
// Every leg lands in one candidate pool; ranking is reciprocal-rank
// fusion like `query`, with binder ownership, price, EDHREC rank, and
// Game Changer flags surfaced per hit.

pub use super::role::Role;
use anyhow::Context;
use rusqlite::Connection;

use super::legal::infer_format;
use super::store::load_deck;
use crate::db::CardRow;

/// One suggestion row.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub card: CardRow,
    /// True when the collection holds at least one copy.
    pub owned: bool,
    /// Cheapest released English print price (None = unpriced).
    pub price_usd: Option<f64>,
    /// Labels the card shares with the role/query (best effort).
    pub tags: Vec<String>,
}

/// Commander color identity letters from the deck's COMMANDER section.
///
/// Unknown commander names contribute nothing; an absent section returns
/// an empty string (no identity filter beyond format legality).
pub(super) fn commander_identity(
    deck: &super::Deck,
    cards: &std::collections::HashMap<String, CardRow>,
) -> String {
    let Some(idx) = deck.section_index("COMMANDER") else {
        return String::new();
    };
    let mut identity = String::new();
    for entry in &deck.sections[idx].1 {
        if let Some(card) = cards.get(&entry.name)
            && let Ok(colors) = serde_json::from_str::<Vec<String>>(&card.color_identity)
        {
            for c in colors {
                if let Some(letter) = c.chars().next()
                    && !identity.contains(letter)
                {
                    identity.push(letter);
                }
            }
        }
    }
    identity
}

/// Tag ids whose labels match the role or the full query text.
///
/// # Errors
/// Propagates SQLite failures.
/// Keyword leg: rows whose oracle text carries a role substring.
///
/// The scan is bounded (`LIMIT`) and identity-filtered so it stays cheap.
#[allow(clippy::too_many_arguments)]
fn keyword_hits(
    conn: &Connection,
    role: Role,
    identity: &str,
    is_commander: bool,
    limit: usize,
) -> anyhow::Result<Vec<CardRow>> {
    if is_commander && identity.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for keyword in role.keywords() {
        let mut stmt = conn.prepare(
            "SELECT name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                    keywords, power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
                    legalities, set_code, collector_number, scryfall_id, released_at,
                    game_changer
             FROM cards
             WHERE oracle_text LIKE '%' || ?1 || '%'
               AND edhrec_rank IS NOT NULL
             ORDER BY edhrec_rank ASC
             LIMIT ?2",
        )?;
        let found = stmt.query_map(rusqlite::params![keyword, limit as i64], map_card)?;
        for row in found {
            let card = row?;
            if is_commander && !identity_ok(&card, identity) {
                continue;
            }
            rows.push(card);
            if rows.len() >= limit {
                return Ok(rows);
            }
        }
    }
    Ok(rows)
}

/// Map one `cards` row into a [`CardRow`] (shared column order).
fn map_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<CardRow> {
    Ok(CardRow {
        name: row.get(0)?,
        oracle_id: row.get(1)?,
        mana_cost: row.get(2)?,
        cmc: row.get(3)?,
        type_line: row.get(4)?,
        colors: row.get(5)?,
        color_identity: row.get(6)?,
        keywords: row.get(7)?,
        power: row.get(8)?,
        toughness: row.get(9)?,
        loyalty: row.get(10)?,
        oracle_text: row.get(11)?,
        rarity: row.get(12)?,
        edhrec_rank: row.get(13)?,
        legalities: row.get(14)?,
        set_code: row.get(15)?,
        collector_number: row.get(16)?,
        scryfall_id: row.get(17)?,
        released_at: row.get(18)?,
        game_changer: row.get(19)?,
    })
}

/// True when every identity color of `card` sits inside `identity`.
pub(super) fn identity_ok(card: &CardRow, identity: &str) -> bool {
    let ci: String = serde_json::from_str::<Vec<String>>(&card.color_identity)
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.chars().next())
        .collect();
    ci.chars().all(|c| identity.contains(c))
}

/// Fuse tag-only hits into the ranked list with an RRF-style boost.
///
/// Semantic hits keep their score; tag-only hits get a flat score scaled
/// by shared-tag count, then EDHREC rank breaks ties.
/// Semantic hits keep their order; tag hits append labels to cards the
/// semantic leg already found and join as new entries otherwise.
fn merge_hits(
    semantic: Vec<CardRow>,
    tag_cards: Vec<(CardRow, Vec<String>)>,
) -> Vec<(CardRow, Vec<String>)> {
    let mut out: Vec<(CardRow, Vec<String>)> = semantic
        .into_iter()
        .map(|card| (card, Vec::new()))
        .collect();
    for (card, labels) in tag_cards {
        if let Some(entry) = out.iter_mut().find(|(c, _)| c.oracle_id == card.oracle_id) {
            if entry.1.is_empty() {
                entry.1 = labels;
            }
        } else {
            out.push((card, labels));
        }
    }
    out
}

/// Run the suggest pipeline.
///
/// `query` is optional free text; `role` is optional structured intent.
/// At least one must be present (unless `--commander` is set, which implies
/// a commander-finder query). Commander decks filter to the commander's
/// color identity; the deck need not be commander-shaped.
#[allow(clippy::too_many_arguments)]
pub fn suggest(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    query: Option<&str>,
    role: Option<&str>,
    commander: bool,
    bracket: Option<u8>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    if commander {
        return run_commander_search(paths, conn, out, deck_name, query, limit, json);
    }
    // No query and no role: the deck's open combo slots are the answer.
    if query.is_none() && role.is_none() {
        return super::suggest_combo::run_combo_suggest(
            paths, conn, out, deck_name, bracket, limit, json,
        );
    }
    let role = match role {
        None => None,
        Some(text) => match Role::parse(text) {
            Some(r) => Some(r),
            None => {
                out.error(&format!("unknown role {text:?}"));
                out.hint("known roles: draw, removal, ramp, wincon, counterspell, land");
                return Ok(crate::cli::codes::USAGE);
            }
        },
    };
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let (_path, deck) = load_deck(paths, deck_name)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck);
    let identity = commander_identity(&deck, &cards_by_name);
    let is_commander = matches!(infer_format(&deck), super::legal::InferredFormat::Commander);

    let ranked = gather_hits(
        paths,
        conn,
        out,
        &identity,
        is_commander,
        role,
        query,
        bracket,
        limit,
    )?;
    if ranked.is_empty() {
        out.error("no suggestions matched");
        out.hint("widen the query or drop --role; the filters may be too tight");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let owned = crate::collection::owned_names_all(conn)?;
    let suggestions: Vec<Suggestion> = ranked
        .into_iter()
        .map(|(card, tags)| {
            let owned = owned.contains(&card.name);
            let price_usd = crate::prints::price_range(conn, &card.name)
                .ok()
                .and_then(|r| r.cheapest.and_then(|p| p.usd));
            Suggestion {
                card,
                owned,
                price_usd,
                tags,
            }
        })
        .collect();
    if json {
        print_json(&suggestions)?;
    } else {
        print_text(out, &suggestions, deck_name);
    }
    Ok(crate::cli::codes::OK)
}

/// True when the card is legal in commander (the only format suggest
/// targets today); unknown legality keys pass with a note from `deck legal`.
pub(super) fn card_is_commander_legal(card: &CardRow) -> bool {
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&card.legalities)
        .ok()
        .and_then(|m| {
            m.get("commander")
                .and_then(|v| v.as_str().map(String::from))
        })
        .is_none_or(|state| state != "banned")
}

/// True when the card fits the bracket: brackets 1-2 allow no Game
/// Changers, so those suggestions are filtered. `None` (no bracket given)
/// and brackets 3-5 pass everything; `deck legal` counts the deck's
/// Game Changers against the allowance.
pub(super) fn bracket_allows(bracket: Option<u8>, card: &CardRow) -> bool {
    if card.game_changer != Some(true) {
        return true;
    }
    !matches!(bracket, Some(1 | 2))
}

/// First `CardRow` for an oracle id.
///
/// # Errors
/// Propagates SQLite failures.
fn card_by_oracle(conn: &Connection, oracle_id: &str) -> anyhow::Result<Option<CardRow>> {
    let mut stmt = conn.prepare(
        "SELECT name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                keywords, power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
                legalities, set_code, collector_number, scryfall_id, released_at,
                game_changer
         FROM cards
         WHERE oracle_id = ?1
         LIMIT 1",
    )?;
    let mut rows = stmt.query_map([oracle_id], map_card)?;
    Ok(rows.next().transpose()?)
}

/// Run the three suggestion legs (semantic, tags, role keywords), then
/// merge and rank them under the identity and bracket filters.
#[allow(clippy::too_many_arguments)]
fn gather_hits(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    identity: &str,
    is_commander: bool,
    role: Option<Role>,
    query: Option<&str>,
    bracket: Option<u8>,
    limit: u32,
) -> anyhow::Result<Vec<(CardRow, Vec<String>)>> {
    let search_text = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string)
        .or_else(|| role.map(default_role_query));
    let filters = identity_filter(identity, is_commander);
    let semantic = match &search_text {
        Some(text) => crate::query::run_search(
            paths,
            conn,
            out,
            text,
            &filters,
            (limit as usize).saturating_mul(2).max(30),
            None,
        )
        .map(|hits| hits.into_iter().map(|h| h.card).collect::<Vec<CardRow>>())
        .unwrap_or_default(),
        None => Vec::new(),
    };
    let tag_ids = matched_tag_ids(conn, role, query)?;
    let tag_by_oracle = tag_hits_by_oracle(conn, &tag_ids)?;
    let mut tag_cards = Vec::new();
    for (oracle_id, labels) in &tag_by_oracle {
        let card = card_by_oracle(conn, oracle_id)?;
        if let Some(card) = card
            && (!is_commander || identity_ok(&card, identity))
            && card_is_commander_legal(&card)
        {
            tag_cards.push((card, labels.clone()));
        }
    }
    if let Some(r) = role {
        let found = keyword_hits(conn, r, identity, is_commander, limit as usize)?;
        for card in found {
            tag_cards.push((card, Vec::new()));
        }
    }
    let merged = merge_hits(semantic, tag_cards);
    Ok(rank_hits(
        merged
            .into_iter()
            .filter(|(card, _)| card_is_commander_legal(card))
            .filter(|(card, _)| bracket_allows(bracket, card))
            .collect(),
        limit as usize,
    ))
}
/// Identity-only filters for commander decks.
fn identity_filter(identity: &str, is_commander: bool) -> crate::search::CardFilters {
    let mut filters = crate::search::CardFilters::default();
    if is_commander && !identity.is_empty() {
        filters.color_identity = Some(identity.to_string());
        filters.format = Some("commander".to_string());
    }
    filters
}

/// Default free text per role (used when only `--role` is given).
fn default_role_query(role: Role) -> String {
    match role {
        Role::Draw => "draw cards engine".to_string(),
        Role::Removal => "destroy target permanent removal".to_string(),
        Role::Ramp => "mana ramp acceleration".to_string(),
        Role::Wincon => "win the game finisher payoff".to_string(),
        Role::Counterspell => "counter target spell".to_string(),
        Role::Land => "land mana fixing dual".to_string(),
    }
}

/// Tag ids whose labels match the role or the full query text.
///
/// # Errors
/// Propagates SQLite failures.
fn matched_tag_ids(
    conn: &Connection,
    role: Option<Role>,
    query: Option<&str>,
) -> anyhow::Result<Vec<(String, String)>> {
    let wanted: Vec<&str> = role.map(|r| r.tag_labels().to_vec()).unwrap_or_default();
    let needle = query
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty());
    if wanted.is_empty() && needle.is_none() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT id, label FROM tags")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, label) = row?;
        let lower = label.to_lowercase();
        if wanted.iter().any(|w| lower.contains(w))
            || needle
                .as_ref()
                .is_some_and(|q| !q.is_empty() && lower.contains(q.as_str()))
        {
            out.push((id, label));
        }
    }
    Ok(out)
}

/// Oracle ids carrying any matched tag, with the shared labels.
///
/// # Errors
/// Propagates SQLite failures.
fn tag_hits_by_oracle(
    conn: &Connection,
    tag_ids: &[(String, String)],
) -> anyhow::Result<std::collections::HashMap<String, Vec<String>>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    if tag_ids.is_empty() {
        return Ok(map);
    }
    for (id, _label) in tag_ids {
        let mut stmt = conn.prepare(
            "SELECT ct.oracle_id, t.label
             FROM card_tags ct JOIN tags t ON t.id = ct.tag_id
             WHERE ct.tag_id = ?1",
        )?;
        let rows = stmt.query_map([id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (oracle_id, tag_label) = row.context("reading tag hits")?;
            map.entry(oracle_id).or_default().push(tag_label);
        }
    }
    Ok(map)
}

/// Rank hits: EDHREC rank first (lower is more played), name as tie-break.
fn rank_hits(hits: Vec<(CardRow, Vec<String>)>, limit: usize) -> Vec<(CardRow, Vec<String>)> {
    let mut ranked = hits;
    ranked.sort_by(|a, b| {
        let a_played = a.0.edhrec_rank.unwrap_or(i64::MAX);
        let b_played = b.0.edhrec_rank.unwrap_or(i64::MAX);
        a_played
            .cmp(&b_played)
            .then_with(|| a.0.name.cmp(&b.0.name))
    });
    ranked.truncate(limit);
    ranked
}

/// `--commander` mode: find commander candidates for the deck.
///
/// The pool is every card that can legally command (legendary creature or
/// planeswalker, or legendary Vehicle/Spacecraft with a P/T box —
/// [`super::legal::is_commander_type`]), ranked by semantic fit to the
/// query (default "frog tribal commander" style: the deck's own theme
/// words) then EDHREC playability. Hits outside the deck's commander color
/// identity are marked, not dropped, so a deck without a commander yet
/// still gets useful options across all colors.
#[allow(clippy::too_many_arguments)]
fn run_commander_search(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    query: Option<&str>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let (_path, deck) = load_deck(paths, deck_name)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck);

    // The query drives semantic ranking; with no query, the deck's own
    // nonland names become the theme ("frog tribal" from a frog deck).
    let search_text = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| deck_theme_words(&deck, &cards_by_name));
    let filters = crate::search::CardFilters::default();
    let semantic = crate::query::run_search(
        paths,
        conn,
        out,
        &search_text,
        &filters,
        (limit as usize).saturating_mul(3).max(60),
        None,
    )
    .map(|hits| hits.into_iter().map(|h| h.card).collect::<Vec<CardRow>>())
    .unwrap_or_default();

    // Tag leg: "commander" and "legendary creature" tag families.
    let tag_ids = matched_tag_ids(conn, None, Some("commander"))?;
    let tag_by_oracle = tag_hits_by_oracle(conn, &tag_ids)?;
    let mut tag_cards = Vec::new();
    for oracle_id in tag_by_oracle.keys() {
        if let Some(card) = card_by_oracle(conn, oracle_id)?
            && super::legal::is_commander_type(&card)
            && card_is_commander_legal(&card)
        {
            tag_cards.push((card, tag_by_oracle[oracle_id].clone()));
        }
    }

    let ranked = rank_hits(
        merge_hits(semantic, tag_cards)
            .into_iter()
            .filter(|(card, _)| {
                super::legal::is_commander_type(card) && card_is_commander_legal(card)
            })
            .collect(),
        limit as usize,
    );
    if ranked.is_empty() {
        out.error("no commander candidates matched");
        out.hint("try a different query; the pool is legendary creatures/planeswalkers with P/T");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let owned = crate::collection::owned_names_all(conn)?;
    let suggestions: Vec<Suggestion> = ranked
        .into_iter()
        .map(|(card, tags)| {
            let owned = owned.contains(&card.name);
            let price_usd = crate::prints::price_range(conn, &card.name)
                .ok()
                .and_then(|r| r.cheapest.and_then(|p| p.usd));
            Suggestion {
                card,
                owned,
                price_usd,
                tags,
            }
        })
        .collect();
    if json {
        print_json(&suggestions)?;
    } else {
        print_text(out, &suggestions, deck_name);
    }
    Ok(crate::cli::codes::OK)
}

/// Theme words from the deck's own cards: the most distinctive creature
/// type words across names and type lines, deduped.
fn deck_theme_words(
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
) -> String {
    let stop: std::collections::HashSet<&str> = [
        "the",
        "of",
        "and",
        "a",
        "an",
        "creature",
        "token",
        "legendary",
        "enchantment",
        "instant",
        "sorcery",
        "artifact",
        "land",
        "planeswalker",
        "battle",
        "spell",
    ]
    .into_iter()
    .collect();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for entry in deck.entries() {
        if let Some(card) = cards_by_name.get(&entry.name) {
            for word in card.type_line.split(&[' ', '—', ',']) {
                let w = word.trim();
                if w.len() >= 4 && !stop.contains(w.to_ascii_lowercase().as_str()) {
                    *counts.entry(w.to_string()).or_insert(0) += entry.quantity as usize;
                }
            }
        }
    }
    let mut words: Vec<(String, usize)> = counts.into_iter().collect();
    words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    words.truncate(4);
    let joined = words
        .iter()
        .map(|(w, _)| w.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        "tribal commander".to_string()
    } else {
        format!("{joined} commander")
    }
}

/// JSON rows for the suggestion list.
fn print_json(suggestions: &[Suggestion]) -> anyhow::Result<()> {
    let items: Vec<serde_json::Value> = suggestions
        .iter()
        .map(|s| {
            let identity: serde_json::Value =
                serde_json::from_str(&s.card.color_identity).unwrap_or_default();
            serde_json::json!({
                "name": s.card.name,
                "mana_cost": s.card.mana_cost,
                "cmc": s.card.cmc,
                "type_line": s.card.type_line,
                "edhrec_rank": s.card.edhrec_rank,
                "game_changer": s.card.game_changer,
                "owned": s.owned,
                "price_usd": s.price_usd,
                "tags": s.tags,
                "oracle_text": s.card.oracle_text,
                "color_identity": identity,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Human table for the suggestion list.
/// Human table for the suggestion list.
fn print_text(out: &crate::output::Output, suggestions: &[Suggestion], deck_name: &str) {
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Suggestions"),
        styles.dim(&format!(
            "for deck {deck_name:?} (owned first, cheapest print)"
        ))
    );
    for (i, s) in suggestions.iter().enumerate() {
        let own_note = if s.owned {
            styles.success("own")
        } else {
            match s.price_usd {
                Some(p) => format!("buy ${p:.2}"),
                None => "unpriced".to_string(),
            }
        };
        // GC marker and tags join with their own leading space, so lines
        // without them carry no trailing blank.
        let tail_note = if s.card.game_changer == Some(true) && s.tags.is_empty() {
            " [GC]".to_string()
        } else if s.card.game_changer == Some(true) {
            format!(
                " [GC] {}",
                s.tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else if s.tags.is_empty() {
            String::new()
        } else {
            format!(
                " {}",
                s.tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!(
            "{:>2}. {} {} {} {} {}{}",
            i + 1,
            styles.card_name(&s.card.name),
            styles.mana_pips(&s.card.mana_cost),
            styles.dim(&s.card.type_line),
            styles.dim(&format!("rank {}", s.card.edhrec_rank.unwrap_or(0))),
            styles.dim(&own_note),
            tail_note
        );
    }
}

#[cfg(test)]
#[path = "tests/suggest_tests.rs"]
mod suggest_tests;
