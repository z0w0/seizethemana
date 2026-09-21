# Seize the Mana

> [!NOTE]
> Seize the Mana is still pre-alpha. Commands, flags, and the data format
> can change without notice.

[![Validate](https://github.com/z0w0/seizethemana/actions/workflows/validate.yml/badge.svg)](https://github.com/z0w0/seizethemana/actions/workflows/validate.yml)
[![Release build](https://github.com/z0w0/seizethemana/actions/workflows/build.yml/badge.svg)](https://github.com/z0w0/seizethemana/actions/workflows/build.yml)
[![crates.io](https://img.shields.io/crates/v/seizethemana.svg)](https://crates.io/crates/seizethemana)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![aislop](https://badges.scanaislop.com/score/z0w0/seizethemana.svg)](https://scanaislop.com)

`stm` is a command-line helper for Magic: The Gathering. It searches every
card ever printed, tracks what you own and what it's worth, builds and
checks decks, and simulates games to find consistency problems.

Everything runs on your own machine. One setup command downloads the data
once; after that, searching and browsing work offline and never hit an
API.

## Try it

Semantic search, not just keywords. Describe the effect you want and
`stm` finds the cards that do it — including cards whose text words it
differently:

```text
$ stm query "whenever an opponent draws a card, they lose life" --limit 3
 1. Sheoldred, the Apocalypse {2}{B}{B} mythic Legendary Creature — Phyrexian Praetor (0.500)
 2. Ghitu Encampment  uncommon Land (0.500)
 3. Vector, Imperial Capital  common Land — Town (0.492)
```

The top hit is exactly the described effect: Sheoldred punishes every
card an opponent draws. The search found it from the described behavior,
not from a card-name lookup.

Import your collection from a [ManaBox](#working-with-your-collection)
export and `stm` knows what you own. Deck views show it directly:

```text
$ stm deck show Stationz
Stationz  107 cards  primer: ~/.seizethemana/decks/Stationz.primer.md

      Own ✓ 61/107 (57%)
    Value $42.10 owned · missing $38.55
    Curve ████████████   2 24 · avg CMC 2.9
          ...

// DECK
    1 Alibou, Ancient Witness (EOC) 113  ✓ (own 1/1)  @$0.35
    1 Baleful Strix (FIC) 318  (own 0/1)  @$1.10
    12 Island (SOS) 274  (basics unlimited)

$ stm collection
Collection: 412 unique cards, 938 total, 210 foils
    Value $975.40 now (paid $1,512.06)
   Colors ████········ W 123
          ██·········· U 45
          ...
```

And it checks the rules so you don't have to:

```text
$ stm deck legal Froggy --format commander --bracket 2
Froggy  commander  61 cards, 4 unique (60 basic-land copies)
  bracket 2

error: not legal (1 violation)
  error: deck size: 61 cards; commander decks are exactly 100

note: not checked automatically:
  - no two-card combos that end the game early
  - no mass land destruction
  ...
```

## Install

```sh
# Prebuilt binary (needs cargo-binstall)
cargo binstall seizethemana

# Or build from source
cargo install seizethemana
```

Then run setup once:

```sh
stm setup
```

Setup downloads card data and tags from
[Scryfall](https://scryfall.com/docs/api/bulk-data) (about 25 MB), combos
from [Commander Spellbook](https://commanderspellbook.com) (about 28 MB),
builds a local search index over all ~32,500 cards (2–3 minutes of CPU
time on the first run), and downloads a small machine-learning model
(about 35 MB). After that you're self-sufficient: daily `stm sync`
refreshes prices, tags, and combos, and read commands refresh on their
own when data is older than a day.

## Let an AI assistant do the deckbuilding

If you use an AI coding assistant (Claude Code, opencode, Codex, Cursor,
and others), you can hand the whole deckbuilding job to it. `stm` ships
with a skill: a set of instructions the assistant reads so it knows how
to drive `stm` properly. It asks you the right questions (format, bracket,
budget), checks what you already own, stays inside your budget, and
keeps the deck legal for its bracket.

Install it with one command:

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

Then just ask. For example:

- "Build me a 100-card Breya commander deck. I want bracket 2, spend at
  most $40, and use cards I already own when they're good enough."
- "My Feldon deck keeps running out of steam. Here's what happened in my
  last game: …what should I change?"

The skill keeps the assistant on-rails: it works in small confirmed steps
instead of dumping a full deck list, and it prices every suggestion
against your budget using `stm`'s own numbers. The skill's source lives at
[`.agents/skills/seizethemana/SKILL.md`](.agents/skills/seizethemana/SKILL.md)
if you want to see exactly what the assistant will be told.

## Commands

| Command | What it does |
| --- | --- |
| `stm setup [--force]` | First-time setup: download data, build the index |
| `stm sync [--force]` | Refresh card data, prices, tags, and combos |
| `stm card <name> [--json]` | Full detail for one card: rules text, price, format legality, community role labels |
| `stm card similar <name> [--owned]` | Cards that play like a given card (see [Finding cards](#finding-cards)) |
| `stm card combos <name> [--format FMT]` | Known combos with a card, from Commander Spellbook |
| `stm query <text> [filters]` | Hybrid search (keywords + meaning) over every card |
| `stm collection [--json]` | What you own: counts, value, colors, mana curve |
| `stm collection import <csv> [--add]` | Import a ManaBox export |
| `stm collection query <text> [filters]` | Hybrid search over only the cards you own |
| `stm deck create / list / show` | Manage deck files |
| `stm deck update <name> --add/--remove/--set` | Edit a deck (see [Building decks](#building-decks)) |
| `stm deck legal <name> [--format FMT] [--bracket N]` | Check deck legality and Commander brackets |
| `stm deck import / export <name> <file>` | Move decks in and out of ManaBox format |
| `stm deck suggest <name> [query]` | Suggest role fills, theme cards, or combo completions, owned first; works in any format |
| `stm deck simulate <name> [--seed N]` | Play thousands of solitaire games to find consistency problems (see [Deck simulation](#deck-simulation)) |
| `stm deck combos <name> [--bracket B]` | Spellbook combo audit, split by section; flags bracket-breaking combos |
| `stm deck cuts <name> [--for ROLE] [--bracket N]` | Rank the deck's cards by expendability, with cut/fill pairing; pass `--bracket` so Game Changers over the cap pin to the top |
| `stm deck diff <A> <B> --markdown` | Exact change instructions between two lists (either side a deck name or a file) |
| `stm deck export <name> <file> --format names` | Plain `qty Name` export (no set decorations) |
| `stm deck primer <name> [--set <file>]` | Read or write a deck's strategy notes |

Every read command takes `--json` for machine-readable output — the
output is pure JSON, safe to pipe into `jq`. Every failure prints an
`error:` line plus a `hint:` line that says what to do next. Exit codes
follow a simple scheme: `0` success, `1` error, `2` bad usage, `3`
nothing found.

## Working with your collection

`stm` reads ManaBox's CSV export (the one with `Binder Name` and
`Binder Type` columns). Cards in binders are tracked by printing and
quantity; wishlist rows are ignored. Deck rows turn into deck files too:

```sh
stm collection import ~/Downloads/collection.csv
```

Re-importing replaces the collection (that's the safe default); pass
`--add` to merge instead. Once imported:

```sh
stm collection                              # overview with value and stats
stm collection query "board wipe" --color BR --json   # search only what you own
```

Prices update daily from Scryfall, so the value figures track the market.

## Building decks

A deck is a plain text file (`decks/<name>.txt`) in ManaBox's format, plus
an optional primer markdown with your strategy notes. You can edit both by
hand or keep editing them in ManaBox and import back and forth:

```sh
stm deck create Froggy
stm deck update Froggy --add "1 Blasphemous Act"
stm deck show Froggy
stm deck export Froggy ~/Downloads/froggy.txt   # import straight into ManaBox
```

Update specs look like `[section:]qty Name [(SET) [cn]] [*F*]`. `--add`
adds copies, `--remove` takes them away, `--set` pins an exact count
(`--set "0 X"` deletes the line). Typo protection included: names are
checked against the real card database, and a miss fails with "did you
mean" suggestions instead of silently writing garbage.

`stm deck show` prices the whole deck. Each card line shows its price,
and the overview shows what your copies are worth versus what buying the
rest would cost. Basic lands (Plains, Island, and friends) are treated as
free and unlimited; you never buy or count them.

### Deck legality

```sh
stm deck legal Froggy                       # guesses the format from the deck
stm deck legal Froggy --format commander --bracket 3
```

The command checks everything that can be checked mechanically: deck size,
copy limits, commander rules and color identity, per-card format legality,
and the Game Changer allowance for Commander brackets 1–5. It exits `0`
when the deck is legal and `1` when it isn't, and it tells you exactly
which cards broke which rule. The judgment-call parts of brackets (tutor
counts, combo speed, mass land destruction) come back as a short checklist
to review rather than a guess.

### Filling holes in a deck

```sh
stm deck suggest Froggy --role ramp        # fill a role: draw, ramp, board-wipe, sacrifice, ...
stm deck suggest Froggy "frog payoff"      # or semantic search, owned first
stm deck suggest Froggy --bracket 2        # stay inside a Commander bracket
stm deck suggest Froggy --format modern    # filter to one format
```

Suggestions rank by how well a card fits the deck's theme and roles, put
cards you own first, and show the cheapest print with a price. The
suggester also knows Commander Spellbook combos in any format: when a card
you don't own would complete a combo the deck is one card away from, it
says which combo. Commander decks draw on commander combos; decks for
other formats drop the combos that need a commander.

### Deck simulation

```sh
stm deck simulate Froggy --runs 10000 --seed 42
stm deck simulate Froggy --bracket 3      # sets the mana-base target band
```

The report includes a `mana_base` verdict: the deck's lands/rocks/ramp
counts against research-derived target bands for the bracket ("trim 3
lands", "add 2 ramp", "on target"). Flood counts lands *seen* (opener +
draws) against the exact hypergeometric expectation, so a land-heavy deck
gets told to trim where a drops-made check would say nothing.

The simulator plays thousands of solitaire games ("goldfish") with the
deck and reports how consistently it does its job: does it hit its land
drops, cast the commander on curve, see removal by turn 5, feed its draw
engines, and find win conditions by turn 8? It is a consistency
diagnostic, not a win-rate predictor — it never plays against an
opponent.

The report turns weak points into named findings:

```text
$ stm deck simulate Froggy --runs 300 --seed 42
Froggy  300 games · 10 turns  100 cards · 37 lands · 13 ramp · avg CMC 3.1

  ████······  opening hand  2.9 lands avg · mulligan 4.0%
  ████████··  land drops by t4  74.3% hit all 4 · screw 12.0% · flood 0.0%
  ███████···  commander (CMC 5)  69.7% by turn 5 · p50 t5 · p95 t8
  █·········  station online  8.3% by t6 · p50 t8
  ██········  mana thru t6  1.2 avg unspent · 15.1 cards seen · 2.6 bodies
  ██████····  removal seen by t5  57.3% of games
  ████████··  draw source by t6  84.7% of games · 15.3% starved

Slow to cast (worst 3)
    Alibou, Ancient Witness  CMC 5  cast by t5 in 29%
    ...

Problems
  error: color_screw: W mana pips missed in 19.0% of games ...
    → swap basics for lands that also tap for W
  error: dead_cards: 5 cards cast on time under 55%; worst: ...
    → cut or discount late cards, or add ramp

note: solitaire sim: no opponents, no counters; enters-tapped honored
```

Each finding comes with a suggested fix. Problems never crash the
command: exit code `1` simply means "findings exist", so a script can
gate on it; `--json` gives the machine-readable report.

The `--seed` flag makes runs reproducible. This is the workflow for
verifying a deck edit: simulate with a seed, edit the deck, re-simulate
with the same seed, and diff. Identical seeds shuffle identically, so
every difference in the report comes from your edit:

```sh
stm deck simulate Froggy --seed 42 --json > /tmp/before.json
# ...edit the deck...
stm deck simulate Froggy --seed 42 --json --baseline /tmp/before.json
```

Two more lenses on the same simulation:

- `--hypgeo` prints the exact hypergeometric probability of each card
  being castable on curve — pure draw math, no simulation — beside the
  simulated number. The two answer different questions and disagreeing
  slightly is expected.
- Commander decks also get a combo section: it checks every relevant
  [Commander Spellbook](https://commanderspellbook.com) combo, reports
  which ones the deck can fully assemble by which turn, and lists
  near-misses (all pieces but one in hand).

The [simulator doc](docs/simulator.md) explains the model: how cards are
read, how a turn runs, what every metric means, and where the model stops
being true.

## Finding cards

Three ways to search, depending on what you know:

- **`stm query`** describes what you want: `stm query "graveyard
  recursion" --color BG`.
- **`stm collection query`** does the same but only over cards you own.
- **`stm card similar`** starts from a card you like and finds cards that
  share its community-assigned role labels and read like it (both signals
  mix into one ranking). Handy for "what plays like
  Cyclonic Rift?" or for finding cheaper versions of an expensive card.
  Add `--owned` to stay inside your collection.
- **`stm card combos`** lists known combos with a card: `stm card combos
  "Demonic Consultation"` names the pieces, what each combo does, and
  where it's legal. Pass `--format modern` to keep only combos that work
  in that format; combos that need a commander are left out of 60-card
  results automatically.

All three accept filters that combine: `--type Creature --color WU
--cmc '<=3' --rarity rare --format commander` and friends. Card detail
(`stm card "Lightning Bolt"`) shows rules text, current price, which
formats allow it, and the role labels Scryfall's tagging community gives
it.

## How it works

`stm` stores everything in one folder (`~/.seizethemana/`, override with
`--data-dir`). Setup builds three things there:

1. **A SQLite database** with one row per card (rules text, prices, format
   legality) plus the parsed Commander Spellbook combo data and your
   collection.
2. **A search index** with two halves. A keyword index scores exact word
   matches against rules text, type lines, and tags. A semantic index
      embeds every card's text with a small on-device model into a vector
   table — this is the 2–3 minutes of first-run CPU.
3. **Deck files** on disk, plain ManaBox txt you can also edit by hand.

Search mixes both halves: keyword hits score by exact match, semantic hits
score by vector distance, and reciprocal rank fusion merges the two lists
into the score you see (a card ranked high on both legs wins). Nothing
calls an API at search time.

`stm sync` (or any read command, once a day) re-downloads the daily bulk
files and updates prices, tags, combos, and re-embeds any card whose text
changed. The [architecture doc](docs/architecture.md) has the full
picture: schema, index layout, and the sync pipeline.

## Docs

| Document | What it covers |
| --- | --- |
| [`docs/architecture.md`](docs/architecture.md) | Data schema, search index, sync pipeline, deck module |
| [`docs/simulator.md`](docs/simulator.md) | The goldfish simulator: card model, turn pipeline, metrics, limits |
| [`docs/design.md`](docs/design.md) | CLI design: output style, human vs agent-friendly modes |

## Data sources

Card data, prices, and tags come from
[Scryfall](https://scryfall.com/docs/api/bulk-data) daily bulk exports,
role labels come from Scryfall's Tagger community project, and combo data
comes from the [Commander Spellbook](https://commanderspellbook.com)
project. This project is unofficial Fan Content permitted under the
Wizards of the Coast Fan Content Policy. Not produced or endorsed by
Scryfall, Commander Spellbook, or Wizards of the Coast.

## Development

```sh
cargo nextest run            # tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the review bar and conventions.
Notable changes are tracked in [CHANGELOG.md](CHANGELOG.md).

## AI disclosure

This codebase is written with AI agents and human review. See
[AI_DISCLOSURE.md](AI_DISCLOSURE.md) for details.

## License

MIT. See [LICENSE](LICENSE).