//! Rebuild the supplement in memory from declared offline sources.

use super::fixture_metadata::{Supplement, validate_colors};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Saved inputs in precedence order when matching metadata occurs more than once.
const SOURCE_PATHS: [&str; 11] = [
    "src/deck/simulator/tests/deck_fixtures/mtgjson_buckle_up.json",
    "src/deck/simulator/tests/deck_fixtures/mtgjson_dance_of_the_elements.json",
    "src/deck/simulator/tests/deck_fixtures/mtgjson_enduring_enchantments.json",
    "src/deck/simulator/tests/popular_card_fixtures/sources/scryfall_1.json",
    "src/deck/simulator/tests/popular_card_fixtures/sources/scryfall_2.json",
    "src/deck/simulator/tests/popular_card_fixtures/sources/scryfall_3.json",
    "src/deck/simulator/tests/popular_card_fixtures/sources/scryfall_5.json",
    "src/deck/simulator/tests/popular_card_fixtures/sources/2026-10-29/sisay.json",
    "src/deck/simulator/tests/fixture_identities/sources/2026-10-29/response.json",
    "src/deck/simulator/tests/fixture_metadata/sources/2026-10-29-vehicles/response.json",
    "src/deck/simulator/tests/fixture_metadata/sources/2026-10-29-vehicle-front/response.json",
];

/// Historical Scryfall sources share this stored provenance manifest.
const POPULAR_MANIFEST: &str = "src/deck/simulator/tests/popular_card_fixtures/provenance.json";

/// A selected card and its validated identity for duplicate checks.
struct CapturedCard {
    record: Value,
    colors: Vec<String>,
}

/// Read JSON without replacing the raw bytes used for source hashes.
fn read_json(
    path: &str,
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(Vec<u8>, Value), String> {
    let bytes = read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON in {path}: {error}"))?;
    Ok((bytes, value))
}

/// Require source text rather than treating malformed metadata as an absent field.
fn text<'a>(value: &'a Value, key: &str, path: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing or invalid {key} in {path}"))
}

/// Reject non-HTTPS provenance links before they enter the supplement.
fn https_uri(value: &str, path: &str) -> Result<String, String> {
    if !value.starts_with("https://") {
        return Err(format!("invalid source URI in {path}"));
    }
    Ok(value.to_string())
}

/// Gather the current legacy commanders and Vehicles without using the supplement.
fn legacy_names(
    root: &Path,
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<BTreeSet<String>, String> {
    let directory = "src/deck/simulator/tests/deck_fixtures";
    let mut paths = std::fs::read_dir(root.join(directory))
        .map_err(|error| format!("cannot list {directory}: {error}"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot list {directory}: {error}"))?;
    paths.sort();
    let mut names = BTreeSet::new();
    for path in paths {
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_str()
            .ok_or_else(|| "fixture path is not UTF-8".to_string())?;
        let (_, value) = read_json(relative, read)?;
        add_legacy_names(&value, relative, &mut names)?;
    }
    if names.is_empty() {
        return Err("legacy fixture metadata inventory is empty".into());
    }
    Ok(names)
}

/// Select legacy rows while keeping explicit source-kind fixtures out of the inventory.
fn add_legacy_names(value: &Value, path: &str, names: &mut BTreeSet<String>) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!("invalid deck fixture in {path}"));
    }
    if !value.get("source_kind").is_none_or(Value::is_null) {
        text(value, "source_kind", path)?;
        return Ok(());
    }
    for field in ["commanders", "cards"] {
        let rows = value[field]
            .as_array()
            .ok_or_else(|| format!("invalid {field} in {path}"))?;
        for row in rows {
            if field == "commanders" || text(row, "type_line", path)?.contains("Vehicle") {
                names.insert(text(row, "name", path)?.to_string());
            }
        }
    }
    Ok(())
}

/// Build expected JSON without reading the pinned supplement or writing any files.
fn build(
    names: &BTreeSet<String>,
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Value, String> {
    if names.is_empty() || names.iter().any(String::is_empty) {
        return Err("invalid legacy fixture metadata inventory".into());
    }
    let (_, manifest) = read_json(POPULAR_MANIFEST, read)?;
    if manifest["schema_version"] != 2 {
        return Err(format!(
            "unsupported historical provenance version: {POPULAR_MANIFEST}"
        ));
    }
    let mut sources = Vec::new();
    let mut cards = BTreeMap::new();
    for path in SOURCE_PATHS {
        let (bytes, value) = read_json(path, read)?;
        sources.push(source_record(path, &bytes, &value, &manifest, read)?);
        collect_cards(&value, "", path, names, &mut cards)?;
    }
    let missing: Vec<_> = names
        .iter()
        .filter(|name| !cards.contains_key(*name))
        .collect();
    if !missing.is_empty() {
        return Err(format!("missing pinned metadata: {missing:?}"));
    }
    let cards: Vec<_> = cards.into_values().map(|card| card.record).collect();
    let document = json!({"schema_version": 2, "sources": sources, "cards": cards});
    Supplement::parse(&document.to_string())?;
    Ok(document)
}

/// Preserve each source's raw-byte hash and declared provenance.
fn source_record(
    path: &str,
    bytes: &[u8],
    value: &Value,
    manifest: &Value,
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Value, String> {
    let hash = format!("{:x}", Sha256::digest(bytes));
    let (uri, provenance) = if path.contains("/deck_fixtures/") {
        if value["source_kind"] != "official_precon"
            || !value["commanders"].is_array()
            || !value["cards"].is_array()
        {
            return Err(format!("invalid stored MTGJSON precon: {path}"));
        }
        let uri = https_uri(text(value, "source", path)?, path)?;
        let provenance = json!({
            "kind": "stored_mtgjson_precon",
            "source_id": text(value, "source_id", path)?,
            "card_data_version": text(value, "card_data_version", path)?,
            "card_data_date": text(value, "card_data_date", path)?,
            "retrieved_at": text(value, "retrieved_at", path)?,
        });
        (uri, provenance)
    } else {
        validate_scryfall_source(value, path)?;
        let uri = if value["object"] == "card" {
            https_uri(text(value, "uri", path)?, path)?
        } else {
            "https://api.scryfall.com/cards/collection".to_string()
        };
        let provenance = if let Some(directory) = path.strip_suffix("response.json") {
            capture_provenance(directory, &hash, value, read)?
        } else {
            validate_popular_hash(manifest, path, &hash)?;
            json!({"kind": "stored_scryfall_capture", "manifest": POPULAR_MANIFEST})
        };
        (uri, provenance)
    };
    Ok(json!({"path": path, "sha256": hash, "uri": uri, "provenance": provenance}))
}

/// Accept saved partial collections while rejecting malformed Scryfall records.
fn validate_scryfall_source(value: &Value, path: &str) -> Result<(), String> {
    match value["object"].as_str() {
        Some("card") => Ok(()),
        Some("list")
            if value["data"].is_array()
                && value["data"]
                    .as_array()
                    .is_some_and(|cards| cards.iter().all(|card| card["object"] == "card")) =>
        {
            named_identifiers(&value["not_found"], path).map_err(|error| {
                format!("invalid or incomplete Scryfall source: {path}: {error}")
            })?;
            Ok(())
        }
        _ => Err(format!("invalid or incomplete Scryfall source: {path}")),
    }
}

/// Read the unique name-only identifiers used by the saved collection captures.
fn named_identifiers<'a>(value: &'a Value, path: &str) -> Result<BTreeSet<&'a str>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("invalid named identifiers in {path}"))?;
    let mut names = BTreeSet::new();
    for row in rows {
        let name = text(row, "name", path)?;
        if row.as_object().is_none_or(|fields| fields.len() != 1) || !names.insert(name) {
            return Err(format!(
                "invalid or duplicate named identifier in {path}: {name}"
            ));
        }
    }
    Ok(names)
}

/// Check historical source bytes against their saved manifest, not canonical JSON.
fn validate_popular_hash(manifest: &Value, path: &str, hash: &str) -> Result<(), String> {
    let relative = path
        .strip_prefix("src/deck/simulator/tests/popular_card_fixtures/")
        .ok_or_else(|| format!("undeclared historical source: {path}"))?;
    let files = manifest["files"]
        .as_array()
        .ok_or_else(|| format!("missing files in {POPULAR_MANIFEST}"))?;
    let matches: Vec<_> = files
        .iter()
        .filter(|file| file["path"] == relative)
        .collect();
    if matches.len() != 1 || matches[0]["raw_bytes_sha256"] != hash {
        return Err(format!(
            "historical source hash does not match manifest: {path}"
        ));
    }
    Ok(())
}

/// Verify the saved request and response before copying capture provenance.
fn capture_provenance(
    directory: &str,
    response_hash: &str,
    response: &Value,
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Value, String> {
    let capture_path = format!("{directory}capture.json");
    let (_, capture) = read_json(&capture_path, read)?;
    let request_path = format!("{directory}request.json");
    let (request_bytes, request) = read_json(&request_path, read)?;
    let request_hash = format!("{:x}", Sha256::digest(request_bytes));
    if capture["source"] != "Scryfall"
        || capture["endpoint"] != "https://api.scryfall.com/cards/collection"
        || capture["method"] != "POST"
        || capture["operation"] != "read-only card collection lookup"
        || capture["response_sha256"] != response_hash
        || capture["request_sha256"] != request_hash
    {
        return Err(format!("invalid capture provenance: {capture_path}"));
    }
    validate_capture_selection(&request, response, &request_path)?;
    Ok(json!({
        "kind": "read_only_scryfall_capture",
        "capture_manifest": capture_path,
        "captured_at": text(&capture, "captured_at", &capture_path)?,
        "request_sha256": request_hash,
    }))
}

/// Require each requested name to appear once as a returned card or a missing lookup.
fn validate_capture_selection(request: &Value, response: &Value, path: &str) -> Result<(), String> {
    let requested = named_identifiers(&request["identifiers"], path)?;
    let missing = named_identifiers(&response["not_found"], path)?;
    let cards = response["data"]
        .as_array()
        .ok_or_else(|| format!("missing captured cards for {path}"))?;
    if requested.is_empty() || !missing.is_subset(&requested) {
        return Err(format!("invalid capture request selection: {path}"));
    }
    for name in &requested {
        let matches = cards
            .iter()
            .filter(|card| card_matches_name(card, name))
            .count();
        if matches + usize::from(missing.contains(name)) != 1 {
            return Err(format!(
                "inconsistent capture request selection for {name} in {path}"
            ));
        }
    }
    for card in cards {
        if !requested.iter().any(|name| card_matches_name(card, name)) {
            return Err(format!("unrequested captured card in {path}"));
        }
    }
    Ok(())
}

/// Match full card names or printed face names used by a collection request.
fn card_matches_name(card: &Value, name: &str) -> bool {
    card["name"] == name
        || card["card_faces"]
            .as_array()
            .is_some_and(|faces| faces.iter().any(|face| face["name"] == name))
}

/// Visit source objects and retain exact JSON Pointer locations.
fn collect_cards(
    value: &Value,
    pointer: &str,
    path: &str,
    names: &BTreeSet<String>,
    cards: &mut BTreeMap<String, CapturedCard>,
) -> Result<(), String> {
    if let Some(name) = value.get("name").and_then(Value::as_str)
        && names.contains(name)
        && (value["object"] == "card"
            || (path.contains("/deck_fixtures/") && value.get("object").is_none()))
    {
        let card = captured_card(value, pointer, path)?;
        if let Some(previous) = cards.get(name) {
            if previous.record["oracle_id"] != card.record["oracle_id"]
                || previous.record["printed_stats"] != card.record["printed_stats"]
                || previous.colors.iter().collect::<BTreeSet<_>>()
                    != card.colors.iter().collect::<BTreeSet<_>>()
            {
                return Err(format!("conflicting captured metadata: {name}"));
            }
        } else {
            cards.insert(name.to_string(), card);
        }
    }
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let key = key.replace('~', "~0").replace('/', "~1");
                collect_cards(child, &format!("{pointer}/{key}"), path, names, cards)?;
            }
        }
        Value::Array(rows) => {
            for (index, child) in rows.iter().enumerate() {
                collect_cards(child, &format!("{pointer}/{index}"), path, names, cards)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Keep front-face printed strings, including explicit nulls and variable values.
fn captured_card(value: &Value, pointer: &str, path: &str) -> Result<CapturedCard, String> {
    let name = text(value, "name", path)?;
    let oracle_id = text(value, "oracle_id", path)?;
    let colors: Vec<String> = serde_json::from_value(value["color_identity"].clone())
        .map_err(|error| format!("invalid color identity for {name} in {path}: {error}"))?;
    validate_colors(name, &colors)?;
    let (stats, stats_pointer) = if let Some(faces) = value.get("card_faces") {
        let front = faces
            .as_array()
            .and_then(|faces| faces.first())
            .filter(|front| front.is_object())
            .ok_or_else(|| format!("missing front face for {name} in {path}"))?;
        (front, format!("{pointer}/card_faces/0"))
    } else {
        (value, pointer.to_string())
    };
    let mut printed_stats = json!({});
    for stat in ["power", "toughness", "loyalty"] {
        let printed = &stats[stat];
        if !printed.is_null() && !printed.is_string() {
            return Err(format!("invalid printed {stat} for {name} in {path}"));
        }
        printed_stats[stat] = printed.clone();
    }
    let uri = match value.get("uri").filter(|uri| !uri.is_null()) {
        Some(_) => text(value, "uri", path)?.to_string(),
        None => format!(
            "https://api.scryfall.com/cards/{}",
            text(value, "scryfall_id", path)?
        ),
    };
    let source_uri = https_uri(&uri, path)?;
    Ok(CapturedCard {
        record: json!({
            "name": name, "oracle_id": oracle_id, "color_identity": colors,
            "source_path": path, "source_pointer": pointer, "stats_pointer": stats_pointer,
            "printed_stats": printed_stats, "source_uri": source_uri,
        }),
        colors,
    })
}

/// Offline rebuild regressions and the explicit review-file maintenance entry point.
#[path = "build_tests.rs"]
mod build_tests;
