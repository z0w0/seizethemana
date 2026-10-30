//! Pinned metadata enrichment shared by offline deck tests and benchmarks.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

/// Exact supplemental inputs, separate from the historical deck snapshots.
pub(super) const SUPPLEMENT: &str = include_str!("supplement.json");

/// Validated records loaded once outside measured benchmark work.
static METADATA: LazyLock<Supplement> = LazyLock::new(|| {
    Supplement::parse(SUPPLEMENT).expect("fixture metadata supplement must be valid")
});

/// One authoritative card record and its exact stored source locations.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataRecord {
    name: String,
    oracle_id: String,
    color_identity: Vec<String>,
    source_path: String,
    source_pointer: String,
    stats_pointer: String,
    printed_stats: PrintedStats,
    source_uri: String,
}

/// Printed strings for the front face, including variable values such as `*`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrintedStats {
    power: Option<String>,
    toughness: Option<String>,
    loyalty: Option<String>,
}

/// Immutable source bytes and the provenance of their card metadata.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataSource {
    path: String,
    sha256: String,
    uri: String,
    provenance: serde_json::Value,
}

/// Serialized source references and unique commander or Vehicle records.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupplementData {
    schema_version: u32,
    sources: Vec<MetadataSource>,
    cards: Vec<MetadataRecord>,
}

/// Unique names and Oracle identities, checked before any loader uses them.
#[derive(Debug)]
pub(super) struct Supplement {
    cards: BTreeMap<String, MetadataRecord>,
}

impl Supplement {
    /// Reject ambiguous identities and incomplete provenance before enriching a row.
    pub(super) fn parse(json: &str) -> Result<Self, String> {
        let data: SupplementData = serde_json::from_str(json).map_err(|error| error.to_string())?;
        if data.schema_version != 2 {
            return Err("unsupported fixture metadata supplement version".into());
        }
        let mut sources = BTreeSet::new();
        for source in data.sources {
            if source.path.is_empty()
                || !sources.insert(source.path)
                || source.sha256.len() != 64
                || !source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                || !source.uri.starts_with("https://")
                || !source.provenance.is_object()
            {
                return Err("invalid or duplicate fixture metadata source".into());
            }
        }
        let mut cards = BTreeMap::new();
        let mut oracle_ids = BTreeSet::new();
        for record in data.cards {
            validate_colors(&record.name, &record.color_identity)?;
            if record.name.is_empty()
                || record.oracle_id.is_empty()
                || !sources.contains(&record.source_path)
                || !(record.source_pointer.is_empty() || record.source_pointer.starts_with('/'))
                || !(record.stats_pointer == record.source_pointer
                    || record.stats_pointer == format!("{}/card_faces/0", record.source_pointer))
                || !record.source_uri.starts_with("https://")
            {
                return Err(format!(
                    "incomplete fixture metadata provenance: {}",
                    record.name
                ));
            }
            if !oracle_ids.insert(record.oracle_id.clone()) {
                return Err(format!(
                    "duplicate or conflicting Oracle identity: {}",
                    record.name
                ));
            }
            if cards.insert(record.name.clone(), record).is_some() {
                return Err("duplicate or conflicting fixture metadata name".into());
            }
        }
        Ok(Self { cards })
    }
}

/// Keep explicit colorless identities valid while rejecting invalid or repeated colors.
pub(super) fn validate_colors(name: &str, colors: &[String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    if colors
        .iter()
        .any(|color| !matches!(color.as_str(), "W" | "U" | "B" | "R" | "G") || !seen.insert(color))
    {
        return Err(format!("invalid color identity for {name}"));
    }
    Ok(())
}

/// Fill only an absent legacy identity, without replacing explicit metadata.
pub(crate) fn enrich(
    name: &str,
    oracle_id: &mut Option<String>,
    color_identity: &mut Option<Vec<String>>,
) -> Result<bool, String> {
    if let Some(colors) = color_identity {
        validate_colors(name, colors)?;
        return Ok(false);
    }
    let record = METADATA
        .cards
        .get(name)
        .ok_or_else(|| format!("no pinned color identity for legacy commander {name}"))?;
    if oracle_id.as_ref().is_some_and(|id| id != &record.oracle_id) {
        return Err(format!(
            "stored Oracle identity conflicts with supplement for {name}"
        ));
    }
    if oracle_id.is_none() {
        *oracle_id = Some(record.oracle_id.clone());
    }
    *color_identity = Some(record.color_identity.clone());
    Ok(true)
}

/// Fill absent printed stats only from the exact pinned front-face record.
pub(crate) fn enrich_stats(
    name: &str,
    oracle_id: Option<&str>,
    power: &mut Option<String>,
    toughness: &mut Option<String>,
    loyalty: &mut Option<String>,
) -> Result<bool, String> {
    let record = METADATA
        .cards
        .get(name)
        .ok_or_else(|| format!("no pinned printed stats for legacy commander or Vehicle {name}"))?;
    if oracle_id.is_some_and(|id| id != record.oracle_id) {
        return Err(format!(
            "stored Oracle identity conflicts with supplement for {name}"
        ));
    }
    let mut changed = false;
    for (stored, pinned) in [
        (power, &record.printed_stats.power),
        (toughness, &record.printed_stats.toughness),
        (loyalty, &record.printed_stats.loyalty),
    ] {
        if stored.is_none() && pinned.is_some() {
            stored.clone_from(pinned);
            changed = true;
        }
    }
    Ok(changed)
}

/// Keep a commander's duplicate row consistent without replacing explicit metadata.
pub(crate) fn inherit(
    name: &str,
    oracle_id: &mut Option<String>,
    color_identity: &mut Option<Vec<String>>,
    known_oracle_id: Option<&str>,
    known_colors: &[String],
) -> Result<bool, String> {
    validate_colors(name, known_colors)?;
    if let Some(colors) = color_identity {
        validate_colors(name, colors)?;
        if colors.iter().collect::<BTreeSet<_>>() != known_colors.iter().collect::<BTreeSet<_>>() {
            return Err(format!("conflicting duplicate color identity for {name}"));
        }
    }
    if let (Some(stored), Some(known)) = (oracle_id.as_deref(), known_oracle_id)
        && stored != known
    {
        return Err(format!("conflicting duplicate Oracle identity for {name}"));
    }
    let changed = (oracle_id.is_none() && known_oracle_id.is_some()) || color_identity.is_none();
    if oracle_id.is_none() {
        *oracle_id = known_oracle_id.map(str::to_string);
    }
    if color_identity.is_none() {
        *color_identity = Some(known_colors.to_vec());
    }
    Ok(changed)
}

/// Identify supplemental metadata that changes a benchmark's effective inputs.
pub(crate) fn supplement_sha256() -> String {
    format!("{:x}", Sha256::digest(SUPPLEMENT.as_bytes()))
}

/// Offline source checks and enrichment edge cases.
#[cfg(test)]
mod tests {
    /// Every record must match the named card in its hashed, offline source.
    #[test]
    fn pinned_sources_match_every_supplemental_field() {
        use super::*;
        use serde_json::Value;

        let data: SupplementData = serde_json::from_str(SUPPLEMENT).expect("supplement parses");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let sources: BTreeMap<_, _> = data
            .sources
            .into_iter()
            .map(|source| {
                let bytes = std::fs::read(root.join(&source.path)).expect("pinned source exists");
                assert_eq!(
                    format!("{:x}", Sha256::digest(&bytes)),
                    source.sha256,
                    "{}",
                    source.path
                );
                let value: Value = serde_json::from_slice(&bytes).expect("pinned source is JSON");
                (source.path, value)
            })
            .collect();
        assert_eq!(data.cards.len(), 66);
        for card in data.cards {
            let source = sources[&card.source_path]
                .pointer(&card.source_pointer)
                .expect("precise source pointer exists");
            assert_eq!(source["name"], card.name);
            assert_eq!(source["oracle_id"], card.oracle_id);
            assert_eq!(
                source["color_identity"],
                serde_json::json!(card.color_identity)
            );
            let uri = source["uri"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "https://api.scryfall.com/cards/{}",
                        source["scryfall_id"]
                            .as_str()
                            .expect("stored Scryfall identity")
                    )
                });
            assert_eq!(uri, card.source_uri);
            let stats = sources[&card.source_path]
                .pointer(&card.stats_pointer)
                .expect("front-face stats pointer exists");
            assert_eq!(
                stats.get("power").and_then(Value::as_str),
                card.printed_stats.power.as_deref()
            );
            assert_eq!(
                stats.get("toughness").and_then(Value::as_str),
                card.printed_stats.toughness.as_deref()
            );
            assert_eq!(
                stats.get("loyalty").and_then(Value::as_str),
                card.printed_stats.loyalty.as_deref()
            );
        }
        assert_eq!(supplement_sha256().len(), 64);
    }

    /// Duplicate records and conflicting names or Oracle identities cannot enter the table.
    #[test]
    fn supplement_rejects_duplicates_and_conflicts() {
        use super::*;
        use serde_json::Value;

        let original: Value = serde_json::from_str(SUPPLEMENT).expect("supplement parses");
        let mut duplicate = original.clone();
        duplicate["cards"]
            .as_array_mut()
            .expect("records")
            .push(original["cards"][0].clone());
        assert!(Supplement::parse(&duplicate.to_string()).is_err());
        let mut wrong_name = original.clone();
        wrong_name["cards"][1]["name"] = original["cards"][0]["name"].clone();
        assert!(Supplement::parse(&wrong_name.to_string()).is_err());
        let mut wrong_id = original.clone();
        wrong_id["cards"][1]["oracle_id"] = original["cards"][0]["oracle_id"].clone();
        assert!(Supplement::parse(&wrong_id.to_string()).is_err());
        let mut wrong_colors = original;
        wrong_colors["cards"][0]["color_identity"] = serde_json::json!(["U", "U"]);
        assert!(Supplement::parse(&wrong_colors.to_string()).is_err());
    }

    /// Source records cannot repeat, and back-face pointers cannot supply front-face stats.
    #[test]
    fn supplement_rejects_duplicate_sources_and_back_face_stats() {
        use super::*;
        use serde_json::Value;

        let original: Value = serde_json::from_str(SUPPLEMENT).expect("supplement parses");
        let mut duplicate = original.clone();
        duplicate["sources"]
            .as_array_mut()
            .expect("sources")
            .push(original["sources"][0].clone());
        assert!(Supplement::parse(&duplicate.to_string()).is_err());
        let mut back_face = original;
        back_face["cards"][0]["stats_pointer"] = serde_json::json!(format!(
            "{}/card_faces/1",
            back_face["cards"][0]["source_pointer"]
                .as_str()
                .expect("source pointer")
        ));
        assert!(Supplement::parse(&back_face.to_string()).is_err());
    }

    /// Explicit empty identities and existing metadata remain unchanged.
    #[test]
    fn explicit_colorless_identity_is_not_missing_metadata() {
        use super::*;

        let mut oracle_id = None;
        let mut identity = Some(Vec::new());
        assert!(
            !enrich("Declared colorless fixture", &mut oracle_id, &mut identity)
                .expect("explicit identity")
        );
        assert_eq!(identity, Some(Vec::new()));
        assert!(oracle_id.is_none());
        let mut identity = Some(vec!["W".to_string()]);
        assert!(
            !enrich("Kenrith, the Returned King", &mut oracle_id, &mut identity)
                .expect("existing identity")
        );
        assert_eq!(identity, Some(vec!["W".to_string()]));
        assert!(oracle_id.is_none());
    }

    /// Duplicate identities preserve explicit colorless values and report actual changes.
    #[test]
    fn duplicate_identity_enrichment_reports_only_missing_fields() {
        use super::*;

        let mut oracle_id = None;
        let mut identity = Some(Vec::new());
        assert!(
            !inherit(
                "Colorless commander",
                &mut oracle_id,
                &mut identity,
                None,
                &[]
            )
            .expect("explicit colorless identity")
        );
        assert_eq!(identity, Some(Vec::new()));
        assert!(
            inherit(
                "Colorless commander",
                &mut oracle_id,
                &mut identity,
                Some("pinned-id"),
                &[]
            )
            .expect("missing Oracle identity")
        );
        assert_eq!(oracle_id.as_deref(), Some("pinned-id"));
        assert!(
            !inherit(
                "Colorless commander",
                &mut oracle_id,
                &mut identity,
                Some("pinned-id"),
                &[]
            )
            .expect("duplicate enrichment is idempotent")
        );
    }

    /// Missing source records and conflicting stored Oracle IDs fail without partial changes.
    #[test]
    fn missing_records_and_conflicting_oracle_ids_are_errors() {
        use super::*;

        let mut oracle_id = None;
        let mut identity = None;
        assert!(enrich("Unpinned commander", &mut oracle_id, &mut identity).is_err());
        assert!(oracle_id.is_none() && identity.is_none());
        oracle_id = Some("different-oracle-identity".into());
        assert!(enrich("Shorikai, Genesis Engine", &mut oracle_id, &mut identity).is_err());
        assert_eq!(oracle_id.as_deref(), Some("different-oracle-identity"));
        assert!(identity.is_none());
    }

    /// Stored identities include colors that do not appear in a commander's casting cost.
    #[test]
    fn enrichment_uses_the_complete_pinned_identity() {
        use super::*;

        let mut oracle_id = None;
        let mut identity = None;
        assert!(
            enrich("Kenrith, the Returned King", &mut oracle_id, &mut identity)
                .expect("pinned commander")
        );
        assert_eq!(
            identity,
            Some(["B", "G", "R", "U", "W"].map(str::to_string).to_vec())
        );
        assert_eq!(
            oracle_id.as_deref(),
            Some("d209b948-9afb-4fd1-a961-72c87282878c")
        );
    }

    /// Printed strings remain exact, and fields with no printed stat stay absent.
    #[test]
    fn numeric_enrichment_preserves_explicit_and_variable_strings() {
        use super::*;

        let mut power = Some("*".to_string());
        let mut toughness = None;
        let mut loyalty = None;
        assert!(
            enrich_stats(
                "Shorikai, Genesis Engine",
                None,
                &mut power,
                &mut toughness,
                &mut loyalty
            )
            .expect("pinned Vehicle stats")
        );
        assert_eq!(power.as_deref(), Some("*"));
        assert_eq!(toughness.as_deref(), Some("8"));
        assert!(loyalty.is_none());
        assert!(
            !enrich_stats(
                "Shorikai, Genesis Engine",
                None,
                &mut power,
                &mut toughness,
                &mut loyalty
            )
            .expect("enrichment is idempotent")
        );
        power = None;
        toughness = Some("1+*".to_string());
        loyalty = Some("0".to_string());
        assert!(
            enrich_stats(
                "Lumra, Bellow of the Woods",
                None,
                &mut power,
                &mut toughness,
                &mut loyalty
            )
            .expect("pinned variable stats")
        );
        assert_eq!(power.as_deref(), Some("*"));
        assert_eq!(toughness.as_deref(), Some("1+*"));
        assert_eq!(loyalty.as_deref(), Some("0"));
    }

    /// Double-faced cards use only their pinned front face, never the back face.
    #[test]
    fn numeric_enrichment_uses_only_front_face_stats() {
        use super::*;

        let mut power = None;
        let mut toughness = None;
        let mut loyalty = None;
        assert!(
            !enrich_stats(
                "Spring-Loaded Sawblades // Bladewheel Chariot",
                None,
                &mut power,
                &mut toughness,
                &mut loyalty
            )
            .expect("front face has no stats")
        );
        assert!(power.is_none() && toughness.is_none() && loyalty.is_none());
        assert!(
            enrich_stats(
                "Etali, Primal Conqueror // Etali, Primal Sickness",
                None,
                &mut power,
                &mut toughness,
                &mut loyalty
            )
            .expect("pinned front face")
        );
        assert_eq!(power.as_deref(), Some("7"));
        assert_eq!(toughness.as_deref(), Some("7"));
        assert!(loyalty.is_none());
    }

    /// Missing sources and mismatched Oracle IDs cannot partly enrich numeric fields.
    #[test]
    fn numeric_enrichment_rejects_missing_sources_and_oracle_conflicts() {
        use super::*;

        let mut power = None;
        let mut toughness = None;
        let mut loyalty = None;
        for (name, oracle_id) in [
            ("Unpinned Vehicle", None),
            ("Shorikai, Genesis Engine", Some("wrong-oracle-id")),
        ] {
            assert!(
                enrich_stats(name, oracle_id, &mut power, &mut toughness, &mut loyalty).is_err()
            );
            assert!(power.is_none() && toughness.is_none() && loyalty.is_none());
        }
    }
}
