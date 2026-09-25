// Query-path benchmarks against a real store.
//
// Run after a release-mode setup (embedding is debug-slow):
//
//   cargo build --release && ./target/release/stm setup   # once
//   cargo bench --bench query_bench
//
// The store is read-only here. `STM_DATA_DIR` overrides the data directory;
// without it the default `~/.seizethemana` is used. Skip the run entirely
// (exit 0) when no store exists, so CI machines without data still build.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

// Warmup + measurement tuned for a 30k-row full scan (~50 ms).
const WARM_UP: Duration = Duration::from_millis(1500);
const MEASURE: Duration = Duration::from_millis(2500);

fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("STM_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".seizethemana"))
}

fn store_ready(dir: &std::path::Path) -> bool {
    dir.join("vectors.bin").is_file() && dir.join("stm.db").is_file()
}

fn open_conn(dir: &std::path::Path) -> rusqlite::Connection {
    let conn = seizethemana::db::open(&dir.join("stm.db")).expect("open store");
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn
}

/// Run one bench group only when a real store exists.
macro_rules! with_store {
    ($c:expr, $body:expr) => {{
        let Some(dir) = data_dir().filter(|d| store_ready(d)) else {
            eprintln!("skipping: no store (run `stm setup` in release first)");
            return;
        };
        let conn = open_conn(&dir);
        let paths = seizethemana::paths::Paths::new(dir.clone());
        let mut out = seizethemana::output::Output::new(false, true, false);
        ($body)($c, &paths, &conn, &mut out);
    }};
}

fn bench_vector_scan(c: &mut Criterion) {
    with_store!(
        c,
        |c: &mut Criterion,
         _paths: &seizethemana::paths::Paths,
         _conn: &rusqlite::Connection,
         out: &mut seizethemana::output::Output| {
            let store = seizethemana::embed::VectorStore::load(_paths.root()).expect("load store");
            let mut model =
                seizethemana::embed::load_query_model(&_paths.models_dir(), false).expect("model");
            let query = store
                .embed_query(&mut model, "sacrifice a creature to draw cards")
                .expect("embed");
            let n = store.meta.names.len();
            let mut group = c.benchmark_group("vector_scan");
            group.throughput(Throughput::Elements(n as u64));
            group.warm_up_time(WARM_UP);
            group.measurement_time(MEASURE);
            group.bench_function(BenchmarkId::new("full_scan", n), |b| {
                // The production scan (query.rs run_search): dot product
                // per row, then a partial top-N sort. usize::MAX keeps the
                // full-sort shape the original bench measured.
                //
                // Maintenance: this loop hand-mirrors the scoring block in
                // `crate::query::run_search` (the `scored` build and its
                // sort). If that function's scan changes shape — a new
                // filter, a different comparator, a chunked scorer — update
                // this bench to the same shape, or it stops measuring the
                // code the CLI actually runs.
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

fn bench_fts(c: &mut Criterion) {
    with_store!(c, |c: &mut Criterion,
                    _paths: &_,
                    conn: &rusqlite::Connection,
                    _out: &mut _| {
        let expr = seizethemana::db::fts_query("sacrifice a creature to draw cards").unwrap();
        let mut group = c.benchmark_group("fts");
        group.warm_up_time(WARM_UP);
        group.measurement_time(MEASURE);
        for depth in [20usize, 80, 320] {
            group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &d| {
                b.iter(|| seizethemana::db::fts_search(conn, &expr, d).expect("fts"))
            });
        }
        group.finish();
    });
}

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
            // The embedding call dominates; give it room.
            group.measurement_time(Duration::from_millis(4000));
            for (i, q) in queries.iter().enumerate() {
                group.bench_function(BenchmarkId::from_parameter(i), |b| {
                    b.iter(|| {
                        seizethemana::query::run_search(paths, conn, out, q, &filters, 20, None)
                            .expect("search")
                    })
                });
            }
            // Collection-restricted: every name restricted to a tiny pool.
            let owned: HashSet<String> =
                seizethemana::collection::owned_names_all(conn).unwrap_or_default();
            let empty = HashSet::new();
            let restrict: &HashSet<String> = if owned.is_empty() { &empty } else { &owned };
            group.bench_function(BenchmarkId::from_parameter("owned"), |b| {
                b.iter(|| {
                    seizethemana::query::run_search(
                        paths,
                        conn,
                        out,
                        "sacrifice a creature to draw cards",
                        &filters,
                        20,
                        (!restrict.is_empty()).then_some(restrict),
                    )
                    .expect("search")
                })
            });
            group.finish();
        }
    );
}

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
                    .expect("load")
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
