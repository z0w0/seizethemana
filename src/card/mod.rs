use anyhow::Context;

// `stm card`: full detail for one card by (fuzzy) name, plus tag-overlap
// neighbors (`stm card similar`) and Spellbook combos (`stm card combos`).

pub mod combos;
pub mod similar;

/// Report an ambiguous name: sample candidates plus the real match count.
pub(crate) fn ambiguous_error(
    out: &mut crate::output::Output,
    typed: &str,
    candidates: &[String],
    total: usize,
) -> anyhow::Result<i32> {
    out.error(&format!(
        "{typed:?} matches {total} cards; be more specific"
    ));
    let listed: Vec<&str> = candidates.iter().take(12).map(String::as_str).collect();
    let mut hint = format!("did you mean: {}", listed.join(", "));
    let shown = listed.len();
    if total > shown {
        hint.push_str(&format!(" (+{} more)", total - shown));
    }
    out.hint(&hint);
    Ok(crate::cli::codes::NO_RESULTS)
}

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
        crate::db::NameMatch::Ambiguous { candidates, total } => {
            return ambiguous_error(out, typed, &candidates, total);
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
            .context("reading card price range")?;
        let range = ranges.get(&card.name).cloned().unwrap_or_default();
        let universe = crate::universe::card_universe(conn, &card.name, &card.set_code)?;
        let owned_all = crate::collection::owned_counts_all(conn)?;
        let available_all = crate::collection::available_counts_all(conn)?;
        print_json(
            &card,
            &tag_index,
            &range,
            &universe,
            &owned_all,
            &available_all,
        )?;
    } else {
        let range =
            crate::prints::price_range(conn, &card.name).context("reading card price range")?;
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let universe = crate::universe::card_universe(conn, &card.name, &card.set_code)?;
        let owned_all = crate::collection::owned_counts_all(conn)?;
        let available_all = crate::collection::available_counts_all(conn)?;
        print_text(
            out,
            &card,
            &range,
            &tag_index,
            &universe,
            &owned_all,
            &available_all,
        );
    }
    Ok(crate::cli::codes::OK)
}

pub(super) fn print_json(
    card: &crate::db::CardRow,
    tag_index: &crate::tags::TagIndex,
    range: &crate::prints::PrintRange,
    universe: &crate::universe::CardUniverse,
    owned_all: &std::collections::HashMap<String, i64>,
    available_all: &std::collections::HashMap<String, i64>,
) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(&card_json(
        card,
        tag_index,
        range,
        universe,
        owned_all,
        available_all,
    ))?;
    println!("{text}");
    Ok(())
}

/// Full card detail as a JSON value (also used by collection JSON output).
///
/// Price fields are per-printing and in US dollars: `price`/`price_foil`
/// carry the cheapest released English printing, `max_price`/`max_price_foil`
/// the most expensive. All four are null when no print is priced.
///
/// `owned` counts every copy across binders and deck assignments;
/// `available` counts binder copies only. Basics are unlimited-supply,
/// so both are null for them.
///
/// `owned_all` maps name to total copies; `available_all` maps name to
/// binder-only copies.
pub fn card_json(
    card: &crate::db::CardRow,
    tag_index: &crate::tags::TagIndex,
    range: &crate::prints::PrintRange,
    universe: &crate::universe::CardUniverse,
    owned_all: &std::collections::HashMap<String, i64>,
    available_all: &std::collections::HashMap<String, i64>,
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
    let basic = crate::collection::is_basic_name(&card.name);
    let owned = if basic {
        serde_json::Value::Null
    } else {
        serde_json::json!(owned_all.get(&card.name).copied().unwrap_or(0))
    };
    let available = if basic {
        serde_json::Value::Null
    } else {
        serde_json::json!(available_all.get(&card.name).copied().unwrap_or(0))
    };
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
        "price": usd,
        "price_foil": usd_foil,
        "max_price": max_usd,
        "max_price_foil": max_usd_foil,
        "owned": owned,
        "available": available,
    })
}

/// Render a framed magic-card-style view on stdout.
///
/// Box width follows the terminal (clamped to 40..=100); falls back to the
/// same frame at width 80 when the terminal size is unknown. The plain (no
/// color) path keeps the frame — it still reads as a card.
fn print_text(
    out: &crate::output::Output,
    card: &crate::db::CardRow,
    range: &crate::prints::PrintRange,
    tag_index: &crate::tags::TagIndex,
    universe: &crate::universe::CardUniverse,
    owned_all: &std::collections::HashMap<String, i64>,
    available_all: &std::collections::HashMap<String, i64>,
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
    // Header: name left, mana cost right — the real card's top row.
    let name = styles.card_name(&card.name);
    let cost = styles.mana_pips(&card.mana_cost);
    let gap = inner.saturating_sub(measure(&name) + measure(&cost));
    println!("│ {name}{}{cost} │", " ".repeat(gap));
    println!("{}", blank);
    // Type + rarity, styled like the printed frame: the rarity reads as
    // its expansion symbol color.
    let rarity_glyph = match card.rarity.as_str() {
        "mythic" => "◆",
        "rare" => "◆",
        "uncommon" => "◆",
        "common" => "◆",
        _ => "",
    };
    println!(
        "{}",
        line(format!(
            "{}  {}{}",
            styles.dim(&card.type_line),
            styles.rarity(&card.rarity),
            styles.rarity(rarity_glyph)
        ))
    );
    // Stat box: P/T bottom-right on a real creature card; loyalty for a
    // planeswalker. Rendered right-aligned like the printed layout.
    if let (Some(p), Some(t)) = (&card.power, &card.toughness) {
        let pt = styles.dim(&format!("{} / {}", p, t));
        let pad = inner.saturating_sub(measure(&pt));
        println!("│ {}{pt} │", " ".repeat(pad));
    } else if let Some(loyalty) = &card.loyalty {
        let pt = styles.rarity(&format!("+{loyalty}"));
        let pad = inner.saturating_sub(measure(&pt));
        println!("│ {}{pt} │", " ".repeat(pad));
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
    // Owned copies: total across binders and decks, with the binder-only
    // count called out so deck-locked copies are visible. Basics are
    // unlimited-supply, so they never get this line.
    if !crate::collection::is_basic_name(&card.name) {
        let owned = owned_all.get(&card.name).copied().unwrap_or(0);
        if owned > 0 {
            let available = available_all.get(&card.name).copied().unwrap_or(0);
            let bit = if available == owned {
                format!("{owned} owned")
            } else {
                format!("{owned} owned ({available} in binders)")
            };
            println!("{}", line(format!("{} {}", styles.dim("Owned:"), bit)));
        }
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
                let rest = text_line.strip_prefix("Legal in: ").unwrap_or(&text_line);
                format!("{} {}", styles.dim("Legal in:"), styles.dim(rest))
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
        let v = card_json(
            &row(),
            &index,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(v["name"], "Test Card");
        assert_eq!(v["set"], "TST");
        assert_eq!(v["scryfall_id"], "sid");
        assert_eq!(v["released_at"], "2020-01-01");
        assert_eq!(v["keywords"][0], "Haste");
        assert_eq!(v["legalities"]["modern"], "legal");
        assert_eq!(v["tags"], serde_json::json!([]));
        // Price members render (null here; no price row passed).
        assert_eq!(v["price"], serde_json::Value::Null);
        assert_eq!(v["price_foil"], serde_json::Value::Null);
        assert_eq!(v["max_price"], serde_json::Value::Null);
        assert_eq!(v["max_price_foil"], serde_json::Value::Null);
        // Owned/available default to 0 for an unowned card.
        assert_eq!(v["owned"], 0);
        assert_eq!(v["available"], 0);
    }

    #[test]
    fn card_json_handles_bad_json_columns() {
        let mut r = row();
        r.colors = "not json".into();
        r.legalities = "also not".into();
        let index = empty_index();
        let v = card_json(
            &r,
            &index,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(v["colors"], serde_json::Value::Null);
        assert_eq!(v["legalities"], serde_json::Value::Null);
    }

    #[test]
    fn card_json_basics_have_null_owned_counts() {
        let mut r = row();
        r.name = "Plains".into();
        let index = empty_index();
        let mut owned = std::collections::HashMap::new();
        owned.insert("Plains".to_string(), 4_i64);
        let v = card_json(
            &r,
            &index,
            &Default::default(),
            &Default::default(),
            &owned,
            &Default::default(),
        );
        // Basics are unlimited-supply: owned/available stay null even when
        // collection rows exist.
        assert_eq!(v["owned"], serde_json::Value::Null);
        assert_eq!(v["available"], serde_json::Value::Null);
    }

    #[test]
    fn card_json_reports_owned_and_available() {
        let index = empty_index();
        let mut owned = std::collections::HashMap::new();
        owned.insert("Test Card".to_string(), 3_i64);
        let mut available = std::collections::HashMap::new();
        available.insert("Test Card".to_string(), 2_i64);
        let v = card_json(
            &row(),
            &index,
            &Default::default(),
            &Default::default(),
            &owned,
            &available,
        );
        assert_eq!(v["owned"], 3);
        assert_eq!(v["available"], 2);
    }
}
