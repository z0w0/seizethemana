use anyhow::Context;

// `stm card`: full detail for one card by (fuzzy) name, plus tag-overlap
// neighbors (`stm card similar`).

/// Entry point for `stm card <name>`.
///
/// Resolution is exact → case-insensitive → unique prefix; ambiguous prefixes
/// exit 3 with candidate names, unknown names exit 3.
pub fn run_card(
    paths: &crate::paths::Paths,
    conn: &mut rusqlite::Connection,
    out: &mut crate::output::Output,
    typed: &str,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let card = match crate::db::resolve_name(conn, typed).context("resolving card name")? {
        crate::db::NameMatch::Found(card) => card,
        crate::db::NameMatch::Ambiguous(candidates) => {
            out.error(&format!(
                "{typed:?} matches {} cards; be more specific",
                candidates.len()
            ));
            out.hint(&format!("did you mean: {}", candidates.join(", ")));
            return Ok(crate::cli::codes::NO_RESULTS);
        }
        crate::db::NameMatch::NotFound => {
            out.error(&format!("no card named {typed:?}"));
            out.hint("names resolve by exact match, case, or unique prefix");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
    };

    if json {
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let ranges = crate::prints::price_ranges(conn, std::slice::from_ref(&card.name))
            .ok()
            .unwrap_or_default();
        let range = ranges.get(&card.name).cloned().unwrap_or_default();
        let universe = crate::universe::card_universe(conn, &card.name, &card.set_code)?;
        print_json(&card, &tag_index, &range, &universe)?;
    } else {
        let range = crate::prints::price_range(conn, &card.name)
            .ok()
            .unwrap_or_default();
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let universe = crate::universe::card_universe(conn, &card.name, &card.set_code)?;
        print_text(out, &card, &range, &tag_index, &universe);
    }
    Ok(crate::cli::codes::OK)
}

fn print_json(
    card: &crate::db::CardRow,
    tag_index: &crate::tags::TagIndex,
    range: &crate::prints::PrintRange,
    universe: &crate::universe::CardUniverse,
) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(&card_json(card, tag_index, range, universe))?;
    println!("{text}");
    Ok(())
}

/// Full card detail as a JSON value (also used by collection JSON output).
///
/// Price fields are per-printing: `price_usd`/`price_usd_foil` carry the
/// cheapest released English printing, `max_price_usd`/`max_price_usd_foil`
/// the most expensive. All four are null when no print is priced.
pub fn card_json(
    card: &crate::db::CardRow,
    tag_index: &crate::tags::TagIndex,
    range: &crate::prints::PrintRange,
    universe: &crate::universe::CardUniverse,
) -> serde_json::Value {
    let colors: serde_json::Value = serde_json::from_str(&card.colors).unwrap_or_default();
    let identity: serde_json::Value =
        serde_json::from_str(&card.color_identity).unwrap_or_default();
    let keywords: serde_json::Value = serde_json::from_str(&card.keywords).unwrap_or_default();
    let legalities: serde_json::Value = serde_json::from_str(&card.legalities).unwrap_or_default();
    let (usd, usd_foil) = (
        range.cheapest.as_ref().and_then(|p| p.usd),
        range.cheapest_foil.as_ref().and_then(|p| p.usd_foil),
    );
    let (max_usd, max_usd_foil) = (
        range.priciest.as_ref().and_then(|p| p.usd),
        range.priciest_foil.as_ref().and_then(|p| p.usd_foil),
    );
    serde_json::json!({
        "name": card.name,
        "oracle_id": card.oracle_id,
        "mana_cost": card.mana_cost,
        "cmc": card.cmc,
        "type_line": card.type_line,
        "colors": colors,
        "color_identity": identity,
        "keywords": keywords,
        "power": card.power,
        "toughness": card.toughness,
        "loyalty": card.loyalty,
        "oracle_text": card.oracle_text,
        "rarity": card.rarity,
        "edhrec_rank": card.edhrec_rank,
        "legalities": legalities,
        "game_changer": card.game_changer,
        "set": card.set_code,
        "set_name": universe.set_name,
        "set_type": universe.set_type,
        "block": universe.block,
        "universe": universe.universe,
        "franchise": universe.franchise,
        "collector_number": card.collector_number,
        "scryfall_id": card.scryfall_id,
        "released_at": card.released_at,
        "tags": tag_index.labels_for(&card.oracle_id),
        "price_usd": usd,
        "price_usd_foil": usd_foil,
        "max_price_usd": max_usd,
        "max_price_usd_foil": max_usd_foil,
    })
}

/// Render a framed magic-card-style view on stdout.
///
/// Box width follows the terminal (clamped to 40..=100); falls back to the
/// same frame at width 72 when the terminal size is unknown. The plain (no
/// color) path keeps the frame — it still reads as a card.
fn print_text(
    out: &crate::output::Output,
    card: &crate::db::CardRow,
    range: &crate::prints::PrintRange,
    tag_index: &crate::tags::TagIndex,
    universe: &crate::universe::CardUniverse,
) {
    let styles = out.styles();
    let width = crate::output::terminal_width().clamp(40, 100);
    let inner = width - 4; // "│ " + content + " │"
    let top = format!("╭{}╮", "─".repeat(width - 2));
    let bottom = format!("╰{}╯", "─".repeat(width - 2));
    let rule = format!("├{}┤", "─".repeat(width - 2));

    // Visible-width helpers: ANSI codes add bytes but no columns, so the
    // frame pads by measured width, not byte length.
    let measure = console::measure_text_width;
    let line = |content: String| -> String {
        let pad = inner.saturating_sub(measure(&content));
        format!("│ {content}{} │", " ".repeat(pad))
    };
    let blank = line(String::new());

    println!("{}", styles.dim(&top));
    // Header: name left, mana cost right.
    let name = styles.card_name(&card.name);
    let cost = styles.mana_pips(&card.mana_cost);
    let gap = inner.saturating_sub(measure(&name) + measure(&cost));
    println!("│ {name}{}{cost} │", " ".repeat(gap));
    println!("{}", blank);
    println!(
        "{}",
        line(format!(
            "{}  {}",
            styles.dim(&card.type_line),
            styles.rarity(&card.rarity)
        ))
    );
    if let (Some(p), Some(t)) = (&card.power, &card.toughness) {
        println!("{}", line(format!("P/T: {p}/{t}")));
    } else if let Some(loyalty) = &card.loyalty {
        println!("{}", line(format!("Loyalty: {loyalty}")));
    }
    // Community role labels from Tagger (omitted when the card has none).
    let labels = tag_index.labels_for(&card.oracle_id);
    if !labels.is_empty() {
        for text_line in crate::output::Styles::wrap(&format!("Tags: {}", labels.join(", ")), inner)
        {
            println!("{}", line(styles.dim(&text_line)));
        }
    }
    println!("{}", styles.dim(&rule));
    println!("{}", blank);
    for text_line in crate::output::Styles::wrap(&card.oracle_text, inner) {
        println!("{}", line(text_line));
    }
    println!("{}", blank);
    println!("{}", styles.dim(&rule));
    println!("{}", blank);

    // Footer: set · collector number · release date, then extras.
    let set_display = match &universe.set_name {
        Some(name) => format!("{name} ({})", card.set_code),
        None => card.set_code.clone(),
    };
    println!(
        "{}",
        line(format!(
            "{} {} #{} · {}",
            styles.dim("Set:"),
            styles.dim(&set_display),
            styles.dim(&card.collector_number),
            styles.dim(&crate::release::release_display(&card.released_at)),
        ))
    );
    // Universe line: beyond/franchise when set, block for multiverse sets.
    let universe_bit = match (universe.universe, &universe.franchise) {
        ("beyond", Some(franchise)) => format!("Universes Beyond · Franchise: {franchise}"),
        ("beyond", None) => "Universes Beyond".to_string(),
        _ => match &universe.block {
            Some(block) => format!("Block: {block}"),
            None => String::new(),
        },
    };
    if !universe_bit.is_empty() {
        println!("{}", line(styles.dim(&universe_bit)));
    }
    if let Some(rank) = card.edhrec_rank {
        println!(
            "{}",
            line(format!(
                "{} {}",
                styles.dim("EDHREC rank:"),
                styles.thousands(rank)
            ))
        );
    }
    // Price line: cheapest and priciest printings across finishes.
    let mut price_bits: Vec<String> = Vec::new();
    if let Some(p) = &range.cheapest
        && let Some(usd) = p.usd
    {
        price_bits.push(format!(
            "from {} ({} {})",
            styles.money(usd),
            p.set_code,
            p.collector_number
        ));
    }
    if let Some(p) = &range.priciest_foil
        && let Some(usd) = p.usd_foil
    {
        price_bits.push(format!("foil up to {}", styles.money(usd)));
    } else if let Some(p) = &range.priciest
        && let Some(usd) = p.usd
        && range.priciest.as_ref() != range.cheapest.as_ref()
    {
        price_bits.push(format!("up to {}", styles.money(usd)));
    }
    if !price_bits.is_empty() {
        println!(
            "{}",
            line(format!(
                "{} {}",
                styles.dim("Price:"),
                styles.dim(&price_bits.join(" · "))
            ))
        );
    }
    // Legalities condensed to playable formats (legal/restricted only —
    // banned and not_legal never count), wrapped inside the frame.
    // Wrap plain text first, then style, so ANSI codes never split mid-wrap.
    let legal: Vec<String> =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&card.legalities)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, v)| {
                v.as_str()
                    .is_some_and(|s| s == "legal" || s == "restricted")
            })
            .map(|(k, _)| k)
            .collect();
    if !legal.is_empty() {
        let joined = legal.join(", ");
        let text_lines = crate::output::Styles::wrap(&format!("Legal in: {joined}"), inner);
        for (i, text_line) in text_lines.into_iter().enumerate() {
            let content = if i == 0 {
                format!(
                    "{} {}",
                    styles.dim("Legal in:"),
                    styles.dim(&text_line["Legal in: ".len()..])
                )
            } else {
                styles.dim(&format!("           {text_line}"))
            };
            println!("{}", line(content));
        }
    }
    println!("{}", blank);
    println!("{}", styles.dim(&bottom));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tag index with no tags; card JSON then carries an empty tags list.
    fn empty_index() -> crate::tags::TagIndex {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        crate::tags::TagIndex::load(&conn).unwrap()
    }

    #[test]
    fn rank_similar_orders_by_shared_tags() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        // Seed: Bolt (oid-bolt) tagged t1, t2, t3.
        // Other cards: Fire (t1,t2 => 2 shared), Ice (t1 => 1), Boltless (none).
        insert_card(&conn, "Bolt", "oid-bolt");
        insert_card(&conn, "Fire", "oid-fire");
        insert_card(&conn, "Ice", "oid-ice");
        insert_card(&conn, "Boltless", "oid-none");
        for oid in ["oid-bolt", "oid-fire", "oid-ice"] {
            conn.execute(
                "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES (?1, 't1')",
                [oid],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES ('oid-bolt', 't2')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES ('oid-fire', 't2')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES ('oid-bolt', 't3')",
            [],
        )
        .unwrap();
        for label in [("t1", "burn"), ("t2", "damage"), ("t3", "instant speed")] {
            conn.execute(
                "INSERT OR IGNORE INTO tags (id, slug, label, use_count) VALUES (?1, ?1, ?2, 1)",
                rusqlite::params![label.0, label.1],
            )
            .unwrap();
        }

        let hits = rank_similar(&conn, "oid-bolt", 10, None).unwrap();
        assert_eq!(hits.len(), 2, "Boltless shares nothing");
        assert_eq!(hits[0].card.name, "Fire");
        assert_eq!(hits[0].shared_count, 2);
        assert_eq!(hits[0].shared_tags, vec!["burn", "damage"]);
        assert_eq!(hits[1].card.name, "Ice");
        assert_eq!(hits[1].shared_count, 1);

        // The owned filter restricts results.
        conn.execute(
            "INSERT INTO collection (name, binder, binder_type) VALUES ('Ice', 'Collect', 'binder')",
            [],
        )
        .unwrap();
        let owned = crate::collection::owned_names_all(&conn).unwrap();
        let hits = rank_similar(&conn, "oid-bolt", 10, Some(&owned)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card.name, "Ice");

        // Over-select: with a tiny limit and mostly-unowned matches, the
        // owned window must still fill from rows past the SQL cut.
        conn.execute(
            "INSERT INTO collection (name, binder, binder_type) VALUES ('Waste', 'Collect', 'binder')",
            [],
        )
        .unwrap();
        // Waste shares one tag with Bolt and ranks low (high EDHREC rank).
        insert_card(&conn, "Waste", "oid-waste");
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-waste', 't1')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE cards SET edhrec_rank = 20000 WHERE name = 'Waste'",
            [],
        )
        .unwrap();
        let owned = crate::collection::owned_names_all(&conn).unwrap();
        let hits = rank_similar(&conn, "oid-bolt", 1, Some(&owned)).unwrap();
        // Fire and Ice are unowned; Waste is owned and must not be cut by
        // the SQL LIMIT 1 before the filter.
        assert_eq!(hits.len(), 1, "owned window filled past the SQL cut");
        assert_eq!(hits[0].card.name, "Waste");
    }

    fn insert_card(conn: &rusqlite::Connection, name: &str, oracle_id: &str) {
        conn.execute(
            "INSERT INTO cards (name, oracle_id) VALUES (?1, ?2)",
            rusqlite::params![name, oracle_id],
        )
        .unwrap();
    }

    fn row() -> crate::db::CardRow {
        crate::db::CardRow {
            name: "Test Card".into(),
            oracle_id: "oid".into(),
            mana_cost: "{1}{R}".into(),
            cmc: 2.0,
            type_line: "Creature — Human".into(),
            colors: r#"["R"]"#.into(),
            color_identity: r#"["R"]"#.into(),
            keywords: r#"["Haste"]"#.into(),
            power: Some("2".into()),
            toughness: Some("2".into()),
            loyalty: None,
            oracle_text: "Haste.\nWhen this enters, deal 1 damage.".into(),
            rarity: "uncommon".into(),
            edhrec_rank: Some(500),
            legalities: r#"{"modern":"legal","legacy":"not_legal"}"#.into(),
            set_code: "TST".into(),
            collector_number: "7".into(),
            scryfall_id: "sid".into(),
            released_at: "2020-01-01".into(),
            game_changer: None,
        }
    }

    #[test]
    fn card_json_carries_all_fields() {
        let index = empty_index();
        let v = card_json(&row(), &index, &Default::default(), &Default::default());
        assert_eq!(v["name"], "Test Card");
        assert_eq!(v["set"], "TST");
        assert_eq!(v["scryfall_id"], "sid");
        assert_eq!(v["released_at"], "2020-01-01");
        assert_eq!(v["keywords"][0], "Haste");
        assert_eq!(v["legalities"]["modern"], "legal");
        assert_eq!(v["tags"], serde_json::json!([]));
        // Price members render (null here; no price row passed).
        assert_eq!(v["price_usd"], serde_json::Value::Null);
        assert_eq!(v["price_usd_foil"], serde_json::Value::Null);
        assert_eq!(v["max_price_usd"], serde_json::Value::Null);
        assert_eq!(v["max_price_usd_foil"], serde_json::Value::Null);
    }

    #[test]
    fn card_json_handles_bad_json_columns() {
        let mut r = row();
        r.colors = "not json".into();
        r.legalities = "also not".into();
        let index = empty_index();
        let v = card_json(&r, &index, &Default::default(), &Default::default());
        assert_eq!(v["colors"], serde_json::Value::Null);
        assert_eq!(v["legalities"], serde_json::Value::Null);
    }
}

/// One tag-overlap neighbor: the card, its shared tag labels, and the count.
#[derive(Debug, Clone)]
pub struct SimilarHit {
    pub card: crate::db::CardRow,
    pub shared_count: i64,
    pub shared_tags: Vec<String>,
    /// Hybrid rank in `[0, 1]` (reciprocal-rank fusion of the tag leg and
    /// the stored-vector cosine leg); `None` when only the tag leg ran.
    pub score: Option<f32>,
}

/// Rank oracle cards by oracle-tag overlap with `seed`.
///
/// Counts shared tag ids via `card_tags` (join on oracle_id), excludes the
/// seed itself, breaks ties toward lower EDHREC rank then name — the same
/// tie-break order as query fusion.
///
/// When `restrict` is set, the SQL over-selects (4× the window) so the
/// Rust-side filter cannot empty the result before the true `limit` best
/// owned matches are reached; the final list is truncated here.
///
/// # Errors
/// Propagates SQLite failures.
pub fn rank_similar(
    conn: &rusqlite::Connection,
    seed_oracle_id: &str,
    limit: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> anyhow::Result<Vec<SimilarHit>> {
    // Over-select when a Rust-side filter will drop rows after the SQL cut.
    let sql_limit = if restrict.is_some() {
        limit.saturating_mul(4).max(50)
    } else {
        limit
    };
    let mut stmt = conn.prepare(
        "SELECT c.name, c.oracle_id, c.mana_cost, c.cmc, c.type_line, c.colors,
                c.color_identity, c.keywords, c.power, c.toughness, c.loyalty,
                c.oracle_text, c.rarity, c.edhrec_rank, c.legalities,
                c.set_code, c.collector_number, c.scryfall_id, c.released_at,
                c.game_changer,
                COUNT(ct2.tag_id) AS shared,
                GROUP_CONCAT(t2.label, '\u{1}') AS labels
         FROM card_tags ct
         JOIN card_tags ct2
           ON ct2.tag_id = ct.tag_id AND ct2.oracle_id != ct.oracle_id
         JOIN cards c ON c.oracle_id = ct2.oracle_id
         JOIN tags t2 ON t2.id = ct2.tag_id
         WHERE ct.oracle_id = ?1
         GROUP BY ct2.oracle_id
         ORDER BY shared DESC, c.edhrec_rank IS NULL, c.edhrec_rank ASC, c.name ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![seed_oracle_id, sql_limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
            row.get::<_, Option<i64>>(13)?,
            row.get::<_, String>(14)?,
            row.get::<_, String>(15)?,
            row.get::<_, String>(16)?,
            row.get::<_, String>(17)?,
            row.get::<_, String>(18)?,
            row.get::<_, Option<bool>>(19)?,
            row.get::<_, i64>(20)?,
            row.get::<_, String>(21)?,
        ))
    })?;
    let mut hits = Vec::new();
    for row in rows {
        let (
            name,
            oracle_id,
            mana_cost,
            cmc,
            type_line,
            colors,
            color_identity,
            keywords,
            power,
            toughness,
            loyalty,
            oracle_text,
            rarity,
            edhrec_rank,
            legalities,
            set_code,
            collector_number,
            scryfall_id,
            released_at,
            game_changer,
            shared,
            labels,
        ) = row.context("reading similar card")?;
        if restrict.is_some_and(|set| !set.contains(&name)) {
            continue;
        }
        let mut shared_tags: Vec<String> = labels.split('\u{1}').map(str::to_string).collect();
        shared_tags.sort_by_key(|a| a.to_lowercase());
        hits.push(SimilarHit {
            card: crate::db::CardRow {
                name,
                oracle_id,
                mana_cost,
                cmc,
                type_line,
                colors,
                color_identity,
                keywords,
                power,
                toughness,
                loyalty,
                oracle_text,
                rarity,
                edhrec_rank,
                legalities,
                set_code,
                collector_number,
                scryfall_id,
                released_at,
                game_changer,
            },
            shared_count: shared,
            shared_tags,
            score: None,
        });
    }
    hits.truncate(limit);
    Ok(hits)
}

/// The seed's vector store row, when the store and the row both exist.
/// Reading the stored row needs no model load: the seed was embedded at
/// sync time.
fn seed_vector(
    paths: &crate::paths::Paths,
    seed_name: &str,
) -> Option<(crate::embed::VectorStore, usize, Vec<f32>)> {
    let store = crate::embed::VectorStore::load(paths.root()).ok()?;
    let idx = store.meta.index_of(seed_name)?;
    let row = store.row(idx).to_vec();
    Some((store, idx, row))
}

/// Cosine ranks against the seed's stored vector: `(name, score)` pairs,
/// best first, excluding the seed row. Rows are unit-normalized, so dot
/// product is cosine.
fn vector_leg(
    store: &crate::embed::VectorStore,
    seed_row: usize,
    seed: &[f32],
    depth: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> Vec<(String, f64)> {
    let mut scored: Vec<(usize, f32)> = (0..store.meta.names.len())
        .filter(|i| *i != seed_row)
        .map(|i| {
            let row = store.row(i);
            let dot: f32 = seed.iter().zip(row).map(|(q, v)| q * v).sum();
            (i, dot)
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    scored
        .into_iter()
        .filter(|(i, _)| {
            let name = &store.meta.names[*i];
            restrict.is_none_or(|set| set.contains(name))
        })
        .take(depth)
        .map(|(i, score)| (store.meta.names[i].clone(), score as f64))
        .collect()
}

/// Entry point for `stm card similar <name>`.
///
/// Exit 3 when the seed card is unknown or has no tags, or when no neighbor
/// survives the filters. Exit 0 otherwise.
#[allow(clippy::too_many_arguments)]
pub fn run_similar(
    paths: &crate::paths::Paths,
    conn: &mut rusqlite::Connection,
    out: &mut crate::output::Output,
    typed: &str,
    limit: u32,
    owned_only: bool,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let seed = match crate::db::resolve_name(conn, typed).context("resolving card name")? {
        crate::db::NameMatch::Found(card) => card,
        crate::db::NameMatch::Ambiguous(candidates) => {
            out.error(&format!(
                "{typed:?} matches {} cards; be more specific",
                candidates.len()
            ));
            out.hint(&format!("did you mean: {}", candidates.join(", ")));
            return Ok(crate::cli::codes::NO_RESULTS);
        }
        crate::db::NameMatch::NotFound => {
            out.error(&format!("no card named {typed:?}"));
            out.hint("names resolve by exact match, case, or unique prefix");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
    };
    let restrict = if owned_only {
        let owned = crate::collection::owned_names_all(conn)?;
        if owned.is_empty() {
            out.error("collection is empty");
            out.hint("import a ManaBox CSV: stm collection import <file>");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
        Some(owned)
    } else {
        None
    };
    let tag_hits = rank_similar(
        conn,
        &seed.oracle_id,
        (limit as usize).saturating_mul(2).max(30),
        restrict.as_ref(),
    )?;
    // Fused hybrid when the seed has a stored vector: tag-overlap ranks and
    // cosine-to-seed ranks fuse by reciprocal rank fusion, like `query`.
    // No stored vector (unembedded card, absent store): tags only, with a
    // note on the human view.
    let (seed_store, seed_row, seed_vec) = match seed_vector(paths, &seed.name) {
        Some(tuple) => tuple,
        None => {
            let mut hits = tag_hits;
            hits.truncate(limit as usize);
            return finish_similar(conn, out, &seed.name, hits, false, json);
        }
    };
    let vector_hits = vector_leg(
        &seed_store,
        seed_row,
        &seed_vec,
        (limit as usize).saturating_mul(2).max(30),
        restrict.as_ref(),
    );
    let tag_list: Vec<(String, f64)> = tag_hits
        .iter()
        .map(|h| (h.card.name.clone(), h.shared_count as f64))
        .collect();
    // EDHREC rank breaks fusion ties; the map covers both legs' names.
    let mut ranks: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for hit in &tag_hits {
        if let Some(rank) = hit.card.edhrec_rank {
            ranks.insert(hit.card.name.as_str(), rank);
        }
    }
    let mut vector_names: Vec<&str> = vector_hits.iter().map(|(n, _)| n.as_str()).collect();
    vector_names.sort_unstable();
    vector_names.dedup();
    let vector_rows: Vec<crate::db::CardRow> = vector_names
        .iter()
        .filter_map(|name| crate::db::get_card(conn, name).ok().flatten())
        .collect();
    for card in &vector_rows {
        if let Some(rank) = card.edhrec_rank {
            ranks.entry(card.name.as_str()).or_insert(rank);
        }
    }
    let fused = crate::query::fuse_rrf(&tag_list, &vector_hits, limit as usize, |name| {
        ranks.get(name).copied()
    });
    // Re-attach cards + tags to the fused ranking. Vector-only names
    // (no shared tags) carry `shared_count: 0` / empty tags so the fused
    // window never shrinks below `--limit`.
    let mut hits: Vec<SimilarHit> = fused
        .into_iter()
        .filter_map(|(name, score)| {
            if let Some(hit) = tag_hits.iter().find(|h| h.card.name == name) {
                let mut hit = hit.clone();
                hit.score = Some(score);
                return Some(hit);
            }
            let card = vector_rows.iter().find(|c| c.name == name)?.clone();
            Some(SimilarHit {
                card,
                shared_count: 0,
                shared_tags: Vec::new(),
                score: Some(score),
            })
        })
        .collect();
    hits.truncate(limit as usize);
    finish_similar(conn, out, &seed.name, hits, true, json)
}

/// Print or emit the final similar list. `hybrid` says whether the vector
/// leg ran (drives the header note and the empty-set hint).
fn finish_similar(
    conn: &mut rusqlite::Connection,
    out: &mut crate::output::Output,
    seed_name: &str,
    hits: Vec<SimilarHit>,
    hybrid: bool,
    json: bool,
) -> anyhow::Result<i32> {
    if hits.is_empty() {
        if json {
            println!("[]");
        } else {
            out.error(&format!("no cards share tags with {seed_name}"));
            out.hint("the card may be untagged, or the filters emptied the result set");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let names: Vec<String> = hits.iter().map(|hit| hit.card.name.clone()).collect();
        // One batched query per finish kind instead of four per card name.
        let ranges = crate::prints::price_ranges(conn, &names).unwrap_or_default();
        let items: Vec<serde_json::Value> = hits
            .iter()
            .filter_map(|hit| {
                let range = ranges.get(&hit.card.name)?;
                let universe =
                    crate::universe::card_universe(conn, &hit.card.name, &hit.card.set_code)
                        .unwrap_or(crate::universe::CardUniverse {
                            universe: "multiverse",
                            franchise: None,
                            set_name: None,
                            set_type: None,
                            block: None,
                        });
                let mut v = card_json(&hit.card, &tag_index, range, &universe);
                // Null score when the seed had no stored vector (tags-only
                // ranking); agents can tell "no score" from "unranked".
                v["score"] = match hit.score {
                    Some(score) => {
                        serde_json::json!((f64::from(score) * 10_000.0).round() / 10_000.0)
                    }
                    None => serde_json::Value::Null,
                };
                v["shared_count"] = serde_json::json!(hit.shared_count);
                v["shared_tags"] = serde_json::json!(hit.shared_tags);
                Some(v)
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        print_similar_text(out, seed_name, &hits, hybrid);
    }
    Ok(crate::cli::codes::OK)
}

/// Human table for `card similar`: rank, name, cost, type, score, shared
/// tags. Owned/price context joins via `--owned` runs.
fn print_similar_text(
    out: &crate::output::Output,
    seed_name: &str,
    hits: &[SimilarHit],
    hybrid: bool,
) {
    let styles = out.styles();
    let why = if hybrid {
        "tag overlap + meaning"
    } else {
        "tag overlap"
    };
    println!(
        "{} {} {}",
        styles.header("Similar to"),
        styles.card_name(seed_name),
        styles.dim(&format!("({why})"))
    );
    for (i, hit) in hits.iter().enumerate() {
        let score_note = match hit.score {
            Some(score) => format!("({:.3})", score),
            None => format!(
                "({} common tag{})",
                hit.shared_count,
                if hit.shared_count == 1 { "" } else { "s" }
            ),
        };
        let tags = if hit.shared_tags.is_empty() {
            String::new()
        } else {
            format!(
                " shares: {}",
                hit.shared_tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!(
            "{:>2}. {} {} {} {}{}",
            i + 1,
            styles.card_name(&hit.card.name),
            styles.mana_pips(&hit.card.mana_cost),
            styles.dim(&hit.card.type_line),
            styles.dim(&score_note),
            styles.dim(&tags),
        );
    }
}

/// One Spellbook combo variant the card takes part in, pieces joined.
pub struct CardCombo {
    pub variant: crate::spellbook::ComboVariant,
    pub pieces: Vec<crate::spellbook::ComboPieceRow>,
    /// True when any piece must be the commander.
    pub requires_commander: bool,
}

/// Entry point for `stm card combos <name>`.
///
/// Exits 3 when the card is unknown or takes part in no combo (after
/// `--format` filtering). The list is sorted by Spellbook popularity, best
/// first.
#[allow(clippy::too_many_arguments)]
pub fn run_combos(
    paths: &crate::paths::Paths,
    conn: &mut rusqlite::Connection,
    out: &mut crate::output::Output,
    typed: &str,
    format: Option<&str>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let seed = match crate::db::resolve_name(conn, typed).context("resolving card name")? {
        crate::db::NameMatch::Found(card) => card,
        crate::db::NameMatch::Ambiguous(candidates) => {
            out.error(&format!(
                "{typed:?} matches {} cards; be more specific",
                candidates.len()
            ));
            out.hint(&format!("did you mean: {}", candidates.join(", ")));
            return Ok(crate::cli::codes::NO_RESULTS);
        }
        crate::db::NameMatch::NotFound => {
            out.error(&format!("no card named {typed:?}"));
            out.hint("names resolve by exact match, case, or unique prefix");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
    };
    let mut names = std::collections::HashSet::new();
    names.insert(seed.name.clone());
    let variants = crate::combos::load_variants_for(conn, &names)?;
    let combos = match format {
        Some(format) => crate::combos::filter_for_format(variants, format),
        None => variants,
    };
    if combos.is_empty() {
        if json {
            println!("[]");
        } else {
            match format {
                Some(format) => {
                    out.error(&format!(
                        "no combos with {} are legal in {format}",
                        seed.name
                    ));
                    out.hint("drop --format to see every combo the card appears in");
                }
                None => {
                    out.error(&format!("no combos include {}", seed.name));
                    out.hint("run 'stm sync' online first; the combo list needs a sync");
                }
            }
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let mut sorted = combos;
    sorted.sort_by(|a, b| {
        b.0.popularity
            .unwrap_or(0)
            .cmp(&a.0.popularity.unwrap_or(0))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    sorted.truncate(limit as usize);
    let rows: Vec<CardCombo> = sorted
        .into_iter()
        .map(|(variant, pieces)| CardCombo {
            requires_commander: crate::combos::requires_commander(&pieces),
            variant,
            pieces,
        })
        .collect();
    if json {
        print_combos_json(&rows)?;
    } else {
        print_combos_text(out, &seed.name, &rows);
    }
    Ok(crate::cli::codes::OK)
}

/// JSON rows for `card combos`: one object per variant.
fn print_combos_json(rows: &[CardCombo]) -> anyhow::Result<()> {
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|combo| {
            serde_json::json!({
                "id": combo.variant.id,
                "produces": combo.variant.produces,
                "mana_value_needed": combo.variant.mana_value_needed,
                "bracket_tag": combo.variant.bracket_tag,
                "popularity": combo.variant.popularity,
                "legalities": combo.variant.legalities,
                "requires_commander": combo.requires_commander,
                "pieces": combo.pieces.iter().map(|p| serde_json::json!({
                    "name": p.name,
                    "zones": p.zones,
                    "must_be_commander": p.must_be_commander,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Human table for `card combos`, popularity first.
fn print_combos_text(out: &crate::output::Output, seed_name: &str, rows: &[CardCombo]) {
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Combos with"),
        styles.card_name(seed_name)
    );
    for (i, combo) in rows.iter().enumerate() {
        let mut names: Vec<String> = combo.pieces.iter().map(|p| p.name.clone()).collect();
        names.sort();
        names.dedup();
        let pieces = names.join(" + ");
        let cmdr = if combo.requires_commander {
            " (commander)"
        } else {
            ""
        };
        let produces = combo.variant.produces.first().cloned().unwrap_or_default();
        let bracket = combo
            .variant
            .bracket_tag
            .as_deref()
            .map(|t| format!(" [{t}]"))
            .unwrap_or_default();
        let pop = match combo.variant.popularity {
            Some(n) => format!(" pop {}", styles.thousands(n)),
            None => String::new(),
        };
        let legal: Vec<&str> = COMBO_NOTE_FORMATS
            .iter()
            .copied()
            .filter(|f| combo.variant.legalities.get(*f).copied().unwrap_or(false))
            .collect();
        let legal_note = if legal.is_empty() {
            String::new()
        } else {
            format!("  legal: {}", legal.join(", "))
        };
        println!(
            "{:>2}. {}{} → {}{}{}{}",
            i + 1,
            styles.card_name(&pieces),
            styles.dim(cmdr),
            styles.dim(&produces),
            styles.dim(&bracket),
            styles.dim(&pop),
            styles.dim(&legal_note),
        );
    }
}

/// Formats shown in the human `legal:` note, most-played first.
const COMBO_NOTE_FORMATS: &[&str] = &[
    "standard",
    "pioneer",
    "modern",
    "legacy",
    "vintage",
    "pauper",
    "commander",
    "brawl",
    "oathbreaker",
    "premodern",
    "alchemy",
    "predh",
];
