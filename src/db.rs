//! SQLite layer: connection bootstrap, schema validation, and the card row
//! type shared by every module. Collection access lives in `collection.rs`,
//! price access in `prints.rs`; both use this module's connection helpers.

use anyhow::Context;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

/// One stored card, deserialized from the `cards` table.
///
/// JSON columns (colors, legalities…) are kept as raw JSON strings; consumers
/// parse what they need. Per-print prices live in the `card_prints` table
/// (`src/prints.rs`), keyed by `scryfall_id`.
#[derive(Debug, Clone)]
pub struct CardRow {
    /// Oracle card name; the primary key callers resolve against.
    pub name: String,
    /// Scryfall oracle ID; join key for `card_tags` (tag associations).
    pub oracle_id: String,
    /// Mana cost in Scryfall notation, e.g. `{2}{R}`.
    pub mana_cost: String,
    /// Converted mana cost.
    pub cmc: f64,
    /// Type line, e.g. `Legendary Creature — Human`.
    pub type_line: String,
    /// Card colors as a JSON array, e.g. `["R"]`.
    pub colors: String,
    /// Color identity as a JSON array; Commander legality reads this.
    pub color_identity: String,
    /// Keyword abilities as a JSON array.
    pub keywords: String,
    /// Power, when the card has one; `*`-style values stay as text.
    pub power: Option<String>,
    /// Toughness, when the card has one.
    pub toughness: Option<String>,
    /// Starting loyalty for planeswalkers.
    pub loyalty: Option<String>,
    /// Rules text.
    pub oracle_text: String,
    /// Rarity: `common`, `uncommon`, `rare`, or `mythic`.
    pub rarity: String,
    /// EDHREC popularity rank (lower is more popular); `None` when unknown.
    pub edhrec_rank: Option<i64>,
    /// Penny Dreadful popularity rank (lower is more popular); `None` when
    /// unknown.
    pub penny_rank: Option<i64>,
    /// True when the card is on the Reserved List (never reprinted). Null
    /// when the bulk did not say.
    pub reserved: Option<bool>,
    /// Format legalities as a JSON object of format → status.
    pub legalities: String,
    /// Set code of the stored representative print (lowercase).
    pub set_code: String,
    /// Collector number of the stored representative print.
    pub collector_number: String,
    /// Scryfall print ID of the stored representative print; join key for
    /// `card_prints`.
    pub scryfall_id: String,
    /// Release date (YYYY-MM-DD) of the oracle's latest recognized printing;
    /// empty when unknown. Display only — ingest gating keeps unreleased
    /// cards out of the store (`scryfall::should_ingest`).
    pub released_at: String,
    /// True when the card is on the Commander Game Changer list (bracket
    /// signal for `deck legal`). Null when the bulk did not say.
    pub game_changer: Option<bool>,
}

/// The `cards` column list every card query selects, without `SELECT` or
/// `FROM`.
///
/// The order must match [`map_card`]'s positional `row.get` calls. Every
/// query that maps a full card row builds its `SELECT` from this constant so
/// the list and the mapper cannot drift apart.
pub const CARD_COLUMNS: &str =
    "name, oracle_id, mana_cost, cmc, type_line, colors, color_identity, keywords,
        power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
        legalities, set_code, collector_number, scryfall_id, released_at,
        game_changer, penny_rank, reserved";

/// All card ids in insertion (bulk) order, aligned with [`load_all_cards`].
///
/// # Errors
/// Propagates SQLite failures.
pub fn card_ids(conn: &Connection) -> anyhow::Result<Vec<i64>> {
    let ids = conn
        .prepare("SELECT id FROM cards ORDER BY id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<i64>, _>>()
        .context("reading card ids")?;
    Ok(ids)
}

/// All cards in insertion (bulk) order.
///
/// Vector-row alignment: `status.json` lists card names in this same order,
/// so row `i` of `vectors.bin` belongs to `load_all_cards()[i]`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn load_all_cards(conn: &Connection) -> anyhow::Result<Vec<CardRow>> {
    let sql = format!("SELECT {CARD_COLUMNS} FROM cards ORDER BY id");
    let mut stmt = conn.prepare(&sql).context("prepare card query")?;
    let rows = stmt.query_map([], map_card)?;
    rows.collect::<Result<Vec<_>, _>>().context("reading cards")
}

/// Fetch one card by exact name.
///
/// # Errors
/// Propagates SQLite failures.
pub fn get_card(conn: &Connection, name: &str) -> anyhow::Result<Option<CardRow>> {
    let mut stmt = conn.prepare(&card_select())?;
    let mut rows = stmt.query_map([name], map_card)?;
    match rows.next() {
        Some(row) => Ok(Some(row.context("reading card")?)),
        None => Ok(None),
    }
}

/// True when an oracle card with this exact name is stored.
///
/// # Errors
/// Propagates SQLite failures.
pub fn card_exists(conn: &Connection, name: &str) -> anyhow::Result<bool> {
    let found: i64 = conn
        .prepare("SELECT COUNT(*) FROM cards WHERE name = ?1 COLLATE NOCASE")?
        .query_row([name], |row| row.get(0))
        .context("checking card name")?;
    Ok(found > 0)
}

/// Result of resolving a user-typed card name.
#[derive(Debug)]
pub enum NameMatch {
    /// Found a card.
    Found(Box<CardRow>),
    /// Prefix matched several names. `candidates` is a sample of real
    /// card names; `total` is the full match count (oracle names plus
    /// flavor-name aliases).
    Ambiguous {
        /// Sample of real card names that matched the prefix.
        candidates: Vec<String>,
        /// Full match count (oracle names plus flavor-name aliases).
        total: usize,
    },
    /// Nothing matched.
    NotFound,
}

/// Resolve a user-typed name: exact, then case-insensitive, then unique
/// prefix, then flavor-name alias (Godzilla series, Secret Lair crossovers).
///
/// Flavor names resolve as a whole string (exact or unique prefix) to the
/// oracle card of the print that carries them; the prefix pass runs across
/// oracle names and aliases together.
///
/// # Errors
/// Propagates SQLite failures.
pub fn resolve_name(conn: &Connection, typed: &str) -> anyhow::Result<NameMatch> {
    if let Some(card) = get_card(conn, typed)? {
        return Ok(NameMatch::Found(Box::new(card)));
    }
    let mut stmt = conn.prepare("SELECT name FROM cards WHERE name = ?1 COLLATE NOCASE")?;
    let mut rows = stmt.query_map([typed], |row| row.get::<_, String>(0))?;
    if let Some(first) = rows.next() {
        let exact_ci = first.context("reading name")?;
        return match get_card(conn, &exact_ci)? {
            Some(card) => Ok(NameMatch::Found(Box::new(card))),
            None => Ok(NameMatch::NotFound),
        };
    }
    prefix_match(conn, typed)
}

/// Resolve a typed name as a unique prefix across oracle names and
/// flavor-name aliases, or as a whole-string alias.
fn prefix_match(conn: &Connection, typed: &str) -> anyhow::Result<NameMatch> {
    // Unique prefix. Escape LIKE wildcards in the user text.
    let pattern = format!(
        "{}%",
        typed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    // Count the full prefix space first so the ambiguity report states
    // the real match count (the candidate lists are samples, not sums).
    let total: i64 = conn
        .prepare(
            "SELECT
                (SELECT COUNT(*) FROM cards WHERE name LIKE ?1 ESCAPE '\\' COLLATE NOCASE) +
                (SELECT COUNT(DISTINCT name) FROM card_prints
                 WHERE flavor_name LIKE ?1 ESCAPE '\\' COLLATE NOCASE)",
        )?
        .query_row([&pattern], |row| row.get(0))
        .context("counting name matches")?;
    let mut stmt = conn.prepare(
        "SELECT name FROM cards WHERE name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
         ORDER BY name LIMIT 2",
    )?;
    let mut rows = stmt.query_map([&pattern], |row| row.get::<_, String>(0))?;
    let first = rows.next().transpose().context("reading name")?;
    match first {
        Some(_) => prefix_candidates(conn, &pattern, total),
        None => alias_match(conn, typed),
    }
}

/// Resolve a name-prefix match: unique candidates resolve, otherwise the
/// ambiguity report carries samples and the real total.
fn prefix_candidates(conn: &Connection, pattern: &str, total: i64) -> anyhow::Result<NameMatch> {
    let mut candidates = conn
        .prepare(
            "SELECT name FROM cards WHERE name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
             ORDER BY name LIMIT 6",
        )?
        .query_map([pattern], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .context("reading names")?;
    // Flavor-name aliases join the same prefix space; each alias
    // maps back to the oracle card of the print carrying them, so a
    // unique alias prefix resolves.
    candidates.extend(
        conn.prepare(
            "SELECT DISTINCT name FROM card_prints
             WHERE flavor_name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
             ORDER BY name LIMIT 6",
        )?
        .query_map([pattern], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .context("reading aliases")?,
    );
    // Resolve when one oracle card backs the whole prefix space:
    // dedup first, because a card's own name and one of its
    // flavor-name aliases can both prefix-match — that is still
    // a single match, not an ambiguity.
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.len() == 1 {
        return resolve_candidate(conn, &candidates[0]);
    }
    let total = total.max(candidates.len() as i64) as usize;
    Ok(NameMatch::Ambiguous { candidates, total })
}

/// Whole-string alias match: the flavor name of some print ("Godzilla,
/// King of the Monsters" → Zilortha), exact first, then unique prefix.
fn alias_match(conn: &Connection, typed: &str) -> anyhow::Result<NameMatch> {
    let alias_names = |sql: &str| -> anyhow::Result<Vec<String>> {
        conn.prepare(sql)?
            .query_map([typed], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()
            .context("reading alias")
    };
    let exact = alias_names(
        "SELECT DISTINCT name FROM card_prints
         WHERE flavor_name = ?1 COLLATE NOCASE ORDER BY name LIMIT 1",
    )?;
    if let Some(name) = exact.into_iter().next() {
        return resolve_candidate(conn, &name);
    }
    // Unique alias prefix ("Godzilla, King" → the one flavor name
    // starting with it) resolves like a unique name prefix. Two or
    // more oracle cards behind the prefix space is ambiguity, not a
    // pick.
    let pattern = format!(
        "{}%",
        typed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut stmt = conn.prepare(
        "SELECT DISTINCT name FROM card_prints
         WHERE flavor_name LIKE ?1 ESCAPE '\\' COLLATE NOCASE ORDER BY name",
    )?;
    let mut rows = stmt.query_map([&pattern], |row| row.get::<_, String>(0))?;
    let mut matches: Vec<String> = Vec::new();
    for row in rows.by_ref() {
        matches.push(row.context("reading alias prefix")?);
        if matches.len() > 1 {
            break;
        }
    }
    if matches.len() > 1 {
        let total = matches.len() + rows.filter_map(Result::ok).count();
        return Ok(NameMatch::Ambiguous {
            candidates: matches,
            total,
        });
    }
    match matches.into_iter().next() {
        Some(name) => resolve_candidate(conn, &name),
        None => Ok(NameMatch::NotFound),
    }
}

/// Fetch the card row behind a resolved name (oracle or alias).
fn resolve_candidate(conn: &Connection, name: &str) -> anyhow::Result<NameMatch> {
    match get_card(conn, name)? {
        Some(card) => Ok(NameMatch::Found(Box::new(card))),
        None => Ok(NameMatch::NotFound),
    }
}

/// SQL for one card row by exact name; columns come from [`CARD_COLUMNS`].
fn card_select() -> String {
    format!("SELECT {CARD_COLUMNS} FROM cards WHERE name = ?1")
}

/// Map one `cards` row into a [`CardRow`].
///
/// Reads columns positionally, in [`CARD_COLUMNS`] order; the two must stay
/// aligned.
pub fn map_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<CardRow> {
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
        penny_rank: row.get(20)?,
        reserved: row.get(21)?,
    })
}

impl crate::search::Filterable for CardRow {
    fn name(&self) -> &str {
        &self.name
    }
    fn mana_cost(&self) -> &str {
        &self.mana_cost
    }
    fn cmc(&self) -> f64 {
        self.cmc
    }
    fn type_line(&self) -> &str {
        &self.type_line
    }
    fn colors(&self) -> &str {
        &self.colors
    }
    fn color_identity(&self) -> &str {
        &self.color_identity
    }
    fn keywords(&self) -> &str {
        &self.keywords
    }
    fn power(&self) -> Option<f64> {
        stat_number(&self.power)
    }
    fn toughness(&self) -> Option<f64> {
        stat_number(&self.toughness)
    }
    fn oracle_text(&self) -> &str {
        &self.oracle_text
    }
    fn rarity(&self) -> &str {
        &self.rarity
    }
    fn set_code(&self) -> &str {
        &self.set_code
    }
    fn legalities(&self) -> &str {
        &self.legalities
    }
}

/// Parse a stat string ("2", "-1", "1.5") into a number; `*`-style stats fail.
fn stat_number(stat: &Option<String>) -> Option<f64> {
    stat.as_deref().and_then(|s| s.trim().parse().ok())
}

/// Combine extracted query terms with OR or AND.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtsTermOperator {
    /// Return cards matching any extracted query term.
    Any,
    /// Return cards matching every extracted query term.
    All,
}

/// Build an FTS5 MATCH expression from free text: one quoted term per
/// whitespace-separated token, OR-joined by default.
///
/// Doubled `"` inside a term escapes it per FTS5 string rules. Returns `None`
/// when nothing usable remains (empty text or punctuation-only terms), which
/// callers treat as "skip the full-text leg".
pub fn fts_query(text: &str) -> Option<String> {
    fts_query_with_operator(text, FtsTermOperator::Any)
}

/// Build an FTS5 MATCH expression with explicit any-term or all-term matching.
pub fn fts_query_with_operator(text: &str, operator: FtsTermOperator) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        let separator = match operator {
            FtsTermOperator::Any => " OR ",
            FtsTermOperator::All => " AND ",
        };
        Some(terms.join(separator))
    }
}

/// Column weights for BM25 ranking, passed to `bm25()` as positional args:
/// name hits dominate, tag labels second, type line third, oracle text last.
pub const FTS_COLUMN_WEIGHTS: [f64; 4] = [8.0, 4.0, 2.0, 1.0];

/// True when `name` is a known non-card (token, art series, emblem) from the
/// last bulk pass. Collection imports skip these silently.
///
/// # Errors
/// Propagates SQLite failures.
pub fn is_token_name(conn: &Connection, name: &str) -> anyhow::Result<bool> {
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM token_names WHERE name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(found > 0)
}

/// BM25-ranked FTS hits joined back to card ids.
///
/// Returns `(cards.id, rank)` pairs, best first. FTS5's `rank` is the negated
/// BM25 score, so ascending order puts the best match first. Column weights
/// come from [`FTS_COLUMN_WEIGHTS`].
///
/// # Errors
/// Propagates SQLite failures.
pub fn fts_search(
    conn: &Connection,
    match_expr: &str,
    limit: usize,
) -> anyhow::Result<Vec<(i64, f64)>> {
    fts_search_with_weights(conn, match_expr, limit, FTS_COLUMN_WEIGHTS)
}

/// BM25-ranked FTS hits using explicit weights for each indexed column.
///
/// The weight order is name, tags, type line, and oracle text. Search
/// evaluation uses this function to compare ranking settings without changing
/// the defaults used by [`fts_search`].
///
/// # Errors
/// Rejects non-finite or negative weights and propagates SQLite failures.
pub fn fts_search_with_weights(
    conn: &Connection,
    match_expr: &str,
    limit: usize,
    weights: [f64; 4],
) -> anyhow::Result<Vec<(i64, f64)>> {
    fts_search_with_allowed_names(conn, match_expr, limit, weights, None)
}

/// Rank FTS hits after an optional restriction to exact canonical card names.
///
/// `None` allows every name. An empty set allows no names.
///
/// # Errors
/// Rejects non-finite or negative weights and propagates JSON and SQLite failures.
pub(crate) fn fts_search_with_allowed_names(
    conn: &Connection,
    match_expr: &str,
    limit: usize,
    weights: [f64; 4],
    allowed_names: Option<&std::collections::HashSet<String>>,
) -> anyhow::Result<Vec<(i64, f64)>> {
    anyhow::ensure!(
        weights
            .iter()
            .all(|weight| weight.is_finite() && *weight >= 0.0),
        "FTS weights must be finite and non-negative"
    );
    let allowed_json = allowed_names
        .map(|names| {
            let mut names: Vec<&str> = names.iter().map(String::as_str).collect();
            names.sort_unstable();
            serde_json::to_string(&names)
        })
        .transpose()?;
    // Restrict before LIMIT so unowned matches cannot hide owned cards beyond
    // the retry window. One JSON parameter avoids SQLite's variable limit.
    // BM25 ties break toward lower EDHREC rank (more popular first), then
    // row order for determinism.
    let mut stmt = conn.prepare(&format!(
        "SELECT cards.id, cards_fts.rank FROM cards_fts
         JOIN cards ON cards.id = cards_fts.rowid
         WHERE cards_fts MATCH ?1
           AND (?3 IS NULL OR cards.name IN (SELECT value FROM json_each(?3)))
         ORDER BY bm25(cards_fts, {}, {}, {}, {}),
             cards.edhrec_rank IS NULL, cards.edhrec_rank, cards.id
         LIMIT ?2",
        weights[0], weights[1], weights[2], weights[3],
    ))?;
    let rows = stmt
        .query_map(
            rusqlite::params![match_expr, limit as i64, allowed_json],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()
        .context("running full-text search")?;
    Ok(rows)
}

/// Fill `cards.tags_text` for every card from the `card_tags` join, then
/// rebuild the FTS rows touched.
///
/// Called after a tag ingest (setup and sync): the tag labels become
/// searchable full-text content (weight second only to the card name), so
/// role words like "ramp" or "sweeper" match the community vocabulary even
/// when oracle text never uses them.
///
/// # Errors
/// Propagates SQLite failures.
pub fn refresh_tags_text(conn: &Connection) -> anyhow::Result<usize> {
    // Restrict to rows whose tags_text would change: skips the FTS
    // trigger for untouched rows and keeps the returned count honest.
    let updated = conn.execute(
        "UPDATE cards SET tags_text = COALESCE((
            SELECT GROUP_CONCAT(label, ' ') FROM (
                SELECT DISTINCT t.label AS label
                FROM card_tags ct JOIN tags t ON t.id = ct.tag_id
                WHERE ct.oracle_id = cards.oracle_id
            )
        ), '')
        WHERE cards.tags_text IS DISTINCT FROM COALESCE((
            SELECT GROUP_CONCAT(label, ' ') FROM (
                SELECT DISTINCT t.label AS label
                FROM card_tags ct JOIN tags t ON t.id = ct.tag_id
                WHERE ct.oracle_id = cards.oracle_id
            )
        ), '')",
        [],
    )?;
    Ok(updated)
}

/// The complete version-1 database schema.
const INITIAL_SCHEMA: &str = include_str!("../migrations/0001_initial_schema.sql");

/// Create the version-1 schema for an empty database.
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(INITIAL_SCHEMA)])
}

/// Read schema definitions without data or storage details.
fn schema_definitions(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT type || ' ' || name || ': ' || sql
         FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name",
    )?;
    let definitions = stmt.query_map([], |row| row.get(0))?;
    Ok(definitions.collect::<Result<Vec<_>, _>>()?)
}

/// Accept empty databases or databases with exactly the version-1 schema.
fn validate_schema(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let stored = schema_definitions(conn)?;
    if version == 0 && stored.is_empty() {
        return Ok(());
    }
    anyhow::ensure!(
        version == 1,
        "unsupported database schema version {version}; expected version 1"
    );
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(INITIAL_SCHEMA)?;
    anyhow::ensure!(
        stored == schema_definitions(&expected)?,
        "database schema does not match version 1"
    );
    Ok(())
}

/// Open the version-1 SQLite database at `path`, creating it if needed.
///
/// WAL mode keeps concurrent reads cheap (a background `stm sync` may write
/// while a command reads); the busy timeout lets writers queue briefly
/// instead of failing. Foreign keys are off because the schema joins on card
/// *name*, not ids.
///
/// # Errors
/// Fails for an incompatible schema. Propagates SQLite failures with context.
pub fn open(path: &std::path::Path) -> anyhow::Result<Connection> {
    let mut conn = Connection::open(path)
        .with_context(|| format!("failed to open database at {}", path.display()))?;
    conn.pragma_update(None, "busy_timeout", 10_000)?;
    let transaction = conn.transaction()?;
    validate_schema(&transaction).context("validating database schema")?;
    transaction.commit()?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrations()
        .to_latest(&mut conn)
        .context("creating database schema")?;
    Ok(conn)
}

/// Reattach after a store-lock wait that may have included a forced reset.
pub(crate) fn reopen(conn: &mut Connection, path: &std::path::Path) -> anyhow::Result<()> {
    let old = std::mem::replace(conn, Connection::open_in_memory()?);
    old.close().map_err(|(_, err)| err)?;
    *conn = open(path)?;
    Ok(())
}

/// Tests for stored cards, name matching, and full-text search.
#[cfg(test)]
#[path = "tests/db_tests.rs"]
mod tests;

/// Tests for version-1 schema creation and validation.
#[cfg(test)]
#[path = "tests/db_schema_tests.rs"]
mod schema_tests;
