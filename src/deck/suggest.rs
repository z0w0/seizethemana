// `stm deck suggest`: role/theme cards for a deck, ranked by fit with
// ownership and Game Changer context per hit.
//
// Candidate legs:
// 1. a semantic query (free text; required unless `--role` alone is used),
// 2. role keyword shapes scanned over the oracle,
// 3. Scryfall Tagger labels ("removal", "card draw", "typal frog", ...)
//    matched against the role or the query.
// The semantic and tag/keyword lists fuse by reciprocal rank fusion (like
// `query`); EDHREC rank breaks ties. The final list is grouped: owned
// cards first, each group in relevance order. Price, ownership, and Game
// Changer flags surface per hit.

pub use super::role::Role;
use anyhow::Context;
use rusqlite::Connection;

use super::store::load_deck;
use crate::db::CardRow;

/// One suggestion row.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub card: CardRow,
    /// Copies in the collection (0 = none owned).
    pub owned: i64,
    /// Cheapest released English print price (None = unpriced).
    pub price_usd: Option<f64>,
    /// Labels the card shares with the role/query (best effort).
    pub tags: Vec<String>,
    /// Fused relevance score in `[0, 1]` (semantic + tag legs, RRF).
    pub score: f32,
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
        // Commander decks search EDHREC-ranked cards first (the meta
        // proxy); 60-card decks rank everything so staples without an
        // EDHREC rank stay visible.
        let (rank_filter, order) = if is_commander {
            ("AND edhrec_rank IS NOT NULL", "ORDER BY edhrec_rank ASC")
        } else {
            ("", "")
        };
        let sql = format!(
            "SELECT name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                    keywords, power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
                    legalities, set_code, collector_number, scryfall_id, released_at,
                    game_changer
             FROM cards
             WHERE oracle_text LIKE '%' || ?1 || '%' {rank_filter} {order}
             LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
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

/// Run the suggest pipeline.
///
/// `query` is optional free text; `role` is optional structured intent.
/// At least one must be present (unless `--commander` is set, which implies
/// a commander-finder query). `format` pins the legality filter; when
/// absent, commander-shaped decks filter to commander and the commander's
/// color identity, and other decks apply no format filter beyond combo
/// legality.
#[allow(clippy::too_many_arguments)]
pub fn suggest(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    query: Option<&str>,
    role: Option<&str>,
    commander: bool,
    format: Option<&str>,
    bracket: Option<u8>,
    max_price: Option<f64>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    if commander {
        return run_commander_search(paths, conn, out, deck_name, query, max_price, limit, json);
    }
    // No query and no role: the deck's open combo slots are the answer.
    if query.is_none() && role.is_none() {
        return super::suggest_combo::run_combo_suggest(
            paths, conn, out, deck_name, format, bracket, max_price, limit, json,
        );
    }
    let role = match role {
        None => None,
        Some(text) => match Role::parse(text) {
            Some(r) => Some(r),
            None => {
                out.error(&format!("unknown role {text:?}"));
                out.hint("known roles: draw, cantrip, scry, wheel, discard, mill, ramp, mana-rock, mana-dork, mana-sink, land, removal, board-wipe, counterspell, bounce, theft, protection, hate, stax, sacrifice, reanimate, recursion, graveyard, token, anthem, equipment, aura, evasion, combat-trick, burn, lifegain, tutor, toolbox, wincon, finisher, combo, storm, extra-turn, blink, landfall, artifact, enchantment, planeswalker, counters-matter, energy, vehicles, group-hug, politics, voltron, spellslinger, typal, tribal, interaction");
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
    let is_commander = super::legal::is_commander(&deck, format);
    let identity = if is_commander {
        commander_identity(&deck, &cards_by_name)
    } else {
        String::new()
    };
    // 60-card decks gate to "legal in at least one 60-card format" when
    // no format is pinned; `--format` stays the precise override (inside
    // `gather_hits`).

    let mut ranked = gather_hits(
        paths,
        conn,
        out,
        &identity,
        is_commander,
        format,
        role,
        query,
        bracket,
        limit,
        max_price,
        json,
    )?;
    // 60-card decks: rank lands by color relevance and demote nonland
    // cards sharing zero colors with the deck. Commander decks filter on
    // identity instead.
    if !is_commander {
        let deck_colors = deck_color_letters(&deck, &cards_by_name);
        ranked.sort_by_key(|(card, _, _)| super::land_colors::land_rank(card, &deck_colors));
    }
    if !is_commander && format.is_none() && !json {
        println!(
            "{}",
            out.styles().note(
                "60-card deck with no --format: suggestions are gated to cards legal in at least one 60-card format; pass --format <fmt> for a precise gate"
            )
        );
    }
    // Commander decks: exclude lands producing nothing in the deck's
    // identity (a mono-B deck has no use for a W/G fetch), and rank
    // mono-color partial fetches below on-color lands unless the deck
    // already runs duals of the extra color.
    if is_commander && !identity.is_empty() {
        ranked.retain(|(card, _, _)| {
            !crate::deck::stats::is_land(card)
                || !crate::deck::land_colors::land_is_off_color(card, &identity)
        });
        let mono = identity.chars().count() == 1;
        let runs_off_color_duals = ranked.iter().any(|(card, _, _)| {
            crate::deck::stats::is_land(card)
                && crate::deck::land_colors::land_fetches_off_color(card, &identity)
        });
        ranked.sort_by_key(|(card, _, _)| {
            let produced = crate::deck::land_colors::land_producible_colors(card, &identity);
            if mono
                && !runs_off_color_duals
                && !produced.any
                && crate::deck::stats::is_land(card)
                && produced.letters.chars().any(|c| !identity.contains(c))
            {
                1
            } else {
                0
            }
        });
    }
    // The cap already ran inside `gather_hits` (cap before the limit
    // cut); `ranked` is final here.
    if ranked.is_empty() {
        if json {
            println!("[]");
        } else {
            out.error("no suggestions matched");
            out.hint(
                "widen the query or drop --role; the filters may be too tight; lower --max-price hides unpriced cards",
            );
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let owned = crate::collection::owned_counts_all(conn)?;
    let ranked = group_owned_first(ranked, &owned);
    to_suggestions(ranked, &owned, conn, out, deck_name, json)
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

/// Run the three suggestion legs (semantic, tags, role keywords), fuse
/// their ranks by RRF, and apply the format, identity, and bracket
/// filters. Returned in fused-relevance order, truncated to `limit`.
#[allow(clippy::too_many_arguments)]
fn gather_hits(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    identity: &str,
    is_commander: bool,
    format: Option<&str>,
    role: Option<Role>,
    query: Option<&str>,
    bracket: Option<u8>,
    limit: u32,
    max_price: Option<f64>,
    out_json: bool,
) -> anyhow::Result<Vec<(CardRow, Vec<String>, f32)>> {
    let search_text = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string)
        .or_else(|| role.map(default_role_query));
    let filters = identity_filter(identity, is_commander, format);
    // Legality gate for every leg: the pinned format, commander for a
    // commander-shaped deck with no format pinned, or the any-60-card
    // gate for other decks.
    let format_gate = format.or_else(|| is_commander.then_some("commander"));
    let gate = |card: &CardRow| -> bool {
        if format_gate.is_some() {
            card_legal_in(card, format_gate)
        } else {
            card_legal_in_any_60(card)
        }
    };
    // Over-fetch per leg when a price cap will shrink the pool.
    let depth = fetch_limit(limit, max_price.is_some())
        .saturating_mul(2)
        .max(30);
    let semantic = match &search_text {
        Some(text) => crate::query::run_search(paths, conn, out, text, &filters, depth, None)
            .map(|hits| hits.into_iter().map(|h| h.card).collect::<Vec<CardRow>>())?,
        None => Vec::new(),
    };
    let tag_ids = matched_tag_ids(conn, role, query)?;
    let tag_by_oracle = tag_hits_by_oracle(conn, &tag_ids)?;
    // The tag/keyword leg in candidate order: label matches (many labels =
    // stronger fit) followed by role-keyword rows. Deduped by oracle id.
    let mut tag_leg: Vec<(CardRow, Vec<String>)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut labeled: Vec<(CardRow, Vec<String>)> = Vec::new();
    for (oracle_id, labels) in &tag_by_oracle {
        let card = card_by_oracle(conn, oracle_id)?;
        if let Some(card) = card
            && (!is_commander || identity_ok(&card, identity))
            && gate(&card)
        {
            labeled.push((card, labels.clone()));
        }
    }
    // More matched labels first: closer to the requested role.
    labeled.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| a.0.name.cmp(&b.0.name))
    });
    for (card, labels) in labeled {
        if seen.insert(card.oracle_id.clone()) {
            tag_leg.push((card, labels));
        }
    }
    if let Some(r) = role {
        let found = keyword_hits(conn, r, identity, is_commander, depth)?;
        for card in found {
            if gate(&card) && seen.insert(card.oracle_id.clone()) {
                tag_leg.push((card, Vec::new()));
            }
        }
    }
    // Fuse on rank. The semantic leg is already the hybrid search result.
    // With a price cap the pool is over-fetched (3x): the cap trims the
    // fused list BEFORE the limit cut, so affordable candidates past the
    // window still surface.
    let fused = fuse_legs(
        &semantic,
        &tag_leg,
        fetch_limit(limit, max_price.is_some()),
        is_commander,
    )
    .into_iter()
    .filter(|(card, _, _)| gate(card))
    .filter(|(card, _, _)| bracket_allows(bracket, card))
    .collect::<Vec<_>>();
    let (fused, hidden) = match max_price {
        Some(max_price) => {
            let (kept, hidden) =
                crate::prints::retain_by_price(conn, fused, |(c, _, _)| &c.name, max_price)?;
            (kept, Some(hidden))
        }
        None => (fused, None),
    };
    if let (Some(max_price), Some(hidden)) = (max_price, hidden)
        && !out_json
        && hidden > 0
    {
        println!(
            "{}",
            out.styles()
                .note(&crate::prints::price_cap_note(max_price, hidden))
        );
    }
    // Dedup same-oracle reprints on the deep pool (a reprinted card
    // appears once) before the limit cut — a duplicate would waste a
    // limit slot after truncation. Commander decks keep every face.
    let mut fused = fused;
    if !is_commander {
        fused.dedup_by(|a, b| a.0.oracle_id == b.0.oracle_id);
    }
    Ok(fused.into_iter().take(limit as usize).collect())
}

/// Identity-only filters for commander decks; a pinned format adds a
/// legality filter for every deck shape.
fn identity_filter(
    identity: &str,
    is_commander: bool,
    format: Option<&str>,
) -> crate::search::CardFilters {
    let mut filters = crate::search::CardFilters::default();
    if is_commander && !identity.is_empty() {
        filters.color_identity = Some(identity.to_string());
        filters.format = Some("commander".to_string());
    } else if let Some(format) = format {
        filters.format = Some(format.to_string());
    }
    filters
}

/// True when the card passes the pinned format (or commander when none is
/// pinned and the deck is commander-shaped). `None` in a non-commander
/// deck passes everything; unknown legality keys fail a pinned format.
pub(super) fn card_legal_in(card: &CardRow, format: Option<&str>) -> bool {
    match format {
        None => true,
        Some(format) => {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&card.legalities)
                .ok()
                .and_then(|m| m.get(format).and_then(|v| v.as_str().map(String::from)))
                .is_some_and(|state| state == "legal" || state == "restricted")
        }
    }
}

/// The deck's color letters: the union of printed colors across all
/// maindeck card faces. 60-card decks rank suggestions against this set.
pub(super) fn deck_color_letters(
    deck: &super::Deck,
    cards: &std::collections::HashMap<String, CardRow>,
) -> String {
    let mut letters = String::new();
    for entry in deck.entries() {
        let Some(card) = cards.get(&entry.name) else {
            continue;
        };
        let colors: Vec<char> = serde_json::from_str::<Vec<String>>(&card.colors)
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c.chars().next())
            .collect();
        for c in colors {
            if !letters.contains(c) {
                letters.push(c);
            }
        }
    }
    letters
}

/// The 60-card competitive formats the default gate accepts.
const SIXTY_CARD_FORMATS: &[&str] = &[
    "standard", "pioneer", "modern", "legacy", "vintage", "pauper",
];

/// True when the card is legal in at least one 60-card competitive
/// format (the default gate for non-commander decks with no `--format`).
pub(super) fn card_legal_in_any_60(card: &CardRow) -> bool {
    SIXTY_CARD_FORMATS
        .iter()
        .any(|f| card_legal_in(card, Some(f)))
}

/// Default free text per role (used when only `--role` is given).
fn default_role_query(role: Role) -> String {
    match role {
        Role::Draw => "draw cards engine".to_string(),
        Role::CardSelection => "scry surveil card selection".to_string(),
        Role::Removal => "destroy target permanent removal".to_string(),
        Role::Ramp => "mana ramp acceleration".to_string(),
        Role::Wincon => "win the game finisher payoff".to_string(),
        Role::Counterspell => "counter target spell".to_string(),
        Role::Land => "land mana fixing dual".to_string(),
        Role::BoardWipe => "destroy all creatures sweeper".to_string(),
        Role::Tutor => "search your library tutor".to_string(),
        Role::Sacrifice => "sacrifice outlet payoff".to_string(),
        Role::Reanimate => "reanimate return from graveyard to the battlefield".to_string(),
        Role::Recursion => "return from graveyard recursion".to_string(),
        Role::Discard => "discard outlet loot".to_string(),
        Role::Mill => "mill library graveyard".to_string(),
        Role::Lifegain => "gain life lifegain".to_string(),
        Role::Burn => "deal damage burn".to_string(),
        Role::Token => "create token tokens".to_string(),
        Role::Anthem => "creatures you control get anthem".to_string(),
        Role::Equipment => "equipment equip".to_string(),
        Role::Evasion => "flying menace unblockable evasion".to_string(),
        Role::CombatTrick => "combat trick gets +".to_string(),
        Role::Theft => "gain control of creature theft".to_string(),
        Role::Protection => "hexproof indestructible protection".to_string(),
        Role::Stax => "players can't tax slug".to_string(),
        Role::GraveyardHate => "exile graveyard hate".to_string(),
        Role::Combo => "win the game combo".to_string(),
        Role::Storm => "copy instant or sorcery storm".to_string(),
        Role::Blink => "exile return battlefield blink".to_string(),
        Role::Landfall => "landfall land enters".to_string(),
        Role::Artifact => "artifact synergy".to_string(),
        Role::Enchantment => "enchantment aura synergy".to_string(),
        Role::Planeswalker => "planeswalker loyalty".to_string(),
        Role::Counters => "+1/+1 counters matter".to_string(),
        Role::Energy => "energy counters".to_string(),
        Role::Vehicles => "vehicle crew".to_string(),
        Role::GroupHug => "each player draws group hug".to_string(),
        Role::Politics => "choose an opponent politics".to_string(),
        Role::Voltron => "equipped creature gets power".to_string(),
        Role::Spellslinger => "whenever you cast instant or sorcery".to_string(),
        Role::Typal => "creatures of the chosen type typal".to_string(),
        Role::ExtraTurn => "take an extra turn".to_string(),
        Role::ManaSink => "mana sink spend mana".to_string(),
    }
}

/// Tag ids whose labels match the role or the full query text.
///
/// # Errors
/// Propagates SQLite failures.
pub(super) fn matched_tag_ids(
    conn: &Connection,
    role: Option<Role>,
    query: Option<&str>,
) -> anyhow::Result<Vec<(String, String)>> {
    let wanted: Vec<String> = role
        .map(|r| r.tag_labels().iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    // Free-text queries split into whole words so "frog payoff" matches
    // labels containing "frog" or "payoff" but "ramp" cannot match
    // "gives trample" and "draw" cannot match "drawback".
    let needle: Option<Vec<String>> = query
        .map(|q| {
            q.split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() >= 3)
                .map(|w| w.to_lowercase())
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty());
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
        let tokens: std::collections::HashSet<&str> =
            lower.split(|c: char| !c.is_alphanumeric()).collect();
        let label_matches = |words: &[String]| words.iter().all(|w| tokens.contains(w.as_str()));
        if wanted.iter().any(|w| tokens.contains(w.as_str()))
            || needle.as_ref().is_some_and(|words| label_matches(words))
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
pub(super) fn tag_hits_by_oracle(
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

/// Fusion window when a price cap is active: 3x the limit so the cap
/// trims the deep pool before the limit cut. Plain searches fuse at the
/// limit (no cap, no waste).
pub(super) fn fetch_limit(limit: u32, capped: bool) -> usize {
    let l = limit as usize;
    if capped {
        l.saturating_mul(3).max(60)
    } else {
        l
    }
}

/// Fuse semantic and tag/keyword candidate lists by RRF and re-attach the
/// cards with their labels, in fused-relevance order. EDHREC rank breaks
/// fusion ties (lower is more played), then name. `edhrec_tiebreak`
/// disables the rank tiebreak for non-commander searches (neutral key:
/// fusion score, then name).
pub(super) fn fuse_legs(
    semantic: &[CardRow],
    tag_leg: &[(CardRow, Vec<String>)],
    limit: usize,
    edhrec_tiebreak: bool,
) -> Vec<(CardRow, Vec<String>, f32)> {
    let semantic_list: Vec<(String, f64)> =
        semantic.iter().map(|c| (c.name.clone(), 0.0)).collect();
    let tag_list: Vec<(String, f64)> = tag_leg.iter().map(|(c, _)| (c.name.clone(), 0.0)).collect();
    let mut cards_by_name: std::collections::HashMap<String, CardRow> =
        std::collections::HashMap::new();
    let mut ranks: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut collect = |card: &CardRow| {
        if let Some(rank) = card.edhrec_rank
            && edhrec_tiebreak
        {
            ranks.entry(card.name.clone()).or_insert(rank);
        }
        cards_by_name.insert(card.name.clone(), card.clone());
    };
    for (card, _) in tag_leg {
        collect(card);
    }
    for card in semantic {
        collect(card);
    }
    crate::query::fuse_rrf(&semantic_list, &tag_list, limit, |name| {
        ranks.get(name).copied()
    })
    .into_iter()
    .filter_map(|(name, score)| {
        let card = cards_by_name.remove(&name)?;
        let tags = tag_leg
            .iter()
            .find(|(c, _)| c.name == name)
            .map(|(_, labels)| labels.clone())
            .unwrap_or_default();
        Some((card, tags, score))
    })
    .collect()
}

/// Owned cards first; each group keeps its relevance order. Grouping only
/// reorders; `owned` maps name -> copy count.
fn group_owned_first(
    ranked: Vec<(CardRow, Vec<String>, f32)>,
    owned: &std::collections::HashMap<String, i64>,
) -> Vec<(CardRow, Vec<String>, f32)> {
    let mut owned_hits = Vec::new();
    let mut rest = Vec::new();
    for (card, tags, score) in ranked {
        if owned.get(&card.name).copied().unwrap_or(0) > 0 {
            owned_hits.push((card, tags, score));
        } else {
            rest.push((card, tags, score));
        }
    }
    owned_hits.extend(rest);
    owned_hits
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
/// The commander path's ranking core: three legs (semantic, commander tag
/// families), fused by RRF, price-capped, truncated to `limit`, and
/// ordered owned-first. Separate from printing so tests can pin the
/// `--limit` cut.
fn commander_candidates(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    deck: &super::Deck,
    query: Option<&str>,
    max_price: Option<f64>,
    limit: u32,
) -> anyhow::Result<Vec<(CardRow, Vec<String>, f32)>> {
    let cards_by_name = super::stats::lookup_names(conn, deck);

    // The query drives semantic ranking; with no query, the deck's own
    // nonland names become the theme ("frog tribal" from a frog deck).
    let search_text = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| deck_theme_words(deck, &cards_by_name));
    let filters = crate::search::CardFilters::default();
    // With a price cap the pool is over-fetched (3x); the cap trims the
    // fused list before the limit cut.
    let fetch = fetch_limit(limit, max_price.is_some());
    let semantic = crate::query::run_search(paths, conn, out, &search_text, &filters, fetch, None)
        .map(|hits| hits.into_iter().map(|h| h.card).collect::<Vec<CardRow>>())?;

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

    let mut ranked = fuse_legs(&semantic, &tag_cards, fetch, true)
        .into_iter()
        .filter(|(card, _, _)| {
            super::legal::is_commander_type(card) && card_is_commander_legal(card)
        })
        .collect::<Vec<_>>();
    ranked = apply_suggest_price(conn, out, ranked, max_price, false)?;
    ranked.truncate(limit as usize);
    let owned = crate::collection::owned_counts_all(conn)?;
    Ok(group_owned_first(ranked, &owned))
}

#[allow(clippy::too_many_arguments)]
fn run_commander_search(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    query: Option<&str>,
    max_price: Option<f64>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let (_path, deck) = load_deck(paths, deck_name)?;
    let ranked = commander_candidates(paths, conn, out, &deck, query, max_price, limit)?;
    if ranked.is_empty() {
        if json {
            println!("[]");
        } else {
            out.error("no commander candidates matched");
            out.hint(
                "try a different query; the pool is legendary creatures/planeswalkers with P/T",
            );
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let owned = crate::collection::owned_counts_all(conn)?;
    to_suggestions(ranked, &owned, conn, out, deck_name, json)
}

/// Budget cap for suggest candidates: keep only cards priced at or
/// under the cap (unpriced cards are excluded) and note the hidden
/// count in human output. No cap passes the list through.
fn apply_suggest_price(
    conn: &Connection,
    out: &mut crate::output::Output,
    ranked: Vec<(CardRow, Vec<String>, f32)>,
    max_price: Option<f64>,
    json: bool,
) -> anyhow::Result<Vec<(CardRow, Vec<String>, f32)>> {
    let Some(max_price) = max_price else {
        return Ok(ranked);
    };
    let (filtered, hidden) =
        crate::prints::retain_by_price(conn, ranked, |(card, _, _)| &card.name, max_price)?;
    if !json && hidden > 0 {
        println!(
            "{}",
            out.styles()
                .note(&crate::prints::price_cap_note(max_price, hidden))
        );
    }
    Ok(filtered)
}

/// Map ranked `(card, tags, score)` triples to suggestions and print them.
fn to_suggestions(
    ranked: Vec<(CardRow, Vec<String>, f32)>,
    owned: &std::collections::HashMap<String, i64>,
    conn: &Connection,
    _out: &mut crate::output::Output,
    deck_name: &str,
    json: bool,
) -> anyhow::Result<i32> {
    let suggestions: Vec<Suggestion> = {
        let names: Vec<String> = ranked
            .iter()
            .map(|(card, _, _)| card.name.clone())
            .collect();
        // One batched query per finish kind instead of four per card name.
        let ranges = crate::prints::price_ranges(conn, &names)?;
        ranked
            .into_iter()
            .map(|(card, tags, score)| {
                let owned = owned.get(&card.name).copied().unwrap_or(0);
                let price_usd = ranges
                    .get(&card.name)
                    .and_then(|r| r.cheapest.as_ref())
                    .and_then(|p| p.usd);
                Suggestion {
                    card,
                    owned,
                    price_usd,
                    tags,
                    score,
                }
            })
            .collect()
    };
    if json {
        print_json(&suggestions)?;
    } else {
        print_text(_out, &suggestions, deck_name);
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
                "oracle_id": s.card.oracle_id,
                "mana_cost": s.card.mana_cost,
                "cmc": s.card.cmc,
                "type_line": s.card.type_line,
                "edhrec_rank": s.card.edhrec_rank,
                "game_changer": s.card.game_changer,
                "owned": s.owned,
                "price": s.price_usd,
                "score": (f64::from(s.score) * 10_000.0).round() / 10_000.0,
                "tags": s.tags,
                "oracle_text": s.card.oracle_text,
                "color_identity": identity,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Human table for the suggestion list: owned block first, then the
/// unowned block, each in relevance order. One line per hit: name, cost,
/// type, EDHREC rank, ownership/price, and why it matched.
fn print_text(out: &crate::output::Output, suggestions: &[Suggestion], deck_name: &str) {
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Suggestions"),
        styles.dim(&format!(
            "for deck {deck_name:?} (owned first, then by fit)"
        ))
    );
    let mut last_owned = true;
    for (i, s) in suggestions.iter().enumerate() {
        if i > 0 && last_owned && s.owned == 0 {
            println!("{}", styles.dim("— not owned —"));
        }
        last_owned = s.owned > 0;
        let own_note = if s.owned > 0 {
            styles.success(&format!("own {}", styles.thousands(s.owned)))
        } else {
            match s.price_usd {
                Some(p) => format!("buy {}", styles.money(p)),
                None => "unpriced".to_string(),
            }
        };
        let gc = if s.card.game_changer == Some(true) {
            " [GC]"
        } else {
            ""
        };
        let why = if s.tags.is_empty() {
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
            "{:>2}. {} {} {} {} {}{}{}",
            i + 1,
            styles.card_name(&s.card.name),
            styles.mana_pips(&s.card.mana_cost),
            styles.dim(&s.card.type_line),
            styles.dim(&format!(
                "rank {}",
                s.card
                    .edhrec_rank
                    .map(|r| styles.thousands(r))
                    .unwrap_or_else(|| "—".into())
            )),
            styles.dim(&own_note),
            styles.dim(gc),
            styles.dim(&why),
        );
    }
}

#[cfg(test)]
#[path = "tests/suggest_tests.rs"]
mod suggest_tests;
