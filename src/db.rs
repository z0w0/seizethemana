use anyhow::Context;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

// SQLite layer: connection bootstrap, schema migrations, and the card row
// type shared by every module. Collection access lives in `collection.rs`,
// price access in `prints.rs`; both use this module's connection helpers.

/// One stored card, deserialized from the `cards` table.
///
/// JSON columns (colors, legalities…) are kept as raw JSON strings; consumers
/// parse what they need. Per-print prices live in the `card_prints` table
/// (`src/prints.rs`), keyed by `scryfall_id`.
#[derive(Debug, Clone)]
pub struct CardRow {
    pub name: String,
    /// Scryfall oracle ID; join key for `card_tags` (tag associations).
    pub oracle_id: String,
    pub mana_cost: String,
    pub cmc: f64,
    pub type_line: String,
    pub colors: String,
    pub color_identity: String,
    pub keywords: String,
    pub power: Option<String>,
    pub toughness: Option<String>,
    pub loyalty: Option<String>,
    pub oracle_text: String,
    pub rarity: String,
    pub edhrec_rank: Option<i64>,
    pub legalities: String,
    pub set_code: String,
    pub collector_number: String,
    /// Scryfall print ID of the stored representative print; join key for
    /// `prices`.
    pub scryfall_id: String,
    /// Release date (YYYY-MM-DD) of the oracle's latest recognized printing;
    /// empty when unknown. Display only — ingest gating keeps unreleased
    /// cards out of the store (`scryfall::should_ingest`).
    pub released_at: String,
    /// True when the card is on the Commander Game Changer list (bracket
    /// signal for `deck legal`). Null when the bulk did not say.
    pub game_changer: Option<bool>,
}

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
    let mut stmt = conn
        .prepare(
            "SELECT name, oracle_id, mana_cost, cmc, type_line, colors, color_identity, keywords,
                    power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
                    legalities, set_code, collector_number, scryfall_id, released_at,
                    game_changer
             FROM cards ORDER BY id",
        )
        .context("prepare card query")?;
    let rows = stmt.query_map([], |row| {
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
    })?;
    rows.collect::<Result<Vec<_>, _>>().context("reading cards")
}

/// Fetch one card by exact name.
///
/// # Errors
/// Propagates SQLite failures.
pub fn get_card(conn: &Connection, name: &str) -> anyhow::Result<Option<CardRow>> {
    let mut stmt = conn.prepare(card_select())?;
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
        candidates: Vec<String>,
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

/// SQL column list for one `cards` row; `map_card` reads in this order.
fn card_select() -> &'static str {
    "SELECT name, oracle_id, mana_cost, cmc, type_line, colors, color_identity, keywords,
            power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
            legalities, set_code, collector_number, scryfall_id, released_at,
            game_changer
     FROM cards WHERE name = ?1"
}

/// Map one `cards` row in `card_select` column order.
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
    anyhow::ensure!(
        weights
            .iter()
            .all(|weight| weight.is_finite() && *weight >= 0.0),
        "FTS weights must be finite and non-negative"
    );
    // BM25 ties break toward lower EDHREC rank (more popular first), then
    // row order for determinism.
    let mut stmt = conn.prepare(&format!(
        "SELECT cards.id, cards_fts.rank FROM cards_fts
         JOIN cards ON cards.id = cards_fts.rowid
         WHERE cards_fts MATCH ?1
         ORDER BY bm25(cards_fts, {}, {}, {}, {}),
             cards.edhrec_rank IS NULL, cards.edhrec_rank, cards.id
         LIMIT ?2",
        weights[0], weights[1], weights[2], weights[3],
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![match_expr, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })?
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

/// Schema migrations, applied forward-only on every open.
///
/// Each migration lives in `migrations/` as `NNNN_description.sql`, named in
/// order (`0001_initial_schema.sql` creates the whole v1 schema). The store
/// has not shipped yet, so `0001` is edited in place when the schema
/// changes; an existing database has no version bump to trigger, so
/// `stm setup --force` deletes the database and rebuilds. After the first
/// release, edits must stop and each schema change becomes a new
/// `NNNN_description.sql` appended to `migrations()`.
///
/// # Errors
/// Propagates migration failures.
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../migrations/0001_initial_schema.sql")),
        M::up(include_str!("../migrations/0002_universe.sql")),
    ])
}

/// Open (creating if needed) the SQLite database at `path`, migrate to the
/// latest schema version, and return the connection.
///
/// WAL mode keeps concurrent reads cheap (a background `stm sync` may write
/// while a command reads); the busy timeout lets writers queue briefly
/// instead of failing. Foreign keys are off because the schema joins on card
/// *name*, not ids.
///
/// # Errors
/// Propagates SQLite open or migration failures with context.
pub fn open(path: &std::path::Path) -> anyhow::Result<Connection> {
    let mut conn = Connection::open(path)
        .with_context(|| format!("failed to open database at {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 10_000)?;
    migrations()
        .to_latest(&mut conn)
        .context("applying database migrations")?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card(conn: &Connection, name: &str) {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, power, toughness, loyalty, oracle_text,
                rarity, edhrec_rank, legalities, set_code, collector_number,
                scryfall_id, released_at)
             VALUES (?1, 'oid', '{R}', 1.0, 'Instant', '[\"R\"]', '[\"R\"]', '[]',
                NULL, NULL, NULL, 'Deal 3 damage', 'uncommon', 42,
                '{\"modern\":\"legal\"}', '3ED', '200', 'id-1', '2006-10-01')",
            rusqlite::params![name],
        )
        .expect("insert");
    }

    #[test]
    fn load_all_cards_maps_every_column() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        sample_card(&conn, "Lightning Bolt");

        let cards = load_all_cards(&conn).expect("load");
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!(c.name, "Lightning Bolt");
        assert_eq!(c.mana_cost, "{R}");
        assert_eq!(c.cmc, 1.0);
        assert_eq!(c.type_line, "Instant");
        assert_eq!(c.colors, "[\"R\"]");
        assert_eq!(c.power, None);
        assert_eq!(c.oracle_text, "Deal 3 damage");
        assert_eq!(c.rarity, "uncommon");
        assert_eq!(c.edhrec_rank, Some(42));
        assert_eq!(c.set_code, "3ED");
        assert_eq!(c.collector_number, "200");
        assert_eq!(c.scryfall_id, "id-1");
        assert_eq!(c.released_at, "2006-10-01");
        assert_eq!(c.game_changer, None);
        let legalities: serde_json::Value = serde_json::from_str(&c.legalities).unwrap();
        assert_eq!(legalities["modern"], serde_json::json!("legal"));
    }

    #[test]
    fn load_all_cards_preserves_order_and_handles_many() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        for name in ["A Card", "B Card", "C Card"] {
            sample_card(&conn, name);
        }
        let cards = load_all_cards(&conn).expect("load");
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["A Card", "B Card", "C Card"]);
    }

    #[test]
    fn load_all_cards_empty_db() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        assert!(load_all_cards(&conn).expect("load").is_empty());
    }

    #[test]
    fn open_creates_schema_and_is_reopenable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("test.db");

        let conn = open(&db_path).expect("first open");
        let (n,): (i64,) = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name IN
                    ('cards','collection','card_prints','sets','tags',
                     'card_tags','token_names','combos','combo_pieces')",
                [],
                |row| Ok((row.get(0).unwrap(),)),
            )
            .expect("query");
        assert_eq!(n, 9);
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(version, 2, "flattened schema plus the universe migration");
        // The game_changer column must exist on the cards table.
        let gc: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'game_changer'",
                [],
                |r| r.get(0),
            )
            .expect("pragma");
        assert_eq!(gc, 1);
        // The universe migration columns must exist.
        let ub: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('card_prints')
                 WHERE name = 'universes_beyond'",
                [],
                |r| r.get(0),
            )
            .expect("pragma");
        assert_eq!(ub, 1);
        let franchise: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sets') WHERE name = 'franchise'",
                [],
                |r| r.get(0),
            )
            .expect("pragma");
        assert_eq!(franchise, 1);
        drop(conn);

        // Reopening must not fail on existing schema.
        let conn = open(&db_path).expect("reopen");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get::<_, i64>(0))
                .expect("count"),
            0
        );
    }

    #[test]
    fn card_name_is_unique() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        conn.execute(
            "INSERT INTO cards (name, oracle_id) VALUES ('Bolt', 'x1')",
            [],
        )
        .expect("insert");
        assert!(
            conn.execute(
                "INSERT INTO cards (name, oracle_id) VALUES ('Bolt', 'x2')",
                []
            )
            .is_err()
        );
    }

    #[test]
    fn get_card_requires_exact_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        sample_card(&conn, "Lightning Bolt");
        assert!(get_card(&conn, "Lightning Bolt").expect("get").is_some());
        assert!(get_card(&conn, "lightning bolt").expect("get").is_none());
    }

    #[test]
    fn resolve_name_falls_back_to_case_then_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        sample_card(&conn, "Lightning Bolt");
        sample_card(&conn, "Lightning Helix");
        sample_card(&conn, "Lightning Strike");

        match resolve_name(&conn, "lightning bolt").expect("resolve") {
            NameMatch::Found(c) => assert_eq!(c.name, "Lightning Bolt"),
            other => panic!("expected case-insensitive hit, got {other:?}"),
        }
        match resolve_name(&conn, "Lightning B").expect("resolve") {
            NameMatch::Found(c) => assert_eq!(c.name, "Lightning Bolt"),
            other => panic!("expected unique prefix, got {other:?}"),
        }
        match resolve_name(&conn, "Lightning").expect("resolve") {
            NameMatch::Ambiguous { candidates, total } => {
                assert_eq!(candidates.len(), 3);
                assert_eq!(total, 3);
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
        assert!(matches!(
            resolve_name(&conn, "Nope").expect("resolve"),
            NameMatch::NotFound
        ));
        // Wildcards in user input are literal.
        assert!(matches!(
            resolve_name(&conn, "Lightning %").expect("resolve"),
            NameMatch::NotFound
        ));
    }

    #[test]
    fn resolve_name_resolves_flavor_name_aliases() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        sample_card(&conn, "Zilortha, Light of Ikoria");
        // An alias (whole string and unique prefix) resolves to the
        // oracle card of the print carrying it.
        let alias = |scryfall_id: &str, flavor: &str| {
            conn.execute(
                "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                    lang, rarity, finishes, released_at, flavor_name, updated_at)
                 VALUES (?1, 'Zilortha, Light of Ikoria', 'iko', '1', 'en',
                    'rare', '[\"nonfoil\"]', '2020-01-01', ?2, 'now')",
                rusqlite::params![scryfall_id, flavor],
            )
            .expect("insert print");
        };
        alias("godzilla", "Godzilla, King of the Monsters");
        match resolve_name(&conn, "Godzilla, King of the Monsters").expect("resolve alias") {
            NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Light of Ikoria"),
            other => panic!("expected whole-string alias hit, got {other:?}"),
        }
        match resolve_name(&conn, "Godzilla, King").expect("resolve alias prefix") {
            NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Light of Ikoria"),
            other => panic!("expected unique alias prefix, got {other:?}"),
        }
        // A card whose own name and its alias both prefix-match still
        // resolves (the dedup collapses the two paths to one card).
        match resolve_name(&conn, "Zilortha").expect("resolve shared prefix") {
            NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Light of Ikoria"),
            other => panic!("expected deduped single match, got {other:?}"),
        }
    }

    #[test]
    fn resolve_name_flags_ambiguous_alias_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        sample_card(&conn, "Zilortha, Light of Ikoria");
        sample_card(&conn, "Zilortha, Apex of Ikoria");
        // Two oracle cards behind one alias prefix ("Godzilla,") must
        // report ambiguity, not resolve to the alphabetically first.
        let alias = |scryfall_id: &str, card: &str, flavor: &str| {
            conn.execute(
                "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                    lang, rarity, finishes, released_at, flavor_name, updated_at)
                 VALUES (?1, ?2, 'iko', '1', 'en',
                    'rare', '[\"nonfoil\"]', '2020-01-01', ?3, 'now')",
                rusqlite::params![scryfall_id, card, flavor],
            )
            .expect("insert print");
        };
        alias(
            "g1",
            "Zilortha, Light of Ikoria",
            "Godzilla, King of the Monsters",
        );
        alias(
            "g2",
            "Zilortha, Apex of Ikoria",
            "Godzilla, Primeval Champion",
        );
        match resolve_name(&conn, "Godzilla, ").expect("resolve shared alias prefix") {
            NameMatch::Ambiguous { candidates, total } => {
                assert_eq!(total, 2);
                assert_eq!(
                    candidates,
                    vec!["Zilortha, Apex of Ikoria", "Zilortha, Light of Ikoria"]
                );
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
        // The whole-string match still resolves to its own card.
        match resolve_name(&conn, "Godzilla, Primeval Champion").expect("resolve exact alias") {
            NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Apex of Ikoria"),
            other => panic!("expected exact alias hit, got {other:?}"),
        }
    }

    #[test]
    fn collection_key_spans_binder_and_foil_kind() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        let insert = |binder: &str, foil: &str, qty: i64| -> rusqlite::Result<usize> {
            conn.execute(
                "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
                 VALUES ('Bolt', 'STA', '1', ?2, ?1, 'binder', ?3)",
                rusqlite::params![binder, foil, qty],
            )
        };
        insert("Collect", "normal", 2).expect("insert");
        // Foil kind and location make distinct rows.
        insert("Collect", "foil", 1).expect("insert");
        insert("Deck Box", "normal", 1).expect("insert");
        assert!(insert("Collect", "normal", 5).is_err());
        // binder_type is constrained.
        assert!(
            conn.execute(
                "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type)
                 VALUES ('Bolt', 'STA', '1', 'normal', 'X', 'list')",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn fts_query_quotes_terms_and_skips_punctuation_only() {
        assert_eq!(
            fts_query("lightning bolt"),
            Some("\"lightning\" OR \"bolt\"".to_string())
        );
        assert_eq!(
            fts_query("a  b\tc"),
            Some("\"a\" OR \"b\" OR \"c\"".to_string())
        );
        assert_eq!(
            fts_query_with_operator("lightning bolt", FtsTermOperator::All),
            Some("\"lightning\" AND \"bolt\"".to_string())
        );
        assert_eq!(
            fts_query("\"quoted\" name"),
            Some("\"\"\"quoted\"\"\" OR \"name\"".to_string())
        );
        assert_eq!(fts_query(""), None);
        assert_eq!(fts_query("   ... !!! ---   "), None);
    }

    #[test]
    fn fts_index_stays_synced_with_cards() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&tmp.path().join("t.db")).expect("open");
        sample_card(&conn, "Lightning Bolt");
        // Stemmed term matches through the porter tokenizer.
        assert_eq!(
            fts_search(&conn, "\"lightning\" OR \"bolt\"", 10)
                .expect("fts")
                .len(),
            1
        );
        assert!(fts_search(&conn, "\"helix\"", 10).expect("fts").is_empty());

        // Update rewrites the index entry (old tokens vanish, new ones rank).
        conn.execute(
            "UPDATE cards SET oracle_text = 'Hurl helix of flame' WHERE name = 'Lightning Bolt'",
            [],
        )
        .expect("update");
        assert!(fts_search(&conn, "\"helix\"", 10).expect("fts").len() == 1);
        assert!(fts_search(&conn, "\"damage\"", 10).expect("fts").is_empty());

        // Delete removes the row from the index.
        conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
            .expect("delete");
        assert!(fts_search(&conn, "\"helix\"", 10).expect("fts").is_empty());
    }
}
