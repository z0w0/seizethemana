# CLI design

How `stm` talks to you: the commands, what the output looks like, and how
one binary serves both a person in a terminal and an agent in a script.

## Principles

1. **Cargo-style output.** One short line per action — bold green verb, dim
   metadata, red errors on stderr. Progress chatter on stderr, results on
   stdout, finished lines that stay in the scrollback. If you have used
   `cargo build`, this looks familiar on purpose.
2. **Friendly by default.** No IDs, no raw JSON, no flags needed for the
   common path. Messages read like sentences. Every error says what to do
   next on a `hint:` line.
3. **Agent-friendly by construction.** The same binary serves a script with
   zero changes to what it prints:
   - stdout is **results only** — safe to grep and paste. Everything else
     goes to stderr.
   - Piped output drops ANSI color automatically. `NO_COLOR` and
     `--no-color` force it off too.
   - `--json` on every read command prints pretty JSON with stable
     snake_case names and nothing else.
   - Exit codes carry meaning: `0` ok, `1` error, `2` usage, `3` no results
     or card not found. An agent can drive the whole tool without parsing
     any output.
4. **Nothing silent, nothing destructive by default.** Overwrites need
   `--force`/`--replace`. Imports replace by default and say so; `--add`
   merges. Every change prints a one-line summary of what it did. `deck
   update` validates `--add`/`--set` names against the oracle so a typo
   fails fast (exit 3, "did you mean" hints) instead of silently writing a
   card that does not exist.

## Output style

| Element | Treatment |
| --- | --- |
| Status line | `   Downloading bulk data` — bold green verb, padded to 12 chars, like `cargo`'s `   Compiling serde` |
| Finished line | `   Finished setup in 170.27s` — same shape, plus a duration |
| Progress | Spinner/bar on stderr, cleared when done; plain lines when stderr is piped |
| Errors | `error: ...` in red on stderr (anyhow context chain), then `hint: ...` in dim |

Card lists get domain styling: card names bold cyan, mana pips colored per
W/U/B/R/G/C, set codes and counts dim, rarity colored (mythic magenta, rare
yellow, uncommon cyan), deck shortfalls yellow `(own 2/4)` or red
`(own 0/4)`. Basic lands show `(basics unlimited)` and a unit price
(`@$1.25`) follows each card line in `deck show`.

Histograms get a shared dim bar glyph (`██····`): collection curve/colors/
rarity, deck curve/ramp/colors/types, and deck completion all use it. The
`stm card` human view renders a box-drawing frame sized to the terminal:
name + mana cost header, type + rarity, wrapped oracle text, then a footer
with set, collector number, release date, EDHREC rank, USD prices, and
legal formats. `stm deck show` puts an overview block under the header
(completion %, nonland curve + avg CMC, ramp sources, color identity, type
breakdown, a `Value` row: owned value + missing cost, and a `To buy` block
of the missing slots sorted by cost with a running total). `stm deck legal`
prints the verdict (legal / N violations, each as `error: <rule>: <detail>`
with the offending card names indented below) plus a
`note: not checked automatically:` checklist for the bracket, upgraded to
`bracket checks:` with `✓`/`!` verdicts when `--bracket` is given. `note:`
on stdout is dim informational text (the collection-import deck hint);
suppressed in `--json`.
All of it collapses to plain text when piped or `--no-color`;
`--json` output never changes.

## Command surface

```
stm [--data-dir DIR] [--no-color] [--offline] [--verbose] <COMMAND>

stm setup [--force]                               # card data + prices + tags; --force rebuilds the DB
stm sync [--force]                                # card data + prices + tags, in-bulk
stm card <name> [--json]                          # `stm card show <name>` sugar
stm card similar <name> [--limit N] [--owned] [--json]
stm query <QUERY> [filters] [--limit N] [--json]

stm collection [--json]                           # stats/overview
stm collection import <file> [--add] [--force]    # replace default; --add merges
stm collection query <QUERY> [filters] [--binder NAME]... [--deck NAME]... [--json]

stm deck create <name>
stm deck list [--json]                            # decklists + unimported collection decks
stm deck show <name> [--json]                     # `stm deck <name>` sugar
stm deck update <name> [--add SPEC]... [--remove SPEC]... [--set SPEC]... [--from FILE]
stm deck dedupe <name> [--json]                   # merge duplicate same-name lines
stm deck suggest <name> [--query TEXT] [--role ROLE] [--commander] [--limit N] [--json]
stm deck legal <name> [--format FMT] [--bracket 1-5] [--json]
stm deck simulate <name> [--runs N] [--turns N] [--seed S] [--format FMT] [--baseline FILE] [--json]
stm deck import <name> <file>                     # upsert the decklist from ManaBox deck txt
stm deck delete <name>                            # decklist only; ownership is kept
stm deck export <name> <file> [--force]
stm deck buylist <name> [--store generic|cardkingdom|tcgplayer] [--json]
stm deck primer <name> [--set FILE]               # no --set prints the primer markdown
```

Conventions:

- Uniform `<noun> <verb>` ordering. Bare nouns (`stm collection`,
  `stm deck`) show the overview.
- The same filter flags exist on both `query` and `collection query`
  (`--type --color --color-identity --cmc --power --toughness --rarity
  --set --keyword --oracle-text --format`); numeric filters take comparison
  operators (`<=`, `<`, `=`, `>`, `>=`).
- `--limit` defaults to 20, capped at 100.
- `query` and `collection query` are hybrid: SQLite full-text (BM25) and
  vector (semantic) legs, fused by reciprocal rank fusion. The printed and
  JSON `score` is the fused score normalized to 0–1, not raw cosine.
- `--json` works on every read command. `--data-dir/--no-color/
  --offline/--verbose` are global.
- When card data or prices are more than 24h old, the command quietly
  refreshes first; `--offline` skips that.
- Two import paths, two concepts. `stm collection import` is the only
  collection-CSV path and writes ownership only; it never touches deck
  files and points at the decklist step with a `note:`. `stm deck import`
  is the only decklist path (ManaBox deck txt export, upsert by name) and
  never touches ownership. The deck name joins the two.
- `stm deck legal` reuses exit 1 for "not legal" (its failure is the
  result, not a crash); JSON still prints the full report. Without
  `--format` the format is inferred from the deck's sections and the
  output says it was assumed. With `--bracket` the human view shows a
  `bracket checks:` block: `✓`/`!` verdict lines from an oracle-text scan
  (tutors, extra turns, mass land destruction, "you win the game"
  effects) naming the offenders.
- `stm deck update` warns (not fails) when an op would push a non-basic
  card past one copy in a commander-shaped deck; `stm deck dedupe` merges
  accidental duplicates. `--from FILE` reads batch specs, one op per line.
- `stm deck suggest` fills roles or finds theme cards: semantic search +
  Scryfall Tagger labels + role keyword scan, filtered to the commander's
  color identity, ranked by EDHREC playability, annotated with ownership,
  price, and Game Changer flags. `--commander` swaps the pool to
  commander-legal legendaries ranked by theme fit. Exit 3 on no matches.
- `stm deck simulate` runs Monte Carlo goldfish games (default 10,000 —
  ±0.5pp on percentages) and prints an overview block of aggregates, the
  worst-3 slow-to-cast cards, and `error:` problem lines with a category +
  magnitude suggestion (`→ add 2-3 draw engines`). Exit 1 when problems
  were found (the result, not a crash); `--seed` makes runs reproducible
  so agents can diff a deck edit's effect. `--baseline prior.json` (human
  output) prints deltas only — shape counts, metric lines, problems
  (`+` new / `-` resolved) — and exits 1 only when a problem is new.
  `--json` is the full detail:
  opening-hand distribution, land-drop curve + percentiles, commander
  timing (with the full pip check for multicolor commanders), station
  online metrics, bodies and engines per turn, unspent mana, velocity,
  role access, per-card castability, a static `color_sources` census of
  land tap yields per color, and the problems array. Model limits
  ship in the JSON `assumptions` array.
- `stm card similar` ranks cards by shared Tagger oracle tags. Ties break
  toward lower EDHREC rank then name; `--owned` limits to the collection.
- `stm card <name> --json` includes `tags` (alphabetical Tagger labels) and
  `game_changer`. `stm card similar <name> --json` adds `shared_count` and
  `shared_tags` per hit.

## Examples

Human view (TTY):

```
$ stm query "sacrifice a creature to draw cards" --limit 3
 1. Phyrexian Vault {3} Artifact (0.842)
 2. Carnage Altar {2} Artifact (0.841)
 3. Greater Good {2}{G}{G} Enchantment (0.839)
```

Agent view (piped):

```
$ stm query "bolt" --type Instant --limit 2 --json | jq -r '.[0].name'
Feedback Bolt
```

Errors always carry a next step:

```
$ stm query "bolt"
error: card index not built yet
hint: run 'stm setup' first
```

## What we will not do

- No interactive prompts in non-TTY contexts. Agents would hang.
- No color when output is piped or `--json` is set, even without
  `--no-color`. ANSI in parsed output is a bug.
- No extra verbosity in `--json`. The JSON is the contract; text is the
  decoration.