---
name: seizethemana
description: Use when building MTG decks, improving existing decks or precons, searching cards, managing a collection or binders, or working with ManaBox exports. Covers the stm CLI (seizethemana) for semantic card search, collection stats and value, deck files, legality checks, and primers. Triggers on Magic, MTG, Scryfall, ManaBox, deckbuilding, commander, bracket, binder, budget.
---

# `stm`: MTG card search, collection, and decks

`stm` (seizethemana) searches local Magic: The Gathering card data, tracks
owned cards and their locations, manages decklists, and reads and writes
ManaBox formats. Prices are USD snapshots, not live store quotes. Use
`--json` for supported read commands and check each command's exit status.

Read the [contract](#contract-read-first) and the relevant command reference
before using commands.
Follow the [deckbuilding workflow](#part-2-deckbuilding-workflow) when the user
asks for a new deck or wants an existing deck improved. Use the
[JSON shapes](#json-shapes-stable) and [deckbuilding targets](#reference-read-when-needed)
as references, not as evidence that a deck is strong or a card fully works.

## Part 1: tooling reference

### Command reference

| Command                                                                                                                                                     | Purpose                                                                                                                                                                     |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `stm query [text] [filters] [--max-price X]`                                                                                                                | Search by words and meaning, or list cards with filters only.                                                                                                               |
| `stm card <name> [--json]`                                                                                                                                  | Show a card's price, rank, tags, legalities, and ownership.                                                                                                                 |
| `stm card similar <name> [--owned]`                                                                                                                         | Find cards with similar roles and text.                                                                                                                                     |
| `stm card combos <name> --json`                                                                                                                             | List Spellbook combos that include a card.                                                                                                                                  |
| `stm collection [--json]`                                                                                                                                   | Show collection value, curve, and rarity counts.                                                                                                                            |
| `stm collection query [text] [filters] [--json]`                                                                                                            | Search or list owned cards in the selected location scope.                                                                                                                  |
| `stm collection conflicts [--json]`                                                                                                                         | Find deck demand that exceeds owned copies, with estimated buy cost.                                                                                                        |
| `stm collection sell [--bulk] [--bulk-rate X] [--min-price X] [--max-price X] [--rarity R] [--format F] [--rank-floor N] [--target X] [--limit N] [--json]` | Recommend surplus binder copies while preserving deck needs and spares. `--target X` uses singles market value.                                                             |
| `stm collection import <csv> [--add]`                                                                                                                       | Replace collection records, or add copies with `--add`. Back up ownership records first.                                                                                    |
| `stm deck create`, `import`, `show`, `export`                                                                                                               | Manage decklists in ManaBox, Moxfield, Archidekt, Arena, or names format. URL import supports Archidekt only.                                                               |
| `stm deck hand <name> [--seed N] [--count N]`                                                                                                               | Sample opening hands with keep advice. The seed matches the simulator's first opener; count is 1 to 10, default 3.                                                          |
| `stm deck update <name> [operations]`                                                                                                                       | Edit quantities or sections with `--add`, `--remove`, `--set`, `--move`, or `--from`.                                                                                       |
| `stm deck update <name> [operations] --dry-run [--sim] [--legal] [--json]`                                                                                  | Preview card and cost changes without saving. Optionally compare simulations or check post-change legality. Apply approved operations without `--dry-run`.                  |
| `stm deck suggest <name> [query] [--role R] [--commander] [--format F] [--bracket B] [--max-price X] [--owned] [--exclude NAME\|FILE\|-] --json`            | Rank role fills, theme cards, commander candidates, or combo completions. Exclusions accept names, a file of names, or `-` for stdin.                                       |
| `stm deck legal <name> [--format F] [--bracket 1-5]`                                                                                                        | Check legality and bracket guidance.                                                                                                                                        |
| `stm deck mana <name> [--format F]`                                                                                                                         | Audit colored sources against static Karsten floors; no simulation.                                                                                                         |
| `stm deck simulate <name> [--seed N] [--threads N] [--json]`                                                                                                | Report consistency under a declared opponent scenario.                                                                                                                      |
| `stm deck combos <name> [--bracket B] --json`                                                                                                               | Audit Spellbook combos by section.                                                                                                                                          |
| `stm deck cuts <name> [--for ROLE] [--make-room-for sideboard\|maybeboard] [--format F] [--bracket B] [--max-price X] --json`                               | Rank cuts or pair them with role fills or bench cards. Pass `--bracket` to rank over-cap Game Changers first. See the JSON reference for pairing limits.                    |
| `stm deck diff <A> <B> [--json\|--markdown\|--as-update]`                                                                                                   | Compare original A with target B. Each accepts a deck name or file path. Output flags are mutually exclusive; `--as-update` emits batch operations.                         |
| `stm deck buylist <name> [--store S]`                                                                                                                       | List missing copies as purchase lines or store CSV.                                                                                                                         |
| `stm deck copy <src> <dst> [--force]`                                                                                                                       | Copy the decklist and any source primer, not ownership rows. Without a source primer, keep the destination primer. `--force` overwrites existing files; back them up first. |
| `stm deck primer <name> --set <file\|->`                                                                                                                    | Replace the primer from a file or stdin. Back up an existing primer first.                                                                                                  |

Angle brackets mark required values; square brackets mark optional arguments.
Replace placeholders before running commands. Quote names, paths, and search
text that contain spaces. Quote comparisons such as `--cmc '<=4'` so the shell
does not treat them as redirection. Use `--help` for exact options.

These commands take `--json`: `card`, `card similar`, `card combos`,
`query`, `collection`, `collection query`, `collection conflicts`,
`collection sell`,
`deck hand`, `deck show`, `deck list`, `deck legal`, `deck mana`,
`deck simulate`, `deck combos`, `deck cuts`, `deck diff`, `deck suggest`,
`deck buylist`, `deck dedupe`, and `deck update` (for `update` it is
the dry-run preview or the update summary). The JSON shapes section
below lists each. Write commands (`setup`, `sync`, `collection import`,
`deck create/delete/export/import/copy`, `deck primer --set`) do not take
`--json`.

### Contract (read first)

- **stdout = results only.** Progress goes to stderr. Successful JSON results
  use stable snake_case keys without terminal styling. Errors can leave stdout empty.
- Prefer `--json` for programmatic reads. Check `command -v jq` once and use
  `jq` to select the fields you need; otherwise read the JSON directly.
  Known prices are numbers in US dollars; unknown prices
  are null. When present, `currency` names the unit; card and suggestion rows
  do not have that key. Help uses `<USD>` for price arguments. Examples:

  ```sh
  set -o pipefail
  # Names only
  stm query "counterspell" --limit 10 --json | jq -r '.[].name'
  # Cheapest regular-finish price for this card
  stm card "Sol Ring" --json | jq -r '.price'
  # Priced missing copies after deck and binder coverage
  stm deck buylist Froggy --json | jq -r '
    .rows[] | "\(.quantity)x \(.name) @ \(.price // "unpriced")"'
  # Top hits sorted by score, name + score + price as TSV
  stm query "sacrifice outlet" --limit 10 --json | jq -r '
    sort_by(-.score) | .[] | "\(.score)\t\(.name)\t\(.price)"'
  # Reported deck totals; see the cost limits below
  stm deck show Froggy --json | jq -r '"\(.cards) cards, missing $\(.missing_cost)"'
  ```

- Exit codes: `0` success, `1` runtime error or a documented command result, `2` usage
  error, `3` no results or card not found. Legality violations and simulation
  findings use `1`. A baseline comparison uses `1` for new findings only.
  A nonzero result can still have valid JSON on stdout. Keep diagnostics on
  stderr separate; warnings can accompany successful commands.
- In Bash or Zsh, use `set -o pipefail` for pipelines. Otherwise a successful
  `jq` can hide `stm` errors or finding statuses behind its own exit status.
- Piped output has no ANSI codes. Set `NO_COLOR=1` or pass `--no-color`
  when you need to disable color explicitly.
- The default data directory is `~/.seizethemana/`. Override it with `--data-dir <DIR>`.
- `--offline` skips automatic refreshes and blocks deck URL imports. Explicit
  `setup` and `sync` still use the network. A text search can also download missing
  model files. Cache the model before working without a connection. Use a local
  file for offline imports.
- `--offline` does not mean filesystem read-only. Database open can create
  schema state, and vector loading can finish a pending local publication
  repair. No-text listings do not load the vector store.
- Card **names** resolve by exact match, case-insensitive match, or unique
  prefix. "lightn" resolves if unambiguous; an ambiguous prefix exits 3 and
  lists the candidates. Names resolve by _prefix_, not substring: "Bolt"
  does not match "Lightning Bolt".
- Unreleased cards never appear: `sync` does not ingest an oracle card
  until its set has released (newly released cards arrive on the daily
  sync). Upcoming reprints of released cards stay visible; their
  `released_at` comes from the stored representative printing, not necessarily
  the newest printing.

### Prerequisites (do this first)

1. Check for the binary (`command -v stm`). Missing or outdated: install
   with `cargo binstall -y seizethemana` (prebuilt, fastest), else
   `cargo install --locked seizethemana`, else fetch a prebuilt archive
   from https://github.com/z0w0/seizethemana/releases and put the
   `stm` binary on `$PATH`.
2. If `stm` says "card index not built yet", run `stm setup`. Setup downloads
   bulk data and the search model, then embeds cards locally. Other exit-1
   results do not by themselves mean setup is missing. Read the diagnostic
   before choosing a repair. Build from source with `--release` before real
   embedding work; debug embedding is slow.

**Warning:** `stm setup --force` deletes `stm.db`, its SQLite sidecars,
`vectors.bin`, and `status.json`. All collection records, deck assignments,
and purchase prices are lost. Stop other `stm` processes and back up the whole
data directory first. Saved decklists and primers in `decks/` remain.
Use `stm sync` for routine data refreshes.

Some read commands check for data older than 24 hours and wait for sync before
returning results. A failed automatic sync warns and continues with available
data. No-text queries and `collection sell` do not start this refresh.
`stm sync [--force]` requests an explicit refresh. Refresh also pulls Commander Spellbook combo variants
(~28 MB) for `deck simulate` combo access and bare `deck suggest` completions.
A failed combo download warns and continues.

### Find cards in the catalog

```sh
stm query "sacrifice a creature to draw cards" --limit 10
stm query "graveyard recursion" --color BG --format commander --cmc '<=4' --json
stm query --type Elf --limit 10
stm query --set BLB --json
stm query --limit 10
stm collection query --type Elf --json
stm collection query --limit 10
stm query --help
stm collection query --help
```

Text is optional for both query commands. Without text, they apply filters
and list cards alphabetically up to `--limit` (default 20, maximum 100).
With no text or filters, they list the first cards in that scope. Collection
listings still require ownership within the existing binder/deck scope.
No-text listings use only the existing database. They do not load a model or
vector index, embed text, or start an automatic refresh. JSON keeps `score: 0`
for this unranked mode; human output omits the score. Do not read zero as a
relevance judgment. A database with no cards returns no matches.

Hybrid search combines keyword full-text search (BM25) and vector similarity
with reciprocal rank fusion. The `score` is 0 to 1; it is not a probability
that the card fits. Role terms such as "sacrifice outlet" can help retrieval.
Use filters to require card properties. Filters are AND-combined and shared by
`query` and `collection query`: `--type` `--color` (subset of WUBRG)
`--color-identity` `--cmc` `--power` `--toughness` (comparisons
`<= < = > >=`) `--rarity` `--set` `--keyword` `--oracle-text` `--format`.
`--set CODE` matches only the card's stored primary printing, in ranked text
search and no-text listings. Other catalog printings do not qualify it.
Collection ownership still matches by card name. This flag does not filter
owned copies by their printing.
`--limit` default 20, cap 100. `--max-price <USD>` (query, deck suggest)
keeps only cards priced at or under the cap;
**unpriced cards are excluded** (strict budget reading). The cap
applies before the limit cut, so the result fills up to `--limit` with
under-cap cards. Suggest's human output notes what the cap hid
("candidates capped at $2.00 USD; 3 unpriced or above-cap cards hidden");
query paths hint "raise --max-price" when the cap empties the results.
`collection query` has no cap: it lists cards you already own.

Card detail (legalities, EDHREC rank, Game Changer flag, release date,
price, Tagger role labels):

```sh
stm card "Lightning Bolt" --json
```

`card --json` includes `game_changer` (Commander Game Changer list),
`tags` (community role labels: "removal", "sacrifice outlet", "wheel"),
and `legalities` (use the exact format key, e.g. `legalities.commander`).

### Find similar cards

```sh
stm card similar "Cyclonic Rift" --limit 10
stm card similar "Smothering Tithe" --owned --json
```

Ranks cards by reciprocal-rank fusion of shared Oracle tags and stored-vector
similarity to the seed card. Use it for "what plays like X". `--owned` includes
only cards with owned binder copies. Copies assigned only to decks do not
qualify. The command has no `--deck` scope option. JSON per hit:
full card fields + `score` (0 to 1, like `query`) + `shared_count` +
`shared_tags`. Use this when a known
exemplar exists; use `stm query` when describing an intent with no
exemplar. **Shared-tag count measures role overlap, not power.** Check
`edhrec_rank` (lower means more popular in Commander) and `price` before
proposing a hit. Popularity does not prove a card is better for this deck or
another format.

### Query combos for a card

```sh
stm card combos "Thassa's Oracle"
stm card combos "Demonic Consultation" --format modern
stm card combos "Kiki-Jiki, Mirror Breaker" --limit 5 --json
```

Lists Commander Spellbook combos the card takes part in, best first
(popularity). Each hit names the pieces (`A + B`), what the combo does,
and the formats it is legal in. `--format <fmt>` keeps only combos legal
in one format (`commander`, `modern`, `pioneer`, ...). Combos marked
`(commander)` need one piece as the commander, so they cannot fire in
60-card formats; they are excluded automatically under a 60-card
`--format`. Exit 3 when the card appears in no combo.

Use this when a user asks "does X combo?" or wants combo pieces for a
card. JSON per hit: `produces`, `bracket_tag`, `popularity`,
`legalities` (format → bool map), `requires_commander`, and
`pieces: [{name, zones, must_be_commander}]`.

Deck-facing view: bare `stm deck suggest <name>` lists one-card-away
combo completions for a deck. Commander decks
filter to commander; other decks drop commander-required variants (or pin
a format with `--format`).

### Work with the collection

Import a ManaBox CSV export. This replaces collection records by default;
back up the data directory first. `--add` adds copies to existing records.
Reimporting the same rows counts them again:

```sh
stm collection import "$HOME/path/collection.csv"
```

**The collection CSV is ownership only.** `Binder Type=deck` rows become
deck-assignment rows in the collection (what you own and where), not
decklists. Decklists are separate files (`decks/<name>.txt`) and are never
written by a collection import. After the import, a `note:` line names the
decks it now tracks and reminds you to import each decklist separately
(`stm deck import <name> <manabox-deck.txt>` from a ManaBox deck screen's
three-dot **Export** txt). Wishlist (`list`) rows are skipped.

Deck rows (`Binder Type=deck`) drive ownership: `deck show`'s `own N/M`,
`owned_value`/`missing_cost`, and buylists all count copies assigned to
that deck name plus copies sitting in binders. Copies assigned to other
decks never fill this deck's slots (that would deconstruct them).

Stats and owned-only search:

```sh
stm collection --json                # totals, value, colors, curve, sets, locations
stm collection query "counterspell for everything" --color U --json
stm collection query "sacrifice outlet" --deck Froggy --json
```

**`collection query` shows the available pool, not everything you own.**
Cards assigned to decks are hidden by default. Search binder copies first.
`--deck <name>` adds that deck's assigned copies to the binder pool; it does not
select only that deck. `--binder <name>` narrows the binder portion only. When
both flags are present, the result combines the named binders and decks.
Each hit's `locations` lists only locations within the queried scope.

`collection query` JSON per hit: full card fields plus `score`, `owned`,
and `locations: [{binder, binder_type, quantity}]`. Use `owned`/`locations`
directly instead of re-querying. `owned` counts copies in the requested scope,
except the five unlimited basics report null. `available` counts copies in all
binders, not just the requested scope. `locations` stays within that scope.

Selling idle cards:

Shell redirection replaces an existing output file. Back up `sell.csv` or
`sell.txt`, or choose a new path, before exporting.

```sh
stm collection sell --json                 # ranked surplus singles at least $1 each
stm collection sell --exclude-binder Collect # protect collector binders
stm collection sell --details --review      # allocations, evidence, and hold candidates
stm collection sell --exclude-binder Collect --output csv > "sell.csv"
stm collection sell --exclude-binder Collect --output txt > "sell.txt"
stm collection sell --min-price 2           # sale printings priced at least $2
stm collection sell --bulk --rarity common  # excess commons after four-copy reserves
stm collection sell --bulk --bulk-rate 5    # supplied $5 per 1,000-copy assumption
stm collection sell --target 38             # singles toward $38 market value
stm collection sell --format modern         # Modern combo evidence and legality
```

`collection sell` protects each deck's unmet demand, then keeps one useful
spare or four spares for cards with bulk printings. Assigned copies and basics
never enter sales. The cheapest owned printings fill reserves first.
Singles require $1 per copy; `--bulk` lists sub-$1 excess copies by quantity.
Price filters apply to recommended printings. Bulk proceeds require an
explicit uniform `--bulk-rate` per 1,000 copies and never fund `--target`.

Priority combines sale market value with the strongest EDHREC, Penny, or
Spellbook demand signal. Missing ranks mean unknown demand. Review rows are
excluded from recommended totals and funding. `--format` scopes combos and
reports legality; it does not measure other formats' usage. `--rank-floor`
sets the low-demand Commander boundary (default 15000). `--limit` caps each
displayed section (default 20), not totals or funding. Human output is a compact
table. `--details` adds sale printings and present evidence; `--review` adds
personal hold candidates. Missing ranks and general combo participation affect
priority rather than automatically requiring review.

Repeat `--exclude-binder NAME` to remove collector binders from the pool.
Match whole names, ignoring ASCII case. Excluded copies do not cover deck
shortages, spare reserves, or personally owned combo options. Unknown binder
names are errors. These flags do not edit the collection.

`--output txt|csv` exports the full filtered sell list to stdout, ignoring
`--limit`. With `--target`, it exports only the funding picks. Export sale
quantities, not retained or review copies, and merge identical printing lines
across source binders. CSV uses ManaBox `Binder Name=Sell`, `Binder Type=list`
and preserves normal, foil, and etched finishes. TXT uses ManaBox quantity,
name, set, collector number, and `*F*` for foil or etched. Use CSV when etched
finish must stay distinct. `--output` conflicts with `--json`, `--details`, and
`--review`. No output flag keeps the usual human view. Empty exports exit 3.

### Build and edit decks

Decks are ManaBox txt files (`decks/<name>.txt`: `// SECTION`,
`qty Name (SET) cn *F*`); the file is the source of truth. Each deck also
has a primer at `decks/<name>.primer.md` for strategy notes. Read the
primer from the CLI (`stm deck primer <name>`); write it with `--set` or
by editing the file.

Run editing commands for the same deck one at a time. Do not overlap `update`,
`import`, `dedupe`, `copy`, `delete`, `primer --set`, or direct file edits. This
also applies to commands launched through parallel tool calls. Deck commands do
not lock the full read-edit-write sequence; overlapping writes can lose changes.
Wait for each edit to finish before starting the next. If another edit occurs
after a preview, reload the deck and repeat the preview before applying it.

Deck import overwrites a decklist with the same name. Deck delete removes its
decklist and primer. `deck primer --set` replaces the primer. `deck copy --force`
and `deck export --force` overwrite their destinations. Back up affected files
before these actions. None of them deletes collection ownership rows.

The examples below are separate actions, not a script to run in order.

```sh
stm deck list                        # saved lists and collection decks without a list
stm deck create Froggy               # empty list; enough for suggest --commander
stm deck copy "Froggy Original" "Froggy Improved" # copy list and primer, not ownership
stm deck show Froggy                 # contents, ownership, estimated costs, and curve
stm deck show Froggy --json          # sections and covered_by for each line
stm deck update Froggy --add "1 Phyrexian Vault" --add "sideboard:2 Lightning Bolt"
stm deck update Froggy --remove "1 Lightning Bolt" --set "0 Breya, Etherium Shaper"
stm deck update Froggy --move "1 Lightning Bolt to:sideboard" # atomic section move
stm deck update Froggy --from "batch.txt" --allow-partial # see partial-update limits below
stm deck dedupe Froggy               # merge duplicate lines; cap Commander singleton copies
stm deck import Froggy "$HOME/Downloads/Froggy.txt" # create or replace the list
stm deck delete Froggy               # delete list and primer; keep ownership
stm deck export Froggy "froggy-export.txt" # --format names gives quantity/name lines
stm deck primer Froggy               # print primer Markdown to stdout
stm deck primer Froggy --set "primer.md" # replace the primer from a file
stm deck primer Froggy --set -       # replace the primer from stdin
stm deck buylist Froggy              # missing copies as `2x Name` lines
stm deck buylist Froggy --store cardkingdom # CSV: Name,Edition,Foil,Qty
stm deck legal Froggy                # format legality
stm deck legal Froggy --format commander --bracket 3
stm deck suggest Froggy --role draw --json     # role fills: owned first, then by fit
stm deck suggest Froggy --role draw --owned --json  # role fills restricted to owned cards
stm deck suggest Froggy "frog payoff" --json   # semantic query for theme cards
stm deck suggest Froggy --commander --json     # commander candidates for the deck
stm deck suggest Froggy --commander --bracket 2 --json # apply the commander bracket filter
stm deck suggest Froggy                        # combo completions: one card away from a Spellbook combo
stm deck suggest Froggy --bracket 2            # completions filtered to no Game Changers
# --limit N widens the candidate pool beyond the default 10 (max 50)
```

The decklist file and ownership rows are separate. The deck name links them:

- `stm collection import` writes ownership only. `stm deck import` writes
  the decklist only. Neither touches the other.
- `stm deck import` creates or overwrites the decklist by
  name. ManaBox's two export shapes are both handled: the commander(s)
  under `// COMMANDER` with a blank line before the rest of the deck
  (the blank splits the section into `COMMANDER` + `DECK`), and the older
  quirk of the whole deck under `// COMMANDER` (detected at more than 4
  entries and treated as the main deck). When no real COMMANDER section
  remains, an interactive terminal import offers the legendary candidates
  (skip to decide later); redirected runs do not prompt and print
  a `note:` with candidates. `// SIDEBOARD` and `// MAYBEBOARD` sections
  import as-is.
- `stm deck delete` removes only the decklist + primer. Ownership rows in
  the collection survive; `stm deck list` shows collection decks that have
  no decklist (`has_decklist: false` in JSON) so the gap is visible.
- `stm deck buylist` autofills owned copies (deck-assigned + binders)
  read-only and lists only what needs purchasing, at the cheapest
  printing. Nothing in the collection is moved. Same-name entries across
  sections aggregate into one row; JSON rows carry `sections` naming
  where the copies live (e.g. `["DECK", "SIDEBOARD"]`).

Update specs: `[section:]qty Name [(SET) [cn]] [*F*]`. `--add` increments,
`--remove` decrements (deleting the line at 0), and `--set` sets an exact quantity.
The optional `(SET) cn` identifies a printing to sleeve; matching
is by card name, so any set version you own fills a deck line. Account for
reprints when pricing buys. Section qualifiers drive format
inference: `commander:1 Breya, Etherium Shaper` puts the card in the COMMANDER section
(do this first; it drives color identity, format rules, and the command zone).
Use `sideboard:2 Lightning Bolt` for the sideboard,
`maybeboard:2 Lightning Bolt` for the maybeboard, and no qualifier for the
maindeck. `--from` batch files also accept `move 1 Name to:sideboard`
lines.

**Update card names resolve to canonical names.** Exact, case-insensitive,
unique-prefix, and supported flavor-name inputs use the stored card identity.
An unknown addition or nonzero `--set` exits 3 and names the card, with nearby
candidates when available. Fix the name and retry. Unresolved names can still
remove stale deck lines. Tokens are added with a warning, not an error.

**`deck update` takes `--add`, `--remove`, `--set`, `--move`, `--from`,
`--allow-partial`, `--dry-run`, `--sim`, `--legal`, `--backfill-basics`,
and `--json`.** These are deck-specific flags; global flags also apply.
`--backfill-basics` adds basic lands after the operations until the deck reaches its size (100 for
commander, 60 otherwise), picking the basic from the color identity's
first color (colorless decks get Wastes). Unreleased
cards cannot be referenced because the store never holds them; a card
shows up once its set releases.

**`--move` for section moves.** `stm deck update <name> --move
"[section:]qty Name to:section"` performs one atomic remove+add (the
singleton guard and missing-checks see the net state, so legal moves do
not warn). `to:` defaults to `deck`; a leading `sideboard:` picks the
source section. `--move "1 Lightning Bolt to:sideboard"` moves a card from the
maindeck; `--move "sideboard:1 Lightning Bolt"` moves it back.

**`--set 0` deletes.** On a sideboard-only card it deletes the
sideboard line (same fallback as `--remove`), with a note; a qualified
`sideboard:0 Name` stays strict.

**`--allow-partial` handles missing deck targets.** By default, a missing
removal or move target prevents saving. With `--allow-partial`, the other
operations apply, missing targets are reported, and the command exits 1.
This also applies to operations passed as flags. It does not bypass unknown
addition names, invalid specs, or singleton-add checks. Dry-run still rejects
missing targets with exit 3, even with `--allow-partial`.

**Singleton guard (commander).** Commander-shaped decks (a COMMANDER
section) reject an `--add` that would push a non-basic card past one copy.
The batch aborts and names the cards. Remove the duplicate add, or use `--set`
for a deliberate exact quantity. `--set` and `--move` can still save a deck
with singleton warnings. `stm deck dedupe <name>` merges duplicate same-name lines per
section and collapses non-basic lines holding more than one copy down to
1 (basics keep quantities; 60-card-style decks keep summed quantities);
exit 3 when there is nothing to fix. After long build sessions run
`dedupe` before `deck legal`.

**`--from` batch specs.** A spec file holds one op per line: `add 1 Name`,
`remove 1 Name`, `set 2 Name`, or a bare spec (treated as add). Blank
lines and `#` comments are skipped. Large batches become one call instead
of a `--add` flag list.

**Basic-land coverage has a limit.** For planning, treat Plains, Island, Swamp,
Mountain, and Forest as free and unlimited. Check Wastes and snow-basic quantities
and prices explicitly with `card` and `collection query`. `deck show` and
`deck buylist` treat every Basic Land except Wastes as available without limit,
including snow basics. Their coverage does not prove you own the snow basics or
that they are free. JSON deck entries use `basic_land: true` for this exemption.

### Legality and brackets

```sh
stm deck legal Froggy                          # infer format from sections
stm deck legal Froggy --format commander       # explicit format
stm deck legal Froggy --format commander --bracket 3
```

- Without `--format`, a `// COMMANDER` section means commander; otherwise
  only structural rules are checked (60-card maindeck minimum, sideboard
  ≤ 15, 4-copy limit). JSON marks this as `"format_assumed": true`.
- **Bench sections (SIDEBOARD / MAYBEBOARD).** Both check per-card
  legality for the deck's format and, in commander, color identity
  (`bench color identity` violation). Neither counts toward main-deck size,
  curve/ramp in `deck show`, or the bracket Game Changer cap. In 60-card formats,
  the sideboard is the best-of-three match zone (≤ 15 cards, with the 4-copy
  rule spanning main deck and sideboard);
  the maybeboard is free-form candidates with no size cap and exempt from
  the copy limit. In commander the sideboard is the upgrade kit (its
  Game Changers surface as advisories); the maybeboard is loose "maybes".
- Deterministic checks: deck size, copy limits (basics and "any number of
  cards named X" cards excepted), commander rules (1 commander, or 2 with
  Partner; legendary creature, planeswalker, or legendary
  Vehicle/Spacecraft with a printed power/toughness box), commander color
  identity, per-card format legality, and Game
  Changer count vs bracket (0 for brackets 1 to 2, at most 3 for bracket 3,
  unlimited for 4 to 5).
- Exit 0 = legal, exit 1 = violations. A completed JSON check prints:
  `{name, format, format_assumed, bracket, legal, violations: [{rule,
cards, detail}], advisories, notes, summary}`.
- **Built-in bracket checks.** The bracket-specific hard limit is
  the Game Changer count: 0 for brackets 1 to 2, at most 3 for bracket 3,
  unlimited for 4 to 5. `deck legal` enforces this cap as a violation.
  Other bracket signals remain advisory;
  normal deck legality rules still apply.
- **Sideboard = upgrade kit (commander).** Sideboard Game Changers never
  count toward the bracket cap, but `deck legal` prints an advisory
  (`advisories` in JSON, `ℹ` lines in the human view) naming them. The
  usual pattern is a bracket-3 upgrade kit parked in the sideboard, so
  the advisory confirms what a bracket bump would bring along.
- **`notes` carries the bracket verdicts.** With `--bracket`, the human
  view shows `bracket checks:` with `✓` PASS, `!` CHECK (genuine
  conflicts: mass land destruction and extra turns in brackets 1 to 2,
  tutors in 1 to 2), or `ℹ` advisory lines. Tutor verdicts follow the
  bracket: brackets 1 to 2 produce a CHECK (`!`), bracket 3 produces an
  advisory (`ℹ`; the cap is Game Changers, which includes the best
  tutors), brackets 4 to 5 stay silent. The scan splits `note hard tutors:`
  (one-shot search spells) from `note soft searchers:`
  (ETB/activated/restricted searchers). Mass land destruction stays a
  hard CHECK in brackets 1 to 3. JSON `advisories` carries the `ℹ` lines
  separately from `violations`; `notes` carry the
  raw strings. When no bracket was given, use the Game Changers list in
  `notes` to ask the user which bracket they want.
- **Land-search ramp is not a tutor.** Cards whose library search targets
  lands (Cultivate, Farseek, Fabled Passage) get their own `note ramp:`
  line and never trip the tutor scan; only nonland tutors do.
- Bracket levels: 1 = exhibition, 2 = core, 3 = upgraded, 4 = optimized,
  5 = cEDH. When unsure, ask the user; describe 2 to 4 in those terms.

### Simulation (goldfish)

```sh
stm deck simulate Froggy                         # 10,000 games, infer format
stm deck simulate Froggy --runs 100 --seed 42    # reproducible smoke run
stm deck simulate Froggy --seed 42 --json        # complete report
stm deck simulate Froggy --threads 4 --batch-size 256 --seed 42 --json
stm deck simulate Froggy --opponents inactive --seed 42 --json
stm deck simulate Froggy --opponent-scenario "opponents.json" --seed 42 --json
```

- The model is a consistency diagnostic, not a win-rate predictor or an
  overall speed bound. Unsupported rules can make results too fast or too slow.
  `--runs` defaults to 10,000 (range 100 to 1,000,000). `--turns` defaults to
  10 for commander-shaped libraries and 8 for constructed (range 1 to 30).
  Extra turns consume the configured player turn slots.
- Format follows the deck's sections unless `--format` overrides it. A
  commander starts in the command zone. Casts pay its pips and add `{2}` per
  previous command-zone cast. Commander-family openers redraw once outside
  the two-to-six-land band and keep the redraw. Constructed redraws full
  seven-card hands outside the two-to-five-land band, up to six times, then
  bottoms one card per redraw. At four or more lands it bottoms a land;
  otherwise it chooses a spell when available. This policy does not assess
  all keeps.
- `--threads` and `--batch-size` require positive values. Omission uses the
  shared Rayon pool and batches of 256. The runner delivers game logs in
  game-ID order. Production statistics and combo counters consume each batch;
  exact percentile samples still use memory proportional to the run count.
- Each game has separate seeded `Opening` and `Library` streams. Worker count
  and batch size do not affect them. Initial shuffles and mulligans use
  `Opening`; explicit runtime library shuffles use `Library`. Pilot and
  opponent choices are deterministic. Their domain helpers do not add random
  actions.
  Equal seed and settings reproduce equal inputs; different lists or card
  order need not produce identical card sequences.
- Commander-shaped libraries default to three synthetic seats; constructed
  defaults to inactive external activity. `--opponents synthetic|inactive`
  selects a preset. `--opponent-scenario FILE` supplies JSON instead and
  cannot combine with the preset flag. The default active settings are
  `{"active":true,"land_target":6,"spell_costs":[2,2,3,3,4,4,5],"tax_policy":{"capped_spare":4}}`.
  Optional `land_colors` supplies one uppercase WUBRG string per seat.
  Omission means unknown colors; an empty string means no eligible color.
- The table is illustrative, not calibrated to real games. Draws, land plays,
  scheduled spells, and taxes share finite hand, library, and source ledgers.
  Cast triggers run before the scheduled permanent enters. Opponents do not
  block, counter spells, or cast removal. Inactive activity disables scheduled
  events, not the table used for player-directed life loss.
- Fellwar Stone-style production requires an active seat. Declared colors use
  active seats with actual lands. Unknown colors give generic-cost credit from
  turn 2 even when scheduled activity is inactive or opponents have no lands.
  This credit never pays colored or true colorless `{C}` pips. A supported bonus
  retains that limitation. Do not call this color fixing.
- Card rules follow Oracle text through a typed abstract syntax tree (AST), then
  separate lowering into runtime data. `card_support` is ability-level compiler evidence with
  source text and statuses `supported`, `compiler_only`, `approximated`, and
  `unsupported`. Even `supported` is not proof of exact runtime behavior or
  full-card support. Empty records do not mean complete coverage. Read the
  evidence with `assumptions`; audit the key cards before trusting timing.
- Generic creature tokens do not preserve all printed stats, colors, subtypes,
  or abilities. Lathril's damage-scaled Elf tokens are not exact, and tapping
  ten untapped Elves for its drain ability is unsupported. Landfall mana
  depends on a live resolution pool. Unsupported costs and conditions keep
  abilities inert. Do not fill gaps with assumed card-name behavior.
- The policy casts cheap payable spells and uses supported activations before
  unblocked combat. New mana or draws can enable another cast pass. The
  postcombat main has no actions. Action caps prevent unbounded loops;
  suspected infinite mana is not proof of a legal combo.
- Zero life or a failed draw from an empty library ends the game at a supported
  resolution boundary. Emptying a library by mill, an empty library alone, or
  a failed search does not cause a loss. `outcomes.finished_games` includes
  player losses and games reaching the horizon. Safety limits with unresolved
  work produce `incomplete_games`, not losses or wins.
- Primary rates exclude incomplete games and include finished player losses.
  This can bias results for decks with many engines. Check the incomplete
  count and reasons. `phase_observations` gives completed-phase counts for
  per-turn means. Whole-game event rates use their stated game denominator;
  do not substitute a phase count. Suspected infinite mana and positive-loop
  diagnostics cover requested games, including incomplete games.
- Ordinary combat reports potential unblocked damage. It does not reduce seat
  life or declare a win. `combat.commander_potential` has one row per commander
  slot, with per-turn and cumulative potential, games reaching 21, the
  finished-game denominator, threshold percentage, and conditional median
  turn. Partners remain separate. Noncombat or attack-trigger damage does not
  count toward a commander's 21-point potential.
- Supported mana-paid X spells choose the largest payable X before paying the
  complete cost. Repeated X symbols share that value and each adds to the cost.
  X can be zero; casts without paying the mana cost use X = 0. Unsupported
  variable effects stay limited. Supported charge-counter X activations also
  choose before payment, support repeated X symbols and zero, and preserve
  unrelated mana. Unknown counter kinds do not become supported.
- Stated additional land counts stack across live sources and effects for the
  current turn. Convoke matches each eligible creature once, including hybrid
  choices. Improvise taps untapped artifacts for generic costs only. A shared
  artifact creature cannot pay twice through both mechanics. Affinity discounts
  generic cost without tapping. Supported live flat spell-cost reducers match
  all spells, card types, or colors using front-face data. They stack and remove
  only generic mana after increases, including commander tax and the chosen X.
  Hidden faces and face-down sources do not grant reductions. Variable or
  conditional static reducers stay unsupported. Warp's cheaper casting option
  is disabled; normal printed-cost casting remains available.
- Typed land entry uses the chosen physical land face. Other-land thresholds
  count the existing battlefield; opponent thresholds use active seats. A
  forced-tapped destination does not pay optional life for untapped entry.
- Reveal-to-battlefield and Descend use the complete front-face type. Hidden
  faces cannot make a nonpermanent front eligible. A chosen land face does not
  inherit a hidden spell front's static buffs or hand-limit flags. Face-down
  permanents do not grant printed hand-limit flags. Sea Gate Restoration's
  lasting no-maximum-hand-size spell effect stays unsupported.
- Treasures use a payment bank, not battlefield artifacts. They do not count
  for artifact totals, costs that require a battlefield artifact, or token
  entry and sacrifice events.
  A committed mana payment consumes them.
  Eligible tokens contribute to payment capacity, but unspent tokens are not
  unused floated mana.
- Token lowering that drops printed characteristics uses the
  `generic_creature_tokens` approximation. Typed syntax alone does not prove
  exact token colors, types, stats, or abilities. Typed tribal-token syntax can
  still execute as generic bodies under this approximation.
- Board-scaled draw executor tests configure synthetic runtime data. They
  check configured counts, including zero, without an eight-card ceiling. The
  corresponding printed wording has no executable lowering. These synthetic
  controls do not certify printed card behavior.
- Supported searches use front-face types, basic-land restrictions, subtypes,
  and other modeled filters. Color and colorless filters use raw stored colors
  only for single-face rows. Multiface candidates cannot match a color filter
  because the front face's colors are unavailable. Printed shuffles use `Library`,
  even after a failed search. Supported single-result top tutors place the chosen
  card after shuffling. Multiple-result searches select distinct physical cards
  in reverse library order, without ranking quality. They fill printed destination
  capacities in order, then apply a printed trailing shuffle. Battlefield
  results use the pre-entry board and enter in one batch before entry triggers.
  Foreign sources, extra source zones, and changed controllers stay inert.
  Required unchosen-card bottom placement, including random order, also stays
  inert. Kinnan's full search instruction is not certified.
- Only named land-type cycling uses the shared search/shuffle path. Basic and
  untyped landcycling stay inert.
  Life-funded cycling draw credit requires an actual successful draw; terminal
  payment, failed draws, and land searches do not earn it.
- Supported targeted returns move a creature from the player's graveyard to
  the battlefield under normal entry rules, or any card to hand. Eternal
  Witness's return can recover a land. Other filtered and forced-tapped return
  wording stays inert. Do not infer general target support from these paths.
  Activated graveyard targets bind before costs; sacrifice payment cannot
  create a newly chosen target for that activation. Targets retain their
  graveyard-entry generation and do not follow a card into a new entry.
  Nontargeted returns stay inert until their resolution-time choice exists.
- Temporary flashback grants bind to (`CardIdx`, graveyard-entry generation).
  Leaving and reentering the graveyard invalidates that permission. These
  temporary permissions also expire at the next turn.
- Zero-loyalty activations share the per-turn loyalty guard. This does not
  make ordinary zero-mana activations loyalty abilities.
- Landfall has no automatic combat-power bonus. A power change needs a
  supported printed effect. Unsupported pump wording and text-derived `+X`
  fallbacks do not affect combat.
- Prowess records a power bonus per permanent that has the keyword at cast
  time. Shared current power serves combat, Ferocious, crew, and station. Later
  entrants or keyword gains do not earn earlier bonuses. The bonus expires
  after successful cleanup, before opponent activity.
- A life-loss reveal starts with a mandatory reveal when a library card exists.
  Repeats use visible life and the largest mana value in the known deck list,
  not the next hidden card. The mandatory reveal can be lethal.
- Supported triggers capture source identity, counters, and definition positions,
  then settle at the owning action's boundary. This is not a full stack model.
  A settlement that reaches 1,024 actions with pending work marks the game
  incomplete and clears the remaining queue. The report counts the limit reason.
- Interaction measures an instant-speed answer still in hand and payable
  after main actions. `interaction.mana_held_avg` describes spare mana, not
  a deliberate hold decision. Never claim removal or a counter resolved.
  `role_access` and draw-source access measure hand arrival, not active engines.
- `velocity.avg_cards_seen` counts the kept opener plus draws, self-mill,
  searches, and modeled reveals. Repeat visits can count again. It is not
  hand size or just draws; scry adds awareness credit only.
- `win_conditions.p50_lethal_turn` is conditional on games reaching the
  damage threshold. Read it with the threshold share and declared scenario.
  It is not a win rate, guaranteed kill turn, or general speed bound.
- `provenance` records versioned inputs and deck fingerprints. Card-data
  fingerprints can be null. `opponent_model` records declared settings and
  `observed_activity` totals and per-game means for draws, casts, successful
  optional tax payments, and skipped casts. Payments count decisions, not mana
  spent. `attempted_games` and `raw_totals` include all requested games;
  `observed_games`, `totals`, and `per_game` use finished games. Excluding
  incomplete games can bias those means. These are shared-table counts, not per-engine player benefits or
  real-world activity estimates. `sampling` gives game counts and 95% Wilson
  intervals for selected event rates. These intervals exclude model error.
  Check observation status: `outside_horizon`, `not_applicable`, and
  `no_observations` do not mean an observed zero.
- `--combo "A + B"` measures pair access. Store-backed `combos` and `win_paths`
  use supported first-zone visits, not simultaneous legal combo execution.
  Exile requirements and untracked command-zone movements are excluded.
  `--combo-limit N` caps each discovered list (default 20).
- `--hypgeo` computes joint spell-and-land access in a uniform sample of seven
  cards plus one draw per turn. The required land count uses modeled
  `card.cost.total()`, not printed mana value. Unresolved X counts as zero, and
  a split card can use its modeled cheaper face. It excludes mulligans,
  selection, extra draws, ramp, colored pips, and land timing. This is exact only
  under those sampling assumptions,
  not a cast-rate or mana-readiness ceiling. The JSON field
  `percent_modeled_access` reports joint access under these sampling assumptions.
- Exit `1` without a baseline means findings exist. Findings carry stable
  identities, explanations, evidence, and category-level suggestions. Check
  card support and static role counts before proposing a deck change.
  With a matching baseline, exit `1` means a new finding. Reports use schema
  version 3. Baseline comparisons require equal model settings.

Compare an edit with the same seed, run count, horizon, format, and scenario:

Redirection replaces an existing `before.json`; use a new path or back it up.

```sh
stm deck simulate Froggy --seed 42 --json > "before.json"
# Apply the deck edit, then compare under the same settings.
stm deck simulate Froggy --seed 42 --baseline "before.json" --json
```

Check the first command's status and saved report before editing. Exit 1 can
mean a complete report with findings, not just a runtime failure.

With `--baseline`, JSON contains `metrics`, `findings`, and `shape` deltas,
not a full report. Without it, JSON contains the complete `SimReport`.
The repository's `docs/simulator.md` defines the runtime boundaries and limits.

### JSON shapes (stable)

The lists below are field outlines, not literal JSON examples. They use exact
field names. A trailing `?` marks a key omitted when unavailable. Other nullable
fields keep their key and use JSON `null`.

- `card <name> --json` → card object: `name mana_cost mana_value type_line colors
color_identity keywords power toughness loyalty oracle_text rarity
edhrec_rank legalities game_changer oracle_id set set_name set_type
block universe franchise collector_number scryfall_id
released_at tags price price_foil max_price max_price_foil owned
available`. All
  prices are US dollars; this object has no `currency` key. The `price*` fields
  are the **cheapest** released
  English printing per finish; the `max_price*` fields the most
  expensive. All null when no print is priced. Use `price` for budget
  math instead of parsing human output. `legalities` maps format keys to strings,
  unlike the boolean legality values in combo rows. `owned` counts copies across
  binders and deck assignments; `available` counts binder copies only
  (tradeable without dismantling a deck). Basics (Plains, Island, Swamp,
  Mountain, Forest) report null for both because their supply is unlimited.
- `card similar <name> --json` → array of hits: full card fields +
  `score shared_count shared_tags` (score semantics above; `null` score
  when the seed had no stored vector).
- `card combos <name> --json` → array of hits: `{id, produces,
mana_value_needed, bracket_tag, popularity, legalities,
requires_commander, pieces: [{name, zones, must_be_commander}]}`.
- `query --json` → array of hits: the same full card object as
  `card <name> --json` (including `owned`/`available`) plus `score`
  (0 to 1, semantics above). Empty result prints `[]` with exit 3.
- `collection --json` → `unique_cards total_cards foils currency
total_value purchase_total color_identity curve rarity top_sets
by_universe by_franchise locations`. All money fields are US dollars;
  `currency` names the unit. `total_value` prices every owned copy by its
  exact printing. `top_sets` rows are `{set, set_name, cards}`.
  `locations` rows are `{name, binder_type, cards}`.
  `by_universe` splits copies into `multiverse` / `beyond` buckets, each
  `{cards, value}`; `by_franchise` subdivides the beyond bucket the same
  way (Marvel, Final Fantasy, …). Secret Lair prints count as beyond but
  never get a franchise. The human view prints full set names
  ("The Hobbit (HOB)") and a `Universes` breakdown line.
- `collection query --json` → array of card objects plus `score` and
  `locations: [{binder, binder_type, quantity}]`. `owned` counts copies in the
  requested scope: binder copies by default, plus named decks with `--deck`.
  `available` still counts all binder copies. Both counts are null for the five
  unlimited basics. `deck suggest` and combo completions use integer `owned`
  counts across binders and deck assignments. Ownership does not mean a copy is
  available without taking it from another deck.
- `collection conflicts --json` → `{rows: [{name, owned, demanded,
decks: [{name, quantity}], gap, buy_cost_usd}], unverified_decks:
[names]}`. Rows = cards whose total slot demand across all decklists
  exceeds owned copies, priced at the cheapest printing × gap, sorted by
  cost. `unverified_decks` names missing or unreadable decklists whose demand
  could not be checked. A completed report exits 0, even with shortages.
- `collection sell --json` → `{currency, view, valuation_basis,
evidence_scope, singles, bulk, protected, rows, review, review_cards,
warnings, scryfall_synced_at, combos_synced_at, excluded_binders, fund?}`. Rows include
  `name`, `owned_binder`, `sell_quantity`, `keep_quantity`, `deck_needed`,
  `spare_reserve`, `rarity`, `edhrec_rank`, `penny_rank`, `combo_variants`,
  `combo_piece_sets`, `owned_combo_options`, `combo_examples`, `game_changer`,
  `reserved`, `legal_in_format`,
  `market_value`, `sell_priority`, `action`, `reasons`, `hold_warnings`, and
  `printings`. Each printing has `set_code`, `collector_number`, `foil`,
  `binder`, `quantity`, `sell_quantity`, `keep_quantity`, and `price`.
  Singles summary: `{cards, copies, market_value}`. Bulk summary:
  `{cards, copies, normal_commons_uncommons, foils, rares_mythics, other,
rate_per_1000, estimated_proceeds}`. The last two values are null without an
  explicit rate. `protected` is `{deck_copies, spare_copies, game_changers}`.
  Review rows do not enter summaries or funding.
  `fund`: `{target_usd, achieved_usd, shortfall_usd, valuation_basis, picks,
allocations}`. Each allocation is `{name, sell_quantity, market_value, printings}`,
  even when the card is outside the displayed row limit. `picks` is an array of names.
  Exit 0 with recommendations or review rows, 3 when the selected view is empty.
- `deck hand <name> --json` → `{seed, hands: [{cards: [{name, mana_cost,
mana_value, type_line}], lands, mulliganed, redraws, bottomed_cards, advice,
assessment}]}`.
  Opening streams and mulligan rules match the simulator under equal deck and
  format settings. `redraws` counts redraws, and `bottomed_cards` lists names
  in bottoming order. `assessment` includes `evaluation_turn`,
  `individually_payable_spells`, `kept_size`, and `reasons`. Advice uses known
  lands, supported colors, and entry timing. It excludes future draws, ramp,
  fetch planning, and extra spell requirements. Individual payments are not
  a promise that the whole hand can play in sequence.
- `deck update <name> --dry-run --json` → `{name, dry_run, changes,
cost: {to_buy: {items: [{name, quantity, price_usd, owned}], total_usd},
freed_usd, net_usd}, sim, legal}`. Each `changes` row is
  `{section, removed: [[name, quantity]], added: [[name, quantity]],
changed: [[name, old_quantity, new_quantity]]}`. These card rows are arrays,
  unlike the objects returned by `deck diff --json`.
  `sim` is a report delta with `--sim`, otherwise null. `legal` is
  `{legal, violations: [{rule, cards, detail}], advisories}` with `--legal`,
  otherwise null. Exit 1 means new simulation findings or failed requested
  legality checks. A missing referenced deck card instead returns
  `{name, dry_run, changes: [], missing: [names]}` with exit 3. No deck file is written.
  Cost values are market estimates. `freed_usd` values owned copies removed
  from the deck; it does not record a sale. `net_usd` subtracts that value from
  the estimated missing-card cost, so it is not a cash-payment total.
  Preview costs use collection-wide ownership, including cards assigned to other
  decks. They do not reserve available copies. Check availability separately
  before treating a proposed addition as free.
- `deck update <name> --json` (apply) → `{name, added, removed, moved,
set, relocated, backfilled_basics, warnings, cards, maindeck_cards,
sideboard_cards, maybeboard_cards, commander_cards}`. `relocated` contains
  `{name, from}` rows. `cards` counts all sections. `maindeck_cards` excludes
  SIDEBOARD and MAYBEBOARD but includes COMMANDER. `commander_cards` is a subset
  of `maindeck_cards`, not an additional count to add to it.
- `deck dedupe <name> --json` → `{name, merged_lines, merged_copies, cards,
merged: [{name, copies}]}`. `cards` counts all sections. A no-op returns the same
  shape with zero merge counts, an empty `merged` array, and exit 3.
- `deck show <name> --json` → `{name, cards, sideboard_cards,
maybeboard_cards, primer,
currency, owned_value, missing_cost,
curve: {average_nonland_mana_value, histogram, target},
ramp: {lands, artifact_mana_sources, creature_mana_sources, other_mana_sources},
universe_census, sections: [{section, cards:
[{quantity, name, set, set_name,
collector_number, foil, owned, assigned_to_this_deck, owned_elsewhere,
covered_by, missing_reason, basic_land, price}]}]}`. `histogram` has seven buckets
  for mana values 0 through 5 and 6+. `cards` is the maindeck count, including
  commanders but excluding bench sections; `sideboard_cards` and
  `maybeboard_cards` report the bench sections separately. Curve and
  ramp describe the maindeck only. `target` starts with `target: ` and has one
  of six curve descriptions: commander decks by
  average MV (< 2.0 "comes together by t4-t6", < 3.0 "by t6-t8", else
  "by t8-t10"); 60-card decks (< 2.0 "does its thing by t4", < 3.0
  "by t4-t6", else "by t6"). These labels are heuristics, not measured timing.
  `primer` is the file path; read the primer's contents
  with `stm deck primer <name>` (plain markdown on stdout; an empty
  primer prints a note instead). `price` is the cheapest printing
  (foil entries price at the cheapest foil print when one exists).
  **`owned` = copies available to this deck** (assigned here + binders);
  it never contradicts `covered_by`. `assigned_to_this_deck` is the
  deck-assigned count; `owned_elsewhere` counts copies parked in other
  decks. `covered_by` is `deck` (deck-assigned copies fill the slots),
  `binder` (binder copies fill them), `basic`, or `missing`. A missing
  slot carries `missing_reason`: `not_owned` (buy it) or
  `held_elsewhere` (owned but in another deck). Do not call that card unowned.
  Offer an available substitute, ask before moving the existing copy, or price
  an additional copy.
  Do not add per-line `owned` counts across repeated names. Each row repeats
  the name's available pool. Use `deck buylist` for aggregate purchase quantities.
  With repeated names across entries or finishes, do not assume `missing_cost`
  equals the buylist total. The human view also prints a `To buy` block
  (top lines by cost with the reason,
  running total) and a full-set-name suffix on entry lines.
  `set`, `set_name`, `collector_number`, `missing_reason`, and `price` can be null.
  `universe_census` is null without set metadata; otherwise it contains
  `{multiverse, universes_beyond, franchises: {Marvel: 3, …}, ub_cards}`.
- `deck combos <name> --json` → per-section Spellbook audit:
  `[{section, complete: [combo rows], near_misses: [combo rows], bracket_breaks}]`.
  Both arrays use the same row fields: `{combo, id, produces, bracket_tag,
popularity, complete, missing, owned, self_contained, requires_commander}`.
  `owned` is a nullable boolean for the first missing card, not a copy count or
  proof that every missing piece is owned. `bracket_breaks` lists combo names.
  With `--bracket 3`, it flags S/K-tagged complete and near-miss rows in each
  section. This tag check does not prove that all infinite combos are absent.
  Review the main deck separately from bench sections.
- `deck cuts <name> --json` → ranked cut rows
  `[{name, qty, remove_qty, reasons: [{kind, detail}], score, pinned, rank,
replace_with}]`. `score` is 0 to 1. `replace_with` is `{role, candidates}` or null.
  Kinds: `game_changer` (over the
  bracket allowance), `illegal` (not legal in the deck's format; pin),
  `off_color_land` (produces or fetches nothing the deck can use; not a
  pin), `castability` (low mana readiness on curve), `curve` (mana-value outlier),
  `price`. Basics and the commander never appear; illegal cards and Game
  Changers over the bracket cap pin to the top (`pinned: true`, rank 1..
  regardless of score). **Pass `--bracket <B>` when reviewing against a
  bracket**. Without it the Game Changer cap is unknown, so no GC card
  ever pins (there is no bracket inference here, unlike `deck
simulate`). `remove_qty` is the copy count for the paired
  `deck update --remove` (full qty for pins, 1 of a 2-of, half of a
  3-4-of; human output prints "cut N of M copies"). `--count <N>` caps
  the rows (default 5). `--format <fmt>`
  judges legality against a pinned format (default: inferred from the
  deck's sections). `--for <role>` pairs every cut with fill candidates
  for the deficit role (owned first; `--max-price <USD>` caps each candidate,
  not the batch total) and discounts incumbents already
  serving that role.
  `--make-room-for sideboard` or `maybeboard` instead returns
  `[{bench_card, cut_candidate, reasons, score}]`, pairing each bench card with a
  main-deck cut. It skips off-identity or illegal bench cards.
- `deck diff <A> <B-or-file> --json` →
  `[{section, removed: [{name, quantity}], added: [{name, quantity}], changed:
[{name, old_quantity, new_quantity}]}]`. **Diff orientation:** `A` is the original,
  `B` the target: `removed` holds cards in A only (take them out), `added`
  holds cards in B only
  (put them in). Both operands accept a deck name or a ManaBox
  txt file path, so `stm deck diff "$HOME/Downloads/Original.txt" Optimized --markdown`
  uses the file as the original. Basics and quantity changes collapse
  to `changed` rows (`Forest: 16 → 12`); `--exact` diffs by print
  identity instead of name. Choose at most one of `--json`, `--markdown`, and
  `--as-update`. `--markdown` prints the change-log
  instruction table (`decks/<name>.changes.md` shape): basics as one
  "Remove 4 Forests and 2 Islands" line plus a remove/add table.
- `deck list --json` → `[{name, has_decklist, cards, sideboard_cards,
maybeboard_cards, owned, has_primer}]` (`cards` is maindeck;
  `has_decklist: false` marks collection decks whose list is not imported).
  `owned` counts copies assigned to that deck, not binder copies available to it.
- `deck legal <name> --json` → `{name, format, format_assumed, bracket,
legal, violations, advisories, notes, summary}`.
- `deck mana <name> --json` → static colored-source audit (no
  simulation): `{format, lands, sources,
credits: {lands, creature_mana_sources, artifact_mana_sources, cantrips},
requirements: [{name, mana_cost, mana_value, needs, have, deficit, ok}],
worst_deficits, tapland_count, untapped_t1_sources}`. `sources`,
  `untapped_t1_sources`, each credit category, and each requirement's `needs`,
  `have`, and `deficit` use `{white, blue, black, red, green}`, not WUBRG keys.
  `worst_deficits` is an array of strings. `format` holds only
  `"commander"` or `"constructed"` (a shape, not a real format key).
  This split follows the deck's sections, even when `--format` is supplied.
  Ordinary lands give 1.0 credit for each recognized color. Ordinary fetches
  give full credit for each recognized fetch color without a three-color-deck
  discount. Creature mana sources give 0.5 and artifact mana sources 0.75.
  Land-search ramp receives no colored-source credit. Recognized cheap cantrips
  give 0.25 per deck color, capped at ten effects. A land/spell modal double-faced
  card gives 0.8 colored-source credit; its separate effective-land weight is
  0.75 for mythic cards and 0.4 otherwise. Requirements use the Karsten 2022
  floors: Commander 12/17/21 by pips, or the 60-card pip-shape table with a
  land-count correction. These are static heuristics, not cast guarantees.
  Human output prints per-color totals and up to three deficit strings, then
  suggests at least the rounded-up source deficit for one unmet floor.
- `deck simulate <name> --json` returns the typed `SimReport` object:
  `{name, format, runs, outcomes, phase_observations, turns, seed, provenance, opponent_model?, sampling,
card_support, deck_shape, assumptions,
opening_hand, land_drops, mana_base, commander, station, companion,
creatures_by_turn, repeatable_sources_by_turn, mana, draw, role_access,
velocity, combat, win_conditions, interaction, color_mana_shortage,
color_pip_blocks, graveyard, milestones, color_sources,
card_castability, findings, summary, combo_access?, colored_sources?,
combos?, win_paths?, hypgeo?}`. `commander`, `station`, and
  `companion` are null when not applicable. Percent fields use the 0 to 100
  scale and round to two decimal places; JSON need not retain trailing zeroes.
  Fixed-turn values can be null outside the
  observed horizon. `outcomes` includes `requested_games`, `finished_games`,
  `player_loss_games`, `incomplete_games`, `incomplete_reasons`, and
  `incomplete_observations`. The latter groups typed reasons, turns, phases,
  configured limits, remaining-work counts, and game counts. Finished
  includes player losses. `phase_observations` counts completed phases among
  finished games. The exact contract lives in `report_schema.rs`.
  `provenance` includes schema, model, policy, RNG, and engine versions,
  deck and execution-order fingerprints, and an optional card-data fingerprint.
  A null card-data fingerprint does not verify equal Oracle inputs.
  `opponent_model` describes the scenario and includes `observed_activity`
  totals and per-game means for draws, casts, successful optional tax payments,
  and skipped casts. Payments count decisions, not mana spent. These counts
  do not measure per-engine player benefits or real-world activity.
  `attempted_games` and `raw_totals` include requested games;
  `observed_games`, `totals`, and `per_game` use finished games.
  `card_support` contains source-based compiler evidence, not full-card truth.
  `sampling` maps metric paths to `{status, observed_games, event_games,
percent, interval}`. Unavailable observations have null event counts,
  percentages, and intervals. Sampling intervals use whole games and do not
  cover unsupported rules or policy error.
  `deck_shape.total_cards` is the library plus commanders. Bench counts are
  separate. `mana_base` supplies land and ramp counts, target bands, a verdict,
  `bracket`, and `bracket_inferred_from_game_changers`. Check it before changing
  land counts. Flood expectations are uniform-sample references, not exact
  predictions after selection or draw-dependent sampling.
  `card_castability` measures mana readiness, independent of whether a card
  was drawn. `role_access` measures first hand arrival;
  `repeatable_sources_by_turn` counts engines the runtime treats as online.
  `interaction` measures remaining-hand payment capacity after main actions.
  `win_conditions.p50_lethal_turn` is conditional on reaching the threshold;
  pair it with `percent_games_at_or_above_table_life_by_turn`.
  `combos` and `win_paths` measure first-zone access, not legal execution.
  `findings` rows contain `{kind, identity, severity, percent_of_games,
color, explanation, suggestion, evidence}`. Evidence rows contain
  `{subject, explanation, percent_of_games}`. Findings use stable identities
  and category-level suggestions. A null finding percentage does not mean zero.
- `deck simulate <name> --baseline prior.json` loads the same typed
  report schema and prints metric, finding, and deck-shape deltas.
  JSON output is `{metrics: [{path, old, new}], findings: [{change,
identity, kind, explanation}], shape: [{name, old, new}]}`. Exit 1 only
  when a new finding appears. Changed schema, model, policy, RNG contract,
  engine version, format, or opponent scenario makes a baseline incompatible.
- `deck suggest <name> --role R --json` → array of `{name, oracle_id, mana_cost,
mana_value, type_line, edhrec_rank, game_changer, owned, price, score,
tags, oracle_text, color_identity}`. Theme queries and `--commander` use the same
  row fields. Ranked by fit: semantic search and
  tag matches fuse (reciprocal rank fusion, same as `query`), EDHREC rank
  breaks ties; owned cards list first, then unowned, each group in fit
  order. `tags` carries the Tagger labels that matched; `owned` is a copy
  count across binders and deck assignments (0 = none); `score` is the fused fit
  0 to 1. `--owned` keeps only owned card identities. It does not reserve copies
  or make cards held in another deck available. Check `deck show` ownership
  before counting a proposed addition as free. Drop `--owned` to search the
  whole catalog. `--commander` swaps the pool to
  commander-legal legendaries (legendary creatures/planeswalkers, plus
  legendary Vehicle/Spacecraft with a P/T box) ranked by theme fit to the
  deck. `--format <fmt>` pins the legality filter (cards legal in that
  format only); with `--commander`, `--format` gates the candidates to
  that format and `--bracket` drops Game Changers the allowance cannot
  hold. Without
  `--format`, commander-shaped decks filter to commander and
  the commander's colors, and other decks gate to cards legal in at
  least one 60-card format (a `note:` advises passing `--format` for a
  precise gate). Non-commander decks demote suggestions sharing zero
  colors with the deck, and lands rank by color relevance (basics and
  on-color fixing first, off-color fetches last). Commander decks
  exclude lands producing nothing in the color identity, and mono-color
  decks rank partial fetches below on-color lands unless the deck
  already runs duals of the extra color.
- Bare `deck suggest <name> --json` returns one-card-away Spellbook completions:
  `[{name, oracle_id, mana_cost, mana_value, type_line, edhrec_rank, game_changer,
owned, score, price, combo: {pieces, bracket_tag, produces, variants_completed},
oracle_text, color_identity}]`. These rows have `combo`, not `tags`.
  `combo.pieces` is a string of names joined with `+`. `combo.produces` contains
  `"Win the game"` when any completed variant has that feature; otherwise it is
  empty. Ranking uses variants completed, the win-the-game feature, popularity,
  then EDHREC rank. `score` blends variant count and popularity on a 0 to 1 scale.
  `owned` counts copies across binders and deck assignments; `--owned` keeps only
  owned completion cards. Commander decks complete Commander combos; other
  decks drop combos that need a commander. Pin a format with `--format` when needed.
  All suggestion modes apply the Game Changer allowance with `--bracket`:
  brackets 1 to 2 exclude them; bracket 3 excludes another Game Changer when
  the main deck already has three or more; brackets 4 to 5 have no such cap. This filter
  does not replace a full `deck legal` or `deck combos` review.
- `deck buylist <name> --json` → `{store, currency, rows: [{name, set,
set_name, collector_number, scryfall_id, foil, quantity, price,
sections}], total}`. All prices are US dollars. `sections` names where
  the copies live (e.g. `["DECK", "SIDEBOARD"]`).
  `rows` = cheapest released English printing of the right finish for the
  missing copies. Unknown card names and cards without a selectable priced
  printing can be absent from these rows. Review card data and missing slots;
  an empty buylist or zero total does not prove those cards cost nothing.

### Buylists

`stm deck buylist <name>` prints what needs purchasing as plain lines
(`2x Lightning Bolt`). Pipe to `pbcopy` or a file. Owned copies
(deck-assigned + binders) autofill deck slots read-only, so the list is
only the gap; nothing in the collection is moved. `--store cardkingdom`
emits `Name,Edition,Foil,Qty` (Edition is the full set name, e.g.
`Magic 2011`); `--store tcgplayer` emits TCGPlayer Mass Entry CSV.
`--json` gives rows and the estimated market total. Buy prices are the
store's job; the USD figure is a market-price estimate. Check unknown cards
and missing price data separately; the buylist can omit them.

### Gotchas

- Text searches can return nearest matches even for gibberish. No-text queries
  are alphabetical listings, not semantic rankings.
  Exit 3 can mean no cards exist in the selected scope or no cards pass the
  filters. It does not by itself prove that setup or index repair is needed.
- Prices are daily estimates from Scryfall's bulk, not storefront quotes.
  Card prices reference the cheapest released English printing; deck
  `missing_cost` prices missing copies at that cheapest print.
- Flavor-name prints ("Godzilla, King of the Monsters") resolve to their
  oracle card ("Zilortha, Strength Incarnate"); both names work everywhere
  a card name is taken.
- Two import paths exist: `stm collection import` for the
  collection CSV, `stm deck import` for a ManaBox deck txt export. The
  collection CSV never writes decklists; re-importing it never touches
  your deck files.
- `deck simulate` role counts are heuristic (oracle-text based). When a
  finding looks wrong, check the `deck_shape` counts in `--json` first;
  a misclassified card shows up there. Unseeded runs differ every time;
  always compare runs with the same `--seed`.
- Never hand-edit `stm.db`, `vectors.bin`, or `status.json`. Read the error
  and try the documented recovery path first. A reset loses the collection;
  see the setup warning above before considering it.
- `deck export` of a 100-card commander deck with an 8-card sideboard
  reports 108 total cards: the sideboard is a separate section in the txt
  file, not part of the legal deck. Check `sideboard_cards` separately in
  `stm deck show <name> --json`.
- Universes Beyond is a display and census concern only: `card --json`
  carries `universe` ("beyond"/"multiverse") and `franchise` (Marvel,
  Middle-earth, Dungeons & Dragons, …; null for Secret Lair). It never
  couples to legality or brackets. `stm deck show <name> --json`'s `universe_census`
  answers "how much crossover is in this deck" in one read; main deck
  only (the sideboard is not counted).

### Reference: read when needed

#### Power targets (the quality bar)

These counts guide deckbuilding; they do not prove deck strength. After each
fill round, check them against `stm deck show <name> --json`'s `ramp` and `curve`
blocks and the decklist. Name each short category, for example: "5 of about
10 draw pieces; add 3 to 5 more in the next batch."

**Commander targets** (brackets 1 to 2 baseline; average-deck template):

| Category         | Target                                                                      | Notes                                                                                        |
| ---------------- | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Lands            | 37 to 38                                                                    | Historical opener estimate: about 31% at 30 lands and 17% at 38 lands. See the limits below. |
| Ramp             | 10 to 12                                                                    | Rocks and dorks; prefer two-mana ramp.                                                       |
| Card draw        | 10 to 12                                                                    | Prefer repeatable engines over one-shot draws.                                               |
| Targeted removal | 10 to 12                                                                    | Aim for 3 to 5 at instant speed.                                                             |
| Board wipes      | 2 to 3 (go-wide), 3 to 4 (board-building), or 5 to 6 (control, aristocrats) | Count separately from targeted removal.                                                      |
| Win conditions   | 2 to 3 routes                                                               | Avoid depending on one route.                                                                |

Bracket modifiers:

| Bracket                       | Lands                      | Ramp                     | Other                                       |
| ----------------------------- | -------------------------- | ------------------------ | ------------------------------------------- |
| 1 to 2 (casual)               | 34 to 40                   | 7 to 12                  | baseline table above                        |
| 3 to 4 (upgraded / optimized) | 33 to 38 (3), 32 to 36 (4) | 8 to 11 (3), 9 to 12 (4) | interaction 10 to 14                        |
| 5 (cEDH)                      | 25 to 31                   | 10 to 16                 | interaction 12+, fast win conditions 2 to 4 |

The land and ramp targets come from EDHREC average decks (46 average decks
across 11 commanders) and Sam Black's cEDH land-count article. Treat the opener
percentages above as historical guidance, not verified results under the current
mulligan and payment policy. These bands are deckbuilding advice, not legal deck
limits. `deck simulate`'s `mana_base` block compares counts against its built-in
bands. Its `verdict` is the check, so a deck at 44 lands with no lands-matter
theme reads "trim 6 lands". When a deck has fewer than 6 nonland ramp sources,
consider adding mana rocks before following an "add lands" suggestion.

**60-card targets** (Karsten's land-count method; archetype consensus):

| Archetype | Lands    | Tap lands                        | Interaction                                         |
| --------- | -------- | -------------------------------- | --------------------------------------------------- |
| Aggro     | 20 to 22 | at most 3 when playing one-drops | 4 to 8 targeted, few or no sweepers                 |
| Midrange  | 23 to 25 | up to about 9 without one-drops  | 6 to 10                                             |
| Control   | 25 to 28 | not specified                    | 8 to 12, including 4 to 6 sweepers in the main deck |
| Combo     | 20 to 24 | not specified                    | 4 to 6 protection pieces                            |

Additional 60-card guidance: use four copies of core pieces. Aim for the deck's
main plan by turn 3 to 4 for aggro, or 4 to 6 for midrange and control. Plan a
15-card sideboard against the expected metagame and use 8 to 12 effective copies
of the win condition. The template counts 3 to 4 cheap draw or ramp spells as
one land.

**Archetype adjustments** (community consensus; SpellArmory's template):

| Archetype   | Adjustment                                        |
| ----------- | ------------------------------------------------- |
| Control     | 12 to 15 removal, 5 to 7 wipes, fewer plan cards  |
| Aggro       | 5 to 6 removal, lower curve                       |
| Combo       | More draw, 4 to 6 protection pieces, less removal |
| Tokens      | 2 to 3 wipes                                      |
| Aristocrats | 5 to 7 wipes                                      |

Build priority: enabler > payoff > enhancer. Anthems, lords, and other boosters
need the right board. Check that the deck has enough enablers. `low_castability`
measures mana readiness; it cannot prove that a payoff had nothing to support it.

**Mana-curve targets:** commander nonlands spread ≈ 9/18/15/10/5/5 across
mana value (MV) 1 to 6+, average MV near 3. 60-card decks scale the same shape to the
archetype; aggro averages near 2.

Use static counts and simulated observations together. `role_access` measures
hand arrival, not resolved ramp or an active engine.
`commander.percent_castable_by_curve` and `card_castability` measure mana
readiness, not actual casts. Run a same-seed check after the skeleton and each
confirmed batch. Check card support, outcomes, and the user's experience before
using findings to choose the next edit.

#### Colored sources (mana-base sanity check)

Before simulating, check the mana base against Karsten's source floors
(Frank Karsten, "How Many Sources Do You Need to Consistently Cast Your
Spells? A 2022 Update"). The tables below give general deckbuilding guidance,
not a full description of the CLI audit. Use the fixed credits in the
[`deck mana` JSON reference](#json-shapes-stable) for command results. Do not
add colored-source credit for land-search ramp or apply a three-color fetch
discount to those results.

Commander (99-card deck, 37 to 41 lands):

| Pips in the cost | Sources needed |
| ---------------- | -------------- |
| 1 (e.g. `2W`)    | ~12            |
| 2 (e.g. `WW`)    | ~17            |
| 3 (e.g. `WWW`)   | ~21            |

60-card deck (24 to 25 lands):

| Requirement     | Sources needed |
| --------------- | -------------- |
| 5C / 4C         | 9              |
| 3C              | 10             |
| 2C (e.g. `1CC`) | 12             |
| 1C              | 13 to 14       |
| CC (e.g. `UU`)  | 16 to 21       |
| 2CC             | 22             |
| 1CC             | 18             |
| CCC             | 23             |

For manual planning, add one source per additional color requirement on a gold
card, such as `GW`. Do not apply this again to the audit's reported requirements.
Name a source deficit in the next batch, just as you would a missing role.
Run `stm deck mana <name>` before buying lands and review every reported deficit
before the first simulation. Static source counts do not prove that a spell can
be cast on time.

## Part 2: deckbuilding workflow

Use [Mode A](#mode-a-build-a-new-deck) to build a new deck. Use
[Mode B](#mode-b-improve-an-existing-deck-or-precon) to improve an existing deck
or precon. Follow the selected workflow and ask its required questions before
proposing a finished list.

### Ground rules (both modes)

- **Small batches.** Propose at most five cards at a time. Present them as a
  table: card, role, price, own/buy, one line on why it fits. Wait for the
  user's confirmation before applying anything with `deck update`.
- **Price each batch before presenting it.**
  First pricing pass: run the fill with `--max-price` set to the
  remaining per-card budget. The cap excludes over-budget and unpriced
  candidates before the result limit. Then,
  per to-buy card in the final table: `stm card <name> --json` →
  `price` (or `price_foil` for a foil entry). Sum the to-buy
  prices, present the batch only when it fits the remaining budget
  (otherwise show cheaper alternatives in the same table), and track the
  running spend after every batch ("$34 of $50 used"). Available owned copies
  cost $0. Copies held in another deck need approval to move; otherwise price
  another copy or use a substitute. Use `stm deck show <name> --json` for cards
  already in the list. For new candidates, run
  `stm collection query "<card name>" --deck "<deck name>" --limit 100 --json`
  and select the exact `name`. Its scoped `owned` count includes binders and that
  deck's assignments. Subtract copies already needed by the deck before assigning
  them again. A missing search result does not prove zero ownership; confirm with
  `stm card "<card name>" --json`, whose `available` count covers binders only.
- **One question minimum per batch.** After each confirmed batch, ask at
  least one question: which direction to push next, whether a choice felt
  right, what to cut. Build the deck with the user, not for them.
- **Collection first.** Before proposing a purchase, search what the user
  owns (`stm collection query`) for a card that fills the same role. Prefer
  a playable owned card. Offer a paid upgrade as an option with the price gap,
  not as a requirement. When a proposed card is too expensive, use
  `stm card similar "<name>" --owned --json` to find owned alternatives.
- **Shared copies first.** Run `stm collection conflicts` once per
  session before filling: a card demanded by more decks than copies is
  short. Do not take that copy out of another deck without approval. Propose
  buying the gap (with the cost) or an owned substitute instead, and say
  when a fill competes with another deck.
- **Fund before buying.** When a purchase exceeds the budget, run
  `stm collection sell --target <shortfall>` to find surplus binder singles.
  Show the selected quantities and reasons. Funding uses market value before
  costs. Never add bulk or review rows to close the shortfall.
- **Show real hands.** When advising land-count or ramp changes, show one
  or two `stm deck hand <name> --seed N` hands so the user sees what the
  change does to real openers, not only percentages.
- **Dry-run before applying.** For any non-trivial batch, run
  `stm deck update "<name>" --from "batch.txt" --dry-run --sim --json` with the
  proposed operations first. Show the cost and consistency delta. After approval,
  apply those operations without `--dry-run`. `--sim` and `--legal` only affect
  previews; applying does not rerun those checks.
- **One writer per deck.** Serialize editing commands, including commands in
  parallel tool calls. Wait for one edit to finish before starting the next.
  Repeat the preview if another edit changes the deck before you apply it.
- Treat Plains, Island, Swamp, Mountain, and Forest as free and unlimited for
  planning. Price Wastes and snow basics explicitly. The deck coverage and buylist
  commands do not report missing snow basics. Basic lands are exempt from copy
  limits. Add counts that fit the mana plan.
- **Confirm legality.** After the deck is complete (or at the user's
  request), run `stm deck legal <name>` and walk the `notes` checklist as
  described in Part 1.

### Mode A: build a new deck

**Round 1: ask before choosing cards.** Get answers to all of these
(ask only what the user hasn't already said):

1. **Format.** Commander, Modern, Standard, Pioneer, Pauper, other?
2. **Budget.** Anchor the question in what they already have
   (`stm collection --json` first): "Your collection holds about $X; how
   much to add on top for this deck?" "Use what I own" = budget $0,
   confirmed. Get a specific amount before proposing purchases.
3. **Strength target.** For commander: **which bracket** (1 to 5; built-in
   terms: 1 exhibition, 2 core, 3 upgraded, 4 optimized, 5 cEDH; mention
   the Game Changer limits). For other formats: casual local play,
   tournament play, or the strongest legal list? Use the answer to choose suitable compromises and
   disruptive cards for the group.
4. **Theme or commander.** A commander, a mechanic ("life drain"), an
   aesthetic, or "suggest options".
5. Playstyle: aggro / control / combo / value? Speed preference?
6. Anything off-limits: cards the user refuses to play, hate pieces
   ("stax"), proxies, or a "don't buy singles above $X" rule.
7. **60-card formats only: the expected metagame.** What will they face:
   an aggro-heavy local scene or a combo-heavy tournament? The answers name the
   sideboard targets and the interaction split.

If the user wants suggestions for questions 4 to 6, offer 2 to 3 commander or theme candidates
with a one-line pitch each. Two search paths, in order:

1. **`stm deck suggest <name> --commander`** ranks commanders by theme
   fit to the deck's own cards, with EDHREC rank, price, and ownership.
   Needs a deck to exist first (`stm deck create <name>` is enough).
2. **`stm query "<theme>" --type "Legendary Creature" --format commander --json`**
   for catalog browsing. The `--type` filter is a type-line substring:
   `Legendary Creature` covers creatures. Run separate queries with
   `--type "Legendary Vehicle"` or `--type "Legendary Spacecraft"` (they
   can command under the 2025+ rules when they have a printed power/toughness box; check `power`/`toughness`
   are not null in the JSON). Combine with `--color-identity GU` etc. to
   pin the identity. `--owned` is not available here; check
   ownership with `stm card "<name>" --json` per candidate. Its `available`
   count excludes copies held in other decks.

**Round 2: confirm the skeleton.** Once round 1 is answered:

1. Review the collection stats from round 1. Refresh them if ownership changed.
2. **Research the archetype before the skeleton** (manual web research):
   - Commander: look the commander up on EDHREC (web). Note the average
     list's land count, ramp count, and the top themes. Compare against
     the power-target tables above; say where you deviate and why.
   - 60-card formats: check the metagame pages on MTGGoldfish (web) for
     the format. Note the top archetypes, their land counts and curves,
     and the cards the expected opponents will play. Use this for the
     curve targets and the sideboard plan.
3. Pick the commander (or core theme cards). For commander, verify color
   identity implications early; everything must fit it.
4. Propose the mana base plan (how many basics, which duals/fetches owned
   vs to buy) and the ramp plan for the bracket (e.g. bracket 2 wants about 8 to 10
   sources; cEDH wants more). Show owned vs to-buy with prices.
5. Show the skeleton to the user, state the projected spend, and get an OK
   before writing anything.
6. Once the skeleton is written, sanity-check it with `stm deck simulate
<name> --seed 42 --json`: commander on-curve and land-drop rates should
   fit the mana plan, and `mana_base.verdict` should read
   "on target" (or the lands-matter note) for the target bracket. Read these
   as diagnostics under the stated card support and outcomes, not proof of
   legal casts. Review the mana plan before filling if they look wrong.

**Round 3: fill by category.** Work in this order, one
batch per round, confirming between each: ramp → card draw → removal/board
wipe → win conditions → flexible slots/theme pieces → mana base finish. For
each card:

- Use `stm deck suggest "<name>" --role <role> --max-price <USD> --json`
  for role fills. It combines semantic matches and role labels, applies the
  deck's format rules, and lists owned hits first. Set `--format` when the
  deck's sections do not identify the intended format. `--limit N` widens the
  pool beyond the default 10, up to 50.
  The 69 listed role names include aliases such as `card-advantage` and
  `cardadv` for draw. An unknown role exits 2 with the known-name list on
  stderr. A known role with no results prints `[]`, exits 3, and lists known
  roles on stderr. For theme cards, pass a query such as `"frog payoff"`.
  If `--role <role> --owned` returns fewer than three hits, try these in order:
  drop `--owned`; search `stm collection query "<role description>" --format <format> --json`;
  use `stm card similar "<exemplar>" --owned --json`. Use `stm query` for wider
  catalog browsing. For Commander collection searches, also pass
  `--color-identity <CI>`. Check each proposed card's role; a search hit is not proof.
- Check `stm card <name> --json` for price, `game_changer`, and
  `legalities` before proposing (suggest already carries most of it).
  Respect the bracket's Game Changer limit.
- State cost-to-user explicitly in every batch ("all owned, $0" or "2 to
  buy: $11.40 total").

**Round 4: finish and review.** Fill the mana base. Ordinary basics are free,
so the budget rule for lands: cover colors with basics first, then check
what the user already owns (`stm collection query --type Land
--json`) before proposing anything. Run `stm deck mana <name>` before
buying lands. Review source deficits before simulating. The audit is static.
Prefer suitable owned lands over buying fetches or shock lands. When the
strength target calls for untapped fixing, state the upgrade cost and let the
user decide. Then run `stm deck
show <name> --json`, check the power targets above, and present the final:
total cards, curve, ramp counts, owned/total, **total spend vs budget**, and
`stm deck legal <name>` output (including the bracket checklist review). Run a
final `stm deck simulate <name> --seed 42 --json` and present its `findings[]`
(or "no findings") as the deck's consistency report. Write a
short primer with `stm deck primer "<name>" --set "primer.md"`. The primer
describes the current deck: game plan, mana base (counts and fixing lands), interaction
inventory, protection, and known limits with the next upgrade path. It is
not a build log: leave out decisions made, cards considered and rejected,
and history of the session.
Offer follow-ups: budget-trim pass, upgrade path for later, or a sideboard
for 60-card formats.

If the user goes over budget: stop and offer options. Cut the most
expensive card for a cheaper substitute, swap a purchase for an owned
"good enough" card, or ask whether to raise the budget. Never silently
exceed it.

### Mode B: improve an existing deck or precon

#### Assess before editing

1. Ask for the upgrade budget, target format or bracket, and what went wrong
   in play. Skip questions the user already answered.
2. Import a missing list with `stm deck import <name> <file>`. This overwrites
   a same-name list, so use a new name or back it up first. Keep the original
   unchanged and work on `"<name> Improved"`; `deck copy` can create that copy.
3. Read `stm deck show <name> --json`, then run `stm deck legal <name>` and
   `stm deck mana <name>`.
   Count open slots before proposing cuts. Bench cards do not create main-deck
   space. Commander needs 100 total cards; most constructed formats need at
   least 60. Do not confuse an undersized list with a list that needs cuts.
4. Save a complete same-seed simulation report. Check `card_support`,
   `assumptions`, `outcomes`, and observation denominators before its findings.
   Report value, missing cost, legality violations, role gaps, and supported
   consistency observations. Low mana readiness does not prove a card was
   drawn or stuck in hand.
5. Ask whether those observations match the user's games. If the cause is
   unclear or the user asks, research EDHREC primers or bracket guidance and
   cite the sources. Use `card similar <name> --owned --json` to look for an
   owned replacement for a named weak card.

Use findings to choose a next check, not an automatic edit:

| Finding kind                    | Next check                                                                     |
| ------------------------------- | ------------------------------------------------------------------------------ |
| `insufficient_land_drops`       | Check land slots; in Commander, also review supported cheap ramp.              |
| `excess_lands_seen`             | Compare the land plan with the uniform-sample flood reference before trimming. |
| `late_commander_cast`           | Check commander mana readiness, early curve, and supported ramp.               |
| `insufficient_color_mana`       | Read `color_sources` and `color_pip_blocks` for the named color.               |
| `limited_draw_access`           | Check draw-card access and the support of proposed engines.                    |
| `unused_mana`                   | Review curve and draw access before adding cheaper spells.                     |
| `low_castability`               | Inspect each card's mana-readiness target; this is not a cast count.           |
| `low_removal_access`            | Review the removal role and classify the cards correctly.                      |
| `low_win_condition_access`      | Review win-condition role access, not a claimed win rate.                      |
| `limited_interaction_readiness` | Check the instant count and remaining-hand payment capacity.                   |

For constructed decks, the default horizon is eight turns with the stated
London mulligan policy. `mana_base.bracket` is null and there is no Commander
ramp band. Do not apply Commander bracket advice. Plan a sideboard of up to
15 cards for the expected field instead of widening the main deck.

#### Review and apply each batch

1. Propose at most five cards in a table. State removals, additions, ownership,
   prices, the budget total, and the reason for each change. Prefer available
   owned cards. A `held_elsewhere` copy belongs to another deck; offer a
   substitute or ask about buying another copy. Do not call it unowned.
2. Use `deck suggest` for fills and `stm deck cuts <name> --for <role> --bracket <b>` for
   cut-and-fill pairs. Cuts exclude the commander and basic lands. Preview the
   proposed operations with `stm deck update "<name>" --from "batch.txt" --dry-run --sim --json` before approval.
3. Wait for confirmation. Export a checkpoint to a new file, then apply the
   reviewed `add`, `remove`, `set`, or `move` operations with
   `stm deck update "<name>" --from "batch.txt"`. Batch files allow `#` comments.
4. Compare with the saved baseline using the same seed, runs, horizon, format,
   and scenario. Baseline JSON gives `metrics`, `findings`, and `shape` deltas.
   Exit `1` means a new finding. Check incomplete games and key-card support
   before calling a metric change an improvement.
5. Show the result, remaining budget, and at least one question before the
   next batch. Save a new complete report if it will be the next baseline;
   delta JSON is not a complete report.

At the end, run `deck show` and `deck legal`. Keep Commander at exactly 100
cards and use 60 as the usual constructed tuning target. If needed, review
`stm deck update <name> --backfill-basics` or add on-identity basics yourself, then check
legality again. Update the primer to describe the current list, not the build
history. State spend versus budget and the result for each user-reported issue.

Generate change instructions with the original first:

The shell replaces the destination files. Preserve existing notes first.

```sh
stm deck diff "Froggy Original" "Froggy Improved" --markdown > "froggy.changes.md"
stm deck diff "Froggy Original" "Froggy Improved" --as-update > "batch.txt"
```

Check that added, removed, and changed quantities match the JSON diff. Apply
the generated batch only after review and approval.

A Commander sideboard can hold an upgrade kit. Review the current main deck
with its actual bracket, including `deck combos <name> --bracket 3` for a
bracket-3 list. S/K-tagged combos are flagged by that audit. Use
`deck cuts <name> --make-room-for sideboard --json` (or `maybeboard`) to pair
`bench_card` with `cut_candidate`, `reasons`, and `score`. State equal-size swaps
so the user can keep both versions. Recheck legality and combo advice after
the swap; parking a card on the bench does not certify the whole deck.

If crossover cards are present, include `universe_census` counts and franchises
in the final review. That census does not affect legality.

### Output style during deckbuilding

Keep prose tight. One table per batch, one question per batch, running
budget line after every `deck update`. Show the full deck only on request
or at the finish, not after every batch.
