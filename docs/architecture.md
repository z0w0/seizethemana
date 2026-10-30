# Architecture

`stm` stores card data and ownership in SQLite, card embeddings in a local
vector file, and decklists in ManaBox text files. Scryfall supplies card data,
printing prices, and Tagger role labels. Commander Spellbook supplies combo
variants. Search runs locally after setup. Some read commands refresh stale
sources first. `--offline` skips automatic refresh, but does not block explicit
setup or sync, or downloads for a missing search model.

The CLI works without an AI assistant. The optional assistant skill supplies a
deckbuilding workflow; it calls the same commands. Ownership records and
decklists are separate: a planned deck does not prove that its cards are owned.

See [Querying](querying.md) for ranking and evaluation, [CLI design](design.md)
for command behavior, and [Deck simulation](simulator.md) for simulator rules
and limits. Start with [Contributing](../CONTRIBUTING.md) for build prerequisites,
offline tests, and first-change paths.

## Module map

`src/main.rs` dispatches commands to modules exported by `src/lib.rs`. Command
handlers manage I/O; search, ownership, and simulator modules hold the rules.

| Paths                                                                            | Responsibility                                                              |
| -------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `src/cli.rs`, `src/main.rs`, `src/output.rs`                                     | Arguments, dispatch, refresh hooks, exit codes, and human/JSON output       |
| `src/paths.rs`, `src/db.rs`, `migrations/`                                       | Store paths, status, schema, card reads, name aliases, and full-text search |
| `src/scryfall.rs`, `src/scryfall/`                                               | Bulk download, parsing, release gating, and card/print mapping              |
| `src/setup.rs`, `src/sync.rs`, `src/sync/`                                       | Ingest, embedding work, locking, publication, recovery, and freshness       |
| `src/embed.rs`, `src/embed/`                                                     | Document layout, encoders, and vector storage                               |
| `src/query.rs`, `src/query/`, `src/search.rs`                                    | Query expansion, snapshots, filters, retrieval, and rank fusion             |
| `src/tags.rs`, `src/spellbook.rs`, `src/combos.rs`                               | Tag snapshots and shared combo data                                         |
| `src/prints.rs`, `src/universe.rs`, `src/release.rs`                             | Pricing, universe classification, and release-date helpers                  |
| `src/card/`                                                                      | Card details, similar cards, and combo views                                |
| `src/collection*.rs`, `src/collection_sell/`                                     | Ownership import/search, statistics, conflicts, and sell decisions          |
| `src/deck/`                                                                      | Deck files, ownership views, legality, suggestions, audits, and simulation  |
| `src/tests/`, co-located `tests/` folders                                        | Offline tests, temporary-store helpers, and pinned simulator fixtures       |
| `benches/`, `examples/query_*.rs`, `examples/query_bakeoff/`, `examples/common/` | Criterion benchmarks, search evaluation, candidate caches, and label review |
| `.agents/skills/seizethemana/SKILL.md`                                           | Optional assistant instructions and deckbuilding workflow                   |
| `site/`                                                                          | Static website, browser assets, tests, and build script                     |
| `scripts/`, `.github/workflows/`, `.config/nextest.toml`                         | Maintenance tools, release packaging, and validation                        |
| `README.md`, `CONTRIBUTING.md`, `docs/`                                          | Usage, contribution tasks, and technical references                         |

## Storage layout

The default data directory is `~/.seizethemana/`. The global `--data-dir` flag
selects another root. `src/paths.rs::Paths` derives paths from that root.

```text
<data-dir>/
  stm.db                     SQLite database
  stm.db-wal / stm.db-shm     SQLite WAL sidecars
  store.lock                 Cross-process store lock
  bulk/default_cards.jsonl.gz Scryfall cards and per-print prices
  bulk/oracle_tags.jsonl.gz   Tagger oracle tags
  bulk/variants.json.gz       Commander Spellbook variants
  vectors.bin                Row-major embedding matrix
  status.json                Setup state, index metadata, sync timestamps
  sync-publication/          Staged vector/status pair for recovery
  models/                    Embedding model cache
  bakeoff/                   Separate search-evaluation vector caches
  decks/<name>.txt           ManaBox decklist
  decks/<name>.primer.md     Deck primer
```

### Status metadata

`status.json` joins each matrix row to a card name and records freshness:

| Field                              | Contract                                                                     |
| ---------------------------------- | ---------------------------------------------------------------------------- |
| `setup_complete`                   | Whether setup has published its index; `Paths::is_setup` reads this flag     |
| `ingested_cards`, `embedded_cards` | Setup counts; sync publication updates both to the vector row count          |
| `model`, `dim`                     | Embedding model identity and vector dimension                                |
| `names`                            | Card names in vector-row order                                               |
| `doc_version`                      | Embedding document layout version                                            |
| `scryfall_synced_at`               | RFC 3339 timestamp used by the 24-hour automatic refresh check               |
| `combos_synced_at`                 | Last successful combo refresh; used for status, not automatic refresh timing |

`Status::write` writes a temporary file, then renames it. Sync publishes index
metadata with the matching matrix before updating freshness timestamps.

## Data model

The SQLite schema is version 1 (`PRAGMA user_version = 1`).
`migrations/0001_initial_schema.sql` defines the current tables, fields,
indexes, and triggers. `db::open` prepares the schema, enables write-ahead
logging (WAL), and sets a 10-second busy timeout. The schema has twelve
application tables, including the FTS virtual table. SQLite also creates FTS
internal tables.

### Cards and names

`cards` has one row per unique card name. Deck and collection joins use
`cards.name`; Tagger joins use `oracle_id`. `token_names` records non-card
names, such as tokens and art cards, so import code can recognize them.

| Fields                                                       | Stored meaning                                                                    |
| ------------------------------------------------------------ | --------------------------------------------------------------------------------- |
| `name`, `oracle_id`                                          | Name lookup key and Scryfall oracle identity                                      |
| `mana_cost`, `cmc`, `type_line`, `oracle_text`               | Card facts and flattened face text                                                |
| `colors`, `color_identity`, `keywords`                       | JSON arrays stored as text                                                        |
| `power`, `toughness`, `loyalty`                              | Nullable printed values, including values such as `*`                             |
| `rarity`, `edhrec_rank`, `penny_rank`, `reserved`            | Rarity, popularity ranks, and Reserved List signal; nullable signals mean unknown |
| `legalities`, `game_changer`                                 | JSON format-legality map and nullable Commander Game Changer flag                 |
| `set_code`, `collector_number`, `scryfall_id`, `released_at` | Stored representative print identity and release date                             |
| `tags_text`                                                  | Tagger labels used by full-text search                                            |

Ingest selects a representative print per name. It prefers more game-platform
memberships, then a lower EDHREC rank, then an earlier release date and
Scryfall ID. Multi-face type lines and missing top-level mana costs use `//`;
rules text uses `\n// `. Colors combine the faces. Printed stats come from the
first face that defines them.

`--set` filters the stored `cards.set_code` in both ranked and filter-only
queries. It does not search all printings or inspect the set of owned copies.
Ownership still matches by name.

Ingest excludes non-card layouts and prints without paper membership. A
future-dated card enters only if some format legality is not `not_legal`.
This keeps upcoming reprints of existing cards searchable while excluding
never-released cards. Query code does not gate visibility on the stored
release date. Daily sync can add cards once they release.

### Full-text index

`cards_fts` is an external-content SQLite FTS5 table over `cards`, with
`content_rowid='id'`. Its columns are `name`, `tags_text`, `type_line`, and
`oracle_text`. The `porter unicode61` tokenizer stems terms, so "sacrifices"
can match "sacrifice".

Insert, update, and delete triggers on `cards` maintain the index.
`db::refresh_tags_text` updates searchable labels after tag ingest. This lets
role words match even when Oracle text does not contain them. Query building,
weighted BM25, and sparse card-ID mapping are covered in [Querying](querying.md).

### Owned copies

`collection` groups identical copies by
`(name, set_code, collector_number, foil, binder, binder_type)`. It stores
quantity and purchase price. Finish values are `normal`, `foil`, and `etched`;
location types are `binder` and `deck`.

The same print can have rows in several locations. ManaBox CSV import writes
these ownership rows and skips wishlist (`list`) rows. It does not write deck
files. Deck assignments and decklists are separate data, joined by deck name.

### Printings, prices, and sets

`card_prints` has one row per Scryfall print ID. Set codes are lowercase.

| Fields                                                       | Stored meaning                                            |
| ------------------------------------------------------------ | --------------------------------------------------------- |
| `scryfall_id`, `name`                                        | Print primary key and oracle card name                    |
| `flavor_name`                                                | Alternate printed name used for alias lookup              |
| `set_code`, `collector_number`, `lang`, `rarity`, `finishes` | Printing identity, language, rarity, and JSON finish list |
| `released_at`, `updated_at`                                  | Printing release date and row refresh timestamp           |
| `usd`, `usd_foil`, `usd_etched`                              | Nullable market prices in USD                             |
| `universes_beyond`                                           | Print-level universe classification                       |

Sync upserts prints and keeps rows absent from a later bulk. The
`(name, set_code, collector_number)` index supports print selection and
valuation. A partial index on nonempty `flavor_name` supports exact and unique
prefix aliases, such as a crossover name resolving to its oracle card.
`prints::price_ranges` batches price reads for deck views, suggestions, and
buylists. Store freshness comes from `scryfall_synced_at`, not individual
printing timestamps.

`sets` stores each set's name, type, block, and optional franchise.
`src/universe.rs` maps Universes Beyond families with curated set codes and
set-name rules. AFR, AFC, and CLB count as honorary Universes Beyond sets.
Secret Lair sets have no franchise and group by set name. Unknown UB-flagged
sets produce a sync warning. Universe data feeds display and collection/deck
censuses; legality and bracket checks do not use it.

### Tagger labels

`tags` stores stable tag UUIDs, slugs, labels, and global association counts
(`use_count`). Only the UUID is an identity; labels and slugs can change.
`card_tags` joins `(oracle_id, tag_id)` and stores the source weight.

The oracle-tags bulk is a complete snapshot. Ingest upserts tags and replaces
associations in one transaction. For a nonempty snapshot, it removes tag IDs
absent from the source. Art tags are outside the search model.

`tags::TagIndex` supplies role lookups and document labels. Embedding documents
use up to 12 labels, chosen by descending global use count with alphabetical
ties. Selection keeps at most one label per first word. Full-text content and
tag-overlap search are not limited to that document selection.

### Combo variants

`combos` stores Commander Spellbook variant IDs, produced features, mana
requirements, bracket tags, format legalities, popularity, and refresh times.
`combo_pieces` stores piece names, order, zones, and `must_be_commander`.
Multi-face pieces have one row per face. Zone codes are B (battlefield), H
(hand), G (graveyard), C (command), L (library), and E (exile).

`spellbook::ingest` replaces both tables in one transaction. A download failure
warns and leaves the old tables available. A parsing or database failure can
still fail setup or sync. Combo data does not affect card embeddings.

`combos::load_variants_for` finds candidate IDs in name chunks of 500, then
loads their pieces in one query. Shared format checks read the source legality
map and exclude commander-required variants from 60-card formats. Card combo
views, deck suggestions, and simulation use these reads.

### Embedding revisions

`pending_embeddings` stores each card's queued document revision.
`embedding_revisions` stores the revision acknowledged after publication. A
revision hashes the rendered document, `embed::DOC_VERSION`, and
`embed::MODEL_NAME`, so selected tag labels are part of its identity.

Card changes queue revisions in the same SQL transaction. Sync also compares
every current document with its acknowledged revision. This catches tag and
layout changes after an interrupted run, even when the next card diff is
empty. An already queued card remains work to retry.

## Setup and sync

Use a release build for workloads that embed cards. Debug inference is too
slow for a real setup or rebuild.

```sh
cargo build --release
./target/release/stm setup
./target/release/stm sync
```

Setup shares card, price, and tag ingest with sync. It embeds the full card
table, writes vectors, writes completed setup metadata, then acknowledges the
document revisions. Without `--force`, setup returns without rebuilding an
already completed store.

`sync::run_sync` holds the store lock while it:

1. Recovers any ready index publication and fetches Scryfall's bulk index.
   Card and tag files download when missing, over 24 hours old, or older than
   Scryfall's source timestamp. `sync --force` forces their download.
2. Streams card data once. It upserts print prices and sets, selects one card
   row per name, and applies new or changed cards with their queued revisions
   in one transaction. Rank-only signal changes do not queue embeddings.
3. Refreshes set/universe metadata, ingests tags, and updates FTS label text.
4. Refreshes Commander Spellbook variants. Download failures warn without
   blocking card-index work.
5. Embeds queued or changed documents and publishes the vector/status pair.
   Existing vector rows retain their order; new names append. Model,
   dimension, or document-version changes rebuild the full matrix.
6. Acknowledges exact published revisions and clears their queue rows. It
   writes `scryfall_synced_at` only after embedding work is complete, using
   the time captured after downloads. It stamps combos only after a
   successful combo refresh.

Text `query` and `collection query` commands wait for automatic sync when card
data is over 24 hours old. Other handlers, including card detail, similarity,
and combo views, also call the shared freshness check. A failed automatic sync
warns and lets the read proceed. The read can still fail if the index is
incompatible or out of alignment. An incomplete card refresh leaves the old
freshness stamp so a later read retries. Combo failure alone does not make
card data stale; the next sync attempts the combo refresh again.

No-text query listings skip automatic refresh, model loading, vector loading,
and query embedding. They use SQLite filters and alphabetical names. See
[query modes](querying.md#text-search-and-filter-only-listing).

## Publication recovery and consistent reads

Sync stages a complete `vectors.bin` and matching `status.json` under
`sync-publication/`, flushes both files, then writes a `ready` marker. Recovery
keeps the staged pair until both public replacements complete. A later run
can repeat those replacements if the process stops between them. Only after
publication succeeds does sync acknowledge the exact queued revisions.

`query::RankedSnapshot` holds the store lock through recovery, vector loading,
SQL scope/card/FTS reads, metadata checks, query embedding, and result details.
Price and ownership reads use its connection. It opens that connection after
locking, so a completed forced setup cannot leave it attached to an old
database. Sync and forced setup wait for the same lock.

A SQL read transaction also isolates ranked results from writers that do not
take the store lock. No-text CLI listings use a read transaction on the
caller's connection without locking or loading the vector index. Both query
modes retain their SQL view through result-detail reads.

`VectorStore::load_locked` reuses a caller's guard; `load` and `load_owned`
acquire their own. These loaders recover ready publications before validating
matrix size, dimension, and name count. The mapped matrix is read-only, but
loading the store can write recovery files. Database open can also write schema
state. `--offline` skips automatic network refresh; it does not make the data
directory immutable or disable explicit setup or sync. URL imports reject
offline mode.

## Search index

Production uses full-precision `BAAI/bge-small-en-v1.5`, 384 dimensions, a
128-token limit, and eight inference threads. Card documents contain identity,
structured facts, selected role labels, and Oracle text. Queries use expanded
role vocabulary and the BGE retrieval instruction. macOS uses Core ML for
document batches and CPU inference for queries. Other systems use CPU for both.

`vectors.bin` holds unit-normalized, little-endian `f32` rows. Names must align
with `db::load_all_cards` order. Queries scan the full mapped matrix with dot
products, which equal cosine similarity for normalized vectors. There is no
approximate-nearest-neighbor index.

Text search fuses weighted BM25 and vector ranks with reciprocal-rank fusion
(RRF). Filters and allowed-name scopes apply before candidate selection.
The normalized fused score is not a probability or a raw cosine score. See
[Querying](querying.md) for settings and the shared search API.

`card similar` instead fuses shared-tag counts with similarity to the seed's
stored vector. It does not load an embedding model. Without a usable seed
vector, it uses tags alone, reports JSON `score: null`, and adds a human note.
`--owned` filters both candidate sets to binder-owned names; deck-only copies
do not qualify. JSON adds `score`, `shared_count`, and `shared_tags` to the
card object. Its ranks can differ from text collection search.

## Schema and destructive reset

`migrations/0001_initial_schema.sql` is the complete v1 schema. Keep table,
index, and trigger definitions there. The tables above define the current
store. Embedding metadata must match the current model, dimension, and
`doc_version`. Sync rebuilds all vectors when those values differ. Ranked
search requires a matching index.

When stored document text changes, advance `embed::DOC_VERSION` and update its
tests. Query expansion and ranking changes alone do not require re-embedding.
See [card document layout](querying.md#card-document-layout).

`stm setup --force` is a destructive, noninteractive reset. It warns, closes
the old connection, deletes the database and its WAL sidecars, removes vectors
and status, and discards staged publication files. This loses all collection
records, deck assignments, and purchase prices. Saved decklists and primers
remain. Stop other `stm` processes and back up the whole data directory before
using it. Use `stm sync` for card refreshes and derived-vector rebuilds.

## Deck files and ownership

Decklists and primers live under `decks/`; ownership lives in SQLite. Collection
import writes ownership only. Deck import and update write decklists. Deck
deletion removes the decklist and primer, not ownership.

The ManaBox grammar preserves sections, quantities, print details, and foil
markers. It recognizes print details only in a ManaBox-shaped `(SET) CN` tail
so parentheses inside card names survive. Import normalizes ManaBox's blank
line between commanders and the rest of a deck. Print details describe the
intended copy; updates and ownership match by card name.

`SIDEBOARD` and `MAYBEBOARD` are bench sections. They do not count toward the
main deck's size, curve, ramp, or Game Changer cap, but their cards still receive
format and Commander color-identity checks. The five ordinary basic lands are
unlimited for ownership and excluded from deck cost. Wastes and snow basics
remain limited, priced inventory.

Deck analysis separates deterministic legality checks from review advice.
Suggestions and buylists read the relevant deck assignments and binder pool;
they do not move copies. See [CLI design](design.md) for the command contracts.

### Simulator runtime

```text
CardRow -> oracle_parser -> OracleCard (typed syntax and source spans)
        -> oracle_lower -> SimCard and ability support evidence
        -> SimDeck + RunConfig + per-game RNG
        -> BatchRunner -> GameLog -> streaming statistics
        -> findings -> SimReport -> output or baseline comparison
```

Rules come from card type data, keywords, and Oracle text, not spell-name
checks. Parsing retains wording; lowering turns supported text into executable
data. A required unsupported cost or condition leaves the ability inactive.
Compiler support records do not prove every static field or runtime behavior.
Role classifications supply diagnostics, not rules.

Typed data covers supported physical land-type gates, live flat generic
spell-cost reducers, singular Behold choices, and bounded library selection and
multiple-result searches. Runtime code executes that data without reparsing
Oracle text. Fixed payment and selection policies still limit the result; a
recognized clause is not proof of complete card execution.

Workers share the compiled deck but own each game's mutable state. They do not
read the database or print output. The runner derives separate `Opening` and
`Library` random streams from the master seed and game ID, then returns logs
in game-ID order. Opening shuffles and mulligans use `Opening`; runtime library
shuffles use `Library`. Worker count and batch size do not change those
streams. Pilot and opponent choices are deterministic.

Supported events settle at action boundaries, not through a full Magic stack.
Settlement, main-action, and activation limits can mark games incomplete.
Zero life or a failed draw ends a game at a supported resolution boundary.
The opponent model tracks scheduled actions and resources; it does not model
blocking, counterspells, or removal. Interaction reports measure readiness.

The production runner aggregates batches without retaining all game logs.
Percentile samples still grow with game count. `SimReport` uses schema version 3
and model and policy versions 2. It records input provenance, opponent settings,
compiler evidence, outcomes, game phase counts, and sampling evidence. Finished
games include player losses; incomplete games do not enter primary estimates.
Excluding incomplete games can bias engine-heavy decks. Per-turn means use
observations from the relevant completed phase. Missing observations are not
zero; percentages use 0 to 100.
Combat is potential output, not applied opponent damage or a win rate.

Baseline comparisons reject incompatible rules, policies, random-number
contracts, engine versions, formats, and scenarios. Card-data fingerprints can
be null, so provenance does not always confirm matching Oracle inputs. Source
versions and input data affect repeatability. See [Deck simulation](simulator.md)
for fixture provenance, denominators, assumptions, and supported behavior.

The separate [Commander source bundle](simulator.md#expanded-commander-sources)
contains 1,208 captured Oracle identities, not 1,208 supported or Commander-legal
cards. It preserves site-ordered EDHREC inputs, complete selected land cycles,
and a Commander-legal Behold query. Its compile, conservation, turn, and report
smoke checks do not replace named runtime assertions. Historical source bundles
and compiler pins remain separate.
