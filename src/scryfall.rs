use anyhow::Context;
use serde::Deserialize;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};

// Scryfall bulk-data download plus streaming ingest into the `cards` table.
//
// The oracle bulk is JSON Lines (one card object per line) inside gzip; we
// parse line-by-line so peak memory stays small regardless of file size.

/// `GET /bulk-data` response items, subset of fields we use.
#[derive(Debug, Deserialize, Clone)]
struct BulkEntry {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "jsonl_download_uri")]
    uri: String,
    #[serde(default)]
    compressed_size: Option<u64>,
    #[serde(default)]
    updated_at: Option<String>,
}

/// `GET /bulk-data` envelope.
#[derive(Debug, Deserialize, Clone)]
struct BulkIndex {
    data: Vec<BulkEntry>,
}

/// One bulk file's coordinates: where to download it and when Scryfall last
/// rewrote it.
#[derive(Debug, Clone, PartialEq)]
pub struct BulkFile {
    pub uri: String,
    pub compressed_size: Option<u64>,
    /// Scryfall's own timestamp for the file's content (RFC 3339), when the
    /// index provided one.
    pub updated_at: Option<String>,
}

/// One Scryfall card object. Only the fields this CLI stores are declared;
/// serde ignores the rest, which keeps parsing fast and forward-compatible.
#[derive(Debug, Deserialize, Clone)]
pub struct ScryfallCard {
    pub name: String,
    /// Print ID of this exact printing.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub oracle_id: String,
    #[serde(default)]
    pub mana_cost: Option<String>,
    #[serde(default)]
    pub cmc: Option<f64>,
    #[serde(default)]
    pub type_line: Option<String>,
    #[serde(default)]
    pub colors: Option<Vec<String>>,
    #[serde(default)]
    pub color_identity: Option<Vec<String>>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub power: Option<String>,
    #[serde(default)]
    pub toughness: Option<String>,
    #[serde(default)]
    pub loyalty: Option<String>,
    #[serde(default)]
    pub oracle_text: Option<String>,
    #[serde(default)]
    pub rarity: Option<String>,
    #[serde(default)]
    pub edhrec_rank: Option<i64>,
    /// Commander Game Changer list flag (bracket signal).
    #[serde(default)]
    pub game_changer: Option<bool>,
    #[serde(default)]
    pub legalities: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(rename = "set", default)]
    pub set_code: Option<String>,
    #[serde(default)]
    pub set_name: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    /// Just-for-fun printed name (Godzilla series, Secret Lair crossovers).
    /// A print-level alias for the oracle name; indexed for name resolution.
    #[serde(default)]
    pub flavor_name: Option<String>,
    #[serde(default)]
    pub collector_number: Option<String>,
    #[serde(default)]
    pub finishes: Option<Vec<String>>,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub prices: Option<Prices>,
    #[serde(default)]
    pub games: Option<Vec<String>>,
    pub layout: String,
    /// Set type of the print's set ("expansion", "commander", …).
    #[serde(rename = "set_type", default)]
    pub set_type: Option<String>,
    /// In-universe block name of the print's set (NULL for most modern sets).
    #[serde(default)]
    pub block: Option<String>,
    /// Just-for-fun promo marks of this print; "universesbeyond" flags a UB
    /// print.
    #[serde(default)]
    pub promo_types: Option<Vec<String>>,
    #[serde(default)]
    pub card_faces: Option<Vec<CardFace>>,
}

/// Price sub-object (USD strings).
#[derive(Debug, Deserialize, Clone)]
pub struct Prices {
    pub usd: Option<String>,
    pub usd_foil: Option<String>,
    pub usd_etched: Option<String>,
}

/// Card face sub-object; only needed for multi-faced cards.
#[derive(Debug, Deserialize, Clone)]
pub struct CardFace {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mana_cost: Option<String>,
    #[serde(default)]
    pub type_line: Option<String>,
    #[serde(default)]
    pub oracle_text: Option<String>,
    #[serde(default)]
    pub power: Option<String>,
    #[serde(default)]
    pub toughness: Option<String>,
    #[serde(default)]
    pub colors: Option<Vec<String>>,
    #[serde(default)]
    pub loyalty: Option<String>,
}

/// Layouts that are not real playable cards (tokens, art, un-cards extras…).
const NON_CARD_LAYOUTS: &[&str] = &[
    "art_series",
    "token",
    "double_faced_token",
    "emblem",
    "planar",
    "scheme",
    "vanguard",
    "front_card",
    "host",
    "augment",
];

/// Scryfall API host all downloads and lookups go through.
pub(crate) const SCRYFALL_HOST: &str = "api.scryfall.com";

/// Shared HTTP client for downloads and API calls.
pub(crate) fn http_client() -> anyhow::Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("seizethemana/0.1 (CLI card search tool)")
        .timeout(std::time::Duration::from_secs(300))
        .build()?)
}

/// Fetch the `GET /bulk-data` index and return the default-cards and
/// oracle-tags entries (one API call serves both).
///
/// # Errors
/// Fails on network/HTTP problems or if either entry is absent.
pub fn fetch_bulk_files() -> anyhow::Result<(BulkFile, BulkFile)> {
    let client = http_client()?;
    let index: BulkIndex = client
        .get(format!("https://{SCRYFALL_HOST}/bulk-data"))
        .send()?
        .error_for_status()?
        .json()?;
    let find = |kind: &str| -> anyhow::Result<BulkFile> {
        index
            .data
            .iter()
            .find(|e| e.kind == kind)
            .map(|e| BulkFile {
                uri: e.uri.clone(),
                compressed_size: e.compressed_size,
                updated_at: e.updated_at.clone(),
            })
            .ok_or_else(|| anyhow::anyhow!("Scryfall bulk index has no {kind} entry"))
    };
    Ok((find("default_cards")?, find("oracle_tags")?))
}

/// Refresh window for bulk-file staleness checks. Chrono cannot const-check
/// `to_std()`, so conversion sites expect success: the value is a positive
/// compile-time constant, and `to_std()` only fails on non-positive or
/// out-of-range values.
const STALE_AFTER_EXPECT: &str = "STALE_AFTER is a positive constant duration";

pub(crate) fn stale_after_std() -> std::time::Duration {
    crate::sync::STALE_AFTER.to_std().expect(STALE_AFTER_EXPECT)
}

/// Make sure `dest` holds a fresh copy of `file`'s content.
///
/// Downloads when the file is missing or older than [`crate::sync::STALE_AFTER`]
/// (mtime-based), or when Scryfall's `updated_at` timestamp for the file is
/// newer than the local mtime. Returns true when a download happened.
pub fn ensure_fresh_bulk(
    file: &BulkFile,
    dest: &std::path::Path,
    out: &mut crate::output::Output,
) -> anyhow::Result<bool> {
    let stale = |path: &std::path::Path| -> bool {
        let Ok(meta) = std::fs::metadata(path) else {
            return true;
        };
        meta.modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_none_or(|age| age > stale_after_std())
    };
    let scryfall_newer = match (&file.updated_at, std::fs::metadata(dest).ok()) {
        (Some(remote), Some(meta)) => meta
            .modified()
            .ok()
            .map(|local| match chrono::DateTime::parse_from_rfc3339(remote) {
                Ok(remote_time) => {
                    let local: chrono::DateTime<chrono::Local> = local.into();
                    remote_time.with_timezone(&chrono::Local) > local
                }
                Err(err) => {
                    // The comparison is skipped, not failed: a stale-local
                    // decision then rests on mtime alone. Make the swallow
                    // visible so a Scryfall format change is not silent.
                    out.warning(&format!("unparseable Scryfall timestamp {remote:?}: {err}"));
                    false
                }
            })
            .unwrap_or(false),
        _ => false,
    };
    if dest.exists() && !stale(dest) && !scryfall_newer {
        out.status("Reusing", &format!("bulk data at {}", dest.display()));
        return Ok(false);
    }
    let size_note = match file.compressed_size {
        Some(bytes) => format!("~{}MB", bytes / 1_000_000),
        None => "unknown size".to_string(),
    };
    out.status("Downloading", &size_note);
    download_to(&file.uri, dest, file.compressed_size, out)?;
    Ok(true)
}

/// Download `uri` to `dest`, streaming to disk.
///
/// Writes go to a temp file beside `dest` and rename onto it only on
/// success, so an interrupted download cannot leave a truncated file with a
/// fresh mtime. On a TTY the progress is an ephemeral cargo-style bar on
/// stderr; piped stderr gets periodic plain lines instead. Returns the
/// bytes written.
pub fn download_to(
    uri: &str,
    dest: &std::path::Path,
    compressed_size: Option<u64>,
    out: &mut crate::output::Output,
) -> anyhow::Result<u64> {
    let client = http_client()?;
    let mut response = client.get(uri).send()?.error_for_status()?;
    let total = response.content_length().or(compressed_size);
    let tmp = dest.with_extension(format!(
        "{}.tmp",
        dest.extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
    ));
    let result = (|| -> anyhow::Result<u64> {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?;

        let piped = !std::io::stderr().is_terminal();
        let mut written: u64 = 0;
        let mut last_logged = 0u64;
        if !piped {
            // TTY: the spinner becomes a real bar once the length is known.
            // Piped stderr keeps the log readable with periodic status
            // lines in the read loop instead of an ephemeral bar.
            out.progress_bar("Downloading", "bulk data", total.unwrap_or(0));
            if let Some(total) = total {
                out.set_progress_total(total);
            }
        }
        let mut chunk = [0u8; 64 * 1024];
        loop {
            let n = response.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            file.write_all(&chunk[..n])
                .with_context(|| format!("failed writing {}", tmp.display()))?;
            written += n as u64;
            if piped {
                // Log roughly every 10% or 5MB, whichever is coarser.
                let step = total.map(|t| t / 10).unwrap_or(5 * 1024 * 1024).max(1);
                if written - last_logged >= step {
                    last_logged = written;
                    match total {
                        Some(total) => {
                            out.status("Downloading", &format!("{written}/{total} bytes"));
                        }
                        None => out.status("Downloaded", &format!("{written} bytes")),
                    }
                }
            } else {
                out.set_progress_position(written);
            }
        }
        out.clear_progress();
        Ok(written)
    })();
    match result {
        Ok(written) => {
            std::fs::rename(&tmp, dest)
                .with_context(|| format!("renaming download onto {}", dest.display()))?;
            Ok(written)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Open the gzipped bulk file and stream each JSONL record through `visit`.
///
/// Malformed lines are skipped (a single bad record must not abort a
/// multi-hundred-MB download); the visit callback receives the parsed
/// object. Returns the malformed-line count so callers report it through
/// their output layer (respecting `--json`/`NO_COLOR`, unlike a raw
/// stderr write).
///
/// # Errors
/// Fails when the file cannot be opened/decompressed.
pub fn stream_records<F: FnMut(ScryfallCard)>(
    path: &std::path::Path,
    mut visit: F,
) -> anyhow::Result<usize> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("cannot open bulk file {}", path.display()))?;
    let decoder = flate2::read::MultiGzDecoder::new(BufReader::with_capacity(1 << 20, file));
    let mut reader = BufReader::with_capacity(1 << 20, decoder);
    let mut line = String::new();
    let mut bad = 0usize;
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        match serde_json::from_str::<ScryfallCard>(line.trim_end()) {
            Ok(card) => visit(card),
            Err(_) => bad += 1,
        }
    }
    Ok(bad)
}

/// True when the layout marks a non-playable card (token, art series,
/// emblem…). These names are recorded so collection imports can skip them.
pub fn is_non_card_layout(layout: &str) -> bool {
    NON_CARD_LAYOUTS.contains(&layout)
}

/// True when this bulk row should not become a searchable card.
///
/// Skips non-card layouts (tokens, art series…) and prints with no paper
/// release (arena/mtgo only). A missing `games` field is treated the same
/// as digital-only: Scryfall always populates it on real cards, so an
/// absent field is malformed input, not a paper print.
pub fn should_skip(card: &ScryfallCard) -> bool {
    if is_non_card_layout(&card.layout) {
        return true;
    }
    match &card.games {
        Some(games) => !games.iter().any(|g| g == "paper"),
        None => true,
    }
}

/// True when this oracle card should become a searchable card.
///
/// Ingests cards released by today, plus upcoming reprints of cards that are
/// already legal somewhere (their oracle has been printed before). Never-
/// released cards have every format set to `not_legal` and stay out of the
/// store until their set releases; the daily sync adds them then.
pub fn should_ingest(card: &ScryfallCard) -> bool {
    if crate::release::is_date_unreleased(card.released_at.as_deref().unwrap_or("")) {
        let any_format = card
            .legalities
            .as_ref()
            .is_some_and(|l| l.values().any(|v| v != "not_legal"));
        return any_format;
    }
    true
}

/// Flatten a possibly multi-faced card into single stored fields.
///
/// For multi-faced cards, face data is joined with ` // ` (mana costs) or
/// `\n// ` (text), mirroring Scryfall's combined rendering. The top-level
/// `mana_cost` is kept when the bulk provides one (transform cards do);
/// only an absent top-level cost is built from the faces.
pub fn flatten_faces(card: &mut ScryfallCard) {
    let Some(faces) = &card.card_faces else {
        return;
    };
    if faces.len() < 2 {
        return;
    }
    if card.mana_cost.is_none() {
        card.mana_cost = Some(
            faces
                .iter()
                .map(|f| f.mana_cost.clone().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" // "),
        );
    }
    card.type_line = Some(
        faces
            .iter()
            .map(|f| f.type_line.clone().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(" // "),
    );
    card.oracle_text = Some(
        faces
            .iter()
            .map(|f| f.oracle_text.clone().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n// "),
    );
    // Colors: union across faces, first face order.
    let mut colors: Vec<String> = Vec::new();
    for face in faces {
        for c in face.colors.iter().flatten() {
            if !colors.contains(c) {
                colors.push(c.clone());
            }
        }
    }
    if !colors.is_empty() {
        card.colors = Some(colors);
    }
    // Power/toughness/loyalty: take the first face that defines them.
    for field_power in faces.iter().map(|f| f.power.clone()) {
        if field_power.is_some() {
            card.power = field_power;
            break;
        }
    }
    for field_tough in faces.iter().map(|f| f.toughness.clone()) {
        if field_tough.is_some() {
            card.toughness = field_tough;
            break;
        }
    }
    for field_loyalty in faces.iter().map(|f| f.loyalty.clone()) {
        if field_loyalty.is_some() {
            card.loyalty = field_loyalty;
            break;
        }
    }
}

/// Choose one row per duplicate card name: prefer the print that is legal in
/// the most game platforms, then the lower EDHREC rank (more popular).
/// Full ties break on the earlier release date, then the Scryfall id, so
/// the pick does not depend on bulk row order.
pub fn prefer_row(current: &ScryfallCard, candidate: &ScryfallCard) -> ScryfallCard {
    fn weight(c: &ScryfallCard) -> (usize, i64) {
        let games = c.games.as_ref().map(|g| g.len()).unwrap_or(0);
        let rank = c.edhrec_rank.unwrap_or(i64::MAX);
        (games, -rank)
    }
    fn tiebreak(c: &ScryfallCard) -> (&str, &str) {
        (
            c.released_at.as_deref().unwrap_or(""),
            c.id.as_deref().unwrap_or(""),
        )
    }
    let chosen = match weight(candidate).cmp(&weight(current)) {
        std::cmp::Ordering::Greater => candidate,
        std::cmp::Ordering::Less => current,
        std::cmp::Ordering::Equal => match tiebreak(current).cmp(&tiebreak(candidate)) {
            std::cmp::Ordering::Less => current,
            _ => candidate,
        },
    };
    chosen.clone()
}

mod ingest;
pub use ingest::{insert_card, update_card, upsert_print};

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str, layout: &str, games: &[&str]) -> ScryfallCard {
        ScryfallCard {
            name: name.to_string(),
            id: Some("sid".into()),
            oracle_id: "oid".into(),
            released_at: Some("2020-01-01".into()),
            mana_cost: Some("{1}{U}".into()),
            cmc: Some(2.0),
            type_line: Some("Creature — Test".into()),
            colors: Some(vec!["U".into()]),
            color_identity: Some(vec!["U".into()]),
            keywords: Some(vec!["Flying".into()]),
            power: Some("2".into()),
            toughness: Some("2".into()),
            loyalty: None,
            oracle_text: Some("Flying".into()),
            rarity: Some("common".into()),
            edhrec_rank: Some(1000),
            game_changer: None,
            legalities: Some(
                serde_json::json!({"modern": "legal"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            set_code: Some("tst".into()),
            set_name: Some("Test Set".into()),
            lang: Some("en".into()),
            flavor_name: None,
            collector_number: Some("1".into()),
            finishes: Some(vec!["nonfoil".into()]),
            prices: Some(Prices {
                usd: Some("0.05".into()),
                usd_foil: None,
                usd_etched: None,
            }),
            games: Some(games.iter().map(|g| g.to_string()).collect()),
            layout: layout.to_string(),
            set_type: Some("expansion".into()),
            block: None,
            promo_types: None,
            card_faces: None,
        }
    }

    #[test]
    fn skips_non_card_layouts() {
        for layout in [
            "token",
            "art_series",
            "emblem",
            "double_faced_token",
            "vanguard",
        ] {
            assert!(
                should_skip(&card("X", layout, &["paper"])),
                "layout {layout}"
            );
        }
        assert!(!should_skip(&card("X", "normal", &["paper"])));
        assert!(!should_skip(&card("X", "transform", &["paper", "mtgo"])));
    }

    #[test]
    fn skips_digital_only_prints() {
        assert!(should_skip(&card("X", "normal", &["arena"])));
        assert!(should_skip(&card("X", "normal", &["mtgo", "arena"])));
        assert!(!should_skip(&card("X", "normal", &["paper", "arena"])));
        assert!(should_skip(&card("X", "normal", &[])));
    }

    #[test]
    fn duplicate_resolution_prefers_most_platforms_then_lower_rank() {
        let paper = card("A", "normal", &["paper"]);
        let wider = card("A", "normal", &["paper", "mtgo"]);
        assert_eq!(prefer_row(&paper, &wider).games.as_ref().unwrap().len(), 2);

        let popular = ScryfallCard {
            edhrec_rank: Some(10),
            ..card("B", "normal", &["paper"])
        };
        let obscure = ScryfallCard {
            edhrec_rank: Some(9000),
            ..card("B", "normal", &["paper"])
        };
        let merged = prefer_row(&popular, &obscure);
        assert_eq!(merged.edhrec_rank, Some(10));
        let merged = prefer_row(&obscure, &popular);
        assert_eq!(merged.edhrec_rank, Some(10));
    }

    #[test]
    fn ingest_keeps_released_cards_and_upcoming_reprints_only() {
        assert!(should_ingest(&card("A", "normal", &["paper"])));
        // Upcoming reprint of a released card: real format memberships exist.
        let reprint = ScryfallCard {
            released_at: Some("2099-01-01".into()),
            legalities: Some(
                serde_json::json!({"commander": "legal", "modern": "legal"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            ..card("B", "normal", &["paper"])
        };
        assert!(should_ingest(&reprint));
        // Never-released card: every format is not_legal.
        let future = ScryfallCard {
            released_at: Some("2099-01-01".into()),
            legalities: Some(
                serde_json::json!({"commander": "not_legal", "modern": "not_legal"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            ..card("C", "normal", &["paper"])
        };
        assert!(!should_ingest(&future));
    }

    #[test]
    fn face_flattening_joins_costs_and_colors() {
        use serde_json::json;
        let mut c = card("Split // Decision", "split", &["paper"]);
        c.mana_cost = None;
        c.type_line = None;
        c.oracle_text = None;
        c.colors = None;
        c.card_faces = Some(vec![
            serde_json::from_value(json!({"name": "Split", "mana_cost": "{R}", "type_line": "Instant", "oracle_text": "Deal 2", "colors": ["R"]})).unwrap(),
            serde_json::from_value(json!({"name": "Decision", "mana_cost": "{U}", "type_line": "Instant", "oracle_text": "Draw 2", "colors": ["U"]})).unwrap(),
        ]);
        flatten_faces(&mut c);
        assert_eq!(c.mana_cost.as_deref(), Some("{R} // {U}"));
        assert_eq!(c.type_line.as_deref(), Some("Instant // Instant"));
        assert_eq!(c.oracle_text.as_deref(), Some("Deal 2\n// Draw 2"));
        assert_eq!(
            c.colors.as_ref().unwrap(),
            &vec!["R".to_string(), "U".to_string()]
        );
    }

    #[test]
    fn single_sided_cards_are_untouched() {
        let mut c = card("Bolt", "normal", &["paper"]);
        let before = c.mana_cost.clone();
        flatten_faces(&mut c);
        assert_eq!(c.mana_cost, before);
    }

    #[test]
    fn insert_card_roundtrips_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        let c = card("Test Card", "normal", &["paper"]);
        insert_card(&conn, &c).unwrap();
        insert_card(&conn, &c).unwrap(); // duplicate must be ignored
        let (n, name): (i64, String) = conn
            .query_row("SELECT COUNT(*), MAX(name) FROM cards", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(name, "Test Card");
    }

    #[test]
    fn upsert_print_skips_idless_prints() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        // No `id` field: upserting would key the row to `scryfall_id = ''`
        // and collide with every other id-less print, so it is skipped.
        let mut c = card("No Id", "normal", &["paper"]);
        c.id = None;
        upsert_print(&conn, &c, "now").unwrap();
        c.id = Some(String::new());
        upsert_print(&conn, &c, "now").unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM card_prints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "id-less prints never land");
    }

    #[test]
    fn upsert_print_stores_universe_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        let mut c = card("Test Card", "normal", &["paper"]);
        c.set_code = Some("TST".to_string()); // stored lowercase
        c.id = Some("print-1".into());
        upsert_print(&conn, &c, "now").unwrap();
        // Idempotent upsert on the same print ID.
        upsert_print(&conn, &c, "now").unwrap();
        let row: (String, String, f64, String) = conn
            .query_row(
                "SELECT name, set_code, usd, released_at FROM card_prints
                 WHERE scryfall_id = 'print-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "Test Card");
        assert_eq!(row.1, "tst", "set code lowercased");
        assert_eq!(row.2, 0.05);
        assert_eq!(row.3, "2020-01-01");
        let set_name: String = conn
            .query_row(
                "SELECT set_name FROM sets WHERE set_code = 'tst'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(set_name, "Test Set");
        // In-universe set: no franchise, unflagged print, set_type stored.
        let (set_type, block, franchise): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT set_type, block, franchise FROM sets WHERE set_code = 'tst'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(set_type, "expansion");
        assert_eq!(block, None);
        assert_eq!(franchise, None);
        let ub: i64 = conn
            .query_row(
                "SELECT universes_beyond FROM card_prints WHERE scryfall_id = 'print-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ub, 0);

        // A UB print in a Marvel set: flagged print, franchise on the set.
        let mut marvel = card("Test Card", "normal", &["paper"]);
        marvel.set_code = Some("MSH".to_string());
        marvel.set_name = Some("Marvel Super Heroes".to_string());
        marvel.promo_types = Some(vec!["universesbeyond".into()]);
        marvel.id = Some("print-2".into());
        upsert_print(&conn, &marvel, "now").unwrap();
        let franchise: Option<String> = conn
            .query_row(
                "SELECT franchise FROM sets WHERE set_code = 'msh'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(franchise.as_deref(), Some("Marvel"));
        let ub: i64 = conn
            .query_row(
                "SELECT universes_beyond FROM card_prints WHERE scryfall_id = 'print-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ub, 1, "universesbeyond promo flag marks the print");

        // A D&D set is honorary UB even with no promo flag (Scryfall leaves
        // its prints unmarked; WotC owns D&D).
        let mut dnd = card("Test Card", "normal", &["paper"]);
        dnd.set_code = Some("AFR".to_string());
        dnd.set_name = Some("Adventures in the Forgotten Realms".to_string());
        dnd.id = Some("print-3".into());
        upsert_print(&conn, &dnd, "now").unwrap();
        let (franchise, ub): (Option<String>, i64) = conn
            .query_row(
                "SELECT s.franchise, p.universes_beyond FROM card_prints p
                 JOIN sets s ON s.set_code = p.set_code
                 WHERE p.scryfall_id = 'print-3'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(franchise.as_deref(), Some("Dungeons & Dragons"));
        assert_eq!(ub, 1, "D&D prints are honorary UB");
    }

    #[test]
    fn upsert_print_indexes_flavor_name_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        let mut c = card("Zilortha, Strength Incarnate", "normal", &["paper"]);
        c.id = Some("print-z".into());
        c.flavor_name = Some("Godzilla, King of the Monsters".into());
        upsert_print(&conn, &c, "now").unwrap();
        let alias: String = conn
            .query_row(
                "SELECT DISTINCT name FROM card_prints
                 WHERE flavor_name = 'Godzilla, King of the Monsters'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(alias, "Zilortha, Strength Incarnate");
        // No flavor name → empty string on the print row, no alias hits.
        let mut plain = card("Bolt", "normal", &["paper"]);
        plain.id = Some("print-b".into());
        upsert_print(&conn, &plain, "now").unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM card_prints WHERE flavor_name != ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);
    }

    #[test]
    fn download_failure_leaves_dest_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("bulk.json");
        std::fs::write(&dest, b"good data").unwrap();
        let mut out = crate::output::Output::new(true, false, false);
        // Unroutable host: the request fails before any write.
        let result = download_to("http://127.0.0.1:9/never", &dest, None, &mut out);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"good data");
        // No temp file left behind.
        assert!(!tmp.path().join("bulk.json.tmp").exists());
    }

    #[test]
    fn stream_records_parses_gzip_jsonl() {
        use flate2::write::GzEncoder;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bulk.jsonl.gz");
        let enc = GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut enc = enc;
        use std::io::Write;
        writeln!(enc, r#"{{"name":"A","layout":"normal","games":["paper"]}}"#).unwrap();
        writeln!(enc, "not json").unwrap();
        writeln!(enc, r#"{{"name":"B","layout":"token","games":["paper"]}}"#).unwrap();
        drop(enc);

        let mut seen = Vec::new();
        stream_records(&path, |c| seen.push(c.name)).unwrap();
        assert_eq!(seen, vec!["A".to_string(), "B".to_string()]);
    }

    // ensure_fresh_bulk

    fn bulk_file(updated_at: Option<&str>) -> BulkFile {
        BulkFile {
            uri: "https://example.com/bulk.jsonl.gz".into(),
            compressed_size: Some(1000),
            updated_at: updated_at.map(String::from),
        }
    }

    /// The freshness decision is pure mtime/updated_at logic; point the
    /// downloader at a dead host so a false "stale" would fail loudly.
    #[test]
    fn ensure_fresh_bulk_reuses_fresh_local_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("bulk.jsonl");
        std::fs::write(&dest, b"data").unwrap();
        // Just-written file: neither mtime-stale nor older than the remote
        // stamp.
        let mut out = crate::output::Output::new(true, false, false);
        let downloaded = ensure_fresh_bulk(
            &bulk_file(Some("2000-01-01T00:00:00.000Z")),
            &dest,
            &mut out,
        )
        .unwrap();
        assert!(!downloaded, "fresh local file must not re-download");
    }

    #[test]
    fn ensure_fresh_bulk_downloads_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("missing.jsonl");
        let mut out = crate::output::Output::new(true, false, false);
        let result = ensure_fresh_bulk(&bulk_file(None), &dest, &mut out);
        // The missing file forces a download; the fake URI fails, proving
        // the freshness gate let it through.
        assert!(result.is_err(), "missing file must attempt a download");
    }

    #[test]
    fn ensure_fresh_bulk_downloads_when_remote_is_newer() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("bulk.jsonl");
        std::fs::write(&dest, b"data").unwrap();
        // A remote stamp far in the future beats any local mtime.
        let mut out = crate::output::Output::new(true, false, false);
        let result = ensure_fresh_bulk(
            &bulk_file(Some("2999-01-01T00:00:00.000Z")),
            &dest,
            &mut out,
        );
        assert!(result.is_err(), "newer remote must attempt a download");
    }

    #[test]
    fn ensure_fresh_bulk_downloads_when_local_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("bulk.jsonl");
        std::fs::write(&dest, b"data").unwrap();
        // Backdate past STALE_AFTER.
        let old = std::time::SystemTime::now()
            - crate::sync::STALE_AFTER.to_std().unwrap()
            - std::time::Duration::from_secs(60);
        let file = std::fs::File::options().write(true).open(&dest).unwrap();
        file.set_modified(old).unwrap();
        drop(file);
        let mut out = crate::output::Output::new(true, false, false);
        let result = ensure_fresh_bulk(&bulk_file(None), &dest, &mut out);
        assert!(result.is_err(), "stale local file must attempt a download");
    }
}
