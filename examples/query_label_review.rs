// Export unjudged search results for agent review and apply approved decisions.
//
//   cargo run --release --example query_label_review -- --export --save batch.json
//   # Give batch.json to an agent and ask it to return the `response` JSON shape.
//   cargo run --release --example query_label_review -- --review batch.json --response response.json
//   cargo run --release --example query_label_review -- --apply batch.json --response response.json --approve --approval-note "Reviewed batch 1"

/// Shared golden-query data and metric helpers.
// This example uses only the query schemas and validation helpers from the shared module.
#[allow(dead_code)]
mod common;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use seizethemana::paths::Paths;
use seizethemana::{db, tags};
use serde::{Deserialize, Serialize};

const GOLDENS: &str = "benches/golden_queries.json";
const AUDIT: &str = "benches/query_label_review_audit.jsonl";
const BAKEOFF_REPORTS: &[&str] = &["benches/query_bakeoff.json"];

/// Minimal baseline report data used to build a review batch.
#[derive(Debug, Deserialize)]
struct BaselineSource {
    benchmark: BaselineBenchmark,
    queries: Vec<BaselineQuery>,
}

/// Baseline identities needed to reject stale review batches.
#[derive(Debug, Deserialize)]
struct BaselineBenchmark {
    label_version: String,
    corpus_fingerprint: String,
}

/// Search results and current labels for one golden query.
#[derive(Debug, Deserialize)]
struct BaselineQuery {
    query_index: usize,
    query: String,
    note: String,
    group: String,
    filters: Option<common::GoldenFilters>,
    relevant: Vec<String>,
    not_relevant: Vec<String>,
    unjudged_pool: Vec<String>,
    rankings: HashMap<String, Vec<String>>,
}

/// Model, format, and query results from one saved bake-off report.
#[derive(Debug, Deserialize)]
struct BakeoffReport {
    label_version: String,
    corpus: BakeoffCorpus,
    candidates: Vec<BakeoffCandidate>,
}

/// Corpus fingerprint copied from a bake-off report.
#[derive(Debug, Deserialize)]
struct BakeoffCorpus {
    fingerprint: String,
}

/// One model and document-format candidate.
#[derive(Debug, Deserialize)]
struct BakeoffCandidate {
    key: String,
    #[serde(default = "production_format")]
    document_format: String,
    queries: Vec<BakeoffQuery>,
}

/// Name the original bake-off's implicit production document format.
fn production_format() -> String {
    "production".to_string()
}

/// Results used to expand the candidate review pool.
#[derive(Debug, Deserialize)]
struct BakeoffQuery {
    query_index: usize,
    cli_baseline: Vec<String>,
    unjudged_pool: Vec<String>,
    fts: Vec<String>,
    vector: Vec<String>,
    hybrid: Vec<String>,
}

/// Candidate retrievals and report names used to build the review pool.
struct BakeoffSources {
    rankings: HashMap<(usize, String), Vec<RetrievalEvidence>>,
    reports: Vec<String>,
}

/// Agent input with evidence and stable identities for each card decision.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewBatch {
    schema_version: u32,
    batch_id: String,
    batch_index: usize,
    batch_count: usize,
    label_version: String,
    corpus_fingerprint: String,
    source_reports: Vec<String>,
    agent_instructions: String,
    response_schema: String,
    items: Vec<ReviewItem>,
}

/// One unjudged card paired with query intent and retrieval evidence.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewItem {
    item_id: String,
    query_index: usize,
    query: String,
    intent: String,
    group: String,
    filters: Option<common::GoldenFilters>,
    relevant_examples: Vec<String>,
    not_relevant_examples: Vec<String>,
    card: CardEvidence,
    retrieval: Vec<RetrievalEvidence>,
}

/// Card facts supplied to the reviewing agent.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CardEvidence {
    name: String,
    type_line: String,
    keywords: Vec<String>,
    tags: Vec<String>,
    oracle_text: String,
}

/// Search leg and rank that caused a card to enter the review pool.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrievalEvidence {
    source: String,
    rank: usize,
}

/// Agent response. Human approval is deliberately not an agent field.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentResponse {
    schema_version: u32,
    batch_id: String,
    label_version: String,
    corpus_fingerprint: String,
    decisions: Vec<Decision>,
}

/// One agent judgment and its concise evidence-based reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    item_id: String,
    decision: String,
    reason: String,
}

/// Resolve the application's data directory.
fn data_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".seizethemana"))
}

/// Return the project root used for checked-in benchmark files.
fn project_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// Return stable item identities using the query index and exact card name.
fn item_id(query_index: usize, card_name: &str) -> String {
    format!("q{query_index}:{}:{card_name}", card_name.len())
}

/// Return a deterministic batch identity for one label and corpus version.
fn batch_id(
    label_version: &str,
    fingerprint: &str,
    items: &[ReviewItem],
    batch_index: usize,
    batch_count: usize,
) -> String {
    let batch_index = batch_index.to_string();
    let batch_count = batch_count.to_string();
    let item_data = serde_json::to_string(items).expect("review items serialize to JSON");
    let values = std::iter::once(label_version)
        .chain(std::iter::once(fingerprint))
        .chain(std::iter::once(batch_index.as_str()))
        .chain(std::iter::once(batch_count.as_str()))
        .chain(std::iter::once(item_data.as_str()));
    format!("query-review-{}", common::fingerprint_strings(values))
}

/// Pool each saved candidate's unjudged hits and retain its leg-specific rank.
fn load_bakeoff_sources(
    label_version: &str,
    corpus_fingerprint: &str,
) -> anyhow::Result<BakeoffSources> {
    let mut sources: HashMap<(usize, String), Vec<RetrievalEvidence>> = HashMap::new();
    let mut source_reports = vec!["production reference".to_string()];
    for report_name in BAKEOFF_REPORTS {
        let report: BakeoffReport = read_json(&project_path(report_name))?;
        anyhow::ensure!(
            report.label_version == label_version,
            "{} uses label version {}, expected {}",
            report_name,
            report.label_version,
            label_version
        );
        anyhow::ensure!(
            report.corpus.fingerprint == corpus_fingerprint,
            "{} uses a different corpus fingerprint",
            report_name
        );
        for candidate in report.candidates {
            let run = format!("{}:{}", candidate.key, candidate.document_format);
            for query in candidate.queries {
                for name in query.unjudged_pool {
                    let entry = sources
                        .entry((query.query_index, name.clone()))
                        .or_default();
                    for (leg, ranked) in [
                        ("fts", &query.fts),
                        ("vector", &query.vector),
                        ("hybrid", &query.hybrid),
                    ] {
                        if let Some(rank) = ranked.iter().position(|hit| hit == &name) {
                            entry.push(RetrievalEvidence {
                                source: format!("{run}/{leg}"),
                                rank: rank + 1,
                            });
                        }
                    }
                }
            }
        }
        source_reports.push((*report_name).to_string());
    }
    Ok(BakeoffSources {
        rankings: sources,
        reports: source_reports,
    })
}

/// Build one agent batch from the CLI-faithful baseline and card store.
fn build_batch(
    source: BaselineSource,
    extra_sources: &HashMap<(usize, String), Vec<RetrievalEvidence>>,
    source_reports: Vec<String>,
    cards: &[db::CardRow],
    tag_index: &tags::TagIndex,
) -> anyhow::Result<ReviewBatch> {
    let cards_by_name: HashMap<&str, &db::CardRow> = cards
        .iter()
        .map(|card| (card.name.as_str(), card))
        .collect();
    let mut items = Vec::new();
    for query in source.queries {
        let judged: HashSet<&str> = query
            .relevant
            .iter()
            .chain(&query.not_relevant)
            .map(String::as_str)
            .collect();
        let mut candidate_names: HashSet<String> = query.unjudged_pool.into_iter().collect();
        candidate_names.extend(
            extra_sources
                .keys()
                .filter(|(index, _)| *index == query.query_index)
                .map(|(_, name)| name.clone()),
        );
        let mut candidate_names: Vec<String> = candidate_names.into_iter().collect();
        candidate_names.sort();
        for name in candidate_names {
            if judged.contains(name.as_str()) {
                continue;
            }
            let card = cards_by_name.get(name.as_str()).with_context(|| {
                format!(
                    "query {} references missing card {name:?}",
                    query.query_index
                )
            })?;
            let mut retrieval: Vec<RetrievalEvidence> = query
                .rankings
                .iter()
                .filter_map(|(source, names)| {
                    names
                        .iter()
                        .position(|candidate| candidate == &name)
                        .map(|rank| RetrievalEvidence {
                            source: source.clone(),
                            rank: rank + 1,
                        })
                })
                .collect();
            retrieval.extend(
                extra_sources
                    .get(&(query.query_index, name.clone()))
                    .into_iter()
                    .flatten()
                    .map(|source| RetrievalEvidence {
                        source: source.source.clone(),
                        rank: source.rank,
                    }),
            );
            retrieval
                .sort_by(|left, right| (&left.source, left.rank).cmp(&(&right.source, right.rank)));
            let keywords: Vec<String> = serde_json::from_str(&card.keywords)
                .context("parsing keywords from stored card")?;
            let tag_line = tag_index.doc_tags_line(&card.oracle_id);
            let tags = tag_line
                .strip_prefix("Tags:")
                .unwrap_or(&tag_line)
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .collect();
            items.push(ReviewItem {
                item_id: item_id(query.query_index, &card.name),
                query_index: query.query_index,
                query: query.query.clone(),
                intent: query.note.clone(),
                group: query.group.clone(),
                filters: query.filters.clone(),
                relevant_examples: query.relevant.clone(),
                not_relevant_examples: query.not_relevant.clone(),
                card: CardEvidence {
                    name: card.name.clone(),
                    type_line: card.type_line.clone(),
                    keywords,
                    tags,
                    oracle_text: card.oracle_text.clone(),
                },
                retrieval,
            });
        }
    }
    items.sort_by(|left, right| {
        (left.query_index, &left.card.name).cmp(&(right.query_index, &right.card.name))
    });
    let id = batch_id(
        &source.benchmark.label_version,
        &source.benchmark.corpus_fingerprint,
        &items,
        0,
        1,
    );
    Ok(ReviewBatch {
        schema_version: 1,
        batch_id: id,
        batch_index: 0,
        batch_count: 1,
        label_version: source.benchmark.label_version,
        corpus_fingerprint: source.benchmark.corpus_fingerprint,
        source_reports,
        agent_instructions: "For every item, decide whether the card satisfies the query intent under its filters. Use the full card text and reviewed examples. Exact-name queries require the intended card. For role queries, a word match alone is not evidence. Return relevant, not_relevant, or unsure with a short reason grounded in card facts. Do not add or omit items. Do not edit golden_queries.json.".to_string(),
        response_schema: r#"{"schema_version":1,"batch_id":"copy from batch","label_version":"copy from batch","corpus_fingerprint":"copy from batch","decisions":[{"item_id":"copy from item","decision":"relevant|not_relevant|unsure","reason":"short evidence-based reason"}]}"#.to_string(),
        items,
    })
}

/// Check all response identities, decisions, reasons, and item coverage.
fn validate_response(batch: &ReviewBatch, response: &AgentResponse) -> anyhow::Result<()> {
    anyhow::ensure!(
        batch.batch_id
            == batch_id(
                &batch.label_version,
                &batch.corpus_fingerprint,
                &batch.items,
                batch.batch_index,
                batch.batch_count,
            ),
        "review batch content does not match its batch ID"
    );
    anyhow::ensure!(
        response.schema_version == 1,
        "unsupported response schema version"
    );
    anyhow::ensure!(
        response.batch_id == batch.batch_id,
        "response batch ID does not match input batch"
    );
    anyhow::ensure!(
        response.label_version == batch.label_version,
        "response label version does not match input batch"
    );
    anyhow::ensure!(
        response.corpus_fingerprint == batch.corpus_fingerprint,
        "response corpus fingerprint does not match input batch"
    );
    let expected: HashSet<&str> = batch
        .items
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    let mut received = HashSet::new();
    for decision in &response.decisions {
        anyhow::ensure!(
            expected.contains(decision.item_id.as_str()),
            "unknown item ID {:?}",
            decision.item_id
        );
        anyhow::ensure!(
            received.insert(decision.item_id.as_str()),
            "duplicate decision for {:?}",
            decision.item_id
        );
        anyhow::ensure!(
            matches!(
                decision.decision.as_str(),
                "relevant" | "not_relevant" | "unsure"
            ),
            "invalid decision {:?}",
            decision.decision
        );
        anyhow::ensure!(
            !decision.reason.trim().is_empty(),
            "decision reason is empty for {:?}",
            decision.item_id
        );
    }
    anyhow::ensure!(
        received == expected,
        "response is incomplete: expected {} decisions, received {}",
        expected.len(),
        received.len()
    );
    Ok(())
}

/// Read strict JSON from a path with useful error context.
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> anyhow::Result<T> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))
}

/// Build the production baseline from the Core ML report and current labels.
fn load_baseline_source() -> anyhow::Result<BaselineSource> {
    let path = project_path(BAKEOFF_REPORTS[0]);
    let report: BakeoffReport = read_json(&path)?;
    anyhow::ensure!(
        report.label_version == common::LABEL_VERSION,
        "{} uses label version {}, expected {}; regenerate the report",
        path.display(),
        report.label_version,
        common::LABEL_VERSION
    );
    let candidate = report
        .candidates
        .first()
        .context("Core ML report has no candidate results")?;
    let golden = common::load_queries(&project_path(GOLDENS))?;
    let mut queries = Vec::with_capacity(golden.len());
    for (index, item) in golden.into_iter().enumerate() {
        let query_index = index + 1;
        let ranked = candidate
            .queries
            .iter()
            .find(|query| query.query_index == query_index)
            .with_context(|| format!("Core ML report has no production query {query_index}"))?
            .cli_baseline
            .clone();
        let judged: HashSet<&str> = item
            .relevant
            .iter()
            .chain(&item.not_relevant)
            .map(String::as_str)
            .collect();
        let unjudged_pool = ranked
            .iter()
            .filter(|name| !judged.contains(name.as_str()))
            .cloned()
            .collect();
        queries.push(BaselineQuery {
            query_index,
            query: item.query.clone(),
            note: item.note.clone(),
            group: item.group(),
            filters: item.filters.clone(),
            relevant: item.relevant,
            not_relevant: item.not_relevant,
            unjudged_pool,
            rankings: HashMap::from([("production".to_string(), ranked)]),
        });
    }
    Ok(BaselineSource {
        benchmark: BaselineBenchmark {
            label_version: report.label_version,
            corpus_fingerprint: report.corpus.fingerprint,
        },
        queries,
    })
}

/// Write one file through a sibling temporary file before replacing it.
fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)
        .with_context(|| format!("writing {}", temporary.display()))?;
    std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
}

/// Apply approved labels to the golden-query file and append an audit record.
fn apply_response(
    batch: &ReviewBatch,
    response: &AgentResponse,
    approval_note: &str,
) -> anyhow::Result<usize> {
    validate_response(batch, response)?;
    anyhow::ensure!(
        !approval_note.trim().is_empty(),
        "--approval-note must explain human approval"
    );
    let golden_path = project_path(GOLDENS);
    let mut queries = common::load_queries(&golden_path)?;
    anyhow::ensure!(
        common::LABEL_VERSION == batch.label_version,
        "stale batch: golden label version is {}, batch uses {}",
        common::LABEL_VERSION,
        batch.label_version
    );
    let baseline = load_baseline_source()?;
    anyhow::ensure!(
        baseline.benchmark.corpus_fingerprint == batch.corpus_fingerprint,
        "stale batch: corpus fingerprint changed"
    );
    let paths = Paths::new(data_dir()?);
    let conn = db::open(&paths.db())?;
    let cards = db::load_all_cards(&conn)?;
    common::validate_labels(&queries, &cards)?;
    let mut applied = 0;
    for decision in &response.decisions {
        if decision.decision == "unsure" {
            continue;
        }
        let item = batch
            .items
            .iter()
            .find(|item| item.item_id == decision.item_id)
            .context("validated item disappeared")?;
        let query = queries
            .get_mut(
                item.query_index
                    .checked_sub(1)
                    .context("query index is zero")?,
            )
            .context("batch query index is outside goldens")?;
        anyhow::ensure!(
            query.query == item.query && query.note == item.intent && query.filters == item.filters,
            "stale batch: query {} changed",
            item.query_index
        );
        let filters = query.search_filters()?;
        let card = cards
            .iter()
            .find(|card| card.name == item.card.name)
            .context("reviewed card no longer exists")?;
        anyhow::ensure!(
            filters.matches(card),
            "query {} filters exclude {}",
            item.query_index,
            item.card.name
        );
        let (target, opposite) = if decision.decision == "relevant" {
            (&mut query.relevant, &query.not_relevant)
        } else {
            (&mut query.not_relevant, &query.relevant)
        };
        anyhow::ensure!(
            !opposite.contains(&item.card.name),
            "conflicting existing label for query {} and {}",
            item.query_index,
            item.card.name
        );
        if !target.contains(&item.card.name) {
            target.push(item.card.name.clone());
            applied += 1;
        }
    }
    if applied > 0 {
        let golden_bytes = serde_json::to_vec_pretty(&queries)?;
        write_atomic(&golden_path, &golden_bytes)?;
    }
    let audit = serde_json::json!({
        "batch_id": batch.batch_id,
        "batch_index": batch.batch_index,
        "batch_count": batch.batch_count,
        "label_version": batch.label_version,
        "corpus_fingerprint": batch.corpus_fingerprint,
        "approval_note": approval_note,
        "applied_count": applied,
        "decisions": response.decisions,
    });
    let audit_path = project_path(AUDIT);
    let mut audit_bytes = std::fs::read(&audit_path).unwrap_or_default();
    audit_bytes.extend(serde_json::to_vec(&audit)?);
    audit_bytes.push(b'\n');
    write_atomic(&audit_path, &audit_bytes)?;
    Ok(applied)
}

/// Advance the code's label version after every approved batch is in the audit.
fn finalize_review(batch_count: usize, approval_note: &str) -> anyhow::Result<String> {
    anyhow::ensure!(batch_count > 0, "batch count must be greater than zero");
    anyhow::ensure!(
        !approval_note.trim().is_empty(),
        "--approval-note must explain human approval"
    );
    let audit_path = project_path(AUDIT);
    let audit_text = std::fs::read_to_string(&audit_path)
        .with_context(|| format!("reading approval audit {}", audit_path.display()))?;
    let mut approved_batches = HashSet::new();
    for line in audit_text.lines() {
        let entry: serde_json::Value =
            serde_json::from_str(line).context("parsing label-review audit row")?;
        if entry["label_version"] == common::LABEL_VERSION
            && entry["batch_count"].as_u64() == Some(batch_count as u64)
            && entry["approval_note"]
                .as_str()
                .is_some_and(|note| !note.trim().is_empty())
            && let Some(index) = entry["batch_index"].as_u64()
        {
            approved_batches.insert(index);
        }
    }
    anyhow::ensure!(
        approved_batches.len() == batch_count,
        "only {} of {batch_count} batches are approved for label version {}",
        approved_batches.len(),
        common::LABEL_VERSION
    );
    anyhow::ensure!(
        (0..batch_count).all(|index| approved_batches.contains(&(index as u64))),
        "approved batch indexes do not cover 0 through {}",
        batch_count - 1
    );
    let next_version = increment_label_version(common::LABEL_VERSION)?;
    let common_path = project_path("examples/common/mod.rs");
    let common_text = std::fs::read_to_string(&common_path)?;
    let old = format!(
        "pub const LABEL_VERSION: &str = \"{}\";",
        common::LABEL_VERSION
    );
    let new = format!("pub const LABEL_VERSION: &str = \"{next_version}\";");
    anyhow::ensure!(
        common_text.contains(&old),
        "could not locate label version declaration"
    );
    write_atomic(&common_path, common_text.replace(&old, &new).as_bytes())?;
    let entry = serde_json::json!({
        "action": "finalize",
        "from_label_version": common::LABEL_VERSION,
        "to_label_version": next_version,
        "batch_count": batch_count,
        "approval_note": approval_note,
    });
    let mut audit_bytes = std::fs::read(&audit_path)?;
    audit_bytes.extend(serde_json::to_vec(&entry)?);
    audit_bytes.push(b'\n');
    write_atomic(&audit_path, &audit_bytes)?;
    Ok(next_version)
}

/// Increment the numeric suffix of a date-based label version.
fn increment_label_version(version: &str) -> anyhow::Result<String> {
    let (date, suffix) = version
        .rsplit_once('.')
        .context("label version must end in .N")?;
    let suffix: u32 = suffix
        .parse()
        .context("label version suffix must be numeric")?;
    Ok(format!("{date}.{}", suffix + 1))
}

/// Print the proposed label changes without writing the goldens.
fn print_review(batch: &ReviewBatch, response: &AgentResponse) -> anyhow::Result<()> {
    validate_response(batch, response)?;
    for decision in &response.decisions {
        let item = batch
            .items
            .iter()
            .find(|item| item.item_id == decision.item_id)
            .context("validated item disappeared")?;
        println!(
            "{}\t{}\t{}\t{}",
            decision.decision, item.query_index, item.card.name, decision.reason
        );
    }
    Ok(())
}

/// Parse and run the explicit export, review, or approved apply mode.
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match args.first().map(String::as_str) {
        Some("--export") => {
            let output = argument_path(&args, "--save", "benches/query_label_review_batch.json")?;
            let batch_size = argument_usize(&args, "--batch-size", 100)?;
            anyhow::ensure!(batch_size > 0, "--batch-size must be greater than zero");
            let batch_index = argument_usize(&args, "--batch-index", 0)?;
            let paths = Paths::new(data_dir()?);
            anyhow::ensure!(
                paths.is_setup(),
                "no completed card store at {}",
                paths.root().display()
            );
            let conn = db::open(&paths.db())?;
            let cards = db::load_all_cards(&conn)?;
            let goldens = common::load_queries(&manifest.join(GOLDENS))?;
            common::validate_labels(&goldens, &cards)?;
            let tag_index = tags::TagIndex::load(&conn)?;
            let source = load_baseline_source()?;
            anyhow::ensure!(
                source.benchmark.label_version == common::LABEL_VERSION,
                "baseline labels are stale; regenerate the baseline first"
            );
            let extra_sources = load_bakeoff_sources(
                &source.benchmark.label_version,
                &source.benchmark.corpus_fingerprint,
            )?;
            let mut batch = build_batch(
                source,
                &extra_sources.rankings,
                extra_sources.reports,
                &cards,
                &tag_index,
            )?;
            let batch_count = batch.items.len().div_ceil(batch_size);
            anyhow::ensure!(
                batch_index < batch_count,
                "batch index {} is out of range; there are {} batches",
                batch_index,
                batch_count
            );
            let start = batch_index * batch_size;
            let end = (start + batch_size).min(batch.items.len());
            batch.items = batch
                .items
                .into_iter()
                .skip(start)
                .take(end - start)
                .collect();
            batch.batch_index = batch_index;
            batch.batch_count = batch_count;
            batch.batch_id = batch_id(
                &batch.label_version,
                &batch.corpus_fingerprint,
                &batch.items,
                batch.batch_index,
                batch.batch_count,
            );
            write_atomic(&output, &serde_json::to_vec_pretty(&batch)?)?;
            println!(
                "Exported batch {}/{} with {} review items, ID {}, to {}",
                batch_index + 1,
                batch_count,
                batch.items.len(),
                batch.batch_id,
                output.display()
            );
        }
        Some("--review") | Some("--apply") => {
            let applying = args[0] == "--apply";
            let batch_path = args.get(1).context("mode needs a batch path")?;
            let response_path = argument_path(&args, "--response", "")?;
            let batch: ReviewBatch = read_json(Path::new(batch_path))?;
            let response: AgentResponse = read_json(&response_path)?;
            if applying {
                anyhow::ensure!(
                    args.iter().any(|arg| arg == "--approve"),
                    "--apply requires explicit --approve after human review"
                );
                let note = argument_value(&args, "--approval-note")?
                    .context("--apply requires --approval-note")?;
                print_review(&batch, &response)?;
                let count = apply_response(&batch, &response, &note)?;
                println!(
                    "Applied {count} approved labels. Approve every batch before finalizing the label version."
                );
            } else {
                print_review(&batch, &response)?;
                println!(
                    "Dry run only: inspect every decision and reason. Apply only after human approval."
                );
            }
        }
        Some("--finalize") => {
            anyhow::ensure!(
                args.iter().any(|arg| arg == "--approve"),
                "--finalize requires explicit --approve after all batches are reviewed"
            );
            let count = argument_usize(&args, "--batch-count", 0)?;
            let note = argument_value(&args, "--approval-note")?
                .context("--finalize requires --approval-note")?;
            let next = finalize_review(count, &note)?;
            println!(
                "Finalized reviewed labels as version {next}. Regenerate all reports before comparing."
            );
        }
        _ => anyhow::bail!(
            "usage: query_label_review --export [--batch-size 100 --batch-index N --save batch.json] | --review batch.json --response response.json | --apply batch.json --response response.json --approve --approval-note TEXT | --finalize --batch-count N --approve --approval-note TEXT"
        ),
    }
    Ok(())
}

/// Return the path following one named option, or its default.
fn argument_path(args: &[String], key: &str, default: &str) -> anyhow::Result<PathBuf> {
    argument_value(args, key)
        .map(|value| value.map_or_else(|| PathBuf::from(default), PathBuf::from))
}

/// Return the value following one named option and reject missing values.
fn argument_value(args: &[String], key: &str) -> anyhow::Result<Option<String>> {
    match args.iter().position(|arg| arg == key) {
        Some(index) => Ok(Some(
            args.get(index + 1)
                .with_context(|| format!("{key} needs a value"))?
                .clone(),
        )),
        None => Ok(None),
    }
}

/// Parse a non-negative integer option or return its default.
fn argument_usize(args: &[String], key: &str, default: usize) -> anyhow::Result<usize> {
    argument_value(args, key)?.map_or(Ok(default), |value| {
        value
            .parse()
            .with_context(|| format!("{key} must be a positive integer"))
    })
}

#[cfg(test)]
#[path = "tests/query_label_review_tests.rs"]
mod tests;
