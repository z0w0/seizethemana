# Querying and search evaluation

This document explains local search and embedding compatibility. It also gives
steps for repeatable search evaluation:

- [Search or list cards](#text-search-and-filter-only-listing).
- [Check retrieval and ranking behavior](#production-query-path).
- [Check embedding documents and vectors](#embedding-documents-and-vectors).
- [Run an evaluation](#repeatable-evaluation) or
  [review relevance labels](#agent-assisted-label-review).
- [Measure query-path timing](#query-path-timing) with fixed lists or a
  disposable store.

[Architecture](architecture.md) describes storage, sync, locking, and recovery.
The main implementation is in `src/query.rs`, `src/query/`, `src/embed.rs`,
`src/db.rs`, and `src/search.rs`.

## Text search and filter-only listing

Both query commands accept optional text:

```sh
stm query "sacrifice outlet" --color BG
stm query --type Elf --limit 10
stm query --set BLB --json
stm collection query --type Elf --json
stm collection query --set BLB --deck Froggy --json
stm query --limit 10
stm collection query --help
```

Nonempty text produces ranked results. Without text, the command applies
structured filters, sorts by card name, and returns up to `--limit` cards
(default 20, maximum 100). With no filters, it lists the first names in scope.
This path does not load vectors or a model, embed a query, or trigger automatic
refresh. It needs card rows in SQLite, but not a completed vector index. An
empty database returns no matches.

Filter-only JSON keeps the ranked result shape with `score: 0`. Human output
omits the score. That zero means unranked, not a poor match.

Collection queries use binder copies by default. `--binder` narrows that pool;
`--deck` adds the named deck's assigned copies. These scopes form a union,
not an intersection. Results never widen to unowned cards.

`--set CODE` matches the card's stored representative `cards.set_code`. It
does not inspect every catalog printing or your owned printing's set. This
applies to text and no-text queries. Ownership remains name-based.

Text queries wait for an automatic sync when card data is over 24 hours old.
If sync fails, the command warns and continues with the available data.
`--offline` skips that automatic network refresh. It does not prevent local
schema writes or publication recovery. No-text listings skip vector recovery
because they do not load the index. Explicit setup, sync, and URL imports still
need network access.

## Production query path

Text search combines two ranked lists: SQLite full-text search (FTS) and a
vector scan. Reciprocal-rank fusion (RRF) produces the final list.

### Query preparation and retrieval

`query::prepare_query` returns expanded embedding text and an optional FTS
expression. `expanded_text` keeps the original text and appends known Tagger
aliases from `EXPANSIONS`. Both search legs use that vocabulary.

FTS quotes each usable whitespace-separated term and joins terms with `OR`.
It searches names, tag labels, type lines, and Oracle text through SQLite FTS5
with `porter unicode61` stemming. Punctuation-only input skips FTS but can
still retrieve vector neighbors. An empty vector provider skips model loading
and uses FTS alone.

Filters and allowed-name restrictions apply before candidate selection in
both legs. Collection search restricts names to its ownership pool. A global
query's price cap restricts names to priced cards under the cap; unpriced
cards stay excluded.

The vector leg scans every allowed row with a dot product and uses a partial
sort to select candidates. FTS maps actual `cards.id` values to loaded rows;
IDs need not be dense or start at one. It fetches extra SQL results, applies
filters, and expands the window when too few candidates remain. This bounded
retry can still return fewer than the requested depth.

### Ranking defaults

`query::DEFAULT_SEARCH_SETTINGS` defines production behavior:

| Setting                                     | Default                              |
| ------------------------------------------- | ------------------------------------ |
| BM25 weights: name, tags, type, Oracle text | `8, 4, 2, 1`                         |
| FTS and vector candidate depths             | Each `max(limit, 20)`                |
| FTS term matching                           | `any` (`OR`)                         |
| Initial FTS SQL window                      | `max(candidate_depth * 4, 50)`       |
| FTS window growth and maximum attempts      | Multiply by 4; up to 4 windows total |
| RRF constant `k`                            | `60`                                 |
| FTS and vector RRF weights                  | `1, 1`                               |

Each leg contributes `1 / (k + rank)`, with ranks starting at one. Fusion uses
ranks, not raw BM25 or cosine scores. It normalizes the sum to `[0, 1]`; a
card ranked first by both legs scores 1. Ties use lower EDHREC rank, then card
name. The score is not a probability.

### Shared API and snapshots

`query::run_search` selects ranked search or the no-text `filter_only` path.
`search_with_vectors` applies production defaults to an already embedded query.
`search_with_settings` accepts explicit ranking settings and returns FTS,
vector, and hybrid results.

The shared search API takes a database connection, cards in database order,
a `VectorRowProvider`, an embedded query, original query text, filters, a limit,
and an optional allowed-name set. Every nonempty provider must have one
name-aligned vector row per card and matching query dimensions. Production
uses `VectorStore`; model comparisons use candidate-specific matrices.

Ranked CLI commands use `query::RankedSnapshot` to hold the store lock through
recovery, index loading, SQL reads, query embedding, and result details. Price
and ownership scopes use the same connection. A SQL read transaction also
isolates writers that do not take the store lock. No-text CLI listings use
only a read transaction. See [recovery and consistent reads](architecture.md#publication-recovery-and-consistent-reads)
before changing these boundaries.

## Embedding documents and vectors

### Production model

Production uses full-precision `BAAI/bge-small-en-v1.5` through fastembed, with
384 dimensions, CLS pooling, a 128-token limit, and eight inference threads.
Longer inputs lose trailing text. Documents have no retrieval prompt. Queries
use:

```text
Represent this sentence for searching relevant passages: <expanded query>
```

Document and query vectors are unit-normalized. `vectors.bin` is a row-major,
little-endian `f32` matrix; `status.json.names` joins rows to cards in
`db::load_all_cards` order. Production maps the matrix read-only and scans it
fully. A dot product equals cosine similarity for these normalized vectors.
There is no approximate-nearest-neighbor index.

### Card document layout

`embed::doc_for_row` calls `build_doc` with card identity, structured facts,
selected Tagger labels, printed stats when present, and Oracle text:

```text
Lightning Bolt. Mana cost {R}. Type Instant. Keywords none. Colors R.
Tags burn, direct damage. Rules text Deal 3 damage to any target.
```

Documents include up to 12 labels. Selection favors higher global use counts,
breaks ties alphabetically, and keeps at most one label with each first word.
Tags add role terms that Oracle text may not contain.

When stored document text changes, update its tests and `embed::DOC_VERSION`.
Normal sync re-embeds changed documents and rebuilds the whole matrix when the
stored model, dimension, or document version differs. It publishes current
metadata with the rebuilt vectors. Ranked search rejects an incompatible
index until sync finishes.

```sh
cargo build --release
./target/release/stm sync
```

Use release mode for real embedding work. Query-only changes, including
expansion and FTS weights, do not change stored vectors and do not need a
document-version bump. Sync refreshes the index without resetting ownership.

`stm setup --force` deletes the database, SQLite sidecars, vectors, status, and
staged publication files. Collection records, deck assignments, and purchase
prices are lost. Decklists and primers remain. See
[schema and reset rules](architecture.md#schema-and-destructive-reset).

### CPU and Core ML

macOS production embeds document batches with Core ML and queries with the
CPU encoder using the same weights. Other systems use CPU for both. Core ML
loads registered ONNX models with their pooling, quantization, and token limit.
It uses fixed tokenizer padding and input shapes. Most candidates use batches
of 32; dynamically quantized models use one-row batches because batch content
can change their output. A partial final batch gets dummy rows that are removed
from the result.

Core ML registration does not prove GPU execution. Inspect the compute plan
for `MLGPUComputeDevice` on main operations, compare vector similarity and
ordered results with CPU, and measure model load plus full-corpus work. An
Apple `.mlpackage` cannot replace an ONNX file in fastembed; it needs a
separate loader and compatibility checks.

## Evaluation inputs and report compatibility

`benches/golden_queries.json` stores query text, exact relevant card names,
optional filters, notes, optional negative labels, and optional report groups.
Unknown fields fail parsing. Filter conversion uses the CLI's
`CardFilters::from_cli` path. Every labeled card must exist and pass its
query's filters.

The split follows query content:

- Labeled queries without effective filters are development queries.
- Labeled queries with filters are held-out checks.
- Queries with no positive labels are behavior probes, excluded from
  relevance means.

Query index and filters distinguish repeated text. Metrics use only explicit
positive labels. An unlisted card earns no gain, even if it may be relevant.
Review pooled unjudged results before drawing conclusions about broad role
queries. Add reviewed cards to `relevant` or `not_relevant`; leave uncertain
cards unjudged.

Reports include recall@10, mean reciprocal rank (MRR)@10, and normalized
discounted cumulative gain (nDCG)@10. The rank sweep also reports candidate
recall@20 for each leg and their union. Behavior probes report result counts
and names, not zero-valued relevance observations.

Advance `common::LABEL_VERSION` in `examples/common/mod.rs` when queries or
labels change. The batch-review workflow below advances it after all batches
are applied. Reports record label, corpus, and query identities. The corpus
fingerprint covers every stored card field and production embedding document,
in row order. Changes to filters, legality, rarity, or ranking signals therefore
change the corpus identity even when card names stay the same.

Use the report checker for candidate reports:

```sh
cargo run --release --example query_report_check -- --mode model first.json second.json
cargo run --release --example query_report_check -- --mode format first.json second.json
cargo run --release --example query_report_check -- --mode ranking first.json second.json
```

Every mode requires matching evaluation version, label version, corpus, and
query fingerprints. Model checks also require the same document format and
ranking settings. Format checks require the same model identity, backend,
dimension, token limit, prompts, pooling, and ranking settings. Ranking checks
hold those candidate fields and document format fixed but allow ranking
settings to differ. Format and ranking checks need one candidate per report.
The checker expects candidate reports; baseline and sweep reports have
different shapes.

Record the exact source revision, command, platform, and cache state beside
saved results. The reports do not record a source commit, and a matching crate
version is not proof of matching search code. Core ML candidate metadata
includes an ONNX model hash; CPU candidate metadata has no model hash. Report
compatibility checks do not replace source and input review. Tune on
development queries. Treat filtered queries as a check unless they were frozen
before tuning.

## Repeatable evaluation

Use a disposable, populated, compatible store. Keep it unchanged across a
comparison.
The evaluation examples below read `STM_DATA_DIR`, or default to
`~/.seizethemana/`; they do not accept the CLI's `--data-dir` option. Set an
explicit path to avoid selecting your working store:

```sh
export STM_DATA_DIR=/path/to/disposable-bench-store
```

Avoid concurrent sync or imports while collecting reports. The separate
[`query_bench` harness](#query-path-timing) requires an explicit store path.

### Baseline

```sh
cargo run --release --example query_baseline -- --json
```

The baseline runs each query through `run_search`, then the prepared shared
path. It requires identical ordered hybrid top-10 lists before reporting
quality. It reports FTS, vector, and hybrid metrics, groups, behavior probes,
review pools, and judgment coverage. Timings separate full `run_search`
latency from prepared query embedding and shared ranking. End-to-end time
includes store reads and model initialization.

### Model and document comparisons

The bake-off runs candidate vectors through the same filters, FTS, sparse-ID
mapping, candidate selection, and fusion as production:

```sh
cargo run --release --example query_bakeoff -- \
  --only bge-small-full --backend cpu --format production --json
cargo run --release --example query_bakeoff -- \
  --only bge-small-full --backend coreml \
  --format production,fielded,compact \
  --json --save benches/query_bakeoff.json
```

`auto` selects CPU, not Core ML. Core ML requires macOS. The `--save` option
requires `--backend coreml`; CPU runs can print JSON for inspection. The tool's
`--save` workflow therefore requires macOS.

`CANDIDATES` in `examples/query_bakeoff/model.rs` defines the model contracts:

| Key                     | fastembed model            | Dimensions | Token limit | Pooling |
| ----------------------- | -------------------------- | ---------: | ----------: | ------- |
| `bge-small`             | `BGESmallENV15Q`           |        384 |         128 | CLS     |
| `bge-small-256`         | `BGESmallENV15Q`           |        384 |         256 | CLS     |
| `bge-small-full`        | `BGESmallENV15`            |        384 |         128 | CLS     |
| `arctic-s`              | `SnowflakeArcticEmbedSQ`   |        384 |         256 | CLS     |
| `minilm-l6`             | `AllMiniLML6V2Q`           |        384 |         256 | mean    |
| `minilm-l12`            | `AllMiniLML12V2Q`          |        384 |         256 | mean    |
| `paraphrase-minilm-l12` | `ParaphraseMLMiniLML12V2Q` |        384 |         128 | mean    |
| `arctic-xs`             | `SnowflakeArcticEmbedXSQ`  |        384 |         256 | CLS     |

All use ONNX. BGE and Arctic candidates use the BGE query prompt; MiniLM
candidates use no query prompt. Document prompts are empty. Core ML candidates
still use a CPU query encoder.

`--only` accepts comma-separated candidate keys. `--format` accepts
`production`, `fielded`, `compact`, `rules-first`, `name-type-rules-first`,
`tags-first`, `tags-last`, and `contextual`. The default is `production` from
`embed::doc_for_row`. Other formats change field order or layout; `contextual`
also omits printed stats. They do not replace production vectors.

Candidate caches live under `<data-dir>/bakeoff/`. Their identity includes
backend, model and available model hash, dimension, ordered document
fingerprint, card count, token limit, document version, prompts, pooling, and
format. Stale or corrupt caches rebuild; reused rows must be unit-normalized.
Cache misses report full-corpus document embedding time. Cache hits do not
measure rebuild cost.

The `bge-small-full` production-format candidate must match ordered production
top-10 results for every query when the tokenizer and ranking settings are
unchanged. A mismatch stops the run. Alternate models, formats, or ranking
settings do not have that parity requirement.

For a quick backend check, add `--throughput-pilot`. It samples documents
across lengths, reports model-load time and estimated corpus throughput, and
can expose Core ML compute plans. A sampled estimate is not a full rebuild
measurement.

To add a candidate, register its exact model, dimension, token limit, prompts,
pooling, model-card URL, and supported sequence limit. Check those values
against the model card and the fastembed version in `Cargo.lock`. Add tests
in `examples/tests/query_bakeoff_tests.rs`, including any output normalization.
Compare development quality, filtered checks, regressions, and unjudged hits
against production before changing the production model.

### Ranking and FTS sweep

```sh
cargo run --release --example query_rank_sweep -- --json
```

The sweep checks default parity with `run_search`, measures FTS-only,
vector-only, and hybrid controls, then varies one ranking setting at a time.
It selects the highest development hybrid nDCG, uses MRR to break ties, and
keeps defaults on an exact tie. It checks the selected setting on filtered
queries but never changes CLI defaults.

The bake-off accepts the same explicit controls:

```text
--fts-weights name,tags,type,oracle
--fts-depth N             --vector-depth N
--rrf-k N                 --fts-rrf-weight N
--vector-rrf-weight N     --fts-terms any|all
--fts-overfetch N         --fts-min-rows N
--fts-max-rounds N        --fts-tokenizer porter|unicode61
```

Reports retain complete settings, per-query lists, group metrics, candidate
recall, paired nDCG changes, and separate search timings. FTS rebuild timing
uses `VACUUM INTO` to copy the database, then rebuilds FTS in that copy. A
tokenizer comparison also uses a temporary database copy. Neither changes
the production FTS index or measures Scryfall ingestion.

### Query-path timing

`query_bench` separates fixed-list fusion from store-based workloads. To time
only reciprocal-rank fusion, unset `STM_DATA_DIR`:

```sh
env -u STM_DATA_DIR cargo bench --bench query_bench -- fuse_rrf
```

The `fuse_rrf` group needs no card store or model. Unsetting the variable also
prevents store-based groups from opening a store during benchmark setup.

Store-based groups require a nonempty `STM_DATA_DIR` and prepared `stm.db` and
`vectors.bin` files. They never select `~/.seizethemana/` by default. Use an
explicit disposable store, not your working collection:

```sh
STM_DATA_DIR=/path/to/disposable-bench-store cargo bench --bench query_bench
```

These workloads are not read-only. They can configure or migrate SQLite,
recover a pending index publication, and load or download query models. Do not
assume that copying files from a live store produces a consistent snapshot.

If you need a new benchmark store, prepare it in release mode. Setup downloads
card data and a model, then embeds the cards. Use a new directory and do not
use `--force` on a store you need to preserve:

```sh
cargo build --release
./target/release/stm --data-dir /path/to/disposable-bench-store setup
```

Keep workload boundaries clear when reading timings:

- `vector_scan` embeds the query before timing, then measures scoring and a
  full sort. Production search uses a partial sort.
- `fts` measures full-text retrieval after database setup.
- `run_search` includes the production search path, locks, publication recovery,
  and model loading. It is not the full CLI command.
- `combo_loader` measures database combo lookup for fixed card names.

`run_search/owned` runs only when the prepared store has collection cards. An
empty collection skips that case; it does not time the whole catalog under an
owned-card label.

Record the source revision, prepared inputs, platform, and cache state. Do not
run sync or imports against the benchmark store while measuring it. A fusion
timing alone does not establish end-to-end search speed.

## Agent-assisted label review

`query_label_review` reads the production reference and candidate rankings
from saved reports listed in `BAKEOFF_REPORTS` (`benches/query_bakeoff.json`).
Generate that report before export; the tool does not read `query_baseline`
output. Each item includes query intent, filters,
reviewed examples, full rules text, tags, and source ranks. Query index and
exact card name identify an item.

Export batches of 100 with indexes from zero through `batch_count - 1`. Keep
the files from one export run together:

```sh
cargo run --release --example query_label_review -- --export \
  --batch-size 100 --batch-index 0 --save query-review-00.json
```

Ask the reviewing agent to return every `item_id` as `relevant`,
`not_relevant`, or `unsure`, with a short reason from card facts. It must
follow the supplied response schema and preserve batch, label, and corpus
identities. Exact-name queries require the intended card. A word match alone
does not prove a role match. The agent must not edit the golden file.

A person must inspect every decision. Run the dry review before applying:

```sh
cargo run --release --example query_label_review -- --review \
  query-review-00.json --response query-review-00-response.json
cargo run --release --example query_label_review -- --apply \
  query-review-00.json --response query-review-00-response.json \
  --approve --approval-note "Reviewed each decision in batch 0"
```

Resolve disagreements or leave them `unsure`. Apply validates batch and
response identities, current label version, and saved-report corpus identity.
Export and apply also compare that identity with the live card data and documents.
For decided items, it also checks current query intent, filters, card presence,
and conflicting labels. Missing, duplicate, or unknown response items fail.
Approved labels update `benches/golden_queries.json`; `unsure` leaves labels
unchanged. Decisions and reasons append to
`benches/query_label_review_audit.jsonl`.

Keep the store and source reports fixed throughout review. Export and apply
reject changed live corpus evidence, including changes to unreviewed cards.
Regenerate reports and exports if card data or queries change. Reports made with
an older corpus fingerprint also need regeneration.

Applying a batch does not advance the label version, so other exported batches
stay usable. After all batches have audit entries, finalize:

```sh
cargo run --release --example query_label_review -- --finalize \
  --batch-count "$BATCH_COUNT" --approve --approval-note "Reviewed all exported batches"
```

Finalization advances `common::LABEL_VERSION`. Regenerate reports before
comparing quality. Agent output is a proposal, not human approval.

## Checks for search changes

Use `src/tests/query_tests.rs` and `src/query/tests/` for row alignment,
sparse IDs, FTS retries, filtering before vector top-k, empty vectors,
punctuation, dimensions, fusion, and snapshot boundaries. Evaluation tests
live under `examples/tests/`.

Run the repository checks in `AGENTS.md`. With a populated release-mode store,
run the baseline and relevant bake-off or ranking sweep. Check ordered parity,
per-query regressions, and judgment coverage, not only aggregate quality.

For document changes, sync in release mode and confirm that document versions
and candidate cache identities change. For query-only changes, keep stored
vectors. For FTS changes, refresh indexed content as needed and rerun the
baseline and sweep.

SQLite FTS5 supports column BM25 weights but does not expose `k1` or `b` as
query arguments; testing them needs a custom ranking function. Prefix indexes
do not help the current query builder because it emits no prefix terms.

- [SQLite BM25 ranking](https://www.sqlite.org/fts5.html#the_bm25_function)
- [SQLite tokenizers and prefix indexes](https://www.sqlite.org/fts5.html#tokenizers)
- [BGE model card](https://huggingface.co/BAAI/bge-small-en-v1.5)
- [fastembed registry](https://github.com/Anush008/fastembed-rs)
