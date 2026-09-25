//! Shared query-evaluation input, metrics, and corpus identity helpers.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context as _;
use seizethemana::{cli, db, search};
use serde::{Deserialize, Serialize};

/// Version of the currently curated positive and negative query labels.
pub const LABEL_VERSION: &str = "2026-09-25.9";

/// One benchmark query and its explicitly reviewed card labels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenQuery {
    /// Search text as a user would enter it.
    pub query: String,
    /// Cards labeled relevant.
    pub relevant: Vec<String>,
    /// Cards explicitly reviewed and labeled not relevant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_relevant: Vec<String>,
    /// Filters that apply to both search legs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<GoldenFilters>,
    /// Short note about the query intent.
    #[serde(default)]
    pub note: String,
    /// Optional explicit group. Missing groups are inferred from filters and notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// Filter fields accepted by `stm query`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenFilters {
    /// Type-line substring.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    /// Allowed colors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Allowed color identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_identity: Option<String>,
    /// Converted mana cost comparison.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmc: Option<String>,
    /// Power comparison.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<String>,
    /// Toughness comparison.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toughness: Option<String>,
    /// Rarity filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity: Option<String>,
    /// Set-code filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Keyword substring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    /// Oracle-text substring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_text: Option<String>,
    /// Format legality filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

impl GoldenFilters {
    /// Build the same CLI filter struct used by `stm query`.
    pub fn to_cli(&self) -> cli::CardFilters {
        cli::CardFilters {
            type_: self.type_.clone(),
            color: self.color.clone(),
            color_identity: self.color_identity.clone(),
            cmc: self.cmc.clone(),
            power: self.power.clone(),
            toughness: self.toughness.clone(),
            rarity: self.rarity.clone(),
            set: self.set.clone(),
            keyword: self.keyword.clone(),
            oracle_text: self.oracle_text.clone(),
            format: self.format.clone(),
        }
    }

    /// Convert CLI-compatible filters into library search filters.
    pub fn to_search(&self) -> anyhow::Result<search::CardFilters> {
        search::CardFilters::from_cli(&self.to_cli())
    }

    /// True when no filter field is set.
    pub fn is_empty(&self) -> bool {
        self.type_.is_none()
            && self.color.is_none()
            && self.color_identity.is_none()
            && self.cmc.is_none()
            && self.power.is_none()
            && self.toughness.is_none()
            && self.rarity.is_none()
            && self.set.is_none()
            && self.keyword.is_none()
            && self.oracle_text.is_none()
            && self.format.is_none()
    }
}

impl GoldenQuery {
    /// Return validated library filters for this query.
    pub fn search_filters(&self) -> anyhow::Result<search::CardFilters> {
        self.filters
            .as_ref()
            .map(GoldenFilters::to_search)
            .unwrap_or_else(|| Ok(search::CardFilters::default()))
    }

    /// Query group used in aggregate reports.
    pub fn group(&self) -> String {
        if let Some(group) = &self.group {
            return group.clone();
        }
        if self.relevant.is_empty() {
            return "behavior".to_string();
        }
        if self
            .filters
            .as_ref()
            .is_some_and(|filters| !filters.is_empty())
        {
            return "filtered".to_string();
        }
        let note = self.note.to_ascii_lowercase();
        if note.contains("mechanic") || note.contains("keyword") {
            "mechanic".to_string()
        } else if note.contains("name") || note.contains("prefix") || note.contains("article") {
            "exact_name".to_string()
        } else {
            "role_intent".to_string()
        }
    }

    /// Fixed split rule: filtered queries are held out; behavior probes are excluded.
    pub fn split(&self) -> &'static str {
        if self.relevant.is_empty() {
            "behavior"
        } else if self
            .filters
            .as_ref()
            .is_some_and(|filters| !filters.is_empty())
        {
            "held_out"
        } else {
            "development"
        }
    }
}

/// Binary retrieval metrics at one cutoff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Fraction of labeled relevant cards returned.
    pub recall: f64,
    /// Reciprocal rank of the first labeled relevant card.
    pub mrr: f64,
    /// Normalized discounted cumulative gain.
    pub ndcg: f64,
}

/// Score a ranked card list against the query's positive labels.
pub fn score(ranked: &[String], query: &GoldenQuery, cutoff: usize) -> Metrics {
    let relevant: HashSet<&str> = query.relevant.iter().map(String::as_str).collect();
    if relevant.is_empty() {
        return Metrics::default();
    }
    let window = ranked.iter().take(cutoff);
    let hits = window
        .clone()
        .filter(|name| relevant.contains(name.as_str()))
        .count();
    let first = ranked
        .iter()
        .take(cutoff)
        .position(|name| relevant.contains(name.as_str()));
    let discount = |position: usize| 1.0 / (position as f64 + 2.0).log2();
    let dcg = ranked
        .iter()
        .take(cutoff)
        .enumerate()
        .filter(|(_, name)| relevant.contains(name.as_str()))
        .map(|(position, _)| discount(position))
        .sum::<f64>();
    let ideal_hits = relevant.len().min(cutoff);
    let idcg = (0..ideal_hits).map(discount).sum::<f64>();
    Metrics {
        recall: hits as f64 / relevant.len() as f64,
        mrr: first.map_or(0.0, |position| 1.0 / (position + 1) as f64),
        ndcg: if idcg == 0.0 { 0.0 } else { dcg / idcg },
    }
}

/// Load and validate the JSON benchmark file.
pub fn load_queries(path: &Path) -> anyhow::Result<Vec<GoldenQuery>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading golden queries from {}", path.display()))?;
    let queries: Vec<GoldenQuery> = serde_json::from_str(&raw)
        .with_context(|| format!("parsing golden queries from {}", path.display()))?;
    anyhow::ensure!(!queries.is_empty(), "golden query set is empty");
    Ok(queries)
}

/// Ensure each explicit label exists and can pass its query's filters.
pub fn validate_labels(queries: &[GoldenQuery], cards: &[db::CardRow]) -> anyhow::Result<()> {
    let by_name: std::collections::HashMap<&str, &db::CardRow> = cards
        .iter()
        .map(|card| (card.name.as_str(), card))
        .collect();
    let mut problems = Vec::new();
    for (index, query) in queries.iter().enumerate() {
        let query_index = index + 1;
        let filters = query.search_filters()?;
        let positives: HashSet<&str> = query.relevant.iter().map(String::as_str).collect();
        let negatives: HashSet<&str> = query.not_relevant.iter().map(String::as_str).collect();
        if !positives.is_disjoint(&negatives) {
            problems.push(format!(
                "golden query {query_index} labels a card both relevant and not relevant"
            ));
        }
        for name in positives.iter().chain(&negatives) {
            match by_name.get(name) {
                None => problems.push(format!(
                    "golden query {query_index} labels missing card {name:?}"
                )),
                Some(card) if !filters.matches(*card) => problems.push(format!(
                    "golden query {query_index} ({:?}) labels {name:?}, but its filters exclude that card",
                    query.query
                )),
                Some(_) => {}
            }
        }
    }
    anyhow::ensure!(
        problems.is_empty(),
        "golden labels do not match the current store:\n{}",
        problems.join("\n")
    );
    Ok(())
}

/// Stable 64-bit fingerprint over ordered strings, including field boundaries.
pub fn fingerprint_strings<'a>(strings: impl IntoIterator<Item = &'a str>) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for value in strings {
        for byte in (value.len() as u64)
            .to_le_bytes()
            .iter()
            .chain(value.as_bytes())
        {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Fingerprint the ordered query set, including intent, labels, and filters.
pub fn fingerprint_queries(queries: &[GoldenQuery]) -> anyhow::Result<String> {
    let encoded = serde_json::to_string(queries).context("serializing golden queries")?;
    Ok(fingerprint_strings([encoded.as_str()]))
}

/// Collect unique ranked names from all retrieval legs in encounter order.
pub fn pool_names<'a>(lists: impl IntoIterator<Item = &'a [String]>) -> Vec<String> {
    let mut seen = HashSet::new();
    lists
        .into_iter()
        .flat_map(|list| list.iter())
        .filter(|name| seen.insert(name.as_str()))
        .cloned()
        .collect()
}

/// Round a metric to three decimal places for reports.
pub fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_json_uses_all_cli_fields_and_rejects_unknown_fields() {
        let filters: GoldenFilters =
            serde_json::from_str(r#"{"type":"Creature","color_identity":"WU","cmc":">=3"}"#)
                .unwrap();
        let cli_filters = filters.to_cli();
        assert_eq!(cli_filters.type_.as_deref(), Some("Creature"));
        assert_eq!(cli_filters.color_identity.as_deref(), Some("WU"));
        assert_eq!(cli_filters.cmc.as_deref(), Some(">=3"));
        assert!(serde_json::from_str::<GoldenFilters>(r#"{"unsupported":"x"}"#).is_err());
    }

    #[test]
    fn query_indices_are_not_used_as_query_identity() {
        let queries: Vec<GoldenQuery> = serde_json::from_str(
            r#"[
                {"query":"bolt","relevant":["A"]},
                {"query":"bolt","relevant":["B"],"filters":{"rarity":"common"}}
            ]"#,
        )
        .unwrap();
        assert_eq!(queries[0].query, queries[1].query);
        assert_ne!(queries[0].filters.is_some(), queries[1].filters.is_some());
        assert_eq!(queries[0].split(), "development");
        assert_eq!(queries[1].split(), "held_out");
    }

    #[test]
    fn fingerprint_includes_order_and_boundaries() {
        assert_ne!(
            fingerprint_strings(["ab", "c"]),
            fingerprint_strings(["a", "bc"])
        );
        assert_ne!(
            fingerprint_strings(["a", "b"]),
            fingerprint_strings(["b", "a"])
        );
    }

    #[test]
    fn checked_in_query_set_has_the_fixed_filtered_holdout() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/golden_queries.json");
        let queries = load_queries(&path).unwrap();
        assert_eq!(queries.len(), 59);
        assert_eq!(
            queries
                .iter()
                .filter(|query| query
                    .filters
                    .as_ref()
                    .is_some_and(|filters| !filters.is_empty()))
                .count(),
            7
        );
        assert_eq!(
            queries
                .iter()
                .filter(|query| query.split() == "held_out")
                .count(),
            7
        );
        assert_eq!(
            queries
                .iter()
                .filter(|query| query.split() == "behavior")
                .count(),
            2
        );
        for group in ["exact_name", "mechanic", "role_intent", "filtered"] {
            assert!(
                queries.iter().any(|query| query.group() == group),
                "{group}"
            );
        }
    }
}
