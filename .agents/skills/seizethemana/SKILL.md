---
name: seizethemana
description: Use when building MTG decks, improving existing decks or precons, searching cards, managing a collection/binders, or working with ManaBox exports — covers the stm CLI (seizethemana) for semantic card search, collection stats/valuation, deck files, legality checks, and primers. Triggers on Magic, MTG, Scryfall, ManaBox, deckbuilding, commander, bracket, binder, budget.
---

# `stm` — MTG card search, collection, and decks

`stm` (seizethemana) semantically searches every MTG card, manages a
binder/deck collection with live USD prices, and reads/writes ManaBox
formats. Results land on stdout as JSON; exit codes carry the control flow.
Parse nothing you don't have to.

Read Part 1 once for the tooling contract. Part 2 is the deckbuilding
workflow; follow it step by step whenever the user asks for a new deck or
wants an existing deck improved.

## Part 1 — Tooling reference

### Contract (read first)

- **stdout = results only.** Progress goes to stderr. `--json` on every
  read command prints stable snake_case JSON and nothing else.
- **Prefer `--json | jq` when `jq` is available.** JSON plus jq is the
  most reliable read path for scripts and agents: no prose to parse, no
  ANSI, stable keys. Check `command -v jq` once; if present, wrap every
  programmatic read. Common shapes:

  ```sh
  # Names only
  stm query "counterspell" --limit 10 --json | jq -r '.[].name'
  # Cheapest price of the top hit
  stm card "Sol Ring" --json | jq -r '.price_usd'
  # Deck lines you still need to buy (owned < quantity)
  stm deck show Froggy --json | jq -r '
    .sections[].cards[] | select(.owned < .quantity) |
    "\(.quantity - .owned)x \(.name) @ \(.price_usd // "unpriced")"'
  # Top hits sorted by score, name + score + price as TSV
  stm query "sacrifice outlet" --limit 10 --json | jq -r '
    sort_by(-.score) | .[] | "\(.score)\t\(.name)\t\(.price_usd)"'
  # Deck totals without parsing text
  stm deck show Froggy --json | jq -r '"\(.cards) cards, missing $\(.missing_cost)"'
  ```
- **Exit codes**: `0` success, `1` runtime error (or "deck is not legal"),
  `2` usage error, `3` no results / card not found. Use them for control
  flow. Read stderr only when a command fails (errors print `error: ...`
  plus `hint: ...`).
- Piped output has no ANSI codes. Pass `NO_COLOR=1` or `--no-color` to be
  extra safe. `--offline` skips the automatic data refresh.
- Default data dir `~/.seizethemana/`; override with `--data-dir <DIR>`.
- Card **names** resolve by exact match, case-insensitive match, or unique
  prefix. "lightn" resolves if unambiguous; an ambiguous prefix exits 3 and
  lists the candidates. Names resolve by *prefix*, not substring: "Bolt"
  does not match "Lightning Bolt".
- Unreleased cards never appear: `sync` does not ingest an oracle card
  until its set has released (newly released cards arrive on the daily
  sync). Upcoming reprints of released cards stay visible; their
  `released_at` reflects the newest printing.

### Prerequisites (do this first)

Before any `stm` command, make sure the CLI exists and the index is built:

1. Check for the binary:

```sh
command -v stm
```

2. If missing (or outdated), install or upgrade the latest version. Try in
   order until one succeeds:

```sh
cargo binstall -y seizethemana        # prebuilt binary, fastest
cargo install --locked seizethemana   # source build, ~1-2 min
```

If neither is available, download a prebuilt archive from
https://github.com/z0w0/seizethemana/releases for the host platform and put
the `seizethemana` binary on `$PATH`.

3. If `stm` says "card index not built yet" (or errors with exit 1 on read
   commands), run the one-time setup:

```sh
stm setup    # ~25MB download + 2-3 min local CPU embedding
```

Data refreshes on its own when older than 24h; `stm sync [--force]` forces
it. `--offline` skips the refresh check. The refresh also pulls Commander
Spellbook combo variants (~28 MB, feeds `deck simulate` combo assembly and
bare `deck suggest` completions); a failed combo download warns and
continues.
`stm setup --force` rebuilds from
scratch (use when `stm` itself was upgraded, or the store looks corrupt).

### Find cards (whole oracle)

```sh
stm query "sacrifice a creature to draw cards" --limit 10
stm query "graveyard recursion" --color BG --format commander --cmc '<=4' --json
```

Hybrid search: keyword (BM25 full-text) + meaning (vector) legs, fused by
reciprocal rank fusion; the `score` is 0–1. Meaning wins on intent
("sacrifice outlet", "wrath effect"), exact words win on keywords
("bolt"); filters pin the mechanics. Filters (AND-combined, shared by
`query` and `collection query`): `--type` `--color` (subset of WUBRG)
`--color-identity` `--cmc` `--power` `--toughness` (comparisons
`<= < = > >=`) `--rarity` `--set` `--keyword` `--oracle-text` `--format`.
`--limit` default 20, cap 100.

Card detail (legalities, EDHREC rank, Game Changer flag, release date,
price, Tagger role labels):

```sh
stm card "Lightning Bolt" --json
```

`card --json` includes `game_changer` (Commander Game Changer list),
`tags` (community role labels: "removal", "sacrifice outlet", "wheel"),
and `legalities` (use the exact format key, e.g. `legalities.commander`).

### Find similar cards (tag overlap)

```sh
stm card similar "Cyclonic Rift" --limit 10
stm card similar "Smothering Tithe" --owned --json
```

Ranks cards by reciprocal-rank fusion of shared oracle tags and stored-
vector meaning to the seed card: the tool for
"what plays like X". `--owned` restricts to the collection (everything you
own, including cards assigned to decks — this is an ownership lookup, not
an available-pool search). JSON per hit:
full card fields + `score` (0–1, like `query`) + `shared_count` +
`shared_tags`. Use this when a known
exemplar exists; use `stm query` when describing an intent with no
exemplar. **Shared-tag count measures role overlap, not power.** Always
cross-check `edhrec_rank` (lower is more played) and `price_usd` in the JSON
before proposing a hit; prefer hits with better ranks than the seed.

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
combo completions for a deck. It works for any format — commander decks
filter to commander; other decks drop commander-required variants (or pin
a format with `--format`).

### Work with the collection

Import a ManaBox CSV export (replaces by default; `--add` merges):

```sh
stm collection import ~/path/collection.csv
```

**The collection CSV is ownership only.** `Binder Type=deck` rows become
deck-assignment rows in the collection (what you own and where), not
decklists. Decklists are separate files (`decks/<name>.txt`) and are never
written by a collection import. After the import, a `note:` line names the
decks it now tracks and reminds you to import each decklist separately
(`stm deck import <name> <manabox-deck.txt>` — a ManaBox deck screen's
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
Cards assigned to decks are hidden by default — they are already spoken
for. Deckbuilding searches should target unassigned cards first; when a
deck's own cards are wanted, pass `--deck <name>` to include them
(`--binder <name>` narrows the binder pool; `--binder` + `--deck` union).
Hit `locations` only lists locations within the queried scope.

`collection query` JSON per hit: full card fields plus `score`, `owned`,
and `locations: [{binder, type, quantity}]`. Use `owned`/`locations`
directly instead of re-querying.

### Build and edit decks

Decks are ManaBox txt files (`decks/<name>.txt`: `// SECTION`,
`qty Name (SET) cn *F*`); the file is the source of truth. Each deck also
has a primer at `decks/<name>.primer.md` for strategy notes. Read the
primer from the CLI (`stm deck primer <name>`); write it with `--set` or
by editing the file.

```sh
stm deck list                        # decklists + collection decks with no list
stm deck show Stationz               # contents with (own N/M) per line + To-buy block
stm deck Stationz --json             # sugar; JSON sections + covered_by per line
stm deck update Froggy --add "1 Phyrexian Vault" --add "sideboard:2 Bolt"
stm deck update Froggy --remove "1 Bolt" --set "0 Breya"   # --set 0 deletes the line
stm deck update Froggy --move "1 Bolt to:sideboard"       # atomic deck→sideboard move
stm deck update Froggy --from /tmp/batch.txt --allow-partial  # batch; skips misses, exits 1 when anything missed
stm deck dedupe Froggy               # merge duplicate same-name lines (sums quantities)
stm deck import Froggy ~/Downloads/Froggy.txt   # upsert the decklist by name
stm deck delete Froggy               # remove the decklist; ownership is kept
stm deck export Froggy /tmp/out.txt --force
stm deck primer Froggy                       # print the primer markdown to stdout
stm deck primer Froggy --set /tmp/primer.md  # replace the primer from a file
stm deck buylist Froggy                      # what to buy: `2x Name` lines
stm deck buylist Froggy --store cardkingdom  # CK CSV (Name,Edition,Foil,Qty)
stm deck legal Froggy                          # format legality
stm deck legal Froggy --format commander --bracket 3
stm deck suggest Froggy --role draw --json     # role fills: owned first, then by fit
# --role names: draw, cantrip, wheel, discard, mill, ramp, mana-rock,
# mana-dork, land, removal, board-wipe, counterspell, theft, protection,
# hate, stax, sacrifice, reanimate, recursion, token, anthem, equipment,
# evasion, combat-trick, burn, lifegain, tutor, wincon, combo, storm,
# extra-turn, blink, landfall, artifact, planeswalker, voltron,
# spellslinger, typal, group-hug, politics, interaction, ...
stm deck suggest Froggy "frog payoff" --json   # semantic query for theme cards
stm deck suggest Froggy --commander --json     # commander candidates for the deck
stm deck suggest Froggy                        # combo completions: one card away from a Spellbook combo
stm deck suggest Froggy --bracket 2            # completions filtered to no Game Changers
```

**Two concepts, one name.** The *decklist* (txt file) and the *ownership*
(collection rows) are independent; the deck name joins them:

- `stm collection import` writes ownership only. `stm deck import` writes
  the decklist only. Neither touches the other.
- `stm deck import` upserts: it creates or overwrites the decklist by
  name. ManaBox's two export shapes are both handled: the commander(s)
  under `// COMMANDER` with a blank line before the rest of the deck
  (the blank splits the section into `COMMANDER` + `DECK`), and the older
  quirk of the whole deck under `// COMMANDER` (detected at more than 4
  entries and treated as the main deck). When no real COMMANDER section
  remains, a TTY import offers the legendary candidates to pick from
  (skip = decide later); piped or `--json` runs never prompt — they print
  a `note:` with candidates. `// SIDEBOARD` and other sections import as-is.
- `stm deck delete` removes only the decklist + primer. Ownership rows in
  the collection survive; `stm deck list` shows collection decks that have
  no decklist (`has_decklist: false` in JSON) so the gap is visible.
- `stm deck buylist` autofills owned copies (deck-assigned + binders)
  read-only and lists only what needs purchasing, at the cheapest
  printing. Nothing in the collection is moved.

Update specs: `[section:]qty Name [(SET) [cn]] [*F*]`. `--add` increments,
`--remove` decrements (line deleted at 0), `--set` pins exact. The `(SET)
cn` part is optional and descriptive only (which copy to sleeve); matching
is by card name, so any set version you own fills a deck line — keep
reprints in mind when pricing buys.

**Card names in `--add`/`--set` are validated against the oracle.** An
unknown name exits 3 naming it (with "did you mean" candidates when close);
fix the name and retry. Tokens are added with a warning, not an error.

**`deck update` takes `--add`, `--remove`, `--set`, `--move`, `--from`,
and `--allow-partial`.** Never invent other flags for it. Unreleased
cards cannot be referenced because the store never holds them; a card
shows up once its set releases.

**`--move` for section moves.** `stm deck update <name> --move
"[section:]qty Name to:section"` performs one atomic remove+add (the
singleton guard and missing-checks see the net state, so legal moves do
not warn). `to:` defaults to `deck`; a leading `sideboard:` picks the
source section. `--move "1 Bolt to:sideboard"` moves a card from the
maindeck; `--move "sideboard:1 Bolt"` moves it back.

**`--set 0` deletes.** On a sideboard-only card it deletes the
sideboard line (same fallback as `--remove`), with a note; a qualified
`sideboard:0 Name` stays strict.

**`--allow-partial` for `--from` batches.** By default a batch is
all-or-nothing (a missing card aborts everything). With
`--allow-partial`, resolvable ops apply, misses are reported, and the
exit code is 1 when anything was missed.

**Singleton guard (commander).** Commander-shaped decks (a COMMANDER
section) warn on `deck update` when an op would push a non-basic card past
one copy — the write still lands, the warning names the card. `stm deck
dedupe <name>` merges accidental duplicates (same-name lines sum per
section; first line's print info wins); exit 3 when there is nothing to
merge. After long build sessions run `dedupe` before `deck legal`.

**`--from` batch specs.** A spec file holds one op per line: `add 1 Name`,
`remove 1 Name`, `set 2 Name`, or a bare spec (treated as add). Blank
lines and `#` comments are skipped. Large batches become one call instead
of a `--add` flag list.

**Basic lands are unlimited.** Plains/Island/Swamp/Mountain/Forest/Snow
Lands/Wastes never count toward ownership or budget; `deck show` marks them
`(basics unlimited)` and JSON entries carry `basic_land: true`. Add the
counts the mana base needs without asking.

### Legality and brackets

```sh
stm deck legal <name>                          # infer format from sections
stm deck legal <name> --format commander       # explicit format
stm deck legal <name> --format commander --bracket 3
```

- Without `--format`, a `// COMMANDER` section means commander; otherwise
  only structural rules are checked (60-card maindeck minimum, sideboard
  ≤ 15, 4-copy limit). JSON marks this as `"format_assumed": true`.
- **Sideboard counting depends on the format.** For commander, a
  `// SIDEBOARD` section is extra deckbuilding advice (a wishlist), not a
  legal zone: it never counts toward the 100-card deck size, and the
  summary line shows it separately (`100 cards … + 3 sideboard`). For
  60-card formats the sideboard is real and subtracted from the maindeck
  count (maindeck ≥ 60, sideboard ≤ 15).
- Deterministic checks: deck size, copy limits (basics and "any number of
  cards named X" cards excepted), commander rules (1 commander, or 2 with
  Partner; legendary creature, planeswalker, or legendary
  Vehicle/Spacecraft with a printed power/toughness box), commander color
  identity, per-card format legality, and Game
  Changer count vs bracket (0 for brackets 1–2, at most 3 for bracket 3,
  unlimited for 4–5).
- Exit 0 = legal, exit 1 = violations. JSON always prints:
  `{name, format, format_assumed, bracket, legal, violations: [{rule,
  cards, detail}], notes, summary}`.
- **Official bracket semantics (WotC, Feb 2025).** The only hard limit is
  the Game Changer count: 0 for brackets 1–2, at most 3 for bracket 3,
  unlimited for 4–5. Everything else is advisory. `deck legal` enforces
  exactly the GC cap as a violation; everything else ships as advisory.
- **`notes` carries the bracket verdicts.** With `--bracket`, the human
  view shows `bracket checks:` with `✓` PASS, `!` CHECK (genuine
  conflicts: mass land destruction in brackets 1–3, extra turns), or `ℹ`
  advisory lines. Tutors are advisory in every bracket ("tutors should be
  sparse" in 1–2; in 3 the cap is Game Changers, which includes the best
  tutors). The scan splits `note hard tutors:` (one-shot search spells)
  from `note soft searchers:` (ETB/activated/restricted searchers). Mass
  land destruction stays a hard CHECK in brackets 1–3. JSON `advisories`
  carries the `ℹ` lines separately from `violations`; `notes` carry the
  raw strings. When no bracket was given, use the Game Changers list in
  `notes` to ask the user which bracket they want.
- **Land-search ramp is not a tutor.** Cards whose library search targets
  lands (Cultivate, Farseek, Fabled Passage) get their own `note ramp:`
  line and never trip the tutor scan; only nonland tutors do.
- Bracket levels: 1 = exhibition, 2 = core, 3 = upgraded, 4 = optimized,
  5 = cEDH. When unsure, ask the user; describe 2–4 in those terms.

### Simulation (goldfish)

```sh
stm deck simulate <name>                        # 10,000 games, infer format
stm deck simulate <name> --runs 100 --seed 42   # fast reproducible smoke run
stm deck simulate <name> --seed 42 --json       # full detail for diffing
```

- Monte Carlo goldfish: shuffles the deck, plays best-case turns (play an
  untapped land when possible, cast the cheapest pip-payable spells, fire
  ETB triggers, spend leftover mana on activated engines, then spend
  remaining creature taps on mana → station → crew), and aggregates over
  `--runs` games (default 10000, range 100–1,000,000; ±0.5pp on
  percentages). `--turns` default 10 (commander) / 8 (60-card).
- Formats: inferred like `deck legal` (a `// COMMANDER` section means
  commander). Commander rules in the model: commander starts in the command
  zone with the full pip check (a {W}{U}{B}{R}{G} commander needs one of
  each), one free mulligan when the opener has <2 or >6 lands (commander
  only; constructed redraws a zero-land opener). Pass `--format <60-card
  format>` to simulate a commander list as a flat library instead.
- Card model: oracle-text driven. Tap yields are source-correct ("Add {G}
  or {U}" is one choice tap; Jegantha's fixed five pips produce all at
  once; a permanent with multiple tap abilities taps once; "add N mana
  of any color" counts N pips; "for each color among permanents you
  control" scales with the board; "any color an opponent's land could
  produce" yields from turn 2). Spend-restricted mana (Secluded
  Courtyard, Plaza of Heroes) pays matching casts only. Charge-counter
  banks (Pentad Prism) fire untapped once per turn while counters last;
  Treasure creators bank one flexible pip per token. Static grants
  (Enduring Vitality, Chromatic Lantern) add one flexible pip per
  matching permanent, capped at 2 per grant. Station cards
  (Spacecraft/Planets) get charge counters from creature taps and unlock
  `{N+}` tiers (only the P/T tier animates); station and crew use
  printed power when known. Vehicles crew with bodies and revert at end
  of turn. Mill fills a graveyard census (self vs opponent direction)
  and counts as cards seen; graveyard return (hand or battlefield) fires
  once per card; wheels reset the hand; loot is draw-n discard-n;
  sacrifice outlets consume real bodies and fire death triggers;
  planeswalker loyalty is tracked and gates loyalty activations. Cost
  cuts (warp, improvise, affinity) approximate to flat discounts.
  Enters-tapped lands follow their oracle text (shock-dual life payments
  are always paid).
- **Wincon and keyword signals (goldfish-aligned only).** Attack power
  per turn + p90 by t8 (a power curve, never a kill estimate) with
  static "+N/+N" buffs, equipment, double strike, prowess, and landfall
  counted in; trample/flying/menace show as an evasion census.
  Combat-damage triggers fire per connecting attacker (proliferate,
  draw, drain). Burn/drain accumulate into `drain_total_by_turn`
  (×3 for "each opponent"). Extra turns each grant one land drop and
  one draw. Win-threshold engines (Darksteel Reactor class) and
  planeswalker ultimates report a first-online share. Scry/surveil give
  zero draw credit — they feed `library_awareness_by_turn` instead
  (share of the library evaluated; surveil puts the cards in the
  graveyard). Imprint and blocking are not modeled.
- **Interaction readiness is capacity, not events.** The goldfish never
  fires a counterspell or removal spell. It measures whether instant-
  speed interaction is in hand **and** affordable with spare mana
  (`interaction.ready_pct_by_turn`), plus the spare amount while ready
  (`interaction.mana_held_avg`). Three terms: **access** = seen in hand
  (`role_access`), **ready** = in hand + affordable, **mana held** =
  spare mana while ready. Report readiness as capacity; never claim a
  counter or removal resolved.
- **Exit 1 means the simulation found problems** (the result, not a crash).
  Problem kinds: `mana_screw`, `mana_flood`, `color_screw` (enough mana,
  wrong colors), `commander_late`, `draw_starvation`, `mana_unused`,
  `dead_cards` (3+ distinct non-reactive spells cast on-time under 60%),
  `category_starved` (removal, wincons), `interaction_unready` (answers
  seen but rarely affordable with spare mana → "add cheaper
  instant-speed answers"). Each problem carries a category +
  magnitude suggestion ("add 2-3 draw engines") — never card names.
  Reactive spells (removal, fogs, protection) are exempt from `dead_cards`;
  judge them by `role_access` — a castability flag on them is noise.
- Human output = aggregates + worst-3 slow-to-cast cards + pip-block
  offenders + problems. `--json` is the full contract: `deck_shape
  opening_hand land_drops commander station bodies_by_turn
  engines_online_by_turn mana draw role_access velocity
  library_awareness_by_turn self_milled_by_turn opp_milled_by_turn
  library_remaining_by_turn combat wincons interaction color_screw
  pip_blocks graveyard card_castability problems assumptions summary`.
  `--combo "A + B"` (repeatable) adds `combo_access` (share of games with
  both pieces in hand by the target turn). A synced combo store adds
  `combos` (Spellbook variants joined to the deck: complete combos with
  assembly rates + one-card-away near-misses; `--combo-limit N` caps each
  list, default 20) and `win_paths` (complete combos whose Spellbook
  `produces` label contains a win feature — "Win the game", "Infinite
  damage", "Infinite turns", …). `--hypgeo` adds `hypgeo` (exact
  cast-on-curve ceilings = an upper bound on the real cast rate; the
  sim's castability is draw-agnostic and naturally sits above its
  ceiling — the two answer different questions, not one scale).
  `station` is null for non-spacecraft commanders.
- The model is a **consistency diagnostic, not a win-rate predictor**. Its
  limits are listed in the JSON `assumptions` array: enters-tapped
  honored, no opponents or interaction, draw engines fire once per turn on
  a fixed delay, opponent-dependent mana sources (Fellwar Stone) produce
  from turn 2 on, no
  commander recast tax, attack-gated commander draws wait for animation
  (no synthetic engine), improvise/affinity discounts grow with the
  artifact count, printed power (else flat 2) for stationing and crewing,
  X-costs pay for one, energy/metalcraft/converge/proliferate/replay
  mechanics not modeled, role classification is heuristic (audit via
  `deck_shape` counts). `draw.pct_seen_by_turn` and `role_access` are
  hand-visibility (share of games with the role in hand), not engines
  online — use `engines_online_by_turn` for online counts. Seed baselines
  are version-local: regenerate the baseline JSON after upgrading `stm`.
- **Fix loop (the standard validation step for any deck edit):** simulate
  with a seed, apply the edit, re-simulate with the same seed and diff:

```sh
stm deck simulate <name> --seed 42 --json > /tmp/base.json
# ... apply deck update ...
stm deck simulate <name> --seed 42 --baseline /tmp/base.json
# prints deltas only: shape counts, metric lines, problems (+ new / - resolved)
# exit 1 only when a problem is new
```

Identical seeds = identical shuffle baselines, so differences isolate the
deck change. For full-detail comparison keep the `--json` diff form.

### JSON shapes (stable)

- `card <name> --json` → card object: `name mana_cost cmc type_line colors
  color_identity keywords power toughness loyalty oracle_text rarity
  edhrec_rank legalities game_changer set collector_number scryfall_id
  released_at tags price_usd price_usd_foil max_price_usd
  max_price_usd_foil`. The `price_usd*` fields are the **cheapest** released
  English printing per finish; the `max_price_usd*` fields the most
  expensive. All null when no print is priced. Use `price_usd` for budget
  math instead of parsing human output.
- `card similar <name> --json` → array of hits: card fields (including
  `oracle_id`) + `score` (hybrid 0–1; `null` when the seed had no stored
  vector) + `shared_count shared_tags`.
- `card combos <name> --json` → array of hits: `{id, produces,
  mana_value_needed, bracket_tag, popularity, legalities,
  requires_commander, pieces: [{name, zones, must_be_commander}]}`.
- `query --json` → array of hits: the same full card object as
  `card <name> --json` (all fields, `tags`, four price fields) plus
  `score` (hybrid 0–1: reciprocal-rank fusion of full-text and vector
  matches). Empty result prints `[]` with exit 3.
- `collection --json` → `unique_cards total_cards foils total_value
  purchase_total color_identity curve rarity top_sets locations`.
  `total_value` prices every owned copy by its exact printing.
- `collection query --json` → card fields + `score owned locations`
  (binder pool by default; `--deck` adds a deck's cards). `owned` is a
  copy count (0 = none) on every command that reports it. `deck suggest`
  and combo completions count copies across binders **and** deck
  assignments (a card owned only inside a deck reports its count there).
- `deck show <name> --json` → `{name, cards, sideboard_cards, primer,
  owned_value, missing_cost, sections: [{section, cards: [{quantity, name,
  set, collector_number, foil, owned, owned_elsewhere, covered_by,
  basic_land, price_usd}]}]}`. `cards` is the maindeck count (the legal
  deck); `sideboard_cards` reports the sideboard separately. `primer` is
  the file path; read the primer's contents
  with `stm deck primer <name>` (plain markdown on stdout; an empty
  primer prints a note instead). `price_usd` is the cheapest printing
  (foil entries price at the cheapest foil print when one exists).
  `covered_by` is `deck` (deck-assigned copies fill the slots), `binder`
  (binder copies fill them), `basic`, or `missing`. Ownership math is
  shared with `deck buylist`: `missing_cost` equals the buylist total, so
  budget math needs only one of the two. The human view also prints a
  `To buy` block (top lines by cost, running total).
- `deck list --json` → `[{name, has_decklist, cards, sideboard_cards,
  owned, has_primer}]` (`cards` is maindeck; `has_decklist: false` marks
  collection decks whose list is not imported).
- `deck legal <name> --json` → `{name, format, format_assumed, bracket,
  legal, violations, advisories, notes, summary}`.
- `deck simulate <name> --json` → `{name, format, runs, turns, seed,
  deck_shape, assumptions, opening_hand, land_drops, commander, station,
  bodies_by_turn, engines_online_by_turn, mana, draw, role_access,
  velocity, combat, wincons, interaction, color_screw, color_sources,
  card_castability, problems, summary, win_paths?}`.
  `deck_shape.total_cards` is the simulated library
  plus commander (sideboard excluded; `deck_shape.sideboard_cards` counts
  it). `commander` is null for non-commander decks; `station` is null for
  non-spacecraft commanders (`{online_by_t6, p50_online_turn}`).
  `bodies_by_turn` counts creatures, animated spacecraft, and ETB tokens
  per turn; `engines_online_by_turn` counts repeatable draw engines.
  `color_sources` is a static census of land tap yields per color
  (`fixed_source_lands` dedicated single-color, `choice_source_lands`
  multi-color pickers) — read it next to `color_screw` to pick fixes.
  `problems` is an array of `{kind, severity, pct_games, color, detail,
  suggestion}`; severity is `high` (≥20% games), `medium` (10–20%), or
  `low`. Every `pct_*` field in the report is 0–100 percent at two
  decimals. Color-screw details name the dedicated source count and shape.
  `card_castability` rows are `{name, cmc, target_turn,
  pct_castable_by_target, avg_first_castable_turn}` (one row per distinct
  card name).
- `deck simulate <name> --baseline prior.json` (human output) diffs the
  fresh run against that JSON and prints deltas only — shape counts,
  metric lines (`path: old → new`), and problems (`+` new, `-` resolved).
  Exit 1 only when a problem is *new*; identical or improved decks exit 0.
  Use it in the fix loop instead of saving and diffing JSON by hand.
- `deck suggest <name> --json` → array of `{name, oracle_id, mana_cost,
  cmc, type_line, edhrec_rank, game_changer, owned, price_usd, score,
  tags, oracle_text, color_identity}`. Ranked by fit: semantic search and
  tag matches fuse (reciprocal rank fusion, same as `query`), EDHREC rank
  breaks ties; owned cards list first, then unowned, each group in fit
  order. `tags` carries the Tagger labels that matched; `owned` is a copy
  count (0 = none); `score` is the fused fit 0–1. `--commander` swaps the pool to
  commander-legal legendaries (legendary creatures/planeswalkers, plus
  legendary Vehicle/Spacecraft with a P/T box) ranked by theme fit to the
  deck. `--format <fmt>` pins the legality filter (cards legal in that
  format only); without it, commander-shaped decks filter to commander and
  the commander's colors, and other decks take any format. With no query
  and no role the pool is one-card-away Spellbook
  completions ranked by variants completed, win-the-game, popularity,
  EDHREC; each row carries `combo` (`{pieces, bracket_tag, produces,
  variants_completed}`) naming the best example. Commander decks complete
  commander combos; other decks drop combos that need a commander (pin a
  format with `--format` to filter to one). `--bracket 1-2` filters
  Game Changers out (their allowance is zero); brackets 3-5 do not
  filter (deck legal counts the deck's allowance).
- `deck buylist <name> --json` → `{store, rows: [{name, set, set_name,
  collector_number, scryfall_id, foil, quantity, price_usd}], total_usd}`.
  `rows` = cheapest released English printing of the right finish for the
  missing copies.

### Buylists

`stm deck buylist <name>` prints what needs purchasing as plain lines
(`2x Lightning Bolt`) — pipe to `pbcopy` or a file. Owned copies
(deck-assigned + binders) autofill deck slots read-only, so the list is
only the gap; nothing in the collection is moved. `--store cardkingdom`
emits `Name,Edition,Foil,Qty` (Edition is the full set name, e.g.
`Magic 2011`); `--store tcgplayer` emits TCGPlayer Mass Entry CSV.
`--json` gives rows and the estimated market total. Buy prices are the
store's job; the USD figure is a market-price estimate.

### Gotchas

- Meaning-based queries always return nearest matches, even for gibberish.
  Exit 3 means the result set was emptied by filters or location limits,
  not that the index is empty. Check the data exists with `stm collection`.
- Prices are daily estimates from Scryfall's bulk, not storefront quotes.
  Card prices reference the cheapest released English printing; deck
  `missing_cost` prices missing copies at that cheapest print.
- Flavor-name prints ("Godzilla, King of the Monsters") resolve to their
  oracle card ("Zilortha, Strength Incarnate"); both names work everywhere
  a card name is taken.
- Two CSV paths exist by design: `stm collection import` for the
  collection CSV, `stm deck import` for a ManaBox deck txt export. The
  collection CSV never writes decklists; re-importing it never touches
  your deck files.
- A stale background sync may run before a read; pass `--offline` to skip
  it.
- `deck simulate` role counts are heuristic (oracle-text based). When a
  finding looks wrong, check the `deck_shape` counts in `--json` first —
  a misclassified card shows up there. Unseeded runs differ every time;
  always compare runs with the same `--seed`.
- Never hand-edit `stm.db`, `vectors.bin`, or `status.json`. If the store
  looks corrupt, rebuild with `stm setup --force`.

## Part 2 — Deckbuilding workflow

Two modes, both mandatory to run as written. **Mode A: build a deck from
scratch.** **Mode B: improve an existing deck or precon.** Never skip the
question rounds in either mode. No jumping to a finished list.

### Ground rules (both modes)

- **Small batches.** Propose at most 3–5 cards at a time. Present them as a
  table: card, role, price, own/buy, one line on why it fits. Wait for the
  user's confirmation before applying anything with `deck update`.
- **Cost every batch before presenting it.** For each to-buy card in a
  proposed batch: `stm card <name> --json` → `price_usd` (or
  `price_usd_foil` for a foil entry). Sum the to-buy prices, compare
  against the remaining budget, and only present the batch if it fits;
  otherwise present cheaper alternatives in the same table. The batch
  table's price column and the running budget line both come from this
  step, never from memory or guesses.
- **One question minimum per batch.** After each confirmed batch, ask at
  least one question: which direction to push next, whether a choice felt
  right, what to cut. Build the deck with the user, not for them.
- **Budget rules everything.** Track the running spend against the user's
  budget after every batch and state it ("$34 of $50 budget used"). Cards
  the user owns cost $0. Pull from `deck show --json`'s `owned` and
  `price_usd` fields.
- **Collection first.** Before proposing a purchase, search what the user
  owns (`stm collection query`) for a card that fills the same role. "Good
  enough" beats "strictly better" when the owned card is playable in the
  slot; offer the upgrade as an option with the price gap, not as a
  requirement. When a proposed card is too expensive, `stm card similar
  <name> --owned` finds cheaper owned cards that play the same role.
- **Basic lands are free and unlimited.** Never count them toward budget,
  ownership, or the 4-copy limit. Add whatever the mana base needs.
- **Confirm legality.** After the deck is complete (or at the user's
  request), run `stm deck legal` and walk the `notes` checklist as
  described in Part 1.

### Mode A: build a new deck

**Round 1 — ask before doing anything.** Get answers to all of these
(ask only what the user hasn't already said):

1. **Format.** Commander, Modern, Standard, Pioneer, Pauper, other?
2. **Budget.** Run `stm collection --json` first and anchor the question in
   what they already have: "Your collection holds about $X of cards; how
   much are you looking to add on top for this deck?" If they say "use what
   I own", set budget to $0 for purchases and confirm. If they give no
   number, ask again; the whole workflow is budget-driven.
3. **Strength target.** For commander: **which bracket** (1–5; describe 2 =
   casual core, 3 = optimized, 4 = high-power ultra, 5 = cEDH, and mention
   the Game Changer limits). For other formats: how competitive (FNM-level,
   RCQ-level, spike-max? This decides how many compromises are OK and how
   much opponents will hate it.
4. **Theme or commander.** A commander, a mechanic ("life drain"), an
   aesthetic, or "suggest options".
5. Playstyle: aggro / control / combo / value? Speed preference?
6. Anything off-limits: cards the user refuses to play, hate pieces
   ("stax"), proxies, or a "don't buy singles above $X" rule.

If the user wants suggestions for 4–6, offer 2–3 commander/theme candidates
with a one-line pitch each. Two search paths, in order:

1. **`stm deck suggest <name> --commander`** — commanders ranked by theme
   fit to the deck's own cards, with EDHREC rank, price, and ownership.
   Needs a deck to exist first (`stm deck create <name>` is enough).
2. **`stm query <theme> --type "Legendary Creature" --format commander`**
   for whole-oracle browsing. The `--type` filter is a type-line substring:
   `Legendary Creature` covers creatures; for the 2025+ rules add
   `--type "Legendary Vehicle"` or `--type "Legendary Spacecraft"` (they
   can command when they have a printed P/T box; check `power`/`toughness`
   are not null in the JSON). Combine with `--color-identity GU` etc. to
   pin the identity, and `--owned` is not available here — check
   ownership with `stm collection query "<name>"` per candidate.

### Power targets (the quality bar)

"Powerful" is verifiable. After each fill round, check the deck against
these counts (using `stm deck show`'s ramp/curve overview and the deck
list) and name the deficit when a category is short, e.g. "5 of ~10 draw
pieces, need 3–5 more in the next batch":

| Category (commander) | Bracket 1–2 | Bracket 3–4 | Bracket 5 (cEDH) |
| --- | --- | --- | --- |
| Ramp (lands, rocks, dorks) | 8–10 | 10–12 | 12+ |
| Card draw | 8–10 | 8–12 | 10+ |
| Interaction (removal, wipes, countermagic) | 8–10 | 10–14 | 12+ |
| Win conditions | 3–5 | 3–5 | 2–4 (fast) |

For 60-card formats: 20–24 lands (more for control), 4-ofs for core pieces,
a curve that lets the deck do its thing by turns 3–4 (aggro) or 4–6
(midrange/control). Targets flex for archetype: a control deck runs more
interaction and fewer creatures; a go-wide deck counts its payoffs as win
conditions. Say when you are deviating and why.

**Simulation is the verification step for these counts.** Static counts
say the deck *contains* 10 ramp pieces; `stm deck simulate` says whether
the deck *draws* them early enough (`role_access`), whether the commander
actually comes down on curve (`commander.on_curve_pct`), and whether the
curve is playable (`card_castability`). Run it after the skeleton and
after every confirmed batch (same `--seed` each time), and let the
`problems[]` findings drive the next batch. Exit 1 = problems found.

**Round 2 — skeleton (get confirmation).** Once round 1 is answered:

1. `stm collection --json` to see the collection's shape and value.
2. Pick the commander (or core theme cards). For commander, verify color
   identity implications early; everything must fit it.
3. Propose the mana base plan (how many basics, which duals/fetches owned
   vs to buy) and the ramp plan for the bracket (e.g. bracket 2 wants ~8–10
   sources; cEDH wants more). Show owned vs to-buy with prices.
4. Show the skeleton to the user, state the projected spend, and get an OK
   before writing anything.
5. Once the skeleton is written, sanity-check it with `stm deck simulate
   <name> --seed 42 --json`: commander on-curve and land-drop rates should
   already look sane at this stage; fix the mana-base plan before filling
   if they don't.

**Round 3 — fill by category, batch by batch.** Work in this order, one
batch per round, confirming between each: ramp → card draw → removal/board
wipe → win conditions → flexible slots/theme pieces → mana base finish. For
each card:

- **`stm deck suggest <name> --role <draw|removal|ramp|wincon|
  counterspell|land> --json` is the one-call fill tool**: it searches owned
  first, matches Scryfall Tagger labels, filters to the commander's color
  identity, and ranks by EDHREC playability — no separate price/GC calls
  needed (both ride on the same JSON). Pass a free-text query for theme
  fills (`"frog payoff"`). `stm collection query "<role>" --color-identity
  <CI> --format commander --json` and `stm query` remain the fallbacks for
  deep browsing. When a candidate card fits the role but costs too much,
  `stm card similar <candidate> --owned --json` surfaces owned cards with
  the same functional profile.
- Check `stm card <name> --json` for price, `game_changer`, and
  `legalities` before proposing (suggest already carries most of it).
  Respect the bracket's Game Changer limit.
- State cost-to-user explicitly in every batch ("all owned, $0" or "2 to
  buy: $11.40 total").

**Round 4 — lands, review, finish.** Fill the mana base. Basics are free,
so the budget rule for lands: cover colors with basics first, then check
what the user already owns (`stm collection query "land" --type "Land"
--json`) before proposing anything. Tap lands, pain lands, and
enter-tapped lands the user owns beat buying fetches/shocks. When the
strength target justifies fast mana, price the upgrade gap explicitly
("fetches would cost $45; the owned tap lands play fine at bracket 2") and
let the user decide. Then run `stm deck
show <name> --json`, check the power targets above, and present the final:
total cards, curve, ramp counts, owned/total, **total spend vs budget**, and
`stm deck legal` output (including the bracket checklist review). Run a
final `stm deck simulate <name> --json` and present its `problems[]`
(or "no findings") as the deck's consistency report. Write a
short primer with `stm
deck primer <name> --set`. The primer describes the deck **as it stands
now**: game plan, mana base (counts + the fixing lands), interaction
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

1. **Load and assess.** Import the decklist if needed (`stm deck import
   <name> <file>` — upserts by name; use a ManaBox deck txt export for a
   precon). Then `stm deck show <name> --json` (curve,
   ramp, prices, ownership) and `stm deck legal <name> --bracket <b>` once
   the user names a target bracket. **Run `stm deck simulate <name>
   --json` too** — its `problems[]` are the primary diagnosis input. Report:
   total value, missing cost, any violations, gaps against the power
   targets, and the simulation findings (screw/flood, commander timing,
   starved categories, dead cards).
2. **Ask what's wrong.** "How did it feel to play? Where did it lose?"
   Common answers map to fixes: ran out of gas → draw; died to creatures →
   removal/wipes; clunky → curve/ramp; never closed games → wincons; lost
   to X specifically → answers for X. If the user doesn't know, use the
   simulate output as the diagnosis: `problems[]` kinds map to the same
   fixes, and `card_castability` names the cards that sit dead in hand.
   Propose the diagnosis (sim data + curve numbers) and ask if it matches
   their experience. For a card the user calls out as underperforming,
   `stm card similar <that card> --owned --json` proposes owned
   replacements with the same role profile.
3. **Optional research.** If the user asks or the problems are unclear,
   search online (EDHREC primer for the commander, bracket guides) and
   summarize findings with sources before proposing anything.
4. **Change batches like Mode A.** For each problem, propose swaps: remove
   N, add M, favoring cards the user already owns, stating the price of
   anything new, staying inside the budget the user confirms at the start
   of Mode B ("what's the budget for these upgrades?").
5. **Apply and verify.** `stm deck update` per confirmed batch. After each
   batch, re-simulate with the same seed and show the metric deltas (the
   fix loop in Part 1) — a batch is working when its target metric moved
   and no new `problems[]` appeared. Then a final `stm deck show` +
   `stm deck legal` + primer update (current state, not build history).
   Present spend vs budget and what
   changed per problem the user named.

### Output style during deckbuilding

Keep prose tight. One table per batch, one question per batch, running
budget line after every `deck update`. Show the full deck only on request
or at the finish, not after every batch.