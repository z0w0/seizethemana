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
        let range = crate::prints::price_range(conn, &card.name)
            .ok()
            .unwrap_or_default();
        print_json(&card, &tag_index, &range)?;
    } else {
        let range = crate::prints::price_range(conn, &card.name)
            .ok()
            .unwrap_or_default();
        let tag_index = crate::tags::TagIndex::load(conn)?;
        print_text(out, &card, &range, &tag_index);
    }
    Ok(crate::cli::codes::OK)
}

fn print_json(
    card: &crate::db::CardRow,
    tag_index: &crate::tags::TagIndex,
    range: &crate::prints::PrintRange,
) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(&card_json(card, tag_index, range))?;
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
    println!(
        "{}",
        line(format!(
            "{} {} #{} · {}",
            styles.dim("Set:"),
            styles.dim(&card.set_code),
            styles.dim(&card.collector_number),
            styles.dim(&crate::release::release_display(&card.released_at)),
        ))
    );
    if let Some(rank) = card.edhrec_rank {
        println!(
            "{}",
            line(format!(
                "{} {}",
                styles.dim("EDHREC rank:"),
                styles.dim(&rank.to_string())
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
        let v = card_json(&row(), &index, &Default::default());
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
        let v = card_json(&r, &index, &Default::default());
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
        });
    }
    hits.truncate(limit);
    Ok(hits)
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
    let hits = rank_similar(conn, &seed.oracle_id, limit as usize, restrict.as_ref())?;
    if hits.is_empty() {
        out.error(&format!("no cards share tags with {}", seed.name));
        out.hint("the card may be untagged, or the filters emptied the result set");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let names: Vec<String> = hits.iter().map(|hit| hit.card.name.clone()).collect();
        let ranges = names
            .iter()
            .map(|n| crate::prints::price_range(conn, n).unwrap_or_default())
            .collect::<Vec<_>>();
        let items: Vec<serde_json::Value> = hits
            .iter()
            .zip(&ranges)
            .map(|(hit, range)| {
                let mut v = card_json(&hit.card, &tag_index, range);
                v["shared_count"] = serde_json::json!(hit.shared_count);
                v["shared_tags"] = serde_json::json!(hit.shared_tags);
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        let styles = out.styles();
        println!(
            "{} {}",
            styles.dim("Similar to"),
            styles.card_name(&seed.name)
        );
        for (i, hit) in hits.iter().enumerate() {
            println!(
                "{:>2}. {} {} {} {} {}",
                i + 1,
                styles.card_name(&hit.card.name),
                styles.mana_pips(&hit.card.mana_cost),
                styles.dim(&hit.card.type_line),
                styles.dim(&format!(
                    "({} common tag{})",
                    hit.shared_count,
                    if hit.shared_count == 1 { "" } else { "s" }
                )),
                styles.dim(&format!(
                    "e.g. {}",
                    hit.shared_tags
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
    }
    Ok(crate::cli::codes::OK)
}
