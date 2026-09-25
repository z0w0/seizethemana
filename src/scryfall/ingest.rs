// Database ingest for Scryfall bulk rows: card-row insert/overwrite and
// the per-printing price upsert. Split from `scryfall.rs` to keep each
// file small; the sync pass calls these inside its own transaction.

use super::ScryfallCard;

/// Insert a parsed card row into the `cards` table.
///
/// Uses `INSERT OR IGNORE` so re-runs over the same bulk are idempotent.
///
/// # Errors
/// Propagates SQLite failures with the card name for context.
pub fn insert_card(conn: &rusqlite::Connection, card: &ScryfallCard) -> anyhow::Result<()> {
    let legalities = serde_json::to_string(&card.legalities.clone().unwrap_or_default())?;
    let colors = serde_json::to_string(&card.colors.clone().unwrap_or_default())?;
    let identity = serde_json::to_string(&card.color_identity.clone().unwrap_or_default())?;
    let keywords = serde_json::to_string(&card.keywords.clone().unwrap_or_default())?;
    conn.execute(
        "INSERT OR IGNORE INTO cards (
            name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
            legalities, set_code, collector_number, scryfall_id, released_at,
            game_changer
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        rusqlite::params![
            card.name,
            card.oracle_id,
            card.mana_cost.clone().unwrap_or_default(),
            card.cmc.unwrap_or(0.0),
            card.type_line.clone().unwrap_or_default(),
            colors,
            identity,
            keywords,
            card.power,
            card.toughness,
            card.loyalty,
            card.oracle_text.clone().unwrap_or_default(),
            card.rarity.clone().unwrap_or_default(),
            card.edhrec_rank,
            legalities,
            card.set_code.clone().unwrap_or_default(),
            card.collector_number.clone().unwrap_or_default(),
            card.id.clone().unwrap_or_default(),
            card.released_at.clone().unwrap_or_default(),
            card.game_changer,
        ],
    )?;
    Ok(())
}

/// Overwrite the ingestable fields of an existing card row.
///
/// The name key stays fixed so collection rows and vectors keep their
/// references; print identity and prices live in `card_prints` (refreshed
/// separately by the sync pass).
///
/// # Errors
/// Propagates SQLite failures; fails if `name` is not stored.
pub fn update_card(
    conn: &rusqlite::Connection,
    name: &str,
    card: &ScryfallCard,
) -> anyhow::Result<()> {
    let legalities = serde_json::to_string(&card.legalities.clone().unwrap_or_default())?;
    let colors = serde_json::to_string(&card.colors.clone().unwrap_or_default())?;
    let identity = serde_json::to_string(&card.color_identity.clone().unwrap_or_default())?;
    let keywords = serde_json::to_string(&card.keywords.clone().unwrap_or_default())?;
    let updated = conn.execute(
        "UPDATE cards SET
            oracle_id = ?2, mana_cost = ?3, cmc = ?4, type_line = ?5,
            colors = ?6, color_identity = ?7, keywords = ?8, power = ?9,
            toughness = ?10, loyalty = ?11, oracle_text = ?12, rarity = ?13,
            edhrec_rank = ?14, legalities = ?15, set_code = ?16,
            collector_number = ?17, scryfall_id = ?18, released_at = ?19,
            game_changer = ?20
         WHERE name = ?1",
        rusqlite::params![
            name,
            card.oracle_id,
            card.mana_cost.clone().unwrap_or_default(),
            card.cmc.unwrap_or(0.0),
            card.type_line.clone().unwrap_or_default(),
            colors,
            identity,
            keywords,
            card.power,
            card.toughness,
            card.loyalty,
            card.oracle_text.clone().unwrap_or_default(),
            card.rarity.clone().unwrap_or_default(),
            card.edhrec_rank,
            legalities,
            card.set_code.clone().unwrap_or_default(),
            card.collector_number.clone().unwrap_or_default(),
            card.id.clone().unwrap_or_default(),
            card.released_at.clone().unwrap_or_default(),
            card.game_changer,
        ],
    )?;
    anyhow::ensure!(updated == 1, "card {name:?} vanished during update");
    Ok(())
}

/// Upsert one physical printing into `card_prints` (and its set name into
/// `sets`).
///
/// Called for every passing print row of the bulk, not just the per-name
/// winner, so per-printing prices stay complete. Prices overwrite in bulk
/// order; prints missing from a fresh bulk keep their last known row.
/// Prints with no `id` are skipped: upserting them would key every row to
/// `scryfall_id = ''` and collide.
///
/// # Errors
/// Propagates SQLite failures.
pub fn upsert_print(
    conn: &rusqlite::Connection,
    card: &ScryfallCard,
    updated_at: &str,
) -> anyhow::Result<()> {
    let Some(set_code) = card.set_code.as_deref().map(str::to_ascii_lowercase) else {
        return Ok(());
    };
    if set_code.is_empty() || card.id.as_deref().is_none_or(str::is_empty) {
        return Ok(());
    }
    let finishes = serde_json::to_string(&card.finishes.clone().unwrap_or_default())?;
    if let Some(set_name) = &card.set_name {
        conn.execute(
            "INSERT INTO sets (set_code, set_name, set_type, block, franchise)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (set_code) DO UPDATE SET
                set_name = excluded.set_name,
                set_type = excluded.set_type,
                block = excluded.block,
                franchise = excluded.franchise",
            rusqlite::params![
                set_code,
                set_name,
                card.set_type.clone().unwrap_or_default(),
                card.block,
                crate::universe::franchise_for(&set_code, set_name),
            ],
        )?;
    }
    let ub_flag = crate::universe::is_universes_beyond(
        &set_code,
        card.promo_types
            .as_ref()
            .is_some_and(|p| p.iter().any(|t| t == "universesbeyond")),
    ) as i64;
    let (usd, usd_foil, usd_etched) = match &card.prices {
        Some(prices) => (
            prices.usd.as_deref().and_then(|s| s.parse::<f64>().ok()),
            prices
                .usd_foil
                .as_deref()
                .and_then(|s| s.parse::<f64>().ok()),
            prices
                .usd_etched
                .as_deref()
                .and_then(|s| s.parse::<f64>().ok()),
        ),
        None => (None, None, None),
    };
    conn.execute(
        "INSERT INTO card_prints (
            scryfall_id, name, flavor_name, set_code, collector_number, lang,
            rarity, finishes, released_at, usd, usd_foil, usd_etched, updated_at,
            universes_beyond
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT (scryfall_id) DO UPDATE SET
            name = excluded.name, flavor_name = excluded.flavor_name,
            set_code = excluded.set_code,
            collector_number = excluded.collector_number, lang = excluded.lang,
            rarity = excluded.rarity, finishes = excluded.finishes,
            released_at = excluded.released_at, usd = excluded.usd,
            usd_foil = excluded.usd_foil, usd_etched = excluded.usd_etched,
            updated_at = excluded.updated_at,
            universes_beyond = excluded.universes_beyond",
        rusqlite::params![
            card.id.clone().unwrap_or_default(),
            card.name,
            card.flavor_name.clone().unwrap_or_default(),
            set_code,
            card.collector_number.clone().unwrap_or_default(),
            card.lang.clone().unwrap_or_else(|| "en".to_string()),
            card.rarity.clone().unwrap_or_default(),
            finishes,
            card.released_at.clone().unwrap_or_default(),
            usd,
            usd_foil,
            usd_etched,
            updated_at,
            ub_flag,
        ],
    )?;
    Ok(())
}
