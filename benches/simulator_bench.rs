//! Offline release benchmarks for compilation, dealing, games, and analysis.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroUsize;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use seizethemana::db::CardRow;
use seizethemana::deck::grammar::{Deck, DeckEntry};
use seizethemana::deck::simulator::benchmark::{
    BenchmarkCombos, BenchmarkDeck, BenchmarkLogs, BenchmarkStats, mana_value,
};
use seizethemana::deck::simulator::{RunConfig, SimReport};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Share the offline metadata supplement with ordinary simulator tests.
#[path = "../src/deck/simulator/tests/fixture_metadata/mod.rs"]
mod fixture_metadata;

/// Fixed seeds make workload changes visible between benchmark runs.
const SEED: u64 = 42;

/// The selected local list, not the Elven Empire precon or another Lathril list.
const LATHRIL_METADATA: &str =
    include_str!("../src/deck/simulator/tests/popular_card_fixtures/lathril.json");
/// Original deck bytes preserve print choices and physical execution order.
const LATHRIL_DECK: &str =
    include_str!("../src/deck/simulator/tests/popular_card_fixtures/lathril.txt");
/// Exact stored rows preserve all simulator inputs rather than rebuilding metadata.
const LATHRIL_ROWS: &str =
    include_str!("../src/deck/simulator/tests/popular_card_fixtures/lathril_rows.json");

/// Checked-in lists cover simple, tribal, token, storm, recursion, and combo decks.
const FIXTURES: &[(&str, &str)] = &[
    (
        "mono-red",
        include_str!("../src/deck/simulator/tests/deck_fixtures/mono-red-aggro.json"),
    ),
    (
        "ezuri",
        include_str!("../src/deck/simulator/tests/deck_fixtures/ezuri_elf_swarm.json"),
    ),
    (
        "tokens",
        include_str!("../src/deck/simulator/tests/deck_fixtures/marneus_calgar_tokens.json"),
    ),
    (
        "storm",
        include_str!("../src/deck/simulator/tests/deck_fixtures/ruby-storm.json"),
    ),
    (
        "graveyard",
        include_str!("../src/deck/simulator/tests/deck_fixtures/meren_graveyard.json"),
    ),
    (
        "combo",
        include_str!("../src/deck/simulator/tests/deck_fixtures/kinnan_combo.json"),
    ),
];

/// Card metadata stored by the simulator's offline fixtures.
#[derive(Deserialize)]
struct FixtureCard {
    name: String,
    cost: String,
    type_line: String,
    keywords: String,
    text: String,
    power: Option<String>,
    toughness: Option<String>,
    loyalty: Option<String>,
    #[serde(default)]
    colors: Vec<String>,
    #[serde(default)]
    color_identity: Option<Vec<String>>,
    oracle_id: Option<String>,
    scryfall_id: Option<String>,
}

impl FixtureCard {
    /// Retain exact fixture rules text and the printed numeric metadata.
    fn row(&self) -> CardRow {
        let keywords: Vec<_> = self
            .keywords
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        CardRow {
            name: self.name.clone(),
            oracle_id: self.oracle_id.clone().unwrap_or_default(),
            mana_cost: self.cost.clone(),
            cmc: mana_value(&self.cost),
            type_line: self.type_line.clone(),
            colors: serde_json::to_string(&self.colors).expect("fixture colors serialize"),
            color_identity: serde_json::to_string(
                self.color_identity.as_deref().unwrap_or_default(),
            )
            .expect("fixture identity serializes"),
            keywords: serde_json::to_string(&keywords).expect("fixture keywords serialize"),
            power: self.power.clone(),
            toughness: self.toughness.clone(),
            loyalty: self.loyalty.clone(),
            oracle_text: self.text.clone(),
            rarity: "rare".to_string(),
            edhrec_rank: None,
            penny_rank: None,
            reserved: None,
            legalities: "{}".to_string(),
            set_code: String::new(),
            collector_number: String::new(),
            scryfall_id: self.scryfall_id.clone().unwrap_or_default(),
            released_at: String::new(),
            game_changer: None,
        }
    }
}

/// A played list and its card data, read without a live store.
#[derive(Deserialize)]
struct Fixture {
    format: String,
    commanders: Vec<FixtureCard>,
    cards: Vec<FixtureCard>,
    list: Vec<(String, i64)>,
}

/// Build a played-list entry without print-specific metadata.
fn entry(name: &str, quantity: i64) -> DeckEntry {
    DeckEntry {
        quantity,
        name: name.to_string(),
        set_code: None,
        collector_number: None,
        foil: false,
    }
}

impl Fixture {
    /// Resolve missing metadata before preparing any measured inputs.
    fn enrich_metadata(&mut self) -> bool {
        let mut changed = false;
        for commander in &mut self.commanders {
            changed |= fixture_metadata::enrich(
                &commander.name,
                &mut commander.oracle_id,
                &mut commander.color_identity,
            )
            .expect("legacy benchmark commander identity must have a pinned source");
            changed |= fixture_metadata::enrich_stats(
                &commander.name,
                commander.oracle_id.as_deref(),
                &mut commander.power,
                &mut commander.toughness,
                &mut commander.loyalty,
            )
            .expect("legacy benchmark commander stats must have a pinned source");
        }
        for card in &mut self.cards {
            let commander = self
                .commanders
                .iter()
                .find(|commander| commander.name == card.name);
            if let Some(commander) = commander {
                changed |= fixture_metadata::inherit(
                    &card.name,
                    &mut card.oracle_id,
                    &mut card.color_identity,
                    commander.oracle_id.as_deref(),
                    commander
                        .color_identity
                        .as_deref()
                        .expect("commander identity was resolved"),
                )
                .expect("duplicate benchmark commander metadata must agree");
            }
            if commander.is_some() || card.type_line.contains("Vehicle") {
                changed |= fixture_metadata::enrich_stats(
                    &card.name,
                    card.oracle_id.as_deref(),
                    &mut card.power,
                    &mut card.toughness,
                    &mut card.loyalty,
                )
                .expect("legacy benchmark commander and Vehicle stats must have a pinned source");
            }
        }
        changed
    }

    /// Resolve every played card before starting a measured workload.
    fn inputs(&self) -> (Deck, HashMap<String, CardRow>) {
        let mut deck = Deck::default();
        for card in &self.commanders {
            deck.section_entries_mut("COMMANDER")
                .push(entry(&card.name, 1));
        }
        for (name, quantity) in &self.list {
            deck.section_entries_mut("DECK")
                .push(entry(name, *quantity));
        }
        let mut cards = HashMap::new();
        for card in self.commanders.iter().chain(&self.cards) {
            let row = card.row();
            cards.insert(card.name.clone(), row.clone());
            for alias in card.name.split(" // ").skip(1) {
                cards.insert(alias.to_string(), row.clone());
            }
        }
        for (name, _) in &self.list {
            assert!(
                cards.contains_key(name),
                "fixture has no rules data for {name}"
            );
        }
        (deck, cards)
    }
}

/// Preloaded inputs keep fixture I/O and validation outside measured phases.
struct Inputs {
    deck: Deck,
    cards: HashMap<String, CardRow>,
    format: String,
    pairs: Vec<(String, String)>,
}

/// Pinned hashes and copy order identify the exact selected Lathril workload.
#[derive(Deserialize)]
struct Snapshot {
    raw_deck_sha256: String,
    canonical_deck_sha256: String,
    card_rows_sha256: String,
    library_order: Vec<String>,
    commander_order: Vec<String>,
}

/// Decode one exact stored row without changing keywords, mana value, or rarity.
fn stored_row(value: &Value) -> CardRow {
    let text = |key: &str| {
        value[key]
            .as_str()
            .expect("stored row has a text field")
            .to_string()
    };
    let optional_text = |key: &str| value[key].as_str().map(str::to_string);
    CardRow {
        name: text("name"),
        oracle_id: text("oracle_id"),
        mana_cost: text("mana_cost"),
        cmc: value["cmc"].as_f64().expect("stored mana value is numeric"),
        type_line: text("type_line"),
        colors: text("colors"),
        color_identity: text("color_identity"),
        keywords: text("keywords"),
        power: optional_text("power"),
        toughness: optional_text("toughness"),
        loyalty: optional_text("loyalty"),
        oracle_text: text("oracle_text"),
        rarity: text("rarity"),
        edhrec_rank: value["edhrec_rank"].as_i64(),
        penny_rank: value["penny_rank"].as_i64(),
        reserved: value["reserved"].as_i64().map(|flag| flag != 0),
        legalities: text("legalities"),
        set_code: text("set_code"),
        collector_number: text("collector_number"),
        scryfall_id: text("scryfall_id"),
        released_at: text("released_at"),
        game_changer: value["game_changer"].as_i64().map(|flag| flag != 0),
    }
}

/// Fingerprint deck contents apart from print choices and source ordering.
fn canonical_deck(deck: &Deck) -> Value {
    let mut counts = BTreeMap::new();
    for (section, entries) in &deck.sections {
        for entry in entries {
            *counts.entry((section, &entry.name)).or_insert(0i64) += entry.quantity;
        }
    }
    Value::Array(counts.into_iter().map(|((section, name), quantity)| {
        json!({"section": section, "name": name, "quantity": quantity})
    }).collect())
}

/// Reject a changed fixture before it can produce benchmark results.
fn verify_hash(bytes: &[u8], expected: &str, label: &str) {
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        expected,
        "{label} hash differs"
    );
}

/// Prepare synthetic first-arrival pairs; these are not executable-combo claims.
fn pair_specs(cards: &HashMap<String, CardRow>) -> Vec<(String, String)> {
    let names: BTreeSet<_> = cards.values().map(|card| card.name.clone()).collect();
    let names: Vec<_> = names.into_iter().collect();
    names
        .windows(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

/// Load the exact selected list and verify all three pinned content hashes.
fn lathril_inputs() -> Inputs {
    let snapshot: Snapshot =
        serde_json::from_str(LATHRIL_METADATA).expect("Lathril metadata parses");
    assert_eq!(
        snapshot.raw_deck_sha256,
        "94cb0b49705aa8716d53c48dcf404c03cbd6c13ef38e743b9e974a8d4a7135ce"
    );
    assert_eq!(
        snapshot.canonical_deck_sha256,
        "ed046b99db857571a6ee18267c4d74b04dd0372d5e4f0b4dc70a63d3e5638fe7"
    );
    assert_eq!(
        snapshot.card_rows_sha256,
        "62b84124e0b641bb1279db8c55346930e142cb6883a0728a65dcad95059f4e7c"
    );
    verify_hash(
        LATHRIL_DECK.as_bytes(),
        &snapshot.raw_deck_sha256,
        "Lathril source",
    );
    let deck = Deck::parse(LATHRIL_DECK).expect("selected Lathril list parses");
    verify_hash(
        &serde_json::to_vec(&canonical_deck(&deck)).expect("deck serializes"),
        &snapshot.canonical_deck_sha256,
        "Lathril contents",
    );
    let rows: Value = serde_json::from_str(LATHRIL_ROWS).expect("stored rows parse");
    verify_hash(
        &serde_json::to_vec(&rows).expect("rows serialize"),
        &snapshot.card_rows_sha256,
        "Lathril rows",
    );
    let cards: HashMap<_, _> = rows
        .as_array()
        .expect("rows are an array")
        .iter()
        .map(stored_row)
        .map(|row| (row.name.clone(), row))
        .collect();
    assert_eq!(cards.len(), 83);
    let compiled = BenchmarkDeck::new(&deck, &cards, Some("commander"));
    compiled
        .verify_card_order(&snapshot.library_order, &snapshot.commander_order)
        .expect("exact Lathril copy order");
    eprintln!(
        "fixture=lathril canonical_deck_sha256={} card_rows_sha256={} seed={SEED}",
        snapshot.canonical_deck_sha256, snapshot.card_rows_sha256
    );
    let pairs = pair_specs(&cards);
    Inputs {
        deck,
        cards,
        format: "commander".to_string(),
        pairs,
    }
}

/// Caller-selected matrix sizes keep focused runs practical without changing code.
struct Settings {
    runs: Vec<u32>,
    turns: Vec<u32>,
    workers: Vec<NonZeroUsize>,
    batches: Vec<NonZeroUsize>,
    fixtures: BTreeSet<String>,
    mode: String,
}

/// Parse positive comma-separated integers or use a documented default matrix.
fn positive_values(name: &str, defaults: &[u32]) -> Vec<u32> {
    let mut values = match std::env::var(name) {
        Ok(value) => value
            .split(',')
            .map(|part| {
                part.parse::<u32>()
                    .expect("benchmark matrix requires positive integers")
            })
            .collect(),
        Err(std::env::VarError::NotPresent) => defaults.to_vec(),
        Err(error) => panic!("invalid {name}: {error}"),
    };
    assert!(
        !values.is_empty() && values.iter().all(|&value| value > 0),
        "{name} must contain positive integers"
    );
    values.sort_unstable();
    values.dedup();
    values
}

impl Settings {
    /// Validate environment filters before executing any measured workload.
    fn load() -> Self {
        let available = u32::try_from(
            std::thread::available_parallelism()
                .expect("available workers")
                .get(),
        )
        .expect("worker count fits u32");
        let mut default_workers = vec![1, 2, 4, available];
        default_workers.retain(|&workers| workers <= available);
        let workers = positive_values("STM_SIM_BENCH_WORKERS", &default_workers)
            .into_iter()
            .map(|value| NonZeroUsize::new(value as usize).expect("positive workers"))
            .collect();
        let batches = positive_values("STM_SIM_BENCH_BATCHES", &[256])
            .into_iter()
            .map(|value| NonZeroUsize::new(value as usize).expect("positive batch size"))
            .collect();
        let known: BTreeSet<_> = std::iter::once("lathril".to_string())
            .chain(FIXTURES.iter().map(|(name, _)| name.to_string()))
            .collect();
        let fixtures = match std::env::var("STM_SIM_BENCH_FIXTURES") {
            Ok(value) => value.split(',').map(str::to_string).collect(),
            Err(std::env::VarError::NotPresent) => known.clone(),
            Err(error) => panic!("invalid fixture filter: {error}"),
        };
        assert!(
            !fixtures.is_empty() && fixtures.is_subset(&known),
            "unknown or empty fixture filter"
        );
        let mode = match std::env::var("STM_SIM_BENCH_MODE") {
            Ok(mode) => mode,
            Err(std::env::VarError::NotPresent) => "all".to_string(),
            Err(error) => panic!("invalid benchmark mode: {error}"),
        };
        assert!(
            ["all", "phases", "scaling", "pipeline", "validate"].contains(&mode.as_str()),
            "unknown benchmark mode"
        );
        Self {
            runs: positive_values("STM_SIM_BENCH_RUNS", &[100, 1_000, 10_000]),
            turns: positive_values("STM_SIM_BENCH_TURNS", &[12, 20]),
            workers,
            batches,
            fixtures,
            mode,
        }
    }

    /// Use identical scenario and batch settings in serial and Rayon runs.
    fn execution(workers: NonZeroUsize, batch_size: NonZeroUsize) -> RunConfig {
        RunConfig {
            workers: Some(workers),
            batch_size,
            opponents: None,
        }
    }
}

/// Shared analysis inputs prepared only when a phase benchmark is selected.
struct PhaseInputs {
    logs: BenchmarkLogs,
    stats: BenchmarkStats,
    report: SimReport,
    specs: BenchmarkCombos,
}

impl PhaseInputs {
    /// Prepare observations outside the selected benchmark's timed loop.
    fn new(name: &str, compiled: &BenchmarkDeck, input: &Inputs, config: &RunConfig) -> Self {
        let logs = compiled
            .run_with_config(100, 12, SEED, config.clone())
            .expect("nonzero workload");
        let stats = compiled.aggregate(&logs);
        let report = compiled.report(&stats, SEED);
        print_workload_observations(name, &report, config);
        let specs = compiled.combo_specs(&input.pairs);
        assert_eq!(
            compiled.combo_access(&logs, &specs).variants_considered(),
            input.pairs.len()
        );
        Self {
            logs,
            stats,
            report,
            specs,
        }
    }
}

/// Measure compilation and analysis without database or fixture-loading costs.
fn bench_phases(c: &mut Criterion, name: &str, input: &Inputs, settings: &Settings) {
    let compiled = BenchmarkDeck::new(&input.deck, &input.cards, Some(&input.format));
    let config = Settings::execution(settings.workers[0], settings.batches[0]);
    let prepared = std::cell::OnceCell::new();
    let data = || prepared.get_or_init(|| PhaseInputs::new(name, &compiled, input, &config));
    let mut phases = c.benchmark_group(format!("phases/{name}"));
    phases.bench_function("compile", |b| {
        data();
        b.iter(|| {
            black_box(BenchmarkDeck::new(
                black_box(&input.deck),
                black_box(&input.cards),
                Some(&input.format),
            ))
        })
    });
    phases.bench_function("deal", |b| {
        data();
        b.iter(|| black_box(compiled.deal(black_box(SEED))))
    });
    phases.bench_function("aggregate_100", |b| {
        let data = data();
        b.iter(|| black_box(compiled.aggregate(black_box(&data.logs))))
    });
    phases.bench_function("pair_access_100", |b| {
        let data = data();
        b.iter(|| black_box(compiled.pair_access(black_box(&data.logs), black_box(&input.pairs))))
    });
    phases.bench_function("combo_access_100", |b| {
        let data = data();
        b.iter(|| black_box(compiled.combo_access(black_box(&data.logs), black_box(&data.specs))))
    });
    phases.bench_function("report", |b| {
        let data = data();
        b.iter(|| black_box(compiled.report(black_box(&data.stats), SEED)))
    });
    phases.bench_function("serialize", |b| {
        let data = data();
        b.iter(|| {
            black_box(serde_json::to_vec(black_box(&data.report)).expect("report serializes"))
        })
    });
    phases.bench_function("end_to_end_100", |b| {
        data();
        b.iter(|| {
            let compiled = BenchmarkDeck::new(&input.deck, &input.cards, Some(&input.format));
            let report = compiled
                .run_report(100, 12, SEED, config.clone())
                .expect("nonzero workload");
            black_box(serde_json::to_vec(&report).expect("report serializes"))
        })
    });
    phases.finish();
}

/// Distinguish retained execution logs from the bounded production pipeline.
#[derive(Clone, Copy)]
enum ExecutionPhase {
    /// Game execution alone keeps its results for later analysis.
    Games,
    /// Production-style execution consumes batches before building the report.
    Pipeline,
}

impl ExecutionPhase {
    /// Keep output names stable for measurement scripts.
    fn name(self) -> &'static str {
        match self {
            Self::Games => "scaling",
            Self::Pipeline => "pipeline",
        }
    }
}

/// Include local-pool setup when comparing worker limits and batch sizes.
fn bench_matrix(
    c: &mut Criterion,
    name: &str,
    compiled: &BenchmarkDeck,
    settings: &Settings,
    phase: ExecutionPhase,
) {
    for &turns in &settings.turns {
        for &runs in &settings.runs {
            let mut scaling = c.benchmark_group(format!("{}/{name}/t{turns}/{runs}", phase.name()));
            scaling.throughput(Throughput::Elements(u64::from(runs)));
            let mut observations_recorded = false;
            for &workers in &settings.workers {
                for &batch_size in &settings.batches {
                    let config = Settings::execution(workers, batch_size);
                    scaling.bench_with_input(
                        BenchmarkId::new(format!("workers{workers}"), format!("batch{batch_size}")),
                        &config,
                        |b, config| {
                            if !observations_recorded {
                                let observations = compiled
                                    .run_report(runs, turns, SEED, config.clone())
                                    .expect("benchmark observation workload");
                                print_workload_observations(name, &observations, config);
                                observations_recorded = true;
                            }
                            match phase {
                                ExecutionPhase::Games => b.iter(|| {
                                    black_box(
                                        compiled
                                            .run_with_config(runs, turns, SEED, config.clone())
                                            .expect("nonzero workload"),
                                    )
                                }),
                                ExecutionPhase::Pipeline => b.iter(|| {
                                    black_box(
                                        compiled
                                            .run_report(runs, turns, SEED, config.clone())
                                            .expect("nonzero workload"),
                                    )
                                }),
                            }
                        },
                    );
                }
            }
            scaling.finish();
        }
    }
}

/// Reject speed comparisons if any indexed game or canonical report differs.
fn validate_workers(name: &str, compiled: &BenchmarkDeck, settings: &Settings) {
    let serial = NonZeroUsize::new(1).expect("one worker is nonzero");
    for &turns in &settings.turns {
        for &runs in &settings.runs {
            let expected = compiled
                .run_with_config(
                    runs,
                    turns,
                    SEED,
                    Settings::execution(serial, settings.batches[0]),
                )
                .expect("serial validation workload");
            let report = compiled.report(&compiled.aggregate(&expected), SEED);
            print_workload_observations(
                name,
                &report,
                &Settings::execution(serial, settings.batches[0]),
            );
            let expected_report = serde_json::to_value(report).expect("serial report serializes");
            for &workers in &settings.workers {
                for &batch in &settings.batches {
                    let actual = compiled
                        .run_with_config(runs, turns, SEED, Settings::execution(workers, batch))
                        .expect("parallel validation workload");
                    assert!(
                        expected == actual,
                        "game logs differ: {name}, runs={runs}, turns={turns}, workers={workers}, batch={batch}"
                    );
                    let report =
                        serde_json::to_value(compiled.report(&compiled.aggregate(&actual), SEED))
                            .expect("parallel report serializes");
                    assert_eq!(
                        expected_report, report,
                        "canonical reports differ for {name}"
                    );
                    let streamed = compiled
                        .run_report(runs, turns, SEED, Settings::execution(workers, batch))
                        .expect("streamed report workload");
                    assert_eq!(
                        expected_report,
                        serde_json::to_value(streamed).expect("streamed report serializes"),
                        "streamed report differs for {name}"
                    );
                }
            }
            eprintln!("validated fixture={name} runs={runs} turns={turns} seed={SEED}");
        }
    }
}

/// State completion and report denominators outside timing, not CPU work counts.
fn print_workload_observations(
    name: &str,
    report: &seizethemana::deck::simulator::SimReport,
    config: &RunConfig,
) {
    eprintln!(
        "workload fixture={name} runs={} turns={} seed={} reference_workers={} reference_batch={} outcomes={} phase_observations={}",
        report.runs,
        report.turns,
        report.seed,
        config
            .workers
            .expect("benchmarks set an explicit worker count"),
        config.batch_size,
        serde_json::to_string(&report.outcomes).expect("outcome counts serialize"),
        serde_json::to_string(&report.phase_observations)
            .expect("phase observation counts serialize"),
    );
}

/// Keep correctness validation separate from timed and memory-profiled execution.
fn bench_input(c: &mut Criterion, name: &str, input: Inputs, settings: &Settings) {
    let compiled = BenchmarkDeck::new(&input.deck, &input.cards, Some(&input.format));
    eprintln!(
        "scenario=default-by-format pipeline=core-offline format={} combo_specs=synthetic-adjacent-pairs count={}",
        input.format,
        input.pairs.len()
    );
    if settings.mode == "validate" {
        validate_workers(name, &compiled, settings);
        return;
    }
    if ["all", "phases"].contains(&settings.mode.as_str()) {
        bench_phases(c, name, &input, settings);
    }
    if ["all", "scaling"].contains(&settings.mode.as_str()) {
        bench_matrix(c, name, &compiled, settings, ExecutionPhase::Games);
    }
    if ["all", "pipeline"].contains(&settings.mode.as_str()) {
        bench_matrix(c, name, &compiled, settings, ExecutionPhase::Pipeline);
    }
}

/// Register independent phase, throughput, and worker-scaling workloads.
fn bench_simulator(c: &mut Criterion) {
    let settings = Settings::load();
    if settings.fixtures.contains("lathril") {
        bench_input(c, "lathril", lathril_inputs(), &settings);
    }
    for &(name, json) in FIXTURES {
        if !settings.fixtures.contains(name) {
            continue;
        }
        eprintln!(
            "fixture={name} sha256={:x} seed={SEED}",
            Sha256::digest(json.as_bytes())
        );
        let mut fixture: Fixture = serde_json::from_str(json).expect("valid simulator fixture");
        if fixture.enrich_metadata() {
            eprintln!(
                "fixture={name} supplemental_metadata_sha256={}",
                fixture_metadata::supplement_sha256(),
            );
        }
        let (deck, cards) = fixture.inputs();
        let pairs = pair_specs(&cards);
        bench_input(
            c,
            name,
            Inputs {
                deck,
                cards,
                pairs,
                format: fixture.format,
            },
            &settings,
        );
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(250))
        .measurement_time(Duration::from_secs(1));
    targets = bench_simulator
}
criterion_main!(benches);
