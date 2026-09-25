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
        if let Some(card) = cards.get(&entry.name) {
            for c in super::legal::identity_letters(&card.color_identity).chars() {
                if !identity.contains(c) {
                    identity.push(c);
                }
            }
        }
    }
    identity
}

/// Rows whose oracle text carries a role keyword substring.
///
/// The scan is bounded (`LIMIT`) and identity-filtered so it stays cheap.
///
/// # Errors
/// Propagates SQLite failures.
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
    // Round-robin over the role's keywords: fill one card per keyword
    // per pass, so the first keyword cannot crowd out the rest and the
    // pool reflects the whole role.
    let mut cursors: Vec<Vec<CardRow>> = Vec::with_capacity(role.keywords().len());
    for keyword in role.keywords() {
        let mut stmt = conn.prepare(&sql)?;
        let found = stmt.query_map(rusqlite::params![keyword, limit as i64], map_card)?;
        let mut per_keyword = Vec::new();
        for row in found {
            let card = row?;
            if is_commander && !identity_ok(&card, identity) {
                continue;
            }
            per_keyword.push(card);
            if per_keyword.len() >= limit {
                break;
            }
        }
        cursors.push(per_keyword);
    }
    let mut rows = Vec::new();
    loop {
        let mut progressed = false;
        for per_keyword in &mut cursors {
            if let Some(card) = per_keyword.pop_front_card() {
                rows.push(card);
                progressed = true;
                if rows.len() >= limit {
                    return Ok(rows);
                }
            }
        }
        if !progressed {
            break;
        }
    }
    Ok(rows)
}

/// Pop the first card off the list without dropping the rest.
trait PopFront {
    fn pop_front_card(&mut self) -> Option<CardRow>;
}

impl PopFront for Vec<CardRow> {
    fn pop_front_card(&mut self) -> Option<CardRow> {
        (!self.is_empty()).then(|| self.remove(0))
    }
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
///
/// Wraps the shared parser in `legal::identity_letters`.
pub(super) fn identity_ok(card: &CardRow, identity: &str) -> bool {
    super::legal::identity_letters(&card.color_identity)
        .chars()
        .all(|c| identity.contains(c))
}

/// 60-card ranking: lands sort by color relevance and nonland cards
/// sharing zero colors with the deck are demoted.
fn rank_by_deck_colors(
    ranked: &mut [(CardRow, Vec<String>, f32)],
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
) {
    let deck_colors = deck_color_letters(deck, cards_by_name);
    ranked.sort_by_key(|(card, _, _)| super::land_colors::land_rank(card, &deck_colors));
}

/// Commander land hygiene: drop lands producing nothing in the deck's
/// identity (a mono-B deck has no use for a W/G fetch), and rank
/// mono-color partial fetches below on-color lands unless the deck
/// already runs duals of the extra color.
fn demote_off_color_commander_lands(ranked: &mut Vec<(CardRow, Vec<String>, f32)>, identity: &str) {
    ranked.retain(|(card, _, _)| {
        !crate::deck::stats::is_land(card)
            || !crate::deck::land_colors::land_is_off_color(card, identity)
    });
    let mono = identity.chars().count() == 1;
    let runs_off_color_duals = ranked.iter().any(|(card, _, _)| {
        crate::deck::stats::is_land(card)
            && crate::deck::land_colors::land_fetches_off_color(card, identity)
    });
    ranked.sort_by_key(|(card, _, _)| {
        let produced = crate::deck::land_colors::land_producible_colors(card, identity);
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

/// Empty-result handling for the role path: JSON stays `[]` with exit 3
/// (a stderr note lists known roles so agents can retry), human output
/// gets the widen-the-query hint. Returns the exit code.
fn empty_suggestions(role: Option<Role>, out: &mut crate::output::Output, json: bool) -> i32 {
    if json && role.is_some() {
        eprintln!(
            "note: role matched but no cards passed the filters; known roles: {}",
            Role::known_names().join(", ")
        );
    }
    if json {
        println!("[]");
    } else {
        out.error("no suggestions matched");
        out.hint(
            "widen the query or drop --role; the filters may be too tight; lower --max-price hides unpriced cards",
        );
    }
    crate::cli::codes::NO_RESULTS
}

/// Run the suggest pipeline.
///
/// `query` is optional free text; `role` is optional structured intent.
/// At least one must be present (unless `--commander` is set, which implies
/// a commander-finder query). `format` pins the legality filter; when
/// absent, commander-shaped decks filter to commander and the commander's
/// color identity, and other decks apply no format filter beyond combo
/// legality. `owned_only` restricts candidates to cards the collection
/// owns.
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
    owned_only: bool,
    exclude: &[String],
    json: bool,
) -> anyhow::Result<i32> {
    if commander {
        return run_commander_search(
            paths, conn, out, deck_name, query, format, bracket, max_price, limit, owned_only,
            exclude, json,
        );
    }
    // No query and no role: the deck's open combo slots are the answer.
    if query.is_none() && role.is_none() {
        return super::suggest_combo::run_combo_suggest(
            paths, conn, out, deck_name, format, bracket, max_price, limit, exclude, json,
        );
    }
    let excluded = resolve_exclusions(exclude)?;
    let role = match role {
        None => None,
        Some(text) => match Role::parse(text) {
            Some(r) => Some(r),
            None => {
                out.error(&format!("unknown role {text:?}"));
                out.hint(&format!("known roles: {}", Role::known_names().join(", ")));
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
    let cards_by_name = super::stats::lookup_names(conn, &deck)?;
    let is_commander = super::legal::is_commander(&deck, format);
    let identity = if is_commander {
        commander_identity(&deck, &cards_by_name)
    } else {
        String::new()
    };
    // The deck's existing maindeck Game Changers; at bracket 3 they count
    // against the 3-cap before a new one is allowed.
    let existing_gcs = super::legal::maindeck_copies_by_name(&deck)
        .into_iter()
        .filter(|(name, _)| {
            cards_by_name
                .get(name)
                .is_some_and(|c| c.game_changer == Some(true))
        })
        .count();

    // Owned-only mode: the owned set reaches `gather_hits` so the trim
    // runs before the limit cut (the collection is the whole candidate
    // pool). `group_owned_first` still reorders owned cards first.
    let owned = crate::collection::owned_counts_all(conn)?;
    let owned_names: std::collections::HashSet<String> = owned.keys().cloned().collect();
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
        existing_gcs,
        &excluded,
        owned_only.then_some(&owned_names),
    )?;
    // 60-card decks: rank lands by color relevance and demote nonland
    // cards sharing zero colors with the deck. Commander decks filter on
    // identity instead.
    if !is_commander {
        rank_by_deck_colors(&mut ranked, &deck, &cards_by_name);
    }
    if !is_commander && format.is_none() && !json {
        println!(
            "{}",
            out.styles().note(
                "60-card deck with no --format: suggestions are gated to cards legal in at least one 60-card format; pass --format <fmt> for a precise gate"
            )
        );
    }
    if is_commander && !identity.is_empty() {
        demote_off_color_commander_lands(&mut ranked, &identity);
    }
    // The cap already ran inside `gather_hits` (cap before the limit
    // cut); `ranked` is final here.
    if ranked.is_empty() {
        return Ok(empty_suggestions(role, out, json));
    }
    let ranked = group_owned_first(ranked, &owned);
    to_suggestions(ranked, &owned, conn, out, deck_name, json)
}

/// True when the card is legal in commander (the only format suggest
/// targets today). A missing legality key counts as not legal, matching
/// `card_legal_in`: suggest must never recommend a card `deck legal`
/// rejects.
pub(super) fn card_is_commander_legal(card: &CardRow) -> bool {
    super::legal::legality_in(&card.legalities, "commander")
        .is_some_and(|state| state == "legal" || state == "restricted")
}

/// True when adding `card` stays inside the bracket.
///
/// Brackets 1-2 allow no Game Changers. At bracket 3 the hard cap is 3
/// Game Changers, so the deck's existing maindeck count counts against
/// the cap before a new one is allowed. Brackets 4-5 are uncapped.
pub(super) fn bracket_allows(bracket: Option<u8>, card: &CardRow, existing_gcs: usize) -> bool {
    if card.game_changer != Some(true) {
        return true;
    }
    match bracket {
        Some(1 | 2) => false,
        Some(3) => existing_gcs < 3,
        _ => true,
    }
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
///
/// `owned_only` trims the pool to owned names before the limit cut: the
/// collection is the whole candidate pool, so an owned card ranked below
/// the limit must still surface (Swords to Plowshares must not lose to
/// ten unowned removal spells).
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
    existing_gcs: usize,
    excluded: &[String],
    owned_names: Option<&std::collections::HashSet<String>>,
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
    // Over-fetch per leg when a price cap, exclusions, or the owned-only
    // trim will shrink the pool; all trims happen before the limit cut.
    // Both fusion paths share `fusion_depth`.
    let depth = fusion_depth(
        limit,
        max_price.is_some() || !excluded.is_empty() || owned_names.is_some(),
    );
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
    // Fuse on rank. The semantic leg is fetched at `depth` (3x the limit
    // when a price cap will shrink the pool, minimum 60) and fused at the
    // same depth, so the deep pool survives to the cap-then-limit cut.
    let fused = fuse_legs(&semantic, &tag_leg, depth, is_commander)
        .into_iter()
        .filter(|(card, _, _)| gate(card))
        .filter(|(card, _, _)| bracket_allows(bracket, card, existing_gcs))
        .collect::<Vec<_>>();
    let (mut fused, hidden) = match max_price {
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
    // Drop exclusions before the limit cut so they never consume a
    // result slot.
    retain_unexcluded(&mut fused, excluded);
    // Owned-only: trim to owned names before the limit cut, mirroring the
    // price cap and exclusions (both trims precede the limit).
    if let Some(owned) = owned_names {
        fused.retain(|(card, _, _)| owned.contains(&card.name));
    }
    // Dedup same-oracle reprints across the whole pool (a reprinted card
    // appears once) before the limit cut — a duplicate would waste a
    // limit slot after truncation. Commander decks keep every face.
    if !is_commander {
        dedup_by_oracle(&mut fused);
    }
    Ok(fused.into_iter().take(limit as usize).collect())
}

/// Remove same-oracle duplicates anywhere in the list (keep the first
/// occurrence; the pool is already rank-ordered). `dedup_by` only drops
/// adjacent equals, so non-adjacent reprints would survive and waste a
/// limit slot.
fn dedup_by_oracle<T, S>(rows: &mut Vec<(CardRow, T, S)>) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    rows.retain(|(card, _, _)| seen.insert(card.oracle_id.clone()));
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
        Some(format) => super::legal::legality_in(&card.legalities, format)
            .is_some_and(|state| state == "legal" || state == "restricted"),
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
pub const SIXTY_CARD_FORMATS: &[&str] = &[
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
/// [`super::legal::is_commander_type`]). Three legs (semantic, commander
/// tag families) fuse by RRF, then price-cap and `--limit` cut, ordered
/// owned-first. Hits outside the deck's commander color identity are
/// marked, not dropped, so a deck without a commander yet still gets
/// useful options across all colors.
#[allow(
    clippy::too_many_arguments,
    reason = "one flat argument per CLI flag; run_commander_search carries the allow upstream"
)]
fn commander_candidates(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    deck: &super::Deck,
    query: Option<&str>,
    format: Option<&str>,
    bracket: Option<u8>,
    max_price: Option<f64>,
    limit: u32,
    excluded: &[String],
    owned_names: Option<&std::collections::HashSet<String>>,
    json: bool,
) -> anyhow::Result<Vec<(CardRow, Vec<String>, f32)>> {
    let cards_by_name = super::stats::lookup_names(conn, deck)?;
    // The deck's existing maindeck Game Changers count against the
    // bracket-3 cap here too, same rule as the role-fill path.
    let existing_gcs = super::legal::maindeck_copies_by_name(deck)
        .into_iter()
        .filter(|(name, _)| {
            cards_by_name
                .get(name)
                .is_some_and(|c| c.game_changer == Some(true))
        })
        .count();

    // The query drives semantic ranking; with no query, the deck's own
    // nonland names become the theme ("frog tribal" from a frog deck).
    let search_text = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| deck_theme_words(deck, &cards_by_name));
    let filters = crate::search::CardFilters::default();
    // The pool is over-fetched when a price cap, exclusions, or the
    // owned-only trim will shrink it; all three happen before the limit
    // cut. Semantic leg and fusion run at the same depth (the shared
    // `fusion_depth` helper, identical to the role-fill path).
    let fetch = fusion_depth(
        limit,
        max_price.is_some() || !excluded.is_empty() || owned_names.is_some(),
    );
    let semantic = crate::query::run_search(paths, conn, out, &search_text, &filters, fetch, None)
        .map(|hits| hits.into_iter().map(|h| h.card).collect::<Vec<CardRow>>())?;

    // Tag leg: "commander" and "legendary creature" tag families.
    let tag_ids = matched_tag_ids(conn, None, Some("commander"))?;
    let tag_by_oracle = tag_hits_by_oracle(conn, &tag_ids)?;
    let mut tag_cards = Vec::new();
    for (oracle_id, labels) in &tag_by_oracle {
        if let Some(card) = card_by_oracle(conn, oracle_id)?
            && super::legal::is_commander_type(&card)
            && card_is_commander_legal(&card)
        {
            tag_cards.push((card, labels.clone()));
        }
    }

    let mut ranked = fuse_legs(&semantic, &tag_cards, fetch, true)
        .into_iter()
        .filter(|(card, _, _)| {
            super::legal::is_commander_type(card) && card_is_commander_legal(card)
        })
        // A pinned format gates to cards legal there; a bracket drops
        // Game Changers the allowance cannot hold (1-2 allow none).
        .filter(|(card, _, _)| format.is_none_or(|f| card_legal_in(card, Some(f))))
        .filter(|(card, _, _)| bracket_allows(bracket, card, existing_gcs))
        .collect::<Vec<_>>();
    ranked = apply_suggest_price(conn, out, ranked, max_price, json)?;
    // Drop exclusions before the limit cut so they never consume a
    // result slot.
    retain_unexcluded(&mut ranked, excluded);
    // Owned-only: trim to owned names before the limit cut (same rule as
    // the role-fill path).
    if let Some(owned) = owned_names {
        ranked.retain(|(card, _, _)| owned.contains(&card.name));
    }
    ranked.truncate(limit as usize);
    let owned = crate::collection::owned_counts_all(conn)?;
    Ok(group_owned_first(ranked, &owned))
}
/// Run the commander role search: expand the role to its tag labels,
/// rank candidates (owned first), and apply the limit. Falls back to a
/// semantic vector search over the store when no tag labels match.
#[allow(clippy::too_many_arguments)]
fn run_commander_search(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    deck_name: &str,
    query: Option<&str>,
    format: Option<&str>,
    bracket: Option<u8>,
    max_price: Option<f64>,
    limit: u32,
    owned_only: bool,
    exclude: &[String],
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let excluded = resolve_exclusions(exclude)?;
    let owned = crate::collection::owned_counts_all(conn)?;
    let owned_names: std::collections::HashSet<String> = owned.keys().cloned().collect();
    let (_path, deck) = load_deck(paths, deck_name)?;
    let ranked = commander_candidates(
        paths,
        conn,
        out,
        &deck,
        query,
        format,
        bracket,
        max_price,
        limit,
        &excluded,
        owned_only.then_some(&owned_names),
        json,
    )?;
    if owned_only && ranked.is_empty() {
        if json {
            println!("[]");
        } else {
            out.error("no owned commander candidates matched");
            out.hint("drop --owned to search every legendary, or widen the query");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
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
    out: &mut crate::output::Output,
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
        print_text(out, &suggestions, deck_name);
    }
    Ok(crate::cli::codes::OK)
}

/// Resolve `--exclude` entries into card names. Each entry is a card
/// name, a text file of names (one per line, `#` comments allowed), or
/// `-` for stdin. Unknown files read as names; a name that is also a
/// path is rare enough that the file read error names the entry.
pub fn resolve_exclusions(entries: &[String]) -> anyhow::Result<Vec<String>> {
    resolve_exclusions_with(entries, &mut std::io::stdin())
}

/// Same resolution with an injected stdin source (the `-` entry).
fn resolve_exclusions_with(
    entries: &[String],
    stdin: &mut dyn std::io::Read,
) -> anyhow::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in entries {
        let text = if entry == "-" {
            let mut buf = String::new();
            stdin
                .read_to_string(&mut buf)
                .context("reading --exclude names from stdin")?;
            Some(buf)
        } else if std::path::Path::new(entry).is_file() {
            Some(
                std::fs::read_to_string(entry)
                    .with_context(|| format!("reading --exclude file {entry}"))?,
            )
        } else {
            None
        };
        match text {
            Some(text) => {
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    names.push(line.to_string());
                }
            }
            None => names.push(entry.clone()),
        }
    }
    Ok(names)
}

/// Drop excluded card names from the ranked candidates.
fn retain_unexcluded<C, T, S>(ranked: &mut Vec<(C, T, S)>, excluded: &[String])
where
    C: HasName,
{
    if excluded.is_empty() {
        return;
    }
    let set: std::collections::HashSet<&str> = excluded.iter().map(String::as_str).collect();
    ranked.retain(|(card, _, _)| !set.contains(card.name()));
}

/// Cards exposing their display name (CardRow and any suggestion row).
trait HasName {
    fn name(&self) -> &str;
}

impl HasName for CardRow {
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
#[path = "tests/suggest_tests.rs"]
mod suggest_tests;

#[path = "suggest/pipeline.rs"]
mod pipeline;
use pipeline::{fuse_legs, tag_hits_by_oracle};
pub(super) use pipeline::{fusion_depth, matched_tag_ids};
mod render;
use render::{deck_theme_words, print_json, print_text};
