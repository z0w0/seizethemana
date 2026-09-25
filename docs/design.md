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
`(own 0/4)`. Basic lands show `(own ∞)` and a unit price
(`$1.25 USD`) follows each card line in `deck show`. Money renders as
`$X.XX USD` everywhere in human output.

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
stm card combos <name> [--format FMT] [--limit N] [--json]
stm query <QUERY> [filters] [--limit N] [--json]

stm collection [--json]                           # stats/overview
stm collection import <file> [--add] [--force]    # replace default; --add merges
stm collection query <QUERY> [filters] [--binder NAME]... [--deck NAME]... [--json]
stm collection conflicts [--json]                 # cards wanted by more decks than owned copies

stm deck create <name>
stm deck list [--json]                            # decklists + unimported collection decks
stm deck show <name> [--json]                     # `stm deck <name>` sugar
stm deck hand <name> [--seed S] [--count N] [--json]   # sample opening hands (sim's deal rules)
stm deck update <name> [--add SPEC]... [--remove SPEC]... [--set SPEC]... [--move SPEC]...
            [--from FILE] [--allow-partial] [--dry-run [--sim]] [--legal]
            [--backfill-basics] [--json]
stm deck dedupe <name> [--json]                   # merge duplicate same-name lines
stm deck suggest <name> [--query TEXT] [--role ROLE] [--commander] [--format FMT]
            [--owned] [--exclude NAME|FILE]... [--max-price USD] [--limit N] [--json]
stm deck legal <name> [--format FMT] [--bracket 1-5] [--json]
stm deck simulate <name> [--runs N] [--turns N] [--seed S] [--format FMT] [--baseline FILE] [--bracket 1-5] [--json]
stm deck combos <name> [--format FMT] [--bracket 1-5] [--json]   # Spellbook combo audit, per section
stm deck cuts <name> [--count N] [--for ROLE] [--bracket 1-5] [--json]   # ranked expendability + cut/fill pairing
stm deck diff <A> <B-or-file> [--exact] [--json] [--markdown] [--as-update]   # change instructions
stm deck import <name> [file] [--url URL] [--format FMT]  # upsert the decklist (file or --url, auto-detect or pinned format)
stm deck delete <name>                            # decklist only; ownership is kept
stm deck mana <name> [--json]                     # mana-base audit against the bracket band
stm deck copy <name> <new-name>
stm deck export <name> <file> [--force] [--format manabox|names|moxfield|archidekt|arena]
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
- `--limit` defaults to 20, capped at 100, except `deck suggest` (default
  10, capped at 50 — suggestion rows are long) and `deck cuts --count`
  (default 5, capped at 50).
- `query` and `collection query` are hybrid: SQLite full-text (BM25)
  and vector (semantic) legs, fused by reciprocal rank fusion. The
  printed and JSON `score` is the fused score normalized to 0–1, not raw
  cosine. Player shorthand expands before retrieval: trigger phrases
  ("mana ramp", "board wipe", "sacrifice outlet", ~150 entries) append
  Tagger tag vocabulary + oracle keywords to both legs, capped so one
  broad trigger cannot flood the BM25 leg. The full-text index covers
  name, Tagger tag labels, type line, and oracle text; BM25 ties break
  toward lower EDHREC rank.
- `--json` works on every read command. `--data-dir/--no-color/
  --offline/--verbose` are global.
- When card data or prices are more than 24h old, the command refreshes
  before it runs (synchronous: a stale store makes the read block for a
  full sync); `--offline` skips that.
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
  accidental duplicates. Cards whose oracle text allows any number of
  copies ("a deck can have any number of cards named ...") are exempt
  from the singleton guard, the warning, and the dedupe collapse, exactly
  as `deck legal` exempts them. `--backfill-basics` pads to the deck's
  format size (100 commander / 60 brawl / 59 oathbreaker / 60 others)
  with the commander's first identity color. `--from FILE` reads batch
  specs, one op per line.
- `stm deck suggest` fills roles or finds theme cards: semantic search +
  Scryfall Tagger labels + role keyword scan, fused by reciprocal rank
  fusion (EDHREC breaks ties), grouped owned cards first with each group
  in fit order, annotated with ownership (copy count), price, and Game
  Changer flags. `--role` takes a role name; ~130 aliases resolve to 42
  structured roles (draw, ramp, removal, board-wipe, sacrifice, voltron,
  spellslinger, typal, group-hug, ...; `card-advantage`/`cardadv` alias
  to draw). `--owned` trims to owned cards before the limit cut, so an
  owned card below `--limit` still
  surfaces.
  `--format <fmt>` pins the legality filter; commander-shaped decks default
  to the commander's color identity and commander legality, other decks
  accept any format. `--commander` swaps the pool to
  commander-legal legendaries ranked by theme fit. Exit 3 on no matches.
- `stm card combos <name>` lists Commander Spellbook combos with the card,
  sorted by popularity. `--format <fmt>` keeps combos legal in that format;
  commander-only combos (a piece must be the commander) are excluded
  automatically for 60-card formats. Human view: `A + B (commander) →
  produces [bracket] pop N legal: …`. Exit 3 when the card appears in no
  combo.
- `stm deck update --dry-run` validates the ops exactly like a real
  update (oracle names, singleton guard) and applies them to a copy in
  memory only. Human output prints three blocks: the change list
  (`-`/`+`/`~` rows), the cost impact (new to-buy slots at the cheapest
  printing, money freed by removals, net spend), and with `--sim` a
  same-seed before/after consistency delta (shape counts, metric lines,
  problems `+` new / `-` resolved; exit 1 only on a new problem). Nothing
  is written; the closing note says to re-run without `--dry-run` to
  apply. `--json` carries `{name, dry_run, changes, cost, sim}` where
  `cost` is `{to_buy: {items: [{name, quantity, price_usd,
  owned}], total_usd}, freed_usd, net_usd}` and `sim` is the
  `ReportDiff` (null without `--sim`). `--json` also works on real
  updates (the one-line summary as JSON).
- `stm collection conflicts` compares every decklist's slot demand
  against owned copies: rows only where demand exceeds supply (basics
  excluded), each with the competing decks and the cheapest-printing cost
  to close the gap. Informational: always exit 0. Decks registered in the
  collection but missing a decklist print once as "unverified".
- `stm deck hand <name> [--seed S] [--count N]` deals sample opening
  hands with the simulator's shuffle + mulligan rules: seed N's first
  hand is sim game #1's opener, so advice and sim runs never disagree
  (`--count` default 3, max 10).
  Each hand prints with a keep/mull sentence (land count vs the keep
  band, early plays); `--json` wraps `{seed, hands}`.
- `stm deck simulate` runs Monte Carlo goldfish games (default 10,000 —
  ±0.5pp on percentages) and prints an overview block of aggregates, the
  worst-3 slow-to-cast cards, and `error:` problem lines with a category +
  magnitude suggestion (`→ add 2-3 draw engines`). Exit 1 when problems
  were found (the result, not a crash); `--seed` makes runs reproducible
  so agents can diff a deck edit's effect. `--baseline prior.json` prints
  deltas only — shape counts, metric lines, problems (`+` new / `-`
  resolved) — and exits 1 only when a problem is new. With `--json` the
  same baseline prints the delta object (`metrics`, `shape`, `problems`)
  instead of the full report. Without a baseline, `--json` is the full
  detail:
  opening-hand distribution, land-drop curve + percentiles, commander
  timing (with the full pip check for multicolor commanders), station
  online metrics, bodies and engines per turn, unspent mana, velocity +
  library awareness + mill census (self and opponent direction) +
  library remaining, combat block (attack power per turn and p90,
  attackers, evasion census), wincons block (drain per turn, extra-turn
  share, win-threshold engines, planeswalker ultimate online), the
  interaction block (readiness by turn, mana held, instant-speed copy
  count; explicitly "capacity, not events"), role access, per-card
  castability, a static `color_sources` census of land tap yields per
  color, `combo_access` + store-backed `combos`, a `win_paths` section
  (complete combos whose Spellbook `produces` labels contain a win
  feature — "Win the game", "Infinite damage", "Infinite turns", …),
  and the problems array, plus the `mana_base` block: the deck's
  lands/rocks/dorks/ramp counts against the bracket target bands
  (`--bracket 1-5`, default 3) with a verdict sentence ("trim 3 lands",
  "add 2 ramp", "on target"). The flood metric counts lands *seen* at end of turn 4
  (opener + draws + cantrips + self-mill) against the hypergeometric expectation at
  that same draw volume, so a land-heavy deck reports real flood where a
  drops-made detector reads zero, and a cantrip deck is not punished for
  seeing more cards. Model limits ship in the JSON
  `assumptions` array.
  Human output adds lines for interaction readiness, attack power,
  drain, extra turns, and threshold/ultimate online when non-zero, and
  a "Win paths" block when a store-backed win path assembles.
- `stm deck combos <name>` joins the deck against the Spellbook store
  per section (COMMANDER / DECK / SIDEBOARD): complete combos,
  one-card-away near misses (with the missing piece), and — with
  `--bracket B` — a `bracket_breaks` list of main-deck combos whose
  Spellbook tag exceeds the bracket. No simulation runs; it is a static
  join.
- `stm deck cuts <name>` ranks the deck's incumbents by expendability
  (banned cards and Game Changers over the bracket allowance pin to the
  top regardless of score; then sim castability faults, curve outliers,
  pricey one-offs). Rows carry `rank` and `pinned`. Basics and the
  commander are never suggested. `--for <role>` pairs every cut with
  fill candidates for the deficit role (owned first, always inside the
  commander's color identity and legal in the deck's format) and
  discounts incumbents already serving that role.
  `--make-room-for sideboard|maybeboard` pairs each bench card with
  the maindeck cut that makes room for it: off-identity or
  format-illegal bench cards are skipped, and pairings prefer
  same-role cuts so the swap keeps the deck's shape. JSON rows are
  `{bench_card, cut_candidate, reasons, score}`.
- `stm deck diff <A> <B>` prints per-section change instructions
  (removed / added / quantity changed; basics and quantity shifts
  collapse to `Name: N → M` rows). `--as-update` emits `deck update
  --from` spec lines instead (print suffixes stripped so the ops parse). `--markdown` renders the change-log
  instruction table for `decks/<name>.changes.md` (basics read
  "Remove 4 Forests and 2 Islands"); `--exact` diffs by print identity
  instead of card name. Both operands are a deck name or a ManaBox txt
  path.
- `stm deck export --format names` writes plain `qty Name` lines (no
  set/collector-number decorations) — the shim-free feed for external
  tools and diffs. `manabox` (default) stays the ManaBox-compatible
  format. `moxfield` (`2x Name (set) cn *F*`), `archidekt`
  (`1x Name [set] cn *F*`), and `arena` (bare-word headers) render the
  other sites' txt shapes; unknown prints export bare in every format.
- `stm deck import` reads every format above plus `names`, auto-detecting
  the shape unless `--format` pins one. `--url <URL>` fetches an
  Archidekt deck instead of reading a file (the only deck host whose API
  serves unauthenticated JSON to a non-browser agent; Scryfall has no
  public deck API and Moxfield blocks bots). Requests send a
  `seizethemana/<version>` user agent. Fetched decks render to ManaBox
  txt first, so the commander-inference + primer + ownership pipeline
  runs unchanged. Moxfield and other sites still import fine as exported
  txt files. Network errors name the host and carry a hint; `--offline`
  refuses `--url` up front with a usage error.
- `stm card similar` ranks cards by reciprocal-rank fusion of shared
  Tagger oracle tags and stored-vector cosine to the seed (tags-only with
  a note when the seed has no stored vector; `score` is `null` then).
  Ties break toward lower EDHREC rank then name; `--owned` limits to the
  collection. `--json` emits the full card object per hit (as
  `card <name> --json`) plus `score`, `shared_count`, and `shared_tags`.
- `stm card <name> --json` includes `oracle_id`, `tags` (alphabetical
  Tagger labels) and `game_changer`. `query --json` emits the same full
  card object plus `score`; empty results print `[]` (exit 3 still
  signals no results). `stm card similar <name> --json` adds `shared_count` and
  `shared_tags` per hit. `stm card combos <name> --json` adds `produces`,
  `bracket_tag`, `popularity`, `legalities`, `requires_commander`, and
  `pieces` (with `zones` and `must_be_commander`) per combo.

## Examples

Human view (TTY):

```
$ stm query "sacrifice a creature to draw cards" --limit 3
 1. Greater Good {2}{G}{G} rare Enchantment (0.876)
 2. Life's Legacy {1}{G} rare Sorcery (0.500)
 3. Griselbrand {4}{B}{B}{B}{B} mythic Legendary Creature — Demon (0.484)
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
- The simulator stays solitaire (goldfish). It never models opponents,
  blockers, or interaction resolving — findings measure capacity, not
   events.
- No jargon in user-facing sim strings. Findings say what they mean in
  plain English; the machine-readable `kind` values are the only place
  the shorthand lives.