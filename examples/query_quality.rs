// Query quality report: measure recall quality and speed of the hybrid
// search against a real store.
//
//   cargo build --release && ./target/release/stm setup   # once
//   cargo run --release --example query_quality
//
// Every golden query runs three legs (FTS-only, vector-only, hybrid) plus
// warm latency samples per pipeline stage. Store location: the default data
// directory, or `STM_DATA_DIR`. Output is a plain table on stdout; `--json`
// emits the full per-query breakdown.
//
// Quality metrics: Recall@k, MRR@k, nDCG@k with binary relevance labels
// from `benches/golden_queries.json` (`benches/golden_queries.json` is the
// living file; refresh labels when the corpus changes materially).

use std::path::PathBuf;

use seizethemana::db::CardRow;
use seizethemana::paths::Paths;
use seizethemana::quality::{
    GoldenQuery, LegScore, latency_stats, load_golden_set, mean_leg_score, score_ranked,
};
use seizethemana::{db, embed, output::Output, query};

const CUTOFF: usize = 10;
const LATENCY_SAMPLES: usize = 15;

fn data_dir() -> PathBuf {
    std::env::var_os("STM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").expect("HOME set")).join(".seizethemana")
        })
}

/// Rank the full-text leg alone (same query building as `run_search`,
/// including the expansion alias).
fn fts_leg(
    conn: &rusqlite::Connection,
    cards: &[CardRow],
    names_by_id: &std::collections::HashMap<i64, usize>,
    text: &str,
    depth: usize,
) -> anyhow::Result<Vec<String>> {
    let expanded = query::expanded_text(text);
    let Some(expr) = db::fts_query(&expanded) else {
        return Ok(Vec::new());
    };
    Ok(db::fts_search(conn, &expr, depth)?
        .into_iter()
        .filter_map(|(id, _)| names_by_id.get(&id).map(|i| cards[*i].name.clone()))
        .collect())
}

/// One timed run of each pipeline stage for one query.
struct StageTimings {
    embed: std::time::Duration,
    vector: std::time::Duration,
    fts: std::time::Duration,
    fuse: std::time::Duration,
}

/// Ranked names per leg plus the stage timings for one query run.
type LeggedRun = (Vec<String>, Vec<String>, Vec<String>, StageTimings);

fn timed_stages(
    store: &embed::VectorStore,
    model: &mut fastembed::TextEmbedding,
    conn: &rusqlite::Connection,
    cards: &[CardRow],
    names_by_id: &std::collections::HashMap<i64, usize>,
    q: &GoldenQuery,
    depth: usize,
) -> anyhow::Result<LeggedRun> {
    let text = q.query.as_str();

    let t0 = std::time::Instant::now();
    let query_vec = store.embed_query(model, &query::expanded_text(text))?;
    let embed = t0.elapsed();

    let t0 = std::time::Instant::now();
    let mut scored: Vec<(usize, f32)> = (0..cards.len().min(store.meta.names.len()))
        .map(|i| {
            let row = store.row(i);
            let score: f32 = query_vec.iter().zip(row).map(|(q, v)| q * v).sum();
            (i, score)
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    let vector_ranked: Vec<String> = scored
        .into_iter()
        .take(depth)
        .map(|(i, _)| cards[i].name.clone())
        .collect();
    let vector = t0.elapsed();

    let t0 = std::time::Instant::now();
    let fts_ranked = fts_leg(conn, cards, names_by_id, text, depth)?;
    let fts = t0.elapsed();

    let t0 = std::time::Instant::now();
    let hybrid = query::fuse_rrf(
        &fts_ranked
            .iter()
            .map(|n| (n.clone(), 0.0))
            .collect::<Vec<_>>(),
        &vector_ranked
            .iter()
            .map(|n| (n.clone(), 0.0))
            .collect::<Vec<_>>(),
        CUTOFF,
        |_| None,
    )
    .into_iter()
    .map(|(name, _)| name)
    .collect::<Vec<String>>();
    let fuse = t0.elapsed();

    Ok((
        fts_ranked,
        vector_ranked,
        hybrid,
        StageTimings {
            embed,
            vector,
            fts,
            fuse,
        },
    ))
}

fn main() -> anyhow::Result<()> {
    let json = std::env::args().any(|a| a == "--json");
    let dir = data_dir();
    anyhow::ensure!(
        dir.join("vectors.bin").is_file() && dir.join("stm.db").is_file(),
        "no store at {} (run 'stm setup' in release first)",
        dir.display()
    );
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("benches/golden_queries.json");
    let golden = load_golden_set(&golden_path)?;

    let paths = Paths::new(dir.clone());
    let conn = db::open(&dir.join("stm.db"))?;
    let store = embed::VectorStore::load(paths.root())?;
    let mut model = embed::load_model(&paths.models_dir(), false)?;
    let mut out = Output::new(false, true, false);
    let cards = db::load_all_cards(&conn)?;
    let names_by_id: std::collections::HashMap<i64, usize> = cards
        .iter()
        .enumerate()
        .map(|(i, _)| (i as i64 + 1, i))
        .collect();
    let names: Vec<String> = cards.iter().map(|c| c.name.clone()).collect();
    let _ = &names;

    // Leg lists + scores per query, plus stage timings.
    struct QueryRun {
        query: String,
        note: String,
        legs: [(String, LegScore); 3],
        timings: Vec<StageTimings>,
    }
    let mut runs: Vec<QueryRun> = Vec::new();
    // Legs contribute at the production fusion depth, not the scoring
    // window: `run_search` fuses depth-20 lists for a --limit 10 query.
    let fusion_depth = query::fusion_depth(CUTOFF);
    for q in &golden {
        let relevant = &q.relevant;
        let (fts_ranked, vector_ranked, hybrid_ranked, first_timing) = timed_stages(
            &store,
            &mut model,
            &conn,
            &cards,
            &names_by_id,
            q,
            fusion_depth,
        )?;
        let mut timings = vec![first_timing];
        for _ in 1..LATENCY_SAMPLES {
            let (_, _, _, t) = timed_stages(
                &store,
                &mut model,
                &conn,
                &cards,
                &names_by_id,
                q,
                fusion_depth,
            )?;
            timings.push(t);
        }
        runs.push(QueryRun {
            query: q.query.clone(),
            note: q.note.clone(),
            legs: [
                ("fts".into(), score_ranked(&fts_ranked, relevant, CUTOFF)),
                (
                    "vector".into(),
                    score_ranked(&vector_ranked, relevant, CUTOFF),
                ),
                (
                    "hybrid".into(),
                    score_ranked(&hybrid_ranked, relevant, CUTOFF),
                ),
            ],
            timings,
        });
        let _ = &mut out;
    }

    let mut report = serde_json::json!({
        "cutoff_k": CUTOFF,
        "latency_samples": LATENCY_SAMPLES,
        "queries": runs.len(),
        "corpus_cards": cards.len(),
        "vector_rows": store.meta.names.len(),
        "model": store.meta.model,
    });

    // Aggregate quality per leg.
    let mut summary = serde_json::Map::new();
    for (leg_name, _) in &runs[0].legs {
        let scores: Vec<LegScore> = runs
            .iter()
            .map(|r| r.legs.iter().find(|(n, _)| n == leg_name).unwrap().1)
            .collect();
        let mean = mean_leg_score(&scores).expect("non-empty");
        summary.insert(
            leg_name.clone(),
            serde_json::json!({
                "recall": (mean.recall * 1000.0).round() / 1000.0,
                "mrr": (mean.mrr * 1000.0).round() / 1000.0,
                "ndcg": (mean.ndcg * 1000.0).round() / 1000.0,
            }),
        );
    }
    // Aggregate latency per stage.
    let mut stages = serde_json::Map::new();
    for (field, label) in [
        ("embed", "embed"),
        ("vector", "vector"),
        ("fts", "fts"),
        ("fuse", "fuse"),
    ] {
        let mut samples = Vec::new();
        for run in &runs {
            for t in &run.timings {
                let d = match field {
                    "embed" => t.embed,
                    "vector" => t.vector,
                    "fts" => t.fts,
                    _ => t.fuse,
                };
                samples.push(d);
            }
        }
        let stats = latency_stats(&samples)?;
        stages.insert(
            label.to_string(),
            serde_json::json!({
                "p50_us": stats.p50_us,
                "p95_us": stats.p95_us,
                "p99_us": stats.p99_us,
                "mean_us": stats.mean_us,
            }),
        );
    }
    report["stages"] = serde_json::Value::Object(stages);

    // Per-query breakdown (JSON output only).
    let per_query: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            serde_json::json!({
                "query": r.query,
                "note": r.note,
                "legs": r.legs.iter().map(|(n, s)| serde_json::json!({
                    "leg": n,
                    "recall": (s.recall * 1000.0).round() / 1000.0,
                    "mrr": (s.mrr * 1000.0).round() / 1000.0,
                    "ndcg": (s.ndcg * 1000.0).round() / 1000.0,
                })).collect::<Vec<_>>(),
                "latency_us": {
                    "embed": (r.timings.iter().map(|t| t.embed.as_micros() as u64).sum::<u64>() / r.timings.len() as u64),
                    "vector": (r.timings.iter().map(|t| t.vector.as_micros() as u64).sum::<u64>() / r.timings.len() as u64),
                    "fts": (r.timings.iter().map(|t| t.fts.as_micros() as u64).sum::<u64>() / r.timings.len() as u64),
                    "fuse": (r.timings.iter().map(|t| t.fuse.as_micros() as u64).sum::<u64>() / r.timings.len() as u64),
                },
            })
        })
        .collect();
    report["per_query"] = serde_json::Value::Array(per_query);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    // Human view: summary table + stage latencies.
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Query quality"),
        styles.dim(&format!(
            "k={CUTOFF} · {} queries · {} cards · {} vector rows",
            runs.len(),
            cards.len(),
            store.meta.names.len()
        ))
    );
    println!("{:<10} {:>8} {:>8} {:>8}", "leg", "recall", "mrr", "ndcg");
    for (leg_name, _) in &runs[0].legs {
        let scores: Vec<LegScore> = runs
            .iter()
            .map(|r| r.legs.iter().find(|(n, _)| n == leg_name).unwrap().1)
            .collect();
        let mean = mean_leg_score(&scores).expect("non-empty");
        println!(
            "{:<10} {:>8.3} {:>8.3} {:>8.3}",
            leg_name, mean.recall, mean.mrr, mean.ndcg
        );
    }
    println!();
    println!(
        "{:<10} {:>10} {:>10} {:>10} {:>10}",
        "stage", "p50", "p95", "p99", "mean"
    );
    for label in ["embed", "vector", "fts", "fuse"] {
        let s = &report["stages"][label];
        println!(
            "{:<10} {:>8.1}ms {:>8.1}ms {:>8.1}ms {:>8.1}ms",
            label,
            s["p50_us"].as_u64().unwrap_or(0) as f64 / 1000.0,
            s["p95_us"].as_u64().unwrap_or(0) as f64 / 1000.0,
            s["p99_us"].as_u64().unwrap_or(0) as f64 / 1000.0,
            s["mean_us"].as_u64().unwrap_or(0) as f64 / 1000.0,
        );
    }
    let worst: Vec<&QueryRun> = runs
        .iter()
        .filter(|r| r.legs[2].1.ndcg == 0.0 && !r.legs[0].1.recall.is_nan())
        .take(5)
        .collect();
    if !worst.is_empty() {
        println!();
        println!("{}", styles.dim("zero-recall queries (hybrid leg):"));
        for r in &worst {
            println!("  - {:?} ({})", r.query, r.note);
        }
    }
    Ok(())
}
