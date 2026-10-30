//! Query-path benchmarks against an explicit, disposable prepared store.
//!
//! Prepare a separate benchmark store in release mode. Debug builds embed too slowly:
//!
//!   cargo build --release
//!   ./target/release/stm --data-dir /path/to/disposable-bench-store setup
//!   STM_DATA_DIR=/path/to/disposable-bench-store cargo bench --bench query_bench
//!
//! Store-based groups require a nonempty `STM_DATA_DIR` and prepared store files.
//! They never select the default user store implicitly. They can configure or
//! migrate SQLite, recover index publication, and load or download query models.
//! Use a disposable store, not one that holds data you need to preserve.
//!
//! The `fuse_rrf` group uses fixed input lists and needs no store. Unset
//! `STM_DATA_DIR` to prevent store preparation even when a filter selects this group:
//!
//!   env -u STM_DATA_DIR cargo bench --bench query_bench -- fuse_rrf

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

/// Warmup period for store-backed benchmark groups.
const WARM_UP: Duration = Duration::from_millis(1500);
/// Measurement period for store-backed groups that do not embed each query.
const MEASURE: Duration = Duration::from_millis(2500);

/// Select a benchmark store only when the caller explicitly supplies its path.
fn data_dir() -> Option<PathBuf> {
    std::env::var_os("STM_DATA_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// Check for the database and vector files required by store-backed groups.
fn store_ready(dir: &std::path::Path) -> bool {
    dir.join("vectors.bin").is_file() && dir.join("stm.db").is_file()
}

/// Apply production database configuration and migrations before measurement.
fn open_conn(dir: &std::path::Path) -> rusqlite::Connection {
    seizethemana::db::open(&dir.join("stm.db"))
        .expect("benchmark database must open with production configuration")
}

/// Run a group only with an explicitly selected, prepared benchmark store.
macro_rules! with_store {
    ($c:expr, $body:expr) => {{
        let Some(dir) = data_dir().filter(|d| store_ready(d)) else {
            eprintln!(
                "skipping store-based benchmarks: set STM_DATA_DIR to a disposable prepared store with stm.db and vectors.bin"
            );
            return;
        };
        let conn = open_conn(&dir);
        let paths = seizethemana::paths::Paths::new(dir.clone());
        let mut out = seizethemana::output::Output::new(true, true, false);
        ($body)($c, &paths, &conn, &mut out);
    }};
}

/// Measure vector scoring and a full sort, with query embedding outside the timer.
fn bench_vector_scan(c: &mut Criterion) {
    with_store!(
        c,
        |c: &mut Criterion,
         _paths: &seizethemana::paths::Paths,
         _conn: &rusqlite::Connection,
         out: &mut seizethemana::output::Output| {
            let store = seizethemana::embed::VectorStore::load(_paths.root())
                .expect("prepared benchmark vectors must load");
            let mut model = seizethemana::embed::load_query_model(&_paths.models_dir(), false)
                .expect("benchmark query model must load");
            let query = store
                .embed_query(&mut model, "sacrifice a creature to draw cards")
                .expect("benchmark query must embed with the prepared model");
            let n = store.meta.names.len();
            let mut group = c.benchmark_group("vector_scan");
            group.throughput(Throughput::Elements(n as u64));
            group.warm_up_time(WARM_UP);
            group.measurement_time(MEASURE);
            group.bench_function(BenchmarkId::new("full_scan", n), |b| {
                // Score each row with a dot product. Keep the full sort that
                // the original benchmark measured, rather than a partial sort.
                //
                // Keep the scoring and comparison rules consistent with
                // `crate::query::run_search` when that implementation changes.
                b.iter(|| {
                    let mut scored: Vec<(usize, f32)> = (0..n)
                        .map(|i| {
                            let row = store.row(i);
                            let score: f32 = criterion::black_box(&query)
                                .iter()
                                .zip(row)
                                .map(|(q, v)| q * v)
                                .sum();
                            (i, score)
                        })
                        .collect();
                    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
                    scored
                })
            });
            group.finish();
            let _ = out;
        }
    );
}

/// Measure full-text retrieval at several depths after production database setup.
fn bench_fts(c: &mut Criterion) {
    with_store!(c, |c: &mut Criterion,
                    _paths: &_,
                    conn: &rusqlite::Connection,
                    _out: &mut _| {
        let expr = seizethemana::db::fts_query("sacrifice a creature to draw cards")
            .expect("fixed benchmark query must contain searchable terms");
        let mut group = c.benchmark_group("fts");
        group.warm_up_time(WARM_UP);
        group.measurement_time(MEASURE);
        for depth in [20usize, 80, 320] {
            group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &d| {
                b.iter(|| {
                    seizethemana::db::fts_search(conn, &expr, d)
                        .expect("full-text search must succeed on the prepared benchmark store")
                })
            });
        }
        group.finish();
    });
}

/// Measure reciprocal rank fusion from fixed input lists without a store.
fn bench_fuse_rrf(c: &mut Criterion) {
    let fts: Vec<(String, f64)> = (0..100).map(|i| (format!("Card {i}"), 0.0)).collect();
    let vector: Vec<(String, f64)> = (10..160).map(|i| (format!("Card {i}"), 0.0)).collect();
    let mut group = c.benchmark_group("fuse_rrf");
    group.warm_up_time(Duration::from_millis(600));
    group.measurement_time(Duration::from_millis(1500));
    for k in [1.0f64, 60.0, 500.0] {
        group.bench_with_input(BenchmarkId::from_parameter(k as u32), &k, |b, &k| {
            b.iter(|| {
                seizethemana::query::fuse_rrf_k(
                    criterion::black_box(&fts),
                    criterion::black_box(&vector),
                    20,
                    k,
                    |_| None,
                )
            })
        });
    }
    group.finish();
}

/// Measure production ranked search, including locks, recovery, and model loading.
fn bench_run_search(c: &mut Criterion) {
    with_store!(
        c,
        |c: &mut Criterion,
         paths: &seizethemana::paths::Paths,
         conn: &rusqlite::Connection,
         out: &mut seizethemana::output::Output| {
            let filters = seizethemana::search::CardFilters::default();
            let queries = ["bolt", "sacrifice a creature to draw cards", "wrath effect"];
            let mut group = c.benchmark_group("run_search");
            group.warm_up_time(WARM_UP);
            // Query embedding needs a longer measurement period.
            group.measurement_time(Duration::from_millis(4000));
            for (i, q) in queries.iter().enumerate() {
                group.bench_function(BenchmarkId::from_parameter(i), |b| {
                    b.iter(|| {
                        seizethemana::query::run_search(paths, conn, out, q, &filters, 20, None)
                            .expect("ranked search must succeed on the prepared benchmark store")
                    })
                });
            }
            // Restrict results to owned cards when the collection is not empty.
            let owned: HashSet<String> = seizethemana::collection::owned_names_all(conn)
                .expect("owned card names must load from the prepared benchmark store");
            if !owned.is_empty() {
                group.bench_function(BenchmarkId::from_parameter("owned"), |b| {
                    b.iter(|| {
                        seizethemana::query::run_search(
                            paths,
                            conn,
                            out,
                            "sacrifice a creature to draw cards",
                            &filters,
                            20,
                            Some(&owned),
                        )
                        .expect("owned search must succeed on the prepared benchmark store")
                    })
                });
            } else {
                eprintln!("skipping run_search/owned: benchmark collection is empty");
            }
            group.finish();
        }
    );
}

/// Measure combo loading for fixed card names from the prepared database.
fn bench_combo_loader(c: &mut Criterion) {
    with_store!(c, |c: &mut Criterion,
                    _paths: &_,
                    conn: &rusqlite::Connection,
                    _out: &mut _| {
        let names: HashSet<String> = ["Sol Ring", "Lightning Bolt", "Phyrexian Altar"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut group = c.benchmark_group("combo_loader");
        group.warm_up_time(WARM_UP);
        group.measurement_time(MEASURE);
        group.bench_function("load_variants_for", |b| {
            b.iter(|| {
                seizethemana::combos::load_variants_for(conn, criterion::black_box(&names))
                    .expect("combo variants must load from the prepared benchmark store")
            })
        });
        group.finish();
    });
}

criterion_group!(
    benches,
    bench_vector_scan,
    bench_fts,
    bench_fuse_rrf,
    bench_run_search,
    bench_combo_loader
);
criterion_main!(benches);
