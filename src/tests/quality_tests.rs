use super::*;

fn ranked(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn relevant(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn recall_counts_top_window_hits() {
    let r = ranked(&["a", "b", "c", "d"]);
    assert_eq!(recall_at_k(&r, &relevant(&["b", "d"]), 3), 0.5);
    assert_eq!(recall_at_k(&r, &relevant(&["b"]), 2), 1.0);
    assert_eq!(recall_at_k(&r, &relevant(&["d"]), 3), 0.0);
    // Empty relevance stays 0: nothing to be right about.
    assert_eq!(recall_at_k(&r, &[], 5), 0.0);
}

#[test]
fn mrr_scores_first_relevant_rank() {
    let r = ranked(&["a", "b", "c"]);
    assert!((mrr_at_k(&r, &relevant(&["b"]), 3) - 0.5).abs() < 1e-9);
    assert!((mrr_at_k(&r, &relevant(&["c"]), 3) - (1.0 / 3.0)).abs() < 1e-9);
    assert_eq!(mrr_at_k(&r, &relevant(&["d"]), 3), 0.0);
    // Cutoff hides later hits.
    assert_eq!(mrr_at_k(&r, &relevant(&["c"]), 2), 0.0);
}

#[test]
fn ndcg_uses_binary_gain_and_ideal() {
    // Standard IR discount: 1/log2(rank + 2), rank 0-based.
    // One relevant doc at rank 2 (pos 1) of 3: DCG = 1/log2(3) = 0.631.
    let r = ranked(&["a", "rel", "c"]);
    assert!((ndcg_at_k(&r, &relevant(&["rel"]), 3) - 1.0 / 3f64.log2()).abs() < 1e-9);
    // Perfect ranking: nDCG 1.
    let r = ranked(&["rel", "a", "b"]);
    assert!((ndcg_at_k(&r, &relevant(&["rel"]), 3) - 1.0).abs() < 1e-9);
    // Both relevant docs found in the top slots: still 1.
    let r = ranked(&["rel1", "rel2", "a"]);
    assert!((ndcg_at_k(&r, &relevant(&["rel1", "rel2"]), 3) - 1.0).abs() < 1e-9);
    // Cutoff clips: one slot, one relevant doc found.
    let r = ranked(&["rel2", "rel1", "a"]);
    assert!((ndcg_at_k(&r, &relevant(&["rel1", "rel2"]), 1) - 1.0).abs() < 1e-9);
    assert_eq!(ndcg_at_k(&r, &[], 3), 0.0);
}

#[test]
fn mean_leg_score_averages_each_metric() {
    let legs = vec![
        LegScore {
            recall: 1.0,
            mrr: 1.0,
            ndcg: 1.0,
        },
        LegScore {
            recall: 0.0,
            mrr: 0.0,
            ndcg: 0.0,
        },
    ];
    let mean = mean_leg_score(&legs).unwrap();
    assert!((mean.recall - 0.5).abs() < 1e-9);
    assert!((mean.ndcg - 0.5).abs() < 1e-9);
    assert!(mean_leg_score(&[]).is_none());
}

#[test]
fn latency_stats_reports_percentiles() {
    // Samples are 100, 200, ..., 10000 µs (100 items, 0-indexed).
    let samples: Vec<std::time::Duration> = (1..=100)
        .map(|i| std::time::Duration::from_micros(i * 100))
        .collect();
    let stats = latency_stats(&samples).unwrap();
    // p50 hits index round(99*0.50)=50 → the 51st sample (5100 µs).
    assert_eq!(stats.p50_us, 5_100);
    assert_eq!(stats.p95_us, 9_500);
    assert_eq!(stats.p99_us, 9_900);
    assert_eq!(stats.mean_us, 5_050);
    assert_eq!(stats.samples, 100);
    assert!(latency_stats(&[]).is_err());
}

#[test]
fn golden_set_roundtrips_and_builds_filters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("golden.json");
    std::fs::write(
        &path,
        r#"[
        {"query": "bolt", "relevant": ["Lightning Bolt"],
         "filters": {"type": "Instant"}, "note": "exact name"},
        {"query": "sacrifice outlet", "relevant": ["Altar of Dementia"]}
      ]"#,
    )
    .unwrap();
    let set = load_golden_set(&path).unwrap();
    assert_eq!(set.len(), 2);
    assert_eq!(set[0].relevant, vec!["Lightning Bolt"]);
    let filters = set[0].filters();
    assert_eq!(filters.type_.as_deref(), Some("Instant"));
    assert!(set[1].filters().type_.is_none());
    assert!(set[0].filters().cmc.is_none());
}
