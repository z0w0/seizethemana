// Check that two query-evaluation reports use the same benchmark inputs.
//
//   cargo run --release --example query_report_check -- --mode model first.json second.json

use std::path::Path;

use anyhow::Context as _;
use serde_json::Value;

/// Benchmark identity fields required for a valid report comparison.
#[derive(Debug, PartialEq, Eq)]
struct ReportIdentity {
    evaluation_version: String,
    label_version: String,
    corpus_fingerprint: String,
    query_fingerprint: String,
    ranking_settings: Value,
    candidates: Vec<Value>,
}

/// Read one JSON report and extract its version and benchmark identity.
fn identity(path: &Path) -> anyhow::Result<ReportIdentity> {
    let report: Value = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    let benchmark = report.get("benchmark").unwrap_or(&report);
    let corpus = report.get("corpus").unwrap_or(&report);
    let get = |key: &str, nested: &Value| -> anyhow::Result<String> {
        nested
            .get(key)
            .or_else(|| report.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
            .with_context(|| format!("{} is missing {key}; regenerate the report", path.display()))
    };
    Ok(ReportIdentity {
        evaluation_version: report
            .get("evaluation_version")
            .and_then(Value::as_str)
            .unwrap_or("cli-baseline-v1")
            .to_string(),
        label_version: get("label_version", benchmark)?,
        corpus_fingerprint: get("corpus_fingerprint", benchmark)
            .or_else(|_| get("fingerprint", corpus))?,
        query_fingerprint: get("query_fingerprint", benchmark)
            .or_else(|_| get("query_fingerprint", corpus))?,
        ranking_settings: report
            .get("ranking_settings")
            .or_else(|| report.get("ranking_defaults"))
            .context("report has no ranking settings")?
            .clone(),
        candidates: report
            .get("candidates")
            .and_then(Value::as_array)
            .context("report has no candidate results")?
            .clone(),
    })
}

/// Check the candidate fields that must stay fixed for one experiment type.
fn candidate_settings_match(mode: &str, first: &[Value], second: &[Value]) -> anyhow::Result<()> {
    anyhow::ensure!(
        !first.is_empty() && !second.is_empty(),
        "both reports need candidate results"
    );
    let fields: &[&str] = match mode {
        "model" => &["document_format"],
        "format" => &[
            "model",
            "model_fingerprint",
            "effective_backend",
            "dimension",
            "max_length",
            "document_prompt",
            "query_prompt",
            "pooling",
        ],
        "ranking" => &[
            "key",
            "model",
            "model_fingerprint",
            "effective_backend",
            "dimension",
            "max_length",
            "document_prompt",
            "query_prompt",
            "pooling",
            "document_format",
        ],
        _ => anyhow::bail!("mode must be model, format, or ranking"),
    };
    if mode != "model" {
        anyhow::ensure!(
            first.len() == 1 && second.len() == 1,
            "format and ranking checks need one candidate per report"
        );
    }
    let reference = &first[0];
    for candidate in first.iter().chain(second) {
        for field in fields {
            anyhow::ensure!(
                candidate.get(field) == reference.get(field),
                "candidate field {field} differs; rerun with aligned settings"
            );
        }
    }
    Ok(())
}

/// Verify report identities before comparing their quality measurements.
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 4 && args[0] == "--mode",
        "usage: query_report_check --mode model|format|ranking <first.json> <second.json>"
    );
    let mode = args[1].as_str();
    let first = identity(Path::new(&args[2]))?;
    let second = identity(Path::new(&args[3]))?;
    anyhow::ensure!(
        first.evaluation_version == second.evaluation_version,
        "evaluation versions differ: {} vs {}",
        first.evaluation_version,
        second.evaluation_version
    );
    anyhow::ensure!(
        first.label_version == second.label_version,
        "label versions differ: {} vs {}",
        first.label_version,
        second.label_version
    );
    anyhow::ensure!(
        first.corpus_fingerprint == second.corpus_fingerprint,
        "corpus fingerprints differ"
    );
    anyhow::ensure!(
        first.query_fingerprint == second.query_fingerprint,
        "query-set fingerprints differ"
    );
    candidate_settings_match(mode, &first.candidates, &second.candidates)?;
    if mode != "ranking" {
        anyhow::ensure!(
            first.ranking_settings == second.ranking_settings,
            "ranking settings differ"
        );
    }
    println!(
        "Compatible {mode} comparison: {} · labels {} · corpus {} · queries {}",
        first.evaluation_version,
        first.label_version,
        first.corpus_fingerprint,
        first.query_fingerprint
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(format: &str, model: &str) -> Value {
        serde_json::json!({
            "key": model,
            "model": model,
            "model_fingerprint": "weights-a",
            "effective_backend": "cpu",
            "dimension": 384,
            "max_length": 256,
            "document_prompt": "doc: ",
            "query_prompt": "query: ",
            "pooling": "mean",
            "document_format": format,
        })
    }

    #[test]
    fn model_comparison_requires_same_document_format() {
        assert!(
            candidate_settings_match(
                "model",
                &[candidate("production", "bge")],
                &[candidate("production", "e5")]
            )
            .is_ok()
        );
        assert!(
            candidate_settings_match(
                "model",
                &[candidate("production", "bge")],
                &[candidate("fielded", "e5")]
            )
            .is_err()
        );
    }

    #[test]
    fn format_comparison_requires_same_model_contract() {
        assert!(
            candidate_settings_match(
                "format",
                &[candidate("production", "bge")],
                &[candidate("fielded", "bge")]
            )
            .is_ok()
        );
        assert!(
            candidate_settings_match(
                "format",
                &[candidate("production", "bge")],
                &[candidate("fielded", "e5")]
            )
            .is_err()
        );
    }

    #[test]
    fn ranking_comparison_requires_same_candidate_configuration() {
        assert!(
            candidate_settings_match(
                "ranking",
                &[candidate("production", "bge")],
                &[candidate("production", "bge")]
            )
            .is_ok()
        );
        assert!(
            candidate_settings_match(
                "ranking",
                &[candidate("production", "bge")],
                &[candidate("fielded", "bge")]
            )
            .is_err()
        );
    }

    #[test]
    fn ranking_comparison_rejects_different_weights_or_backend() {
        let cpu = candidate("production", "bge");
        let mut gpu = cpu.clone();
        gpu["effective_backend"] = "coreml".into();
        assert!(candidate_settings_match("ranking", &[cpu.clone()], &[gpu]).is_err());
        let mut changed_weights = cpu.clone();
        changed_weights["model_fingerprint"] = "weights-b".into();
        assert!(candidate_settings_match("ranking", &[cpu], &[changed_weights]).is_err());
    }
}
