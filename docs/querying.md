# Querying and search evaluation

This guide explains how `stm query` searches cards and how to measure changes
to that search. It covers the production embedding index, shared search API,
golden-query baseline, model bake-off, and ranking sweep.

For database tables, synchronization, and the wider module map, see
[`architecture.md`](architecture.md). The live implementation is in
`src/query.rs`, `src/embed.rs`, `src/db.rs`, and `src/search.rs`.

## Production query path

`stm query` combines two ranked lists:

1. SQLite full-text search (FTS) finds cards with matching names, tags, type
   lines, or oracle text.
2. A vector scan finds cards whose embedding is close to the query embedding.
3. Reciprocal-rank fusion (RRF) combines the two lists.

The search code lives in `query::run_search` and
`query::search_with_settings`. The command wrapper loads the production vector
store and embedding model. The shared search function accepts the database
connection, cards in database order, a read-only vector-row provider, an
already embedded query, the original query text, filters, a result limit, and
an optional set of allowed names. It returns the FTS, vector, and fused hits.

`query::VectorRowProvider` exposes row count, dimension, card name, and vector
values. Every non-empty provider must have one row per card. Row names must
match the card table order. The production provider is `embed::VectorStore`;
the bake-off uses its own matrix for models with other dimensions.

### Query preparation

`query::prepare_query` expands one input string for both search legs. It returns
the expanded text for embedding and an optional SQLite FTS expression. This
keeps the query vocabulary in sync across the two legs.

`query::expanded_text` appends Tagger vocabulary when it finds known role or
mechanic terms. For example, a shorthand term can add words that appear in
`tags_text` or in the model's card documents. The original query stays at the
start of the expanded text. Queries without an expansion pass through
unchanged.

FTS quotes each usable word and joins the terms with `OR`. A query with no
letters or numbers skips FTS. A vector search can still return nearest
neighbors for such a query when vectors exist. If the vector store is empty,
the command skips model loading and uses FTS alone.

### Filters and candidate selection

The CLI converts `cli::CardFilters` to `search::CardFilters` before it calls
`query::run_search`. Filters apply to both retrieval legs before ranking.
`collection query` passes owned names through the same optional name
restriction. A price cap also becomes an allowed-name restriction.

The vector scan filters rows before selecting its top candidates. FTS maps
actual `cards.id` values back to the matching card rows; it does not assume
that IDs are dense or start at one. FTS over-fetches results, filters them,
and expands its SQL window when filters leave too few candidates.

Each leg returns at least `max(limit, 20)` candidates with the default
settings. FTS starts with `max(candidate_depth * 4, 50)` SQL rows and can retry
with a four-times larger window. The vector leg uses a partial sort to keep
its highest-scoring allowed rows.

### Fusion and defaults

`query::DEFAULT_SEARCH_SETTINGS` defines the production ranking defaults:

| Setting                                    |                                  Default |
| ------------------------------------------ | ---------------------------------------: |
| FTS weights: name, tags, type, oracle text |                             `8, 4, 2, 1` |
| FTS candidate depth                        | `20`, or the requested limit when larger |
| Vector candidate depth                     | `20`, or the requested limit when larger |
| RRF constant `k`                           |                                     `60` |

RRF adds `1 / (k + rank)` for each leg that contains a card. It uses ranks,
not the raw BM25 or vector scores. Equal fused scores sort by lower EDHREC rank,
then card name. The fused score is normalized to `[0, 1]`.

`search_with_vectors` uses these defaults. `search_with_settings` accepts
explicit weights, candidate depths, and an RRF constant for controlled tests.
`stm query` stays on `DEFAULT_SEARCH_SETTINGS`. The existing
`db::fts_search` function keeps its current weights; evaluation calls
`db::fts_search_with_weights` when it needs a different weight set.

## Embedding documents and vectors

### Production model

The production model is full-precision BGE Small English v1.5 from fastembed.
It returns 384-dimensional vectors. Production documents use a 128-token
limit; query text uses the BGE retrieval instruction:

```text
Represent this sentence for searching relevant passages: <expanded query>
```

The document does not use a prompt. Fastembed uses CLS pooling for BGE and
normalizes model output. The production vector store also validates dimensions
and normalizes rows before writing them.

The vector file is a row-major little-endian `f32` matrix. Its row names are in
`status.json` and must match `db::load_all_cards` order. Because the query and
document vectors are unit-normalized, their dot product is cosine similarity.
The current implementation scans the full matrix. It does not use an
approximate-nearest-neighbor index.

### Card document layout

`embed::doc_for_row` builds one natural-language document per card with
`embed::build_doc`. It keeps the card name, structured facts, Tagger labels,
and oracle text. A typical document looks like this:

```text
Lightning Bolt. Mana cost {R}. Type Instant. Keywords none. Colors R.
Tags burn, direct damage. Rules text Deal 3 damage to any target.
```

The layout improves the current relevance scores over the former production
layout. Tag labels add community role vocabulary. `DOC_VERSION` in
`src/embed.rs` records the production layout version in `status.json`.

When changing production document text or the model, update the document tests
and version. `stm sync` detects the version change and re-embeds every card.
Search rejects an old model or document index until sync finishes. Use a
release build for a real setup or sync:

```sh
cargo build --release
./target/release/stm sync
```

Use `stm setup --force` when you need to rebuild the complete store. Do not
change `DOC_VERSION` for query-only changes such as query expansion or FTS
weights; those changes do not change stored card vectors.

### Core ML bake-off backend

The bake-off can request Core ML for any registered ONNX candidate on a Mac.
Compare the same model and document layout through both backends. For example:

```sh
cargo build --release --example query_bakeoff
./target/release/examples/query_bakeoff --only bge-small-full --backend cpu --format production
./target/release/examples/query_bakeoff --only bge-small-full --backend coreml --format production --json --save /tmp/bge-coreml.json
```

Fastembed downloads models already converted to ONNX. The Core ML backend
uses each model's registered pooling, quantization mode, and token limit. It
sets fixed tokenizer padding and named input dimensions for an ML Program.
Most models use 32-row batches. Dynamically quantized models use one-row
batches because their outputs can change with batch composition. A partial
final batch has dummy rows that are removed from the results. Production
document embeddings use Core ML on macOS. Production query embeddings use the
CPU encoder with the same model weights, avoiding fixed-batch overhead for
single-query requests. Other operating systems use CPU inference for both.
Core ML registration alone does not prove GPU use: check the compute plan for
`MLGPUComputeDevice` on the model's main operations. Measure each candidate
before calling it accelerated.

Check vector similarity and ordered query results against the CPU run before
using a Core ML vector cache for a model comparison. Include model load time
when comparing rebuild costs. Run `stm sync` after changing the production
model or document layout. It rebuilds stale vectors before search uses the new
encoder.

Other online variants include a BGE Small Core ML `.mlpackage` and ONNX float16
weights. The `.mlpackage` needs a separate Core ML loader and cannot replace an
ONNX file in fastembed. Neither variant removes the need to verify operator
placement, embedding compatibility, and full workload speed on its target
hardware.

## Golden queries and labels

`benches/golden_queries.json` is the shared evaluation set. Each entry has a
query, relevant card names, an optional filter object, and a short note. It can
also include `not_relevant` for explicit negative judgments and `group` for an
explicit report group. Unknown filter fields fail parsing. The evaluator
converts filters through the same `cli::CardFilters` to
`search::CardFilters::from_cli` path as the command.

The evaluator checks that every labeled card exists and passes that query's
filters. Keep labels exact and review them when card data or query intent
changes. Update `common::LABEL_VERSION` in `examples/common/mod.rs` whenever
the positive labels, negative labels, or query set changes. Reports include
that version and an ordered fingerprint of card names and rendered documents.

The current fixed split is based on query data, not row position:

- Queries with relevance labels and no filters are development queries.
- Queries with filters are held-out queries.
- Queries with no relevant labels are behavior probes and do not enter
  relevance means.

Repeated query text is evaluated by query index and filter values, so identical
text with different filters remains distinct.

The metrics use the explicit `relevant` list as the positive set. An unlisted
card therefore receives no relevance gain in the score. The reports also show
the pooled results, explicitly judged cards, and unjudged cards. Review those
unjudged cards before treating the labels as complete, especially for broad
role or mechanic queries. Add a positive label for a plausible relevant card
or add it to `not_relevant` after review.

The tools report recall@10, mean reciprocal rank (MRR)@10, and normalized
discounted cumulative gain (nDCG)@10. The rank sweep also reports candidate
recall@20 for FTS, vectors, and the union of both candidate lists. Empty-label
behavior probes report result counts and names instead of contributing zeroes
to the relevance averages.

### Agent-assisted label review

The review tool exports candidates from the current baseline and saved
bake-off reports. It keeps query index and exact card name as the item key, so
repeated query text with different filters stays separate. It includes query
intent, filters, reviewed examples, complete card rules text, tags, and each
source model's FTS, vector, and hybrid rank. It does not ask the agent to trust
an automatic relevance guess.

Export batches of 100 items. The default batch index is zero; pass each index
from zero through the reported batch count minus one. Keep every file from the
same export run until the review is complete:

```sh
cargo run --release --example query_label_review -- --export \
  --batch-size 100 --batch-index 0 --save /tmp/query-review-00.json
```

Give the JSON file to an AI agent with these instructions: judge whether each
card satisfies the query intent under its filters; use card rules text and
reviewed examples; require the intended card for exact-name queries; do not
count a word match alone for role queries; return exactly one `relevant`,
`not_relevant`, or `unsure` decision for every `item_id`; give a short reason
based on card facts; do not invent facts, omit items, or edit the golden file.
The batch includes an exact response schema and its `batch_id`, label version,
and corpus fingerprint. The agent returns a separate JSON response file with
those identities and a `decisions` array. `unsure` means leave the item
unlabeled.

Inspect every proposal before applying it. First run the dry review, which
checks every response identity and item and prints the decisions and reasons:

```sh
cargo run --release --example query_label_review -- --review \
  /tmp/query-review-00.json --response /tmp/query-review-00-response.json
```

A person must resolve disagreements or leave them `unsure`. Apply a reviewed
batch with explicit approval and an audit note:

```sh
cargo run --release --example query_label_review -- --apply \
  /tmp/query-review-00.json --response /tmp/query-review-00-response.json \
  --approve --approval-note "Reviewed batch 1"
```

Apply each batch before finalizing. The command rejects stale query, filter,
label, or corpus identities; missing, duplicate, and unknown items; conflicting
labels; missing cards; and cards excluded by filters. It writes approved
positive and negative labels to `benches/golden_queries.json`, leaves
`unsure` items untouched, and appends decisions and reasons to
`benches/query_label_review_audit.jsonl`. It does not advance the label version
per batch, so the other exported batches remain valid. After every batch has
been reviewed and applied, finalize the version:

```sh
cargo run --release --example query_label_review -- --finalize \
  --batch-count "$BATCH_COUNT" --approve --approval-note "All exported batches reviewed"
```

Finalization requires an audit entry for every batch index. Then regenerate the
Core ML bake-off report before comparing scores. Agent output alone is
not approval. Reports include label, corpus, and query fingerprints. Compare
reports only when those identities match. Tune on development queries and use
filtered queries only as a check unless they were frozen before tuning.

Check report compatibility before comparing candidates:

```sh
cargo run --release --example query_report_check -- --mode model first.json second.json
cargo run --release --example query_report_check -- --mode format first.json second.json
cargo run --release --example query_report_check -- --mode ranking first.json second.json
```

All modes require the same evaluation version, label version, corpus, and query
set. Model comparisons also require the same document format and ranking
settings. Format comparisons require the same model and prompts and the same
ranking settings. Ranking comparisons require the same model, prompts, and
format; they allow ranking settings to differ. The checker needs one candidate
per report for format and ranking comparisons. Regenerate older reports that
do not contain `query_fingerprint` before using this check.

## Baseline example

Run the baseline after the store is built:

```sh
cargo run --release --example query_baseline -- --json
```

The example:

- Runs every query through `query::run_search` for the production hybrid
  reference and end-to-end latency.
- Embeds the same prepared text, then calls the shared search path for FTS and
  vector diagnostics.
- Compares the ordered hybrid top 10 from both paths. It stops and prints
  both lists if they differ.
- Applies each query's filters to both legs and validates its labels before
  scoring.
- Reports quality overall and by query group, separates behavior probes,
  and returns a per-query review pool with judgment coverage.
- Separates complete `run_search` latency from query embedding and the
  prepared shared-search call.

Use `--json` for machine-readable output. The report includes the crate and
database schema versions, model, vector dimension, document version, card
count, label version, corpus fingerprint, settings, and ordered top-10 lists.

## Embedding-model bake-off

Run selected candidates with Core ML and save the stable comparison report:

```sh
cargo run --release --example query_bakeoff -- \\
  --only bge-small-full --backend coreml \\
  --format production,fielded,compact,name-type-rules-first,tags-first,tags-last,contextual \\
  --json --save benches/query_bakeoff.json
```

The bake-off gets production card documents from `embed::doc_for_row` by
default and sends each candidate's vectors through
`query::search_with_settings`. FTS,
filtering, sparse-ID mapping, candidate selection, tie-breaking, and fusion
are shared with `stm query`. Each candidate differs in its document and query
embedding contract.

Candidate vectors live under `<data-dir>/bakeoff/`. A cache records the model
identity, dimension, card count and order fingerprint, configured token
limit, document version, document and query prompts, pooling, and cache format
version and document format. The example rejects stale metadata and corrupt or
non-unit vectors. It rebuilds stale or damaged caches before ranking. Existing
production-layout caches remain usable.

The production `bge-small-full` candidate must match the ordered `run_search`
hybrid top 10 for every query. A mismatch stops the run before candidate
comparisons are reported. The check includes filtered and repeated-text cases.

### Candidate models

The registry includes compact English and multilingual encoders available in
the pinned fastembed version. Existing candidates include BGE Small and Base,
Arctic S and M, MiniLM-L6, Nomic v1.5, and Mixedbread Large. The bake-off records each model's prompt,
dimensions, token limit, pooling, and model-card link. External benchmark
scores help narrow candidates, but the local gold queries decide whether a
model works for Magic card search.

| Candidate key       | Model                           | Dimensions | Main reason to test                                                                                                      |
| ------------------- | ------------------------------- | ---------: | ------------------------------------------------------------------------------------------------------------------------ |
| `bge-small-full`    | BGE Small English v1.5          |        384 | Full-precision production model candidate                                                                                |
| `arctic-xs`         | Snowflake Arctic Embed XS       |        384 | Small retrieval model with low CPU and storage cost                                                                      |
| `e5-small`          | Multilingual E5 Small           |        384 | Multilingual model with explicit `query:` and `passage:` inputs                                                          |
| `gte-base`          | GTE Base English v1.5           |        768 | Stronger general retrieval model with long context                                                                       |
| `bge-m3`            | BGE-M3 dense output             |       1024 | Multilingual dense vectors; the model also supports sparse and multi-vector retrieval, which this candidate does not use |
| `embeddinggemma-q4` | EmbeddingGemma 300M, 4-bit ONNX |        768 | Recent small retrieval model with task and title prompts; its weights use Google's Gemma terms                           |
| `nomic-v2-moe`      | Nomic Embed Text v2 MoE         |        768 | Multilingual mixture-of-experts encoder with query and document prompts                                                  |
| `qwen3-0.6b`        | Qwen3 Embedding 0.6B            |       1024 | Instruction-aware retrieval model with long context and custom output dimensions                                         |

`nomic-v2-moe` and `qwen3-0.6b` use fastembed's Candle backends and run on the
CPU in this evaluator. Their model files use the Hugging Face cache; set
`HF_HOME` to control that cache. They need more memory and setup time than the
small ONNX candidates. The model report records their exact candidate settings.

### Compare card-document formats

Use `--format` to render the same card facts in several layouts. Save the
Core ML comparison to the stable report path:

| Format       | Layout                                                |
| ------------ | ----------------------------------------------------- |
| `production` | Current production document from `embed::doc_for_row` |
| `fielded`    | One labeled field per line                            |
| `compact`    | Labeled fields separated by a vertical bar            |

For example:

```sh
cargo run --release --example query_bakeoff -- \
  --only bge-small-full --backend coreml \
  --format production,fielded,compact \
  --json --save benches/query_bakeoff.json
```

The format name and ordered document fingerprint are part of each vector
cache key. Alternate layouts do not overwrite production vectors. Candidate
model instructions, such as E5's `passage:` or EmbeddingGemma's title prompt,
are applied after the selected layout is rendered.

### Add a model candidate

1. Add a `Candidate` entry in `examples/query_bakeoff.rs` with a unique key,
   fastembed model, output dimension, configured token limit, document and
   query prompts, pooling description, model-card URL, and the model-card
   sequence limit.
2. Check the model card for retrieval prompts, maximum sequence length,
   output dimension, pooling, and normalization. Check fastembed's model
   registry for the actual pooling and output dimensions used by this build.
   If the model needs output processing beyond fastembed's output, implement
   it in `normalize_embedding` for both card and query vectors.
3. Add assertions to `examples/tests/query_bakeoff_tests.rs` for prompt
   selection, pooling, dimension, token limit, and any output processing.
4. Run the candidate with `--only <key>`. Its first run downloads or loads
   the model and embeds the corpus. Later runs use its vector cache.
5. Compare its development metrics, held-out metrics, regressions, and
   unjudged result pool with the production reference. Do not compare scores
   from different label versions or prompts as if they used the same setup.

The cache identity includes both prompts and the corpus fingerprint. A
prompt, token limit, model, document version, or ordered corpus change makes
the old cache unusable. The production BGE parity gate is a check of the
shared production model, not a requirement for alternate models to return the
same ranks.

`--only` accepts any candidate key in the registry table. `--format` accepts
one or more of `production`, `fielded`, `compact`, and the ordering variants.

## Ranking and FTS sweep

Run the controlled ranking sweep with:

```sh
cargo run --release --example query_rank_sweep -- --json
```

The sweep first checks current defaults against `query::run_search`. It
measures FTS-only, vector-only, and hybrid controls, then varies one setting
at a time: each FTS column weight, FTS and vector candidate depths, RRF `k`,
FTS term matching (`any` or `all`), FTS and vector RRF leg weights, FTS
over-fetch multiplier, minimum SQL window, and retry rounds. Every comparison
uses the shared library path. `query_bakeoff` accepts the same ranking settings
as command-line flags, so a model or document format can be compared under a
selected ranking configuration.

The report includes recall@10, MRR@10, nDCG@10, candidate recall@20, group
means, per-query ordered hits, paired nDCG changes, and shared-search time.
It selects the highest development hybrid nDCG, uses MRR as a tie-breaker,
and keeps production defaults on an exact tie. It then checks the selected
settings against production defaults once on the filtered held-out set.
The sweep never changes CLI defaults.

Pass ranking choices to `query_bakeoff` with `--fts-weights
name,tags,type,oracle`, `--fts-depth`, `--vector-depth`, `--rrf-k`,
`--fts-rrf-weight`, `--vector-rrf-weight`, `--fts-terms any|all`,
`--fts-overfetch`, `--fts-min-rows`, and `--fts-max-rounds`. Reports record the
complete settings. The production BGE parity gate runs only for the production
document format, Porter tokenizer, and unchanged production ranking defaults.

`--fts-tokenizer porter|unicode61` tests FTS tokenization. Porter with Unicode
61 is the production tokenizer. Use Core ML for saved candidate reports.

End-to-end `run_search` latency, query embedding time, and shared-search time
are reported separately. The sweep copies the database with `VACUUM INTO` and
times an FTS5 rebuild on that temporary copy, so it does not change the user's
store. This does not include Scryfall ingestion. The bake-off reports
full-corpus candidate document embedding time on cache misses, which measures
vector embedding cost without writing over the production index.

## Testing search changes

Use the shared unit tests in `src/tests/query_tests.rs` for vector-row
alignment, sparse database IDs, FTS retries, filters before vector top-k,
empty vector stores, punctuation-only queries, vector dimensions, and fusion
ties. Use `examples/tests/query_bakeoff_tests.rs` for prompts, cache identity,
cache corruption, repeated query text, and the BGE parity gate. Use
`examples/tests/query_rank_sweep_tests.rs` to keep parameter changes isolated
and the development tie-break stable.

For a production change, run the test suite, Clippy, formatting, and the
`aislop` scan listed in `AGENTS.md`. With a populated store, build in release
mode and run the baseline and bake-off to check ordered parity and refreshed
quality results. Compare reports only when their model, prompts, store
fingerprints, settings, and labels match.

For a document-layout change, also rebuild or sync the store in release mode.
The production document version and candidate cache fingerprint must both
change as expected. For an FTS-only change, refresh the FTS content as needed
and rerun the baseline and ranking sweep; do not re-embed production vectors
unless the embedding documents changed.

## Research notes and references

The production FTS table uses SQLite's Porter tokenizer over Unicode 61 and
column-weighted BM25. FTS5 exposes per-column BM25 weights, but its built-in
BM25 does not expose the `k1` and `b` constants as query arguments. Testing
different `k1` or `b` values would need a custom ranking function. FTS5 prefix
indexes mainly help prefix terms; the current query builder does not emit
prefix terms. The tokenizer comparison is practical with the current schema:
the bake-off rebuilds a temporary FTS index for each selected tokenizer.

FTS5 source references:

- [BM25 ranking and column weights](https://www.sqlite.org/fts5.html#the_bm25_function)
- [Tokenizer and prefix-index options](https://www.sqlite.org/fts5.html#tokenizers)
- [fastembed Rust model registry](https://github.com/Anush008/fastembed-rs/tree/v6.0.3)

Model cards:

- [BGE Small English v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5)
- [BGE Base English v1.5](https://huggingface.co/BAAI/bge-base-en-v1.5)
- [Arctic Embed S](https://huggingface.co/Snowflake/snowflake-arctic-embed-s)
- [Arctic Embed M](https://huggingface.co/Snowflake/snowflake-arctic-embed-m)
- [MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2)
- [Nomic Embed Text v1.5](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5)
- [Mixedbread Embed Large v1](https://huggingface.co/mixedbread-ai/mxbai-embed-large-v1)
- [Arctic Embed XS](https://huggingface.co/Snowflake/snowflake-arctic-embed-xs)
- [Multilingual E5 Small](https://huggingface.co/intfloat/multilingual-e5-small)
- [GTE Base English v1.5](https://huggingface.co/Alibaba-NLP/gte-base-en-v1.5)
- [BGE-M3](https://huggingface.co/BAAI/bge-m3)
- [EmbeddingGemma 300M](https://huggingface.co/google/embeddinggemma-300m)
- [Nomic Embed Text v2 MoE](https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe)
- [Qwen3 Embedding 0.6B](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B)
