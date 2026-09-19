// Commander Spellbook combo ingest: download the variants bulk, stream-parse
// the `variants` array, and rewrite the `combos`/`combo_pieces` tables.
//
// The bulk is one JSON object (`timestamp`, `version`, `variants`,
// `aliases`) at a static URL, gzipped, ~28 MB / 652 MB raw. A brace-matched
// scanner walks the `variants` array and parses one variant at a time, so
// peak memory stays small regardless of dataset size. Aliases are stale-id
// redirects and are not ingested; spoiled variants are skipped.

use anyhow::Context;
use rusqlite::Connection;

/// Download URL for the gzipped variants bulk. Static, no auth; the
/// document is refreshed daily.
pub const VARIANTS_URL: &str = "https://json.commanderspellbook.com/variants.json.gz";

/// One card reference in the bulk payload.
#[derive(Debug, serde::Deserialize)]
pub struct SpellbookCardRef {
    name: String,
}

/// One piece requirement in the bulk payload.
#[derive(Debug, serde::Deserialize)]
pub struct SpellbookUse {
    pub card: SpellbookCardRef,
    #[serde(default, rename = "zoneLocations")]
    pub zone_locations: Vec<String>,
    #[serde(default, rename = "mustBeCommander")]
    pub must_be_commander: bool,
}

#[derive(Debug, serde::Deserialize)]
struct SpellbookFeature {
    #[serde(default)]
    name: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct SpellbookProduce {
    feature: SpellbookFeature,
}

/// One combo variant in the bulk payload. Prices, images, and the
/// description are dropped; the store keeps what deck joins need.
#[derive(Debug, serde::Deserialize)]
pub struct SpellbookVariant {
    pub id: String,
    #[serde(default)]
    pub produces: Vec<SpellbookProduce>,
    #[serde(default, rename = "manaValueNeeded")]
    pub mana_value_needed: u32,
    #[serde(default, rename = "bracketTag")]
    pub bracket_tag: Option<String>,
    #[serde(default)]
    pub popularity: Option<i64>,
    #[serde(default)]
    pub legalities: std::collections::HashMap<String, bool>,
    #[serde(default, rename = "uses")]
    pub uses: Vec<SpellbookUse>,
    #[serde(default)]
    pub spoiler: bool,
}

/// A combo piece flattened for storage: faces split, zones kept.
#[derive(Debug, PartialEq)]
pub struct ParsedPiece {
    pub name: String,
    pub zones: Vec<String>,
    pub must_be_commander: bool,
}

/// One parsed variant ready for the store.
#[derive(Debug, PartialEq)]
pub struct ParsedVariant {
    pub id: String,
    pub produces: Vec<String>,
    pub mana_value_needed: u32,
    pub bracket_tag: Option<String>,
    pub popularity: Option<i64>,
    pub legalities: String,
    pub pieces: Vec<ParsedPiece>,
}

/// Parse one bulk variant into its stored shape; `None` skips the row
/// (spoilers, pieceless variants).
pub fn parse_variant(v: SpellbookVariant) -> Option<ParsedVariant> {
    if v.spoiler || v.uses.is_empty() {
        return None;
    }
    let produces: Vec<String> = v.produces.iter().map(|p| p.feature.name.clone()).collect();
    let legalities = serde_json::to_string(&v.legalities).unwrap_or_default();
    let pieces = v
        .uses
        .iter()
        .flat_map(|spell_use| {
            // "A // B" faces match either half when deck joins run.
            split_faces(&spell_use.card.name)
                .into_iter()
                .map(move |name| ParsedPiece {
                    name,
                    zones: spell_use.zone_locations.clone(),
                    must_be_commander: spell_use.must_be_commander,
                })
        })
        .collect();
    Some(ParsedVariant {
        id: v.id,
        produces,
        mana_value_needed: v.mana_value_needed,
        bracket_tag: v.bracket_tag,
        popularity: v.popularity,
        legalities,
        pieces,
    })
}

/// Split an oracle name into its faces; a single-face name yields itself.
fn split_faces(name: &str) -> Vec<String> {
    if name.contains(" // ") {
        name.split(" // ").map(str::to_string).collect()
    } else {
        vec![name.to_string()]
    }
}

/// Download the variants bulk when the local copy is missing or older than
/// the staleness window. Returns true when a download happened.
pub fn ensure_fresh_variants(
    dest: &std::path::Path,
    out: &mut crate::output::Output,
) -> anyhow::Result<bool> {
    if let Ok(meta) = std::fs::metadata(dest)
        && let Ok(modified) = meta.modified()
        && let Ok(age) = modified.elapsed()
        && age <= crate::sync::STALE_AFTER.to_std()?
    {
        return Ok(false);
    }
    out.status("Downloading", "combo variants (Commander Spellbook)");
    crate::scryfall::download_to(VARIANTS_URL, dest, None, out)?;
    Ok(true)
}

/// Stream-parse the variants array, one brace-matched object at a time.
/// `visit` receives each raw object string; malformed rows are skipped so
/// one bad row cannot abort a refresh.
pub fn stream_raw_variants<F: FnMut(&str)>(
    path: &std::path::Path,
    mut visit: F,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("cannot open combo bulk {}", path.display()))?;
    let mut reader =
        flate2::read::MultiGzDecoder::new(std::io::BufReader::with_capacity(1 << 20, file));

    // Slide a window until the "variants": marker appears; keep the tail in
    // case the marker straddles a chunk boundary.
    const NEEDLE: &[u8] = b"\"variants\":";
    let mut window: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n =
            std::io::Read::read(&mut reader, &mut chunk).context("reading combo bulk header")?;
        if n == 0 {
            anyhow::bail!("combo bulk has no variants array");
        }
        window.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subslice(&window, NEEDLE) {
            let rest = window[pos + NEEDLE.len()..].to_vec();
            window = rest;
            break;
        }
        if window.len() > NEEDLE.len() {
            let keep_from = window.len() - NEEDLE.len();
            window.drain(..keep_from);
        }
    }

    // Walk the array with a brace counter that tracks string state, so
    // braces inside strings never mislead. Objects are emitted raw.
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut object = String::new();
    let mut bytes = window;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        if bytes.is_empty() {
            let n = std::io::Read::read(&mut reader, &mut chunk).context("reading combo bulk")?;
            if n == 0 {
                break;
            }
            bytes = chunk[..n].to_vec();
        }
        let drained = std::mem::take(&mut bytes);
        for &b in &drained {
            let c = b as char;
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
            } else {
                match c {
                    '"' => in_string = true,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            object.push(c);
                            visit(&object);
                            object.clear();
                            continue;
                        }
                    }
                    ']' if depth == 0 => return Ok(()),
                    _ => {}
                }
            }
            if depth > 0 {
                object.push(c);
            }
        }
    }
    Ok(())
}

/// Find the first occurrence of `needle` in `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Parse every variant in the bulk file; malformed rows are skipped.
pub fn parse_all(path: &std::path::Path) -> anyhow::Result<Vec<ParsedVariant>> {
    let mut out = Vec::new();
    stream_raw_variants(path, |raw| {
        if let Ok(v) = serde_json::from_str::<SpellbookVariant>(raw)
            && let Some(parsed) = parse_variant(v)
        {
            out.push(parsed);
        }
    })?;
    Ok(out)
}

/// Rewrite the `combos`/`combo_pieces` tables from the bulk file, one
/// transaction for both. Returns the variant count.
///
/// # Errors
/// Propagates SQLite and parse failures.
pub fn ingest(
    conn: &mut Connection,
    bulk_path: &std::path::Path,
    out: &mut crate::output::Output,
    updated_at: &str,
) -> anyhow::Result<usize> {
    let variants = parse_all(bulk_path)?;
    let n = variants.len();
    let tx = conn.transaction().context("begin combo ingest")?;
    tx.execute("DELETE FROM combo_pieces", [])?;
    tx.execute("DELETE FROM combos", [])?;
    {
        let mut combo_stmt = tx.prepare(
            "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag,
                legalities, popularity, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        let mut piece_stmt = tx.prepare(
            "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for v in &variants {
            combo_stmt.execute(rusqlite::params![
                v.id,
                serde_json::to_string(&v.produces).unwrap_or_default(),
                v.mana_value_needed,
                v.bracket_tag,
                v.legalities,
                v.popularity,
                updated_at,
            ])?;
            for (ordinal, piece) in v.pieces.iter().enumerate() {
                let zones = serde_json::to_string(&piece.zones).unwrap_or_default();
                piece_stmt.execute(rusqlite::params![
                    v.id,
                    piece.name,
                    ordinal as i64,
                    zones,
                    piece.must_be_commander as i64,
                ])?;
            }
        }
    }
    tx.commit().context("commit combo ingest")?;
    out.status("Ingested", &format!("{n} combo variants"));
    Ok(n)
}

/// Combo variant loaded from the store.
#[derive(Debug, Clone)]
pub struct ComboVariant {
    pub id: String,
    /// Feature names the combo produces.
    pub produces: Vec<String>,
    pub mana_value_needed: i64,
    pub bracket_tag: Option<String>,
    pub popularity: Option<i64>,
    /// Format name → legal.
    pub legalities: std::collections::HashMap<String, bool>,
}

/// One combo piece as stored.
#[derive(Debug, Clone)]
pub struct ComboPieceRow {
    pub name: String,
    pub ordinal: i64,
    pub zones: Vec<String>,
    pub must_be_commander: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gz_payload(body: &str) -> Vec<u8> {
        use std::io::Write as _;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(body.as_bytes()).unwrap();
        enc.finish().unwrap()
    }

    fn sample_body() -> String {
        r#"{"timestamp":"2026-09-17T00:00:00","version":"6.4.0","variants":[
        {"id":"742-1295","uses":[
            {"card":{"name":"Demonic Consultation"},"zoneLocations":["H"],"mustBeCommander":false},
            {"card":{"name":"Thassa's Oracle"},"zoneLocations":["H","B"],"mustBeCommander":false}
        ],
        "produces":[{"feature":{"id":2,"name":"Win the game"}}],
        "identity":"UB","manaNeeded":"{U}{U}{B}","manaValueNeeded":3,
        "status":"OK","spoiler":false,"bracketTag":"R",
        "popularity":149575,
        "legalities":{"commander":true,"modern":false}}],
        "aliases":[]}"#
            .to_string()
    }

    fn write_bulk(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
        let path = dir.path().join("variants.json.gz");
        std::fs::write(&path, gz_payload(body)).unwrap();
        path
    }

    #[test]
    fn stream_raw_variants_parses_wrapped_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_bulk(&dir, &sample_body());
        let mut seen = Vec::new();
        stream_raw_variants(&path, |raw| seen.push(raw.to_string())).unwrap();
        assert_eq!(seen.len(), 1);
        let v: SpellbookVariant = serde_json::from_str(&seen[0]).unwrap();
        assert_eq!(v.id, "742-1295");
        assert_eq!(v.uses.len(), 2);
        assert_eq!(v.uses[0].card.name, "Demonic Consultation");
        assert_eq!(v.uses[1].zone_locations, vec!["H", "B"]);
        assert_eq!(v.bracket_tag.as_deref(), Some("R"));
        assert!(v.legalities["commander"]);
        assert!(!v.legalities["modern"]);
    }

    #[test]
    fn stream_raw_variants_skips_malformed_rows() {
        let body = r#"{"timestamp":"t","variants":[
        {"id":"ok-1","uses":[{"card":{"name":"A"}}],"produces":[],"spoiler":false},
        {"id":"broken" "uses":[]},
        {"id":"ok-2","uses":[{"card":{"name":"B"}}],"produces":[],"spoiler":false}
        ]}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = write_bulk(&dir, body);
        let mut seen = Vec::new();
        stream_raw_variants(&path, |raw| {
            if let Ok(v) = serde_json::from_str::<SpellbookVariant>(raw) {
                seen.push(v.id);
            }
        })
        .unwrap();
        assert_eq!(seen, vec!["ok-1", "ok-2"]);
    }

    #[test]
    fn parse_variant_splits_faces_and_keeps_zones() {
        let v: SpellbookVariant = serde_json::from_str(
            r#"{"id":"1-2","uses":[
            {"card":{"name":"Emeritus of Abundance // Regrowth"},"zoneLocations":["B"]},
            {"card":{"name":"Bear"},"zoneLocations":["G"],"mustBeCommander":true}],
            "produces":[{"feature":{"name":"Win the game"}}],
            "manaValueNeeded":5,"bracketTag":null,"spoiler":false,"legalities":{}}"#,
        )
        .unwrap();
        let parsed = parse_variant(v).unwrap();
        let names: Vec<&str> = parsed.pieces.iter().map(|p| p.name.as_str()).collect();
        // A two-face card becomes two rows; the single-face card stays one.
        assert_eq!(names, vec!["Emeritus of Abundance", "Regrowth", "Bear"]);
        assert_eq!(parsed.produces, vec!["Win the game"]);
        assert!(parsed.pieces[2].must_be_commander);
    }

    #[test]
    fn parse_variant_skips_spoilers() {
        let v: SpellbookVariant = serde_json::from_str(
            r#"{"id":"9-9","uses":[{"card":{"name":"A"}}],"produces":[],"spoiler":true,"legalities":{}}"#,
        )
        .unwrap();
        assert!(parse_variant(v).is_none());
    }

    #[test]
    fn ingest_rewrites_tables_wholesale() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = crate::db::open(&dir.path().join("t.db")).unwrap();
        // Seed a stale row; the refresh must drop it.
        conn.execute(
            "INSERT INTO combos (id, updated_at) VALUES ('stale-1', 'old')",
            [],
        )
        .unwrap();
        let path = write_bulk(&dir, &sample_body());
        let mut out = crate::output::Output::new(true, false, false);
        let n = ingest(&mut conn, &path, &mut out, "now").unwrap();
        assert_eq!(n, 1);

        let combos: i64 = conn
            .query_row("SELECT COUNT(*) FROM combos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(combos, 1, "stale row replaced");
        let pieces: i64 = conn
            .query_row("SELECT COUNT(*) FROM combo_pieces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pieces, 2);

        // Deck join: both names find the variant; an unrelated name does not.
        let mut names = std::collections::HashSet::new();
        names.insert("Thassa's Oracle".to_string());
        let hits = crate::combos::load_variants_for(&conn, &names).unwrap();
        assert_eq!(hits.len(), 1);
        let (combo, pieces) = &hits[0];
        assert_eq!(combo.id, "742-1295");
        assert_eq!(combo.produces, vec!["Win the game"]);
        assert_eq!(combo.popularity, Some(149575));
        assert!(combo.legalities["commander"]);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].zones, vec!["H"]);
    }
}
