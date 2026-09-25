use anyhow::Context;

use crate::cli;
use crate::cli::codes;
use crate::db::{self, CardRow};
use crate::embed::{self, VectorStore};
use crate::output::Output;
use crate::paths::Paths;
use crate::search::CardFilters;

/// Query-expansion table: player shorthand -> community tag vocabulary.
///
/// Each row is a trigger phrase and extra search terms appended to both
/// retrieval legs — the FTS expression gains them as additional OR terms,
/// the embedded query gains them as trailing words. Appending (never
/// replacing) keeps the original intent terms in play.
///
/// Triggers match anywhere in the query with word boundaries, so "cheap
/// counterspell" expands via `counterspell`. Expansions are grounded in
/// Scryfall Tagger's vocabulary (`ramp`, `sweeper`, `reanimate`,
/// `sacrifice outlet`, `wheel`, `cantrip`, `anthem`, ...) plus the oracle
/// phrasing of the mechanic, so role words match the `tags_text` full-text
/// column and the embedding even when oracle text never uses the shorthand.
const EXPANSIONS: &[(&str, &str)] = &[
    // Mana development
    (
        "ramp",
        "ramp mana acceleration add one mana of any color search your library for a land",
    ),
    (
        "mana ramp",
        "ramp mana acceleration land ramp add one mana of any color",
    ),
    (
        "land ramp",
        "land ramp multi land ramp tutor-land search your library for a land",
    ),
    (
        "mana acceleration",
        "ramp mana acceleration adds multiple mana",
    ),
    (
        "mana rock",
        "mana rock utility mana rock mana artifact adds multiple mana",
    ),
    (
        "mana dork",
        "mana dork ramp adds multiple mana tap for mana",
    ),
    (
        "mana fixing",
        "mana fixing color fixer dual land utility land",
    ),
    ("color fixing", "mana fixing dual land utility land"),
    ("mana sink", "mana sink bottomless mana sink"),
    (
        "mana value reduction",
        "cheaper than mv discount-self alternative cost free-cast",
    ),
    (
        "cheaper to cast",
        "cheaper than mv discount-self alternative cost",
    ),
    (
        "free cast",
        "free-cast-another alternative cost cast on resolution",
    ),
    // Card advantage
    (
        "card draw",
        "draw engine pure draw burst draw card advantage",
    ),
    (
        "draw cards",
        "draw engine pure draw burst draw card advantage",
    ),
    (
        "draw engine",
        "draw engine repeatable pure draw repeatable card advantage",
    ),
    ("cantrip", "cantrip burst draw pure draw"),
    (
        "card advantage",
        "draw engine repeatable card advantage pure draw",
    ),
    ("wheel", "wheel draw discard"),
    ("wheel effect", "wheel draw discard"),
    ("loot", "loot impulse discard draw"),
    ("rummage", "loot impulse discard draw"),
    (
        "impulse draw",
        "impulse repeatable impulsive draw exiled cards",
    ),
    ("scry", "scry surveil card selection"),
    ("surveil", "surveil scry card selection mill-self"),
    ("card selection", "scry surveil card selection"),
    (
        "mill",
        "mill-self mill-opponent mill-any cards in graveyard matter",
    ),
    (
        "self mill",
        "mill-self graveyard fuel cards in graveyard matter",
    ),
    ("discard outlet", "discard outlet loot rummage discard"),
    ("looting", "loot impulse discard draw"),
    // Interaction
    (
        "removal",
        "spot removal removal-creature removal-destroy removal-exile destroy exile",
    ),
    (
        "spot removal",
        "spot removal removal-creature removal-destroy destroy target",
    ),
    (
        "kill spell",
        "spot removal removal-creature removal-destroy",
    ),
    (
        "board wipe",
        "sweeper sweeper-one-sided removal destroy all creatures each creature gets",
    ),
    (
        "wrath effect",
        "sweeper removal destroy all creatures each creature gets",
    ),
    ("wrath", "sweeper removal destroy all creatures"),
    ("board clear", "sweeper removal destroy all creatures"),
    ("mass removal", "sweeper removal destroy all"),
    ("wipe the board", "sweeper removal destroy all creatures"),
    (
        "counterspell",
        "counterspell counter target spell counter target",
    ),
    (
        "counter",
        "counterspell counter target spell counter target",
    ),
    ("counters", "counterspell counter target spell"),
    ("countermagic", "counterspell counter target spell"),
    (
        "bounce",
        "removal-bounce bounce-self return to owner's hand",
    ),
    ("tap down", "tapper-creature freeze-creature tap target"),
    ("tax", "toll group slug rhystic opponent pays"),
    ("stax", "group slug tax toll prevent attack hate"),
    (
        "pillow fort",
        "protects-all damage prevention group slug tax",
    ),
    (
        "protection",
        "damage prevention protects-creature protects-all hexproof ward indestructible",
    ),
    ("hexproof", "gives hexproof hexproof ward"),
    ("ward", "ward gives hexproof"),
    (
        "indestructible",
        "gives indestructible indestructible damage prevention",
    ),
    (
        "hate piece",
        "hate-graveyard hate-artifact hate-attacker hate-blocker",
    ),
    ("graveyard hate", "hate-graveyard exile graveyard"),
    ("anti graveyard", "hate-graveyard exile graveyard"),
    // Sacrifice and graveyard
    (
        "sacrifice outlet",
        "sacrifice outlet repeatable sacrifice outlet sacrifice",
    ),
    (
        "sac outlet",
        "sacrifice outlet repeatable sacrifice outlet sacrifice",
    ),
    (
        "sacrifice payoff",
        "death trigger martyr sacrifice outlet cards in graveyard matter",
    ),
    (
        "aristocrats",
        "death trigger sacrifice outlet martyr drain life opponent loses life",
    ),
    (
        "reanimate",
        "reanimate reanimate-creature return from graveyard to the battlefield",
    ),
    (
        "reanimator",
        "reanimate reanimate-creature graveyard return from graveyard to the battlefield",
    ),
    (
        "graveyard recursion",
        "reanimate recursion castable from graveyard cards in graveyard matter return from graveyard",
    ),
    (
        "recursion",
        "reanimate castable from graveyard cards in graveyard matter return from graveyard",
    ),
    (
        "graveyard fill",
        "graveyard fuel mill-self cards in graveyard matter",
    ),
    (
        "graveyard matters",
        "cards in graveyard matter graveyard fuel castable from graveyard",
    ),
    (
        "return from graveyard",
        "reanimate castable from graveyard return from graveyard",
    ),
    // Board presence
    (
        "tokens",
        "repeatable creature tokens creature token creates tokens",
    ),
    (
        "token generators",
        "repeatable creature tokens creature token creates tokens",
    ),
    (
        "token",
        "repeatable creature tokens creature token creates tokens",
    ),
    (
        "go wide",
        "repeatable creature tokens multiple bodies anthem",
    ),
    (
        "anthem",
        "anthem keyword anthem power boost to all toughness boost to all",
    ),
    (
        "pump team",
        "anthem power boost to all toughness boost to all",
    ),
    (
        "buff all",
        "anthem power boost to all toughness boost to all",
    ),
    // Combat
    ("evasion", "evasion gives flying unblockable menace"),
    ("flying", "gives flying evasion flying"),
    ("unblockable", "unblockable gives unblockable evasion"),
    ("menace", "gives menace evasion menace"),
    ("trample", "gives trample evasion trample"),
    (
        "deathtouch",
        "gives deathtouch deathtouch removal-toughness",
    ),
    ("combat trick", "combat trick enlarge giant growth burst"),
    ("pump", "combat trick enlarge giant growth"),
    (
        "voltron",
        "equipment aura enlarge scales with power power matters",
    ),
    ("equipment", "equipment synergy-equipment equip"),
    ("equips", "equipment synergy-equipment equip"),
    ("aura", "aura enchantment bestow"),
    ("enchantments", "enchantment aura disenchant/naturalize"),
    ("burn", "burn burn creature burn player direct damage"),
    (
        "direct damage",
        "burn burn player burn any opponent loses life",
    ),
    ("pinger", "pinger burn any deal 1 damage"),
    ("damage", "burn damage opponent loses life"),
    ("lifegain", "lifegain repeatable lifegain gain life"),
    ("life gain", "lifegain repeatable lifegain gain life"),
    ("gain life", "lifegain repeatable lifegain gain life"),
    ("drain", "drain life opponent loses life lifegain"),
    ("lifedrain", "drain life opponent loses life lifegain"),
    // Tutors
    ("tutor", "tutor tutor-to-hand search your library"),
    ("search library", "tutor tutor-to-hand search your library"),
    ("toolbox", "tutor modal search your library"),
    // Themes
    ("landfall", "landfall land ramp cards in graveyard matter"),
    ("lands matter", "landfall utility land land ramp"),
    (
        "artifacts matter",
        "synergy-artifact synergy-artifact-creature artifact",
    ),
    (
        "artifact deck",
        "synergy-artifact synergy-artifact-creature artifact",
    ),
    ("typal", "noncreature typal creature count matters synergy"),
    ("tribal", "noncreature typal creature count matters synergy"),
    (
        "kindred",
        "noncreature typal creature count matters synergy",
    ),
    (
        "storm",
        "copy-spell copy-instant castable from exile cheap spells",
    ),
    (
        "spellslinger",
        "copy-spell copy-instant cast trigger-you impulse",
    ),
    ("copy", "copy-spell copy-instant copy-sorcery copy-creature"),
    ("spell copy", "copy-spell copy-instant copy-sorcery"),
    ("extra turn", "extra turn take an extra turn"),
    ("time walk", "extra turn take an extra turn"),
    ("combo piece", "requires another card combo win the game"),
    ("two card combo", "combo win the game"),
    ("combo", "combo win the game"),
    (
        "win the game",
        "win the game you win the game alternate win",
    ),
    (
        "alt wincon",
        "win the game you win the game alternate win mill-opponent",
    ),
    (
        "alternate win",
        "win the game you win the game mill-opponent",
    ),
    (
        "mill win",
        "mill-opponent win the game cards in graveyard matter",
    ),
    ("theft", "theft-creature theft-cast gain control"),
    ("steal", "theft-creature theft-cast gain control"),
    ("aikido", "aikido combat trick redirect damage"),
    (
        "group hug",
        "group hug selective group hug symmetrical draw",
    ),
    ("politics", "group hug selective group hug per-player"),
    (
        "blink",
        "bounce-self enters-the-battlefield multiple bodies rescue-creature",
    ),
    (
        "flicker",
        "bounce-self enters-the-battlefield multiple bodies rescue-creature",
    ),
    (
        "etb",
        "enters-the-battlefield cast trigger death trigger multiple bodies",
    ),
    (
        "enters the battlefield",
        "enters-the-battlefield cast trigger multiple bodies",
    ),
    (
        "death trigger",
        "death trigger death trigger-self sacrifice",
    ),
    ("attack trigger", "attack trigger attacking matters"),
    (
        "planeswalker",
        "planeswalker loyalty counters synergy-planeswalker",
    ),
    (
        "counters matter",
        "pp counters matter counters matter gains pp counters",
    ),
    (
        "plus one plus one",
        "gains pp counters pp counters matter counters matter",
    ),
    (
        "counters plus one",
        "gains pp counters pp counters matter counters matter",
    ),
    ("energy", "mm counters energy gains mm counters"),
    ("regenerate", "regenerates self damage prevention"),
    ("untap", "untapper-creature untaps self untap"),
    ("crew", "crew vehicles"),
    ("vehicle", "crew vehicles"),
    (
        "fight",
        "one-sided fight removal-fight deals damage equal to",
    ),
    (
        "freeze",
        "freeze-creature tap target creature doesn't untap",
    ),
    ("discard", "discard outlet loot rummage"),
    (
        "big creature",
        "enlarge scales with power creates bigger body high power",
    ),
    (
        "finisher",
        "enlarge scales with power win the game creates bigger body",
    ),
    (
        "cheap spells",
        "cheaper than mv low mana value matters cantrip",
    ),
    ("ritual", "adds multiple mana ramp impulse"),
    ("exile removal", "removal-exile exile-self exile target"),
    ("exile target", "removal-exile exile-self exile target"),
    (
        "exile graveyard",
        "hate-graveyard exile-self exile all graveyards",
    ),
    (
        "regrowth",
        "regrowth-creature regrowth-self return from graveyard to your hand",
    ),
    (
        "rebuy",
        "regrowth-creature regrowth-self return from graveyard to your hand",
    ),
    (
        "flicker enabler",
        "bounce-self rescue-creature enters-the-battlefield",
    ),
    ("tapper", "tapper-creature tap target"),
    ("untapper", "untapper-creature untaps self untap"),
    ("crime", "repeatable crime targeting opponent"),
];

/// Extra search terms for the query text: every trigger phrase found
/// (word-boundary match) contributes its expansion. Public so the quality
/// harness measures the same query the CLI runs.
pub fn expanded_text(text: &str) -> String {
    let normalized: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    let padded = format!(" {normalized} ");
    let mut extras: Vec<&str> = Vec::new();
    // Cap expansion bulk so one broad trigger cannot flood the full-text
    // leg: expansions add ~24 words at most, keeping the query's own words
    // dominant in the BM25 ranking.
    const MAX_EXPANSION_WORDS: usize = 24;
    let mut used = 0usize;
    for (trigger, expansion) in EXPANSIONS {
        let padded_trigger = format!(" {} ", trigger.replace('+', " "));
        if !padded.contains(&padded_trigger) || extras.contains(expansion) {
            continue;
        }
        extras.push(expansion);
        used += expansion.split_whitespace().count();
        if used >= MAX_EXPANSION_WORDS {
            break;
        }
    }
    if extras.is_empty() {
        return text.to_string();
    }
    format!("{text} {}", extras.join(" "))
}

/// Expanded query text and its matching full-text expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedQuery {
    /// Text callers must embed for the vector leg.
    pub expanded_text: String,
    /// SQLite FTS5 expression, or `None` when the text has no searchable terms.
    pub fts_expression: Option<String>,
}

/// Prepare both retrieval legs from one original query string.
pub fn prepare_query(text: &str) -> PreparedQuery {
    prepare_query_with_operator(text, db::FtsTermOperator::Any)
}

/// Prepare one query with an explicit FTS term operator.
pub fn prepare_query_with_operator(text: &str, operator: db::FtsTermOperator) -> PreparedQuery {
    let expanded_text = expanded_text(text);
    let fts_expression = db::fts_query_with_operator(&expanded_text, operator);
    PreparedQuery {
        expanded_text,
        fts_expression,
    }
}

/// Candidate depth each retrieval leg contributes to the fusion. Public
/// so the quality harness measures the same candidate pools the CLI fuses.
///
/// The floor is 20: with only `--limit` candidates, filters cutting rows
/// during fusion starve the final window on tight queries. A fixed floor
/// keeps the fused window full regardless of how aggressive the filters
/// are, at a fixed small scan cost.
pub fn fusion_depth(limit: usize) -> usize {
    limit.max(20)
}

/// Reciprocal-rank fusion constant. The original paper's value (60) works
/// without tuning; larger values soften the advantage of top ranks.
const RRF_K: f64 = 60.0;

/// Fuse full-text and vector result lists with reciprocal rank fusion.
///
/// Each list contributes `1 / (k + rank)` per document; a card ranked well by
/// both legs beats a card ranked first by only one. Equal fusion scores break
/// toward lower EDHREC rank (more popular first), then by name for
/// determinism.
///
/// Inputs are `(card name, score)` pairs, best first (scores are ignored;
/// only ranks matter). Returns `(name, score)` with score normalized to
/// `[0, 1]`. `k` is the RRF constant; [`RRF_K`] is the production value,
/// the parameter exists so benchmarks can sweep it.
pub fn fuse_rrf(
    fts_hits: &[(String, f64)],
    vector_hits: &[(String, f64)],
    limit: usize,
    edhrec_rank: impl Fn(&str) -> Option<i64>,
) -> Vec<(String, f32)> {
    fuse_rrf_k(fts_hits, vector_hits, limit, RRF_K, edhrec_rank)
}

/// [`fuse_rrf`] with an explicit fusion constant (bench sweeps).
pub fn fuse_rrf_k(
    fts_hits: &[(String, f64)],
    vector_hits: &[(String, f64)],
    limit: usize,
    k: f64,
    edhrec_rank: impl Fn(&str) -> Option<i64>,
) -> Vec<(String, f32)> {
    fuse_weighted_rrf(fts_hits, vector_hits, limit, k, 1.0, 1.0, edhrec_rank)
}

/// Fuse ranked legs with independent weights for full-text and vector search.
fn fuse_weighted_rrf(
    fts_hits: &[(String, f64)],
    vector_hits: &[(String, f64)],
    limit: usize,
    k: f64,
    fts_weight: f64,
    vector_weight: f64,
    edhrec_rank: impl Fn(&str) -> Option<i64>,
) -> Vec<(String, f32)> {
    let mut scores: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for (rank, (name, _)) in fts_hits.iter().enumerate() {
        *scores.entry(name.as_str()).or_insert(0.0) += fts_weight / (k + rank as f64 + 1.0);
    }
    for (rank, (name, _)) in vector_hits.iter().enumerate() {
        *scores.entry(name.as_str()).or_insert(0.0) += vector_weight / (k + rank as f64 + 1.0);
    }
    // Normalize to [0, 1]: a card ranked first on both legs reaches the
    // sum of the leg weights divided by k + 1.
    let max_score = (fts_weight + vector_weight) / (k + 1.0);
    let mut ranked: Vec<(String, f32)> = scores
        .into_iter()
        .map(|(name, score)| (name.to_string(), (score / max_score) as f32))
        .collect();
    ranked.sort_unstable_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| {
                edhrec_rank(&a.0)
                    .unwrap_or(i64::MAX)
                    .cmp(&edhrec_rank(&b.0).unwrap_or(i64::MAX))
            })
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(limit);
    ranked
}

/// Read-only access to card vectors used by the shared search path.
pub trait VectorRowProvider {
    /// Number of vector rows.
    fn row_count(&self) -> usize;
    /// Vector dimension for each row.
    fn dimension(&self) -> usize;
    /// Card name associated with a row.
    fn card_name(&self, row: usize) -> Option<&str>;
    /// Vector values for a row.
    fn vector(&self, row: usize) -> Option<&[f32]>;
}

impl VectorRowProvider for VectorStore {
    fn row_count(&self) -> usize {
        self.len()
    }

    fn dimension(&self) -> usize {
        self.meta.dim
    }

    fn card_name(&self, row: usize) -> Option<&str> {
        self.meta.names.get(row).map(String::as_str)
    }

    fn vector(&self, row: usize) -> Option<&[f32]> {
        (row < self.len()).then(|| self.row(row))
    }
}

/// Candidate and ranking settings for a shared search run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchSettings {
    /// BM25 weights in name, tags, type line, and oracle text order.
    pub fts_column_weights: [f64; 4],
    /// Minimum FTS candidate count before fusion.
    pub fts_candidate_depth: usize,
    /// Minimum vector candidate count before fusion.
    pub vector_candidate_depth: usize,
    /// Reciprocal-rank fusion constant.
    pub rrf_k: f64,
    /// Relative contribution of the FTS leg during reciprocal-rank fusion.
    pub fts_rrf_weight: f64,
    /// Relative contribution of the vector leg during reciprocal-rank fusion.
    pub vector_rrf_weight: f64,
    /// Whether FTS requires any or all extracted query terms.
    pub fts_term_operator: db::FtsTermOperator,
    /// Initial FTS SQL window as a multiple of the candidate depth.
    pub fts_overfetch_multiplier: usize,
    /// Minimum number of FTS SQL rows in the initial window.
    pub fts_min_sql_rows: usize,
    /// Maximum number of FTS SQL windows, including the initial window.
    pub fts_max_rounds: usize,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            fts_column_weights: db::FTS_COLUMN_WEIGHTS,
            fts_candidate_depth: 20,
            vector_candidate_depth: 20,
            rrf_k: RRF_K,
            fts_rrf_weight: 1.0,
            vector_rrf_weight: 1.0,
            fts_term_operator: db::FtsTermOperator::Any,
            fts_overfetch_multiplier: 4,
            fts_min_sql_rows: 50,
            fts_max_rounds: 4,
        }
    }
}

/// One card and its score in a ranked search leg.
#[derive(Debug, Clone)]
pub struct Hit {
    /// Matched card.
    pub card: CardRow,
    /// FTS rank, vector dot product, or normalized hybrid score, depending on
    /// which [`SearchResults`] list contains the hit.
    pub score: f32,
}

/// Ordered results from the full-text, vector, and fused search legs.
#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    /// BM25 hits, best first.
    pub fts: Vec<Hit>,
    /// Vector hits, best first.
    pub vector: Vec<Hit>,
    /// Reciprocal-rank-fused hits, best first.
    pub hybrid: Vec<Hit>,
}

/// Search settings that match the current `stm query` ranking.
pub const DEFAULT_SEARCH_SETTINGS: SearchSettings = SearchSettings {
    fts_column_weights: db::FTS_COLUMN_WEIGHTS,
    fts_candidate_depth: 20,
    vector_candidate_depth: 20,
    rrf_k: RRF_K,
    fts_rrf_weight: 1.0,
    vector_rrf_weight: 1.0,
    fts_term_operator: db::FtsTermOperator::Any,
    fts_overfetch_multiplier: 4,
    fts_min_sql_rows: 50,
    fts_max_rounds: 4,
};

/// Run FTS, vector, and hybrid search using an already embedded query vector.
///
/// `cards` must be in database order. Each non-empty vector provider must
/// have one name-aligned row per card. Filters and restricted names apply to
/// both retrieval legs before candidate selection.
///
/// # Errors
/// Fails when card IDs, vector rows, or dimensions do not align, when search
/// settings are invalid, or when SQLite search fails.
#[allow(clippy::too_many_arguments)]
pub fn search_with_vectors(
    conn: &rusqlite::Connection,
    cards: &[CardRow],
    vectors: &impl VectorRowProvider,
    query_vector: &[f32],
    text: &str,
    filters: &CardFilters,
    limit: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> anyhow::Result<SearchResults> {
    search_with_settings(
        conn,
        cards,
        vectors,
        query_vector,
        text,
        filters,
        limit,
        restrict,
        &DEFAULT_SEARCH_SETTINGS,
    )
}

/// Run shared search with explicit candidate depths, BM25 weights, and RRF k.
///
/// The selected depths grow to at least `limit`, so settings cannot starve a
/// larger requested result window.
///
/// # Errors
/// Fails when card IDs, vector rows, or dimensions do not align, when search
/// settings are invalid, or when SQLite search fails.
#[allow(clippy::too_many_arguments)]
pub fn search_with_settings(
    conn: &rusqlite::Connection,
    cards: &[CardRow],
    vectors: &impl VectorRowProvider,
    query_vector: &[f32],
    text: &str,
    filters: &CardFilters,
    limit: usize,
    restrict: Option<&std::collections::HashSet<String>>,
    settings: &SearchSettings,
) -> anyhow::Result<SearchResults> {
    validate_settings(settings)?;
    let vector_count = vectors.row_count();
    if vector_count > 0 {
        anyhow::ensure!(
            vector_count == cards.len(),
            "vector store holds {vector_count} rows but the card table lists {}; rebuild the index with 'stm setup --force'",
            cards.len()
        );
        anyhow::ensure!(
            vectors.dimension() > 0 && query_vector.len() == vectors.dimension(),
            "query vector has dimension {} but vector rows have dimension {}; use matching embedding models",
            query_vector.len(),
            vectors.dimension()
        );
        for (row, card) in cards.iter().enumerate() {
            anyhow::ensure!(
                vectors.card_name(row) == Some(card.name.as_str()),
                "vector row {row} does not match card {}; rebuild the index with 'stm setup --force'",
                card.name
            );
            anyhow::ensure!(
                vectors
                    .vector(row)
                    .is_some_and(|vector| vector.len() == vectors.dimension()),
                "vector row {row} has the wrong dimension; rebuild the vector cache"
            );
        }
    }

    let cards_by_name: std::collections::HashMap<&str, &CardRow> = cards
        .iter()
        .map(|card| (card.name.as_str(), card))
        .collect();
    let allowed: Vec<bool> = cards
        .iter()
        .map(|card| {
            restrict.is_none_or(|names| names.contains(&card.name)) && filters.matches(card)
        })
        .collect();
    let fts_depth = settings.fts_candidate_depth.max(limit);
    let vector_depth = settings.vector_candidate_depth.max(limit);
    let mut vector_hits = Vec::new();
    if vector_count > 0 && vector_depth > 0 {
        let mut scored: Vec<(usize, f32)> = Vec::new();
        for (row, is_allowed) in allowed.iter().copied().enumerate() {
            if !is_allowed {
                continue;
            }
            let vector = vectors
                .vector(row)
                .ok_or_else(|| anyhow::anyhow!("vector row {row} disappeared during search"))?;
            let score: f32 = query_vector
                .iter()
                .zip(vector)
                .map(|(query, value)| query * value)
                .sum();
            scored.push((row, score));
        }
        if scored.len() > vector_depth {
            scored.select_nth_unstable_by(vector_depth - 1, |a, b| b.1.total_cmp(&a.1));
            scored.truncate(vector_depth);
        }
        scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        vector_hits = scored
            .into_iter()
            .map(|(row, score)| {
                let card = &cards[row];
                (card.name.clone(), f64::from(score))
            })
            .collect();
    }

    let prepared = prepare_query_with_operator(text, settings.fts_term_operator);
    let mut fts_hits = Vec::new();
    if let Some(expression) = prepared.fts_expression {
        let ids = db::card_ids(conn)?;
        anyhow::ensure!(
            ids.len() == cards.len(),
            "database lists {} card IDs but {} card rows; reload cards before searching",
            ids.len(),
            cards.len()
        );
        let row_by_id: std::collections::HashMap<i64, usize> = ids
            .into_iter()
            .enumerate()
            .map(|(row, id)| (id, row))
            .collect();
        let mut sql_limit = fts_depth
            .saturating_mul(settings.fts_overfetch_multiplier)
            .max(settings.fts_min_sql_rows);
        for _ in 0..settings.fts_max_rounds {
            let round: Vec<(String, f64)> = db::fts_search_with_weights(
                conn,
                &expression,
                sql_limit,
                settings.fts_column_weights,
            )?
            .into_iter()
            .filter_map(|(id, score)| {
                let row = *row_by_id.get(&id)?;
                allowed[row].then(|| (cards[row].name.clone(), score))
            })
            .collect();
            if round.len() >= fts_hits.len() || round.len() >= fts_depth {
                fts_hits = round;
            }
            if fts_hits.len() >= fts_depth {
                break;
            }
            sql_limit = sql_limit.saturating_mul(settings.fts_overfetch_multiplier);
        }
        fts_hits.truncate(fts_depth);
    }

    let hybrid = fuse_weighted_rrf(
        &fts_hits,
        &vector_hits,
        limit,
        settings.rrf_k,
        settings.fts_rrf_weight,
        settings.vector_rrf_weight,
        |name| cards_by_name.get(name).and_then(|card| card.edhrec_rank),
    );
    Ok(SearchResults {
        fts: to_hits(fts_hits, &cards_by_name),
        vector: to_hits(vector_hits, &cards_by_name),
        hybrid: to_hits(hybrid, &cards_by_name),
    })
}

/// Reject settings that cannot be represented safely by the shared ranking path.
fn validate_settings(settings: &SearchSettings) -> anyhow::Result<()> {
    anyhow::ensure!(
        settings
            .fts_column_weights
            .iter()
            .all(|weight| weight.is_finite() && *weight >= 0.0),
        "FTS weights must be finite and non-negative"
    );
    anyhow::ensure!(
        settings.rrf_k.is_finite() && settings.rrf_k >= 0.0,
        "RRF k must be finite and non-negative"
    );
    anyhow::ensure!(
        settings.fts_rrf_weight.is_finite()
            && settings.fts_rrf_weight >= 0.0
            && settings.vector_rrf_weight.is_finite()
            && settings.vector_rrf_weight >= 0.0
            && settings.fts_rrf_weight + settings.vector_rrf_weight > 0.0,
        "RRF leg weights must be finite, non-negative, and not both zero"
    );
    anyhow::ensure!(
        settings.fts_overfetch_multiplier > 0
            && settings.fts_min_sql_rows > 0
            && settings.fts_max_rounds > 0,
        "FTS window multiplier, minimum rows, and maximum rounds must be positive"
    );
    Ok(())
}

/// Run a hybrid search: full-text (BM25) + vector legs fused by RRF.
///
/// Shared by `stm query` and `stm collection query` (the latter restricts to
/// owned names via `restrict`).
///
/// # Errors
/// Fails when the vector store or model is missing (not set up) or embedding
/// fails.
pub fn run_search(
    paths: &Paths,
    conn: &rusqlite::Connection,
    _out: &Output,
    text: &str,
    filters: &CardFilters,
    limit: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> anyhow::Result<Vec<Hit>> {
    if !paths.is_setup() {
        anyhow::bail!("card index not built yet");
    }
    let store =
        VectorStore::load(paths.root()).context("loading vector index (run 'stm setup' first)")?;
    let cards = db::load_all_cards(conn)?;
    let prepared = prepare_query(text);
    let query_vector = if store.is_empty() {
        Vec::new()
    } else {
        let status = crate::paths::Status::read(&paths.status_file())?;
        anyhow::ensure!(
            store.meta.model == embed::MODEL_NAME
                && status.model == embed::MODEL_NAME
                && status.doc_version == embed::DOC_VERSION,
            "the card index uses an older embedding model or document layout; run 'stm sync' before searching"
        );
        // Alignment is checked before model loading so corrupt metadata does
        // not spend time starting the inference runtime.
        validate_vector_names(&store, &cards)?;
        let mut model = embed::load_query_model(&paths.models_dir(), false)?;
        store.embed_query(&mut model, &prepared.expanded_text)?
    };
    let results = search_with_vectors(
        conn,
        &cards,
        &store,
        &query_vector,
        text,
        filters,
        limit,
        restrict,
    )?;
    Ok(results.hybrid)
}

/// Check the production vector store against cards before loading the model.
fn validate_vector_names(store: &VectorStore, cards: &[CardRow]) -> anyhow::Result<()> {
    anyhow::ensure!(
        store.len() == cards.len()
            && store
                .meta
                .names
                .iter()
                .zip(cards)
                .all(|(name, card)| name == &card.name),
        "vector store holds {} rows but the card table lists {} and they are out of alignment; rebuild the index with 'stm setup --force'",
        store.len(),
        cards.len()
    );
    Ok(())
}

/// Convert ordered card names and scores into public search hits.
fn to_hits<T: Into<f64>>(
    ranked: Vec<(String, T)>,
    cards_by_name: &std::collections::HashMap<&str, &CardRow>,
) -> Vec<Hit> {
    ranked
        .into_iter()
        .filter_map(|(name, score)| {
            let card = cards_by_name.get(name.as_str())?;
            Some(Hit {
                card: (*card).clone(),
                score: score.into() as f32,
            })
        })
        .collect()
}

/// Entry point for `stm query`.
#[allow(clippy::too_many_arguments)]
pub fn run_query(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    out: &mut Output,
    text: &str,
    cli_filters: &cli::CardFilters,
    max_price: Option<f64>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    let filters = CardFilters::from_cli(cli_filters)?;
    // --max-price: a SQL-side budget filter (cheapest released English
    // print at or under the cap). Passed as the search's restrict set,
    // so the fusion fills the limit window with under-cap candidates.
    let price_allow: Option<std::collections::HashSet<String>> = match max_price {
        Some(max_price) => Some(
            crate::prints::names_under_price(conn, max_price)?
                .into_iter()
                .collect(),
        ),
        None => None,
    };
    let hits = match run_search(
        paths,
        conn,
        out,
        text,
        &filters,
        limit as usize,
        price_allow.as_ref(),
    ) {
        Ok(hits) => hits,
        Err(err) => {
            // Distinguish "not set up" so the agent knows what to run.
            if !paths.is_setup() {
                out.error(&format!("{err:#}"));
                out.hint("run 'stm setup' first");
                return Ok(codes::ERROR);
            }
            return Err(err);
        }
    };
    if hits.is_empty() {
        if json {
            println!("[]");
        } else if max_price.is_some() {
            out.error("no cards matched");
            out.hint("try broader words, or raise --max-price");
        } else {
            out.error("no cards matched");
            out.hint("try broader words, or drop filters");
        }
        return Ok(codes::NO_RESULTS);
    }
    if json {
        let names: Vec<String> = hits.iter().map(|h| h.card.name.clone()).collect();
        // One batched query per finish kind instead of four per card name.
        let ranges = crate::prints::price_ranges(conn, &names)?;
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let owned_all = crate::collection::owned_counts_all(conn)?;
        let available_all = crate::collection::available_counts_all(conn)?;
        let items: Vec<serde_json::Value> = hits
            .iter()
            .map(|h| -> anyhow::Result<serde_json::Value> {
                let range = ranges.get(&h.card.name).cloned().unwrap_or_default();
                let universe =
                    crate::universe::card_universe(conn, &h.card.name, &h.card.set_code)?;
                let mut v = crate::card::card_json(
                    &h.card,
                    &tag_index,
                    &range,
                    &universe,
                    &owned_all,
                    &available_all,
                );
                v["score"] = serde_json::json!((f64::from(h.score) * 10_000.0).round() / 10_000.0);
                Ok(v)
            })
            .collect::<anyhow::Result<_>>()?;
        print_json(items)?;
    } else {
        print_text(out, &hits);
    }
    Ok(codes::OK)
}

/// Print results as a JSON array.
fn print_json(items: Vec<serde_json::Value>) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Print results as numbered lines with score, cost, and type line.
fn print_text(out: &Output, hits: &[Hit]) {
    let styles = out.styles();
    let rank_width = hits.len().to_string().len().max(2);
    for (i, hit) in hits.iter().enumerate() {
        let line = format!(
            "{:>width$}. {} {} {} {} {}",
            i + 1,
            styles.card_name(&hit.card.name),
            styles.mana_pips(&hit.card.mana_cost),
            styles.rarity(&hit.card.rarity),
            styles.dim(&hit.card.type_line),
            styles.dim(&format!("({:.3})", hit.score)),
            width = rank_width,
        );
        println!("{line}");
    }
}

#[cfg(test)]
#[path = "tests/query_tests.rs"]
mod query_tests;
