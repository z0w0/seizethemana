# Architecture

How `stm` stores data, builds the search index, and keeps everything fresh.

## The short version

One download of Scryfall's daily oracle bulk powers the card side: card
data, prices, and the local embedding index. A second small download (the
daily oracle-tags bulk from Tagger) powers role labels and tag-overlap
lookups. `stm setup` builds the store from scratch; `stm sync` refreshes
everything; read commands refresh automatically when the data is more than
a day old. Decks are plain ManaBox txt files on disk, so you can keep
editing them in ManaBox or by hand.

## Storage layout

Everything lives under one data directory (default `~/.seizethemana/`,
overridable with the global `--data-dir` flag):

```
<data-dir>/
  stm.db                  SQLite database (schema below)
  stm.db-wal / -shm       SQLite WAL sidecar files
  bulk/default_cards.jsonl.gz  Scryfall default-cards bulk (every English
                          printing, with per-print prices)
  bulk/oracle_tags.jsonl.gz    Scryfall Tagger oracle-tags bulk
  vectors.bin             f32 matrix, row-major, 384 dims per card
  status.json             Setup state + vector index metadata + sync stamp
  models/                 fastembed ONNX model cache
  decks/<name>.txt        Deck lists (ManaBox txt format; written by
                          `deck import` from a ManaBox deck txt export, or
                          edited by hand / `deck update`)
  decks/<name>.primer.md  Deck primers (empty stub until written)
```

All paths are derived from [`crate::paths::Paths`], so tests can redirect the
whole store with `--data-dir`.

### `status.json`

One state file with setup state, index metadata, and the sync stamp:

| Field | Meaning |
| --- | --- |
| `setup_complete` | True when `stm setup` finished; the not-set-up check reads this |
| `ingested_cards`, `embedded_cards` | Counts from the last setup run |
| `model` | Embedding model name (e.g. `BAAI/bge-small-en-v1.5-Q`) |
| `dim` | Vector dimension (384) |
| `names` | Card names in vector-row order (joins `vectors.bin` rows) |
| `synced_at` | RFC 3339 timestamp of the last full sync; drives the 24h refresh check |
| `doc_version` | Document layout version the vectors were built with |

Setup writes this file last, via temp file + rename. A failed run leaves the
store marked not set up. `stm sync` updates `synced_at` after card data and
prices land.

## Data schema

Three tables plus two print tables, created by `src/db.rs::open` (WAL mode
is on; see the migration notes at the end).

### `cards` — the oracle snapshot

One row per card **name**. The bulk is oracle-level: one row per card, not
one per printing.

| Column                                 | Notes                                                 |
| -------------------------------------- | ----------------------------------------------------- |
| `name`                                 | UNIQUE; lookup key for deck and collection joins      |
| `oracle_id`                            | Scryfall oracle ID                                    |
| `mana_cost`, `cmc`, `type_line`        | Multi-faced cards are flattened (`{2}{B} // {B}`)     |
| `colors`, `color_identity`, `keywords` | JSON arrays (stored as text)                          |
| `power`, `toughness`, `loyalty`        | Nullable; `*`-style stats stored as printed           |
| `oracle_text`                          | Faces joined with `\n// `                             |
| `rarity`, `edhrec_rank`                | `edhrec_rank` nullable                                |
| `legalities`                           | JSON object: `{"modern":"legal",...}`                 |
| `set_code`, `collector_number`         | Representative print for the name (display identity)  |
| `scryfall_id`                          | Print ID of the representative print (display only)   |
| `released_at`                          | YYYY-MM-DD of the oracle's latest recognized printing; display only, no gating |
| `game_changer`                         | True when on the Commander Game Changer list (bracket signal for `deck legal`); null when unknown |

Indexing covers `type_line`, `colors`, `rarity`, and `edhrec_rank` for
filter queries, and the `cards_fts` FTS5 index (below) covers keyword
search. The vector index handles semantic lookup (below).

### `cards_fts` — full-text search index

A SQLite FTS5 virtual table over `cards`, declared external-content
(`content='cards', content_rowid='id'`) so the text is stored once. Columns:
`name`, `tags_text`, `type_line`, `oracle_text`; tokenizer
`porter unicode61` (stemming, so "sacrifices" matches "sacrifice").
`tags_text` carries the card's Tagger labels (filled by `db::refresh_tags_text`
after tag ingest in both setup and sync), so role words like "ramp" or
"sweeper" resolve through the community vocabulary even when oracle text
never uses them. Four triggers on `cards`
(insert/update/delete) keep the index in sync with every write path, and
migration v1 populates it with a final `rebuild`. Query building and BM25
ranking live in `src/db.rs` (`fts_query`, `fts_search`; BM25 weights
name 8 / tags 4 / type 2 / oracle 1, ties break toward lower EDHREC
rank). Player shorthand expands before retrieval — `query::expanded_text`
appends Tagger-vocabulary aliases (see `EXPANSIONS` in `src/query.rs`)
to both the BM25 terms and the embedded query.

### `collection` — owned copies

One row per group of identical copies.

| Column                                 | Notes                                   |
| -------------------------------------- | --------------------------------------- |
| `name`                                 | Joins to `cards.name`                   |
| `set_code`, `collector_number`, `foil` | Print identity; `foil` is `normal`/`foil`/`etched` |
| `binder`, `binder_type`                | Location: binder name + `binder`/`deck` |
| `quantity`, `purchase_price`           | Copy count and cost basis               |

The key is `(name, set_code, collector_number, foil, binder, binder_type)`.
The same print can sit in several locations (2 in a binder plus 1 in a
deck). Rows come from `stm collection import` (ManaBox CSV). Wishlist
(`list`) rows are skipped.

### `sets` and `card_prints` — per-printing identity and prices

One `card_prints` row per physical printing, harvested from Scryfall's
default-cards bulk. Set codes are lowercase everywhere in this store.

| Column | Notes |
| --- | --- |
| `scryfall_id` | PRIMARY KEY; Scryfall print ID |
| `name` | Oracle card name; joins to `cards`/`collection`/decks |
| `flavor_name` | Just-for-fun printed name (Godzilla series, Secret Lair crossovers); empty for regular prints |
| `set_code`, `collector_number` | Print identity, lowercase set code |
| `lang` | ISO language code; default `en` |
| `rarity`, `finishes` | Finish list is a JSON array (`["nonfoil","foil"]`) |
| `released_at` | This printing's release date |
| `usd`, `usd_foil`, `usd_etched` | Latest USD prices, nullable |
| `updated_at` | Refresh timestamp for the row |

The index on `(name, set_code, collector_number)` serves cheapest-print
picks and the collection-valuation join. Price reads go through
`prints::price_range` (one name) or `prints::price_ranges` (many names:
two window-function statements per batch instead of two per name — deck
views, suggest, and buylist all batch). The partial index on
`flavor_name` (non-empty rows only) backs alias resolution: a flavor name
like "Godzilla, King of the Monsters" resolves to its oracle card
("Zilortha, Strength Incarnate") for exact and unique-prefix lookups.

`sets` maps lowercase set code to full set name (Card Kingdom's buylist
matches by name). `stm sync` upserts both tables; prints that drop out of
the bulk keep their last known row. Freshness is tracked once in
`status.json` (`synced_at`), not per row.

### `tags` and `card_tags` — Tagger oracle tags

Community-maintained functional role labels (from the Tagger project via
Scryfall's daily `oracle_tags` bulk). Art tags (illustration descriptions)
are out of scope: they join through the unique-artwork bulk and describe
artwork, not deck roles.

| Table        | Columns | Notes |
| ------------ | ------- | ----- |
| `tags`       | `id` (stable UUID, PK), `slug`, `label`, `use_count` | `use_count` is the global tagging count, a popularity signal |
| `card_tags`  | `oracle_id`, `tag_id`, `weight` (PK on the pair) | joins to `cards.oracle_id`; rewritten wholesale on every tags ingest |

Only the tag `id` is treated as an identity — slugs and labels can change
between daily files. The bulk is a complete snapshot, so ingest replaces
`card_tags` and upserts `tags` in one transaction. `src/tags.rs` owns
parsing (`TagRecord`), ingest, and the in-process [`TagIndex`] lookup, which
also picks the embedding-document labels (below). Tag refreshes never
trigger re-embedding: only card-content changes or a document-layout bump
do.

### `combos` and `combo_pieces` — Commander Spellbook variants

One `combos` row per combo variant, harvested from Commander Spellbook's
daily variants bulk (`json.commanderspellbook.com`, ~28 MB gzipped).

| Column              | Notes                                                   |
| ------------------- | ------------------------------------------------------- |
| `id`                | Spellbook variant ID (PK)                               |
| `produces`          | JSON array of feature names ("Win the game")            |
| `mana_value_needed` | Total mana the combo needs                              |
| `bracket_tag`       | Spellbook bracket letter (R/S/P/O/C/E; B = commander-banned) |
| `legalities`        | JSON map, format name → legal (16 keys; the backend forces 60-card keys false when a piece must be the commander) |
| `popularity`        | Spellbook popularity count                              |
| `updated_at`        | Refresh stamp                                           |

`combo_pieces` is one row per piece: `combo_id`, `name` (each face of a
two-faced card is its own row), `ordinal`, `zones` (JSON array of Spellbook
zone codes: B battlefield, H hand, G graveyard, C command, L library, E
exile), and `must_be_commander` (the piece must be the commander; the combo
cannot fire in a 60-card format). The bulk is a complete snapshot, so
ingest replaces both tables in one transaction (`src/spellbook.rs`); a
failed download warns and leaves the old tables in place.

`src/combos.rs` owns every read: a set-based
`load_variants_for` (candidate ids by name chunks of 500, then one bulk
load, never per-variant queries), `requires_commander` (any piece flagged),
`variant_legal_in` (legality-map lookup), and `filter_for_format` (legality
plus the commander-required exclusion for 60-card formats). Consumers:
`card combos`, `deck suggest` completions, and `deck simulate` assembly.

## Sync pipeline (setup and `stm sync` share it)

`src/sync.rs::sync_cards` streams the card bulk once and does everything in
that single pass:

1. **Download.** One `GET /bulk-data` call returns both file coordinates.
   Each file downloads when missing, older than 24h by mtime, or when
   Scryfall's `updated_at` for the file is newer than the local copy — so
   the daily refresh actually sees new data instead of reusing a stale
   file.
2. **Diff.** Keep the best print per name (setup's dedup rules), compare a
   content signature (cost, cmc, type line, colors, keywords, stats, oracle
   text, release date, game changer) against stored rows → added / changed.
3. **Apply.** Update changed rows in place, insert added rows (one
   transaction).
4. **Prints.** Upsert one `card_prints` row per printing seen in the bulk
   (second transaction; `updated_at` uniform) and harvest set names into
   `sets`. Prints absent from the bulk keep their last known row.
5. **Tags.** Ingest the oracle-tags bulk (third pipeline step in `tags.rs`):
   upsert `tags`, replace `card_tags`. Never triggers re-embedding.
6. **Embed.** Load the vector store; for each added/changed name, embed the
   rebuilt document and write the vector in place (row order stable, new
   names append). When `status.json.doc_version` lags behind the current
   document layout (`embed::DOC_VERSION`), every stored card re-embeds once
   and the version stamps current — no `--force` needed for layout bumps.
   Save `vectors.bin` + rewrite `status.json` names/counts.
7. **Stamp.** `synced_at` = now, last of all — an interrupted run stays
   stale and is retried by the next read command.

Setup runs the same pipeline, just with every bulk row landing as "added",
followed by a full embedding pass. Read commands (`query`,
`collection query`) trigger `sync` quietly when `synced_at` is older than
24h; `--offline` skips it and failures degrade to a warning, never blocking
the read.

## Vector index and query path

Search is hybrid: a full-text (BM25) leg and a vector (semantic) leg, fused
by reciprocal rank fusion (RRF). `src/embed.rs` owns the vector side.

1. **Documents.** Each card becomes one text (`build_doc`):

   ```text
   Lightning Bolt
   {R} · Instant
   Keywords: none
   Colors: R
   Tags: burn, direct damage
   P/T: 2/2            (creatures; Loyalty: 4 for planeswalkers)
   Deal 3 damage to any target.
   ```

   The name on its own line makes exact-name lookups rank first; the
   keyword/color/stat lines carry facts the text model underweights. The
   `Tags:` line carries Tagger's community role labels — up to 12 labels
   picked by global popularity with one label per first-word family (see
   `tags::select_labels`), so role vocabulary like "wheel" or "sacrifice
   outlet" matches even when oracle prose does not say it. The line is
   omitted when a card has no tags. The layout has a version
   (`DOC_VERSION` in `src/embed.rs`); `status.json` records which version
   the vectors were built with, so layout changes trigger a one-time
   re-embed on the next sync.
2. **Model.** `BAAI/bge-small-en-v1.5` (quantized ONNX via fastembed),
   384 dims, max length 128 tokens, 8 threads. Picked after a speed and
   quality bake-off; the numbers live in comments in `src/embed.rs`.
3. **Storage.** Unit-normalized vectors in `vectors.bin` (little-endian
   f32, row-major) plus metadata in `status.json` (model name, dim, card
   names in row order, `doc_version`). Both files are written via temp
   file + rename, so a crash cannot leave a half-written store.
4. **Vector leg.** The query text gets the BGE instruction prefix
   (`Represent this sentence for searching relevant passages: `), is
   embedded, normalized, then scored against every row by dot product. With
   normalized vectors, dot product equals cosine. A full scan of 32k × 384
   f32 takes well under 50 ms, so no ANN index is needed at this size.
5. **Full-text leg.** SQLite FTS5 over `name`, `type_line`, and
   `oracle_text` (`cards_fts`, external-content on `cards`, porter
   stemming, kept in sync by triggers on `cards`). The query becomes one
   quoted OR term per word (`db::fts_query`); BM25 ranks with column
   weights 8/2/1 (name hits dominate). Punctuation-only queries skip this
   leg.
6. **Fusion.** Reciprocal rank fusion (`query::fuse_rrf`, k = 60): each
   leg's rank contributes `1/(k + rank)`; a card ranked well by both legs
   beats a card ranked first by only one. The reported `score` is the RRF
   sum normalized to `[0, 1]` (first on both legs = 1.0). Filters apply
   inside each leg before fusion; EDHREC rank breaks fusion-score ties so
   popular cards surface first, then name for determinism. Cut to
   `--limit`.
7. **Ingest-time release gating.** The store never holds never-released
   cards: `sync` skips oracle cards dated in the future with no format
   memberships (`scryfall::should_ingest`). Released cards stay visible even
   when the bulk picks a future reprint as the representative print; the
   daily sync adds newly released cards automatically.

`stm card similar` bypasses both legs: it ranks by shared oracle tags
(one grouped SQL query over `card_tags`, ordered by shared-tag count, then
EDHREC rank, then name) and returns the shared labels per hit. `--owned`
restricts to collection names. This complements semantic search: when the
user has a known exemplar card, tag overlap finds "plays like this" cards
the vector model may rank lower.

`status.json` records the model name and document version. If the model
ever changes, rebuild the store with `stm setup --force`; document-layout
changes re-embed automatically on the next sync.

## Decks (`src/deck/`)

Deck contents live in ManaBox txt files under `decks/`. The module has five
parts:

- **`grammar`** — pure parser and writer. Sections (`// NAME`) and entries
  (`qty Name (SET) cn [*F*]`); set/cn/foil optional. Only a ManaBox-shaped
  `(SET) CN` tail counts as print info, so card names with parentheses
  survive. A blank line inside `// COMMANDER` splits the section into
  `COMMANDER` + `DECK`: ManaBox exports the commander(s), one blank line,
  then the rest of the deck. Print info is preserved on the line but never
  used as a key; update ops and ownership both match on the card name.
- **`store`** — deck files on disk: `create`, `list`, `show` (with `own N/M`
  from the collection, `(+N elsewhere)` binder hint, primer path), plus the
  owned-count lookups and deck pricing (unit price per card, owned value,
  missing cost — basics excluded). Ownership matches by card **name**, not
  by print: any set version you own fills a deck line, so reprints in the
  collection cover lines that name a different printing. Print info on a
  deck line stays descriptive (which copy to sleeve), never a match key.
- **`update`** — op parsing (`[section:]qty Name`) and math: `--add` adds
  copies, `--remove` takes them away (line deleted at 0), `--set` pins an
  exact count (`0` deletes). `--add`/`--set` names are validated against
   the oracle: unknown names exit 3 (with "did you mean" candidates),
   token names add with a warning. Unqualified removals
   target `DECK` first, then fall back to other sections with a note.
  `set`/`remove` of absent cards exit 3 and name them.
- **`legal`** — format and Commander-bracket legality. Deterministic
  checks only: deck size (exactly 100 maindeck for commander — the
  `// SIDEBOARD` section is a wishlist there and does not count; 60+
  maindeck and sideboard ≤ 15 for constructed, sideboard subtracted),
  copy limits (4, singleton for commander-style formats; basics and cards
  whose oracle text allows "any number" are exempt), commander rules
  (exactly 1, or 2 with Partner/`Friends forever`; legendary creature,
  planeswalker, or legendary Vehicle/Spacecraft with a printed
  power/toughness box — the Edge of Eternities rules change), commander
  color identity, per-card format legality
  (`legal`/`restricted` pass; `banned`/`not_legal` fail), and Game Changer
  count vs bracket (0 for brackets 1–2, ≤ 3 for bracket 3, unlimited 4–5,
  from the `cards.game_changer` column). Judgment calls (tutor density,
  early extra turns, mass land destruction, combo speed) are printed as a
  review checklist, not a verdict. Exit 0 = legal, 1 = violations.
- **`io`** — decklist `import` (upsert by name; ManaBox's whole-deck-under-
  `// COMMANDER` quirk normalized; interactive commander pick on a TTY) and
  `export` (`--force` guard), plus the primer file (`deck primer <name>`
  prints the markdown on stdout; `--set <file>` replaces it; an empty
  primer prints a note instead of nothing).
- **`buylist`** — `deck buylist <name>`: missing copies (deck-assigned +
  binder copies autofill slots read-only; other decks never count; basics
  excluded) as the cheapest released English printing of the right finish.
  Output formats: plain `2x Name` lines (default), Card Kingdom CSV (name,
  full set name, foil, qty), TCGPlayer Mass Entry CSV, or JSON with per-row
  prices and the estimated market total. Nothing is moved.
- **`simulate`** — `deck simulate <name>`: Monte Carlo goldfish
  (`stm deck simulate`), split into the `simulator/` submodule tree
  (`model`/`parse`/`deck`/`game`/`aggregate`/`report`). Pure core driven
  by one seeded `ChaCha8Rng` (`run_game`), aggregation and findings
  separate, so a fixed `--seed` reproduces a run exactly (the
  simulate → deck update → re-simulate workflow depends on that). Cards
  are modeled as data, not as rules: `parse` converts oracle text into a
  tap yield (one tap = the listed mana: "or" choices, fixed simultaneous
  sets like Jegantha, colorless, any-color), station tiers
  (`{N+}` striations per CR 702.184/721; only the P/T striation animates),
  crew costs (from the Crew keyword), and abilities (activated, ETB,
  upkeep, attack, cast-spell triggers with draw/tutor/tokens/mana/
  counters effects). `game` plays best-case turns: play an untapped land
  when possible (enters-tapped honored per oracle; shock-dual life
  payments always paid), cast the cheapest pip-payable spells, fire ETB
  triggers (tokens become battlefield bodies), spend leftover mana on
  unlocked activations, then spend remaining creature taps — mana only
  while casting still needs it, then station the highest-threshold
  spacecraft/planet, then crew Vehicles. Station tiers unlock
  permanently; crew animations last the turn. Commanders cast with the
  full pip check (a 5c commander needs one of each pip) and count as a
  draw engine when their oracle shows a repeatable draw. Cost
  reductions (warp, improvise, affinity) approximate to flat cuts. The
  sideboard never enters the library: it is a wishlist, not a legal zone.
  Reported per run: opening-hand shape, land-drop curve, commander
  on-curve timing, unspent mana, cards seen, role access, color screw
  (enough mana but the wrong colors), per-card castability, station
  online metrics, and bodies/engines timelines. Findings (mana
  screw/flood, color screw, commander late, draw starvation, dead cards,
  starved categories) carry a category + magnitude suggestion and drive
  exit 1. Model limits are printed in `assumptions`: enters-tapped
  honored (shock duals untapped), no opponents or interaction, draw
  engines fire once per turn on a fixed delay, no commander recast tax,
  flat body power (2) for stationing and crewing, hybrid pips pay from
  any of their colors, X-costs pay for one, and
  energy/metalcraft/converge/proliferate/replay mechanics are not
  modeled. This is a consistency diagnostic, not a win-rate predictor.
  The living reference for the model, assumptions, and limits is
  `docs/simulator.md`.

Basic lands are treated as unlimited throughout: `deck show` counts them as
owned (marked `(basics unlimited)`), excludes them from deck cost, and
`deck legal` exempts them from copy limits.

Two independent write paths, joined by the deck name:

1. `collection import` writes **ownership only**: `Binder Type=deck` CSV
   rows become deck-assignment rows in the `collection` table. It never
   writes deck files; a `note:` after the import names the tracked decks
   and points at `stm deck import`.
2. `deck import` (ManaBox deck txt export, upsert by name) and
   `deck update` (ops) write the decklist. `deck delete` removes the
   decklist + primer and leaves ownership intact. Ownership display always
   comes from the `collection` table.

## Database usage and migrations

- `Connection::open` + `PRAGMA journal_mode=WAL`; the schema comes from
  [rusqlite_migration](https://docs.rs/rusqlite_migration/latest/rusqlite_migration/)
  (see `src/db.rs`).
- Queries are prepared per call site. Joins go by card name, and
  `cards.name` UNIQUE is the anchor.
- Inserts use `INSERT OR IGNORE` so bulk re-runs are idempotent.

**Migrations.** The schema is versioned via rusqlite_migration, applied
forward-only on every `db::open`:

- Each migration lives in `migrations/` as `NNNN_description.sql`, loaded
  with `include_str!` in `db::migrations` (`0001_initial_schema.sql`
  creates the full current schema: cards, collection, card_prints, sets,
  combos, cards_fts). The store has not shipped yet, so `0001` is edited
  in place when the schema changes. Because an existing v1 database has
  no version bump to trigger, `stm setup --force` deletes `stm.db` (and
  its WAL sidecars) and rebuilds from the bulk — that is the documented
  path for schema changes (or delete `stm.db` by hand).
- After the first release, edits to `0001` stop: each schema change
  becomes a new `NNNN_description.sql` file plus an `M::up(include_str!(...))`
  entry in `db::migrations`, so existing databases migrate in place.

## Module map

| Module          | Responsibility                                                 |
| --------------- | -------------------------------------------------------------- |
| `cli.rs`        | clap command tree; the only place argv is interpreted          |
| `paths.rs`      | Data-dir layout, all path derivation, `status.json` type       |
| `db.rs`         | Schema + migrations, connections, row types (`CardRow`), name resolution, FTS query/search, `Filterable` impl |
| `scryfall.rs`   | Bulk download + streaming parse + ingest/update mapping (cards bulk; bulk index serves cards + tags files) |
| `tags.rs`       | Oracle-tags bulk parsing + ingest (`tags`, `card_tags`), `TagIndex` lookups, embedding-label selection |
| `spellbook.rs`  | Commander Spellbook variants bulk: download + stream parse + ingest (`combos`, `combo_pieces`) |
| `combos.rs`     | Shared combo reads: set-based variant loading, format legality, commander-required filtering |
| `embed.rs`      | Model loading, doc building, vector store save/load/search     |
| `search.rs`     | Structured filters (`--type/--color/--cmc/...`), ranking       |
| `query.rs`      | `stm query` orchestration + hybrid search pipeline (FTS + vector, RRF fusion) |
| `setup.rs`      | Setup pipeline (sync + full embed) with progress output        |
| `sync.rs`       | Bulk sync: card diff + price harvest + incremental embed, staleness |
| `card.rs`       | `stm card` detail rendering (text + JSON)                      |
| `release.rs`    | Release-date parsing/checking helpers for ingest gating        |
| `collection.rs` | ManaBox CSV import (ownership only; deck rows are deck-assignment rows), collection stats, owned-only search |
| `deck/`         | ManaBox txt grammar, deck files, update ops, primer, legality/bracket checks, format-aware suggestions, overview stats, goldfish simulation (`grammar`/`store`/`update`/`io`/`legal`/`stats`/`simulator`) |
| `prints.rs`     | `card_prints` reads: cheapest/priciest print, owned-print pricing |
| `output.rs`     | Human/JSON output hub: color detection, style helpers (bars, framing, wrapping), progress plumbing |
| `main.rs`       | Dispatch, exit codes, auto-refresh hook, error reporting       |