# Seize the Mana

> [!NOTE]
> Seize the Mana is still pre-alpha. Commands, flags, and the data format
> can change without notice.

[![Validate](https://github.com/z0w0/seizethemana/actions/workflows/validate.yml/badge.svg)](https://github.com/z0w0/seizethemana/actions/workflows/validate.yml)
[![Release build](https://github.com/z0w0/seizethemana/actions/workflows/build.yml/badge.svg)](https://github.com/z0w0/seizethemana/actions/workflows/build.yml)
[![crates.io](https://img.shields.io/crates/v/seizethemana.svg)](https://crates.io/crates/seizethemana)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![aislop](https://badges.scanaislop.com/score/z0w0/seizethemana.svg)](https://scanaislop.com)

`stm` helps you build Magic: The Gathering decks on a budget. It searches card
data, tracks the cards you own, estimates missing-card costs, manages decklists,
checks legality, and simulates consistency. Use the CLI directly or add the
optional assistant skill for guided deckbuilding.

Your collection records what you own. A decklist records what you want to play;
adding a card to a decklist does not add it to your collection.

The database and search index run on your machine. Setup downloads the data and
search model. Some read commands refresh stale data before returning results;
use `--offline` to skip those refreshes. Setup, sync, and URL imports need a
network connection. URL imports fail in offline mode. `--offline` does not block
explicit setup or sync, or downloads for a missing search model.

To work on the project, start with [contributor setup and first-change paths](CONTRIBUTING.md).

## Install

Use [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) for a prebuilt
binary, or build from source with Rust 1.88 or later and Cargo:

```sh
# Prebuilt binary (needs cargo-binstall)
cargo binstall seizethemana

# Or build from source
cargo install seizethemana
```

Run setup to download the data and build the local index:

```sh
stm setup
```

Setup downloads card data, prices, and tags from
[Scryfall](https://scryfall.com/docs/api/bulk-data), plus combo data from
[Commander Spellbook](https://commanderspellbook.com). The first index build
uses several minutes of CPU time. Use `stm sync` to refresh the data later.

**Warning:** `stm setup --force` deletes `stm.db`, its SQLite sidecars,
`vectors.bin`, and `status.json` before rebuilding. This loses all collection
records, including deck assignments and purchase prices. Stop other `stm`
processes and back up the whole data directory first. Saved decklists and
primers in `decks/` remain. `stm sync --force` refreshes data without deleting
the collection.

Forced setup warns and deletes the reset files without asking for confirmation.

## Optional assistant skill

The CLI does not need an AI assistant. Install the `seizethemana` skill to give
your assistant a deckbuilding workflow. It asks about format, bracket, and budget,
checks your collection, and uses `stm` price estimates:

```sh
npx skills add z0w0/seizethemana
```

Or copy it by hand into your assistant's skills folder
(`~/.claude/skills/`, `~/.agents/skills/`, and so on):

```sh
git clone https://github.com/z0w0/seizethemana /tmp/seizethemana
mkdir -p ~/.agents/skills
cp -r /tmp/seizethemana/.agents/skills/seizethemana ~/.agents/skills/
```

Then ask for a deck or an update:

- "Build me a 100-card Breya commander deck. I want bracket 2, spend at
  most $40, and use cards I already own when they're good enough."
- "My Feldon deck keeps running out of steam. What should I change?"

Read [the skill source](.agents/skills/seizethemana/SKILL.md) for its workflow.

## Commands

| Command                                                  | What it does                                                                      |
| -------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `stm setup`                                              | Download data and build the index. Read the reset warning before using `--force`. |
| `stm sync [--force]`                                     | Refresh card data, prices, tags, and combos.                                      |
| `stm card <name>`                                        | Show rules text, price, legality, and community role labels.                      |
| `stm card similar <name> [--owned]`                      | Find cards with similar tags and stored vectors.                                  |
| `stm card combos <name> [--format FMT]`                  | List known Spellbook combos.                                                      |
| `stm query [text] [filters]`                             | Search by words and meaning, or list cards by filters.                            |
| `stm collection`                                         | Show owned counts, estimated value, colors, and mana curve.                       |
| `stm collection conflicts`                               | Find decklist demand that exceeds owned copies.                                   |
| `stm collection import <csv> [--add]`                    | Replace collection records, or merge with `--add`. Back up first.                 |
| `stm collection query [text] [filters]`                  | Search or list owned cards in selected locations.                                 |
| `stm collection sell`                                    | Recommend surplus copies after deck needs and reserves.                           |
| `stm deck create / list / show / delete / copy / dedupe` | Manage deck files.                                                                |
| `stm deck update <name> --add/--remove/--set/--move`     | Edit card quantities and sections.                                                |
| `stm deck hand <name>`                                   | Sample opening hands with mulligan advice.                                        |
| `stm deck mana <name>`                                   | Check colored sources against static requirement floors.                          |
| `stm deck legal <name> [--format FMT] [--bracket N]`     | Check legality and Commander bracket signals.                                     |
| `stm deck import <name> [file] [--url URL]`              | Import a deck file or public Archidekt URL.                                       |
| `stm deck suggest <name> [query]`                        | Suggest role fills, theme cards, or combo pieces, owned first.                    |
| `stm deck simulate <name>`                               | Measure consistency under a stated opponent scenario.                             |
| `stm deck combos <name> [--bracket N]`                   | Audit Spellbook combos by deck section.                                           |
| `stm deck cuts <name> [--for ROLE] [--bracket N]`        | Rank cut candidates and optional role fills.                                      |
| `stm deck diff <A> <B> --markdown`                       | Show change instructions between deck names or files.                             |
| `stm deck buylist <name>`                                | List missing copies at the cheapest eligible printing.                            |
| `stm deck export <name> <file> --format names`           | Export plain `qty Name` lines.                                                    |
| `stm deck primer <name> [--set <file>]`                  | Read or write strategy notes.                                                     |

Use a command's `--help` for its options. Most read commands support `--json`.
Results go to stdout; progress and diagnostics go to stderr. Use `--no-color`
or `NO_COLOR` for plain output. Global options also include `--data-dir DIR`,
`--offline`, and `--verbose`.

Exit codes are `0` for success, `1` for runtime errors, `2` for usage errors,
and `3` for no results or a missing card. Legality violations and simulation
findings also use `1`; baseline comparisons use `1` only for new findings.
See [CLI design](docs/design.md#command-results-and-examples) for pipeline use.

## Working with your collection

`stm` reads ManaBox CSV exports with `Binder Name` and `Binder Type` columns.
It tracks cards in binders by printing and quantity. It ignores wishlist rows
and keeps deck rows as ownership assignments. It does not create deck files:

```sh
stm collection import ~/Downloads/collection.csv
```

Importing replaces the collection by default. Back up the data directory before
replacing records you need. `--add` adds copies to existing records; importing
the same rows again counts them again. To save a decklist, export that deck as
text and run `stm deck import <name> <file>`.
The collection records owned cards. Deck files record intended deck contents;
a card in a deck file is not proof that you own it.

```sh
stm collection                              # overview with value and stats
stm collection query "board wipe" --color BR --json   # search only what you own
stm collection query --type Elf              # list owned Elves in the default binder scope
```

Scryfall price data refreshes with sync. Collection values are estimates based
on that data, not guaranteed sale proceeds.

## Selling cards

`stm collection sell` recommends surplus binder copies without changing your
collection. It protects each deck's unmet demand, keeps useful spare copies,
and retains cheaper owned printings first. Assigned copies never enter the
sale pool. Use `--exclude-binder` to leave collector copies out of both sales
and deckbuilding reserves.

```sh
stm collection sell                        # ranked singles priced at least $1 each
stm collection sell --exclude-binder Collect # leave collector copies out of the pool
stm collection sell --details               # sale printings and demand evidence
stm collection sell --review                # include personal hold candidates
stm collection sell --exclude-binder Collect --output csv > sell.csv
stm collection sell --min-price 2           # narrow sale printings to $2 or more
stm collection sell --bulk                  # excess sub-$1 copies, largest lots first
stm collection sell --bulk --bulk-rate 5    # uniform $5 per 1,000-copy estimate
stm collection sell --target 38             # ranked singles toward $38 market value
stm collection sell --format modern --json # Modern combo scope and card legality
```

Singles use market value before fees and shipping, not guaranteed proceeds.
Bulk needs an explicit `--bulk-rate`; it never counts toward `--target`.
Review candidates stay outside recommendation totals, funding, and exports.
`--format` scopes combo evidence and reports legality, not format usage.

Use `--output txt` or `--output csv` for a full ManaBox sale list. Exports ignore
`--limit`; with `--target`, they contain only funding picks. CSV distinguishes
etched and regular foil. `--output` cannot combine with `--json`, `--details`,
or `--review`.

See [Sell rules and evidence](docs/collection-sell.md) for reserves, filters,
ranking, review rules, output fields, and data limits.

## Building decks

A saved deck is a ManaBox text file at `decks/<name>.txt`, with an optional
Markdown primer for strategy notes. Edit these files by hand or use deck
commands:

```sh
stm deck create Froggy
stm deck update Froggy --add "1 Blasphemous Act"
stm deck show Froggy
stm deck export Froggy ~/Downloads/froggy.txt   # import straight into ManaBox
```

Run commands that edit the same deck one at a time. Concurrent edits can
overwrite each other's changes.

Import and export support `manabox`, `moxfield`, `archidekt`, `arena`, and
`names`. Import detects the text format; `--format FMT` selects it explicitly.
Export defaults to `manabox`. URL import supports public Archidekt decks only;
use exported files for other sites.

Import replaces a decklist with the same name without changing ownership.
Back up that deck file first. Export needs `--force` to overwrite its destination.

Update specs look like `[section:]qty Name [(SET) [cn]] [*F*]`. `--add`
adds copies, `--remove` takes them away, `--set` pins an exact count
(`--set "0 X"` deletes the line). Updates resolve supported name forms to
canonical card names. An unknown name fails with suggestions rather than
writing an unchecked entry.

`stm deck show` compares your copies' estimated market value with the estimated
cost of missing cards. These are not sale proceeds or checkout quotes.
Plains, Island, Swamp, Mountain, and Forest are free and
unlimited for ownership and budget checks. Wastes and snow basics remain
priced and tracked.

### Deck legality

```sh
stm deck legal Froggy                       # guesses the format from the deck
stm deck legal Froggy --format commander --bracket 3
```

The command checks deck size, copy limits, commander rules, color identity,
format legality, and Game Changer allowances for Commander brackets 1 to 5.
It names violations and exits `1` for an illegal deck. Pass `--bracket N` for
a checklist of bracket signals. Checks such as tutor counts and combo speed
still need judgment.

### Filling holes in a deck

```sh
stm deck suggest Froggy --role ramp        # fill a role: draw, ramp, removal, board-wipe, sacrifice, ...
stm deck suggest Froggy "frog payoff"      # or semantic search, owned first
stm deck suggest Froggy --bracket 2        # stay inside a Commander bracket
stm deck suggest Froggy --format modern    # filter to one format
```

Suggestions rank cards by fit with the deck's theme and roles. Owned cards
appear first, with the cheapest priced printing shown. Suggestions can also
include Commander Spellbook combos. They name a combo when one card would
complete it. Non-Commander formats omit combos that need a commander.

### Deck simulation

```sh
stm deck simulate Froggy --runs 10000 --seed 42
stm deck simulate Froggy --bracket 3       # land and ramp target band
stm deck simulate Froggy --threads 4 --batch-size 256 --seed 42 --json
stm deck simulate Froggy --opponents inactive --seed 42
stm deck simulate Froggy --opponent-scenario opponents.json --seed 42
```

The simulator measures mana, card access, commander timing, and supported
engines under a fixed play policy. It is a consistency check, not a win-rate
predictor. Commander decks use three synthetic opponent seats by default;
other decks use inactive activity. Opponents do not block, counter spells, or
cast removal.

Read the report's ability-level support and assumptions before trusting a key
engine. Unsupported rules can make results too fast or too slow. Primary rates
exclude incomplete games, which can bias engine-heavy decks. Combat and
commander damage are potential only; they do not reduce opponent life or declare
wins.

Use the same seed, run count, rules, format, and scenario to compare an edit:

```sh
stm deck simulate Froggy --seed 42 --json > before.json
# Edit the deck, then compare under the same settings.
stm deck simulate Froggy --seed 42 --json --baseline before.json
```

Exit `1` means findings exist, or new findings exist when comparing a baseline.
See [Deck simulation](docs/simulator.md) for scenarios, metrics, supported rules,
fixture coverage, and benchmarks.

The [Commander source fixtures](docs/simulator.md#expanded-commander-sources)
contain 1,208 captured card identities: 1,000 site-ordered EDHREC cards, 185 added
lands, and 23 Commander-legal Behold cards. This is a source and smoke-test
corpus, not a claim that all cards are Commander-legal or fully simulated.

## Finding cards

Describe an effect with `stm query`. It combines keyword and meaning-based
search. Use `stm collection query` to search only owned cards:

```sh
stm query "whenever an opponent draws a card, they lose life" --limit 3
stm query "graveyard recursion" --color BG
stm card "Lightning Bolt"
stm card similar "Cyclonic Rift" --owned
stm card combos "Demonic Consultation" --format commander
```

`card similar` ranks by shared community role labels and stored vectors.
`--owned` includes binder copies, not copies held only in deck assignments.
`card combos` shows known pieces, results, and legality. `--format modern`
limits combos to that format; 60-card formats omit commander-required combos.

Text is optional for `stm query` and `stm collection query`:

```sh
stm query --type Elf --limit 10
stm query --set BLB --json
stm query --limit 10
stm collection query --set BLB --json
```

Without text, queries list matching cards alphabetically. `--limit` defaults
to 20, with a maximum of 100. Collection queries use binders by default;
`--binder` and `--deck` change the location scope. Filter-only listings do not
load the model or vectors, embed text, or start an automatic refresh. JSON
uses `score: 0`, not a relevance score.

In both text search and no-text listings, `--set` matches only the card's
stored primary printing. Other printings do not make the card match. In
collection queries, ownership still matches by card name; `--set` does not
filter the printing of your owned copies.

Both query commands accept combined filters such as
`--type Creature --color WU --cmc '<=3' --rarity rare --format commander`.
See [query modes](docs/querying.md#text-search-and-filter-only-listing) for details.

## How it works

`stm` stores its SQLite database, search index, and deck files in
`~/.seizethemana/`. Use `--data-dir DIR` to choose another location.

Search combines keyword and meaning-based rankings with reciprocal rank
fusion. It uses local data rather than per-card API requests. Sync re-embeds
cards whose searchable content changed. Some read commands run a sync before
returning results when data is over 24 hours old, unless you use `--offline`.
See [architecture](docs/architecture.md) for the schema and sync pipeline.

## Docs

| Document                                             | What it covers                                            |
| ---------------------------------------------------- | --------------------------------------------------------- |
| [`CONTRIBUTING.md`](CONTRIBUTING.md)                 | Source setup, first-change paths, checks, and benchmarks  |
| [`docs/architecture.md`](docs/architecture.md)       | Data schema, search index, sync pipeline, and deck module |
| [`docs/simulator.md`](docs/simulator.md)             | Simulator model, turn pipeline, metrics, and limits       |
| [`docs/design.md`](docs/design.md)                   | CLI output and human- and agent-readable modes            |
| [`docs/querying.md`](docs/querying.md)               | Search settings, embedding models, and evaluation         |
| [`docs/collection-sell.md`](docs/collection-sell.md) | Sale reserves, prices, evidence, and exports              |

## Data sources

Card data, prices, and tags come from
[Scryfall](https://scryfall.com/docs/api/bulk-data) daily bulk exports. Role
labels come from Scryfall's Tagger community project. Combo data comes from
[Commander Spellbook](https://commanderspellbook.com). This project is
unofficial Fan Content permitted under the Wizards of the Coast Fan Content
Policy. Scryfall, Commander Spellbook, and Wizards of the Coast do not produce
or endorse this project.

## Development

Most changes use offline tests and do not need `stm setup` or your collection.
See [CONTRIBUTING.md](CONTRIBUTING.md) for build prerequisites, required checks,
and benchmarks. The [module map](docs/architecture.md#module-map) shows where the
CLI, search, storage, simulator, assistant skill, and website live.

Build with `--release` before running real embedding workloads. Debug builds are
too slow for setup or re-embedding. [CHANGELOG.md](CHANGELOG.md) records notable
changes.

## AI disclosure

AI agents help write this codebase, and humans review the changes. See
[AI_DISCLOSURE.md](AI_DISCLOSURE.md) for details.

## License

MIT. See [LICENSE](LICENSE).
