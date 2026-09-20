# Goldfish Simulation (`stm deck simulate`)

This document is the living reference for `src/deck/simulator/`. Update it
when you change the model. It explains what the simulation is for, how it
models cards, how a turn runs, what the numbers mean, and where the model
stops being true.

## Intent

The simulation answers one question: **can this deck do its job on time?**

It is a consistency diagnostic. It is not a win-rate predictor. It plays
thousands of solitaire games and reports whether the deck draws its lands,
hits its curve, casts its commander, feeds its engines, and sees its
removal and win conditions early enough. It never plays against an
opponent. It never scores a win.

Use it to:

- Check the commander comes down on curve (and whether the mana base can
  pay its exact pips).
- Find mana screw/flood, color screw, draw starvation, and dead cards.
- Verify a deck edit: simulate with a seed, edit, re-simulate with the
  same seed, and diff. Identical seeds give identical shuffles, so every
  difference comes from the edit.

Do not use it to:

- Compare against opponents or metagames (no opponents exist).
- Score card power (role classification is heuristic).
- Predict brackets or win rates.

## Where the code lives

```
src/deck/simulator/
├── mod.rs        Entry point: CLI wiring, run loop, exit codes
├── format.rs     Per-format rules: mulligan policy, turn defaults
├── model.rs      Card data model: Cost, TapYield, Tier, Ability, Role
├── parse.rs      Oracle text → model (the sim's whole intelligence)
├── deck.rs       Deck text + card rows → SimDeck
├── game.rs       One game: shuffle, mulligan, turn loop (pure, seeded)
├── aggregate.rs  Game logs → statistics + problem findings
├── combos.rs     Store-backed combo assembly (zones → GameLog lookups)
├── hypgeo.rs     Exact hypergeometric cast-on-curve ceilings (--hypgeo)
└── report.rs     JSON payload + human stdout render
```

Functional core, imperative shell: `run_game` and everything under it is
pure and driven by one seeded `ChaCha8Rng`. Same deck + same seed = the
same games, byte for byte. `aggregate`, `find_problems`, and the renders
are separate functions the entry point joins. Mutable per-game state
(battlefield, library, hand, graveyard, cards seen) lives in one
`GameState` struct the effect helpers share.

## Format rules

Per-format behavior lives in one table (`format.rs`), never in branches.
`FormatRules` carries the format key, the library shape
(command-zone singleton vs 60-card), the default turn count, and the
mulligan policy. Commander-family formats redraw once when the opener's
land count leaves the 2–6 band; 60-card formats play London mulligans
(outside 1 land: redraw and bottom the same count at random — the sim
cannot evaluate keep choices). Commander features (cast loop, engine
tier) gate on the deck having commanders, not on the format enum. An
unknown `--format` key plays as generic constructed.

## The card model

Cards are data, not rules. The parser (`parse.rs`) reads each card's
oracle text once, at deck build time, and produces a `SimCard`. Everything
the turn loop does later is execution of that data. Shapes the parser
cannot read become plain cards with no abilities: they still cost mana,
still count toward the curve, and still show up in castability — they just
do nothing on the battlefield.

### Tap yields

A permanent taps once per turn. What one tap yields splits into four
shapes, keyed off the "or" in the text:

| Oracle shape | Example | Model |
| --- | --- | --- |
| "or" choice | `Add {G} or {U}` (shock duals, Verge lands) | one tap, one mana of either color |
| any-color prose | `Add one mana of any color` (Command Tower, Signet) | one tap, any color |
| fixed simultaneous set | `Add {W}{U}{B}{R}{G}` (Jegantha) | one tap, all five at once |
| colorless | `Add {C}{C}` (Sol Ring) | one tap, N colorless |

A spend restriction ("Spend this mana only to cast a creature spell" —
Secluded Courtyard, Unclaimed Territory) marks the yield restricted:
restricted mana sits in its own pool bucket and pays matching casts only
(creature / legendary / artifact / instant-and-sorcery). A merged tap
with any restricted mode counts as restricted (the unrestricted
colorless mode produces nothing of value for other casts).

A permanent with several tap abilities (Plaza of Heroes, Relic of Legends,
Blazemire Verge) merges them into one tap: the union of its colors, still
one mana per turn — never several independent sources. This was the core
correctness fix over the first simulator, which counted each listed color
as its own source and inflated the mana pool.

Gated modes ("Activate only if you control a Swamp or a Mountain") stay
locked until another land of the matching type is in play. The ungated
mode always works. Shock-dual life payments are always paid (best-case).

Beyond the four shapes, three extras:

- **Any-pip counts.** "Add N mana of any color" / "any combination of
  colors" parse as N flexible pips (Gilded Lotus → 3), not one.
- **Opponent-dependent.** "Any color a land an opponent controls could
  produce" (Fellwar Stone) yields one any-color pip from turn 2 on;
  nothing on turn 1.
- **Scaling.** "For each color among permanents you control" (Faeburrow
  Elder) counts the colors actually on the board; "for each charge
  counter" (Astral Cornucopia) reads one pip per counter.

Two banked shapes:

- **Charge-counter banks.** "Remove a charge counter: add one mana of
  any color" (Pentad Prism) is a banked activation: the source stays
  untapped, fires once per turn while counters last, and one counter
  buys one pip. Sunburst enters with one counter per color paid
  (best-case 2).
- **Treasures.** "Create a Treasure token" effects on the *acting card*
  bank one flexible pip per token (sacrificed to use) instead of
  creating a body. Only Treasure-labeled effects convert; other token
  spells in the same deck still create bodies. Smothering Tithe stays
  inert — it needs opponents.

Static mana grants ("creatures you control have {T}: add one mana of
any color" — Enduring Vitality; "lands you control have…" — Chromatic
Lantern) add one flexible pip per matching permanent per turn, capped
at two per grant, while the source is on the battlefield.

### Station (CR 702.184, 721)

Spacecraft and Planets carry one or two `{N+}` striations. Each striation
means "as long as this permanent has N or more charge counters, it has
these abilities". Only the striation with a printed P/T box animates the
card as a creature; Planets never animate. The parser finds the animation
threshold from the reminder text ("It's an artifact creature at 12+") and
attaches tier abilities from the `N+ | ...` segments.

Charge counters come from creature taps (the tap budget, below), from
"put N charge counters" spells (Drill Too Deep), and from cards that enter
with counters (Reckoner Bankbuster). Counters persist across turns.
Animation is permanent: once a spacecraft is a creature, it stays one.

### Crew

Vehicles crew with bodies: total untapped body power ≥ the crew cost.
Crew animation lasts until end of turn. Bodies are creatures, animated
spacecraft, and ETB tokens. Body power uses the card's printed power when
the row carries one; tokens and unknowns stay flat (`BODY_POWER = 2`).

### Roles

Every card classifies into one role (first match wins):
`Land`, `Rock`, `Dork`, `RampSpell`, `Draw`, `Removal` (also fogs,
regeneration, protection — reactive spells), `Lock` (tax/restrict pieces),
`Booster` (equipment, auras, pump), `Wincon`, `Other`. Reactive spells
(removal, fogs, protection) never fire in solitaire: `role_access` judges
them by hand visibility, and they are exempt from the `dead_cards`
finding.

### Abilities

Each ability has a trigger, an optional cost, and an effect.

Triggers modeled: `Activated` (pay cost, consumes a tap or loyalty),
`OnEnter`, `OnUpkeep` (also end-step engines — one per turn), `OnAttack`,
`OnCastSpell`, `OnDeath` (dies or is sacrificed).

Effects modeled: `Draw(n)`, `Tutor` (search/look-into-hand),
`Mana(TapYield)` (unlocked mana activations feed the pool),
`ManaPerCounter(TapYield)` (upkeep release of banked mana: Coalition
Relic's "remove all charge counters, add that many mana"),
`Tokens(n)` (become battlefield bodies; "for each" shapes cap at 8),
`Counters(n)` (charge the host),
`ExtraLand`, `Mill(n)` (library top → graveyard census), 
`ReturnFromGraveyard { to_hand, count }` (hand-return = draw credit;
battlefield-return = a body once per card), `Wheel` (hand → graveyard,
draw seven), `Loot(n)` (draw n, discard n), `Drain(n)` (life loss at a
player: ×3 in commander, ×1 in 60-card formats).

Activations carry optional costs beyond mana: `sacrifice_bodies` consumes
an untapped body and fires its death triggers (aristocrats outlets);
`loyalty_cost` spends planeswalker loyalty instead of mana;
`loyalty_gain` adds loyalty (planeswalker plus abilities). Planeswalker
abilities fire through the same activation pass: one loyalty ability per
turn, gated on affordability, never on mana. Ultimates stay flags — they
mark `ultimate_online` when loyalty covers the minus cost, they do not
resolve.

Commanders get one extra synthetic ability: when the commander's oracle
shows an *unconditional* repeatable draw (upkeep or end-step triggers),
the command zone registers a draw-1-per-turn engine from the turn after it
is cast. Attack-gated draws ("Whenever N attacks, draw…") never become
synthetic engines: they need the permanent to animate and attack, which
the normal combat path models. Opponent-gated draw triggers
("whenever an opponent…") register no engine — a goldfish has no
opponents.

Tap yields that reference opponents ("Add one mana of any color that a
land an opponent controls could produce") produce nothing: the goldfish
has no opponents. Lands whose oracle grants them a basic type ("This land
is the chosen type") tap for one mana of any color — the chosen type is
the player's choice each game. Static type-granting on *other* lands (The
World Tree's "lands you control have {T}: …") is not modeled.

Sagas stage one chapter per turn. Chapter lines ("I — Draw a card.",
"II — Mill three.") parse into per-chapter abilities through the same
effect shapes as triggers; a chapter with no readable effect draws one
card instead. Combined numeral lines ("I, II, III — Create a token")
give every listed chapter the same effect. A four-chapter saga runs its
fourth chapter; once the final chapter resolves, the permanent leaves
the battlefield (it sacrificed in real Magic).

Blink-shaped ETBs ("exile … return it to the battlefield") re-fire the
host's OnEnter triggers once, the turn after (Skyskipper Duo, flicker
engines). Monarch acquisition ("you become the Monarch") counts as an
extra card per turn from acquisition (the Monarch draws at upkeep).

### Keywords modeled (goldfish-aligned)

A keyword models only when it moves a metric:

| Keyword | Model |
| --- | --- |
| Haste | hasted creatures skip summoning sickness: they attack, tap, and crew the turn they enter |
| Landfall | "+1/+1 counter" shapes feed the combat power sum; full trigger lines ("Whenever a land you control enters, draw/create/add") parse as real OnEnter engines |
| Double strike | attack power ×2 (no blockers exist) |
| Prowess | +1 power per noncreature spell cast that turn |
| Flying / Trample / Menace | evasion census (no math) |
| Flash | instant-speed flag (feeds interaction readiness); "flashback" does not count as flash |

"Whenever … enters" ETBs parse like "When … enters". Landfall trigger
lines form their own family (draw / tokens / mana-adds read as an
extra-land-style ramp credit rather than a mana tap — the effect lands
in `cards_seen`, not the turn's mana pool; search-for-land reads as
extra land). "You may play an additional land" (Aesi class) grants one
extra land drop per turn while on the battlefield, recorded in
`land_drops[]`.

### X-costs, kicker, and per-cast mana

| Shape | Model |
| --- | --- |
| `{X}` spells with a scaling effect (drain/draw/mill/tokens) | the cast pays the whole leftover pool as X; the effect scales with it (drain ×N, draw ×N, mill ×N, up to 8 tokens) |
| Kicker / multikicker | paid from spare mana when affordable; drain amounts scale with the kick; multikicker parses as one kick |
| "Add one mana … for each spell you've cast this turn" (Vivi class) | the yield joins the pool once per spell cast every turn the host is on the battlefield; the same clause never also reads as a plain one-mana tap |
| Additional costs ("as an additional cost …, sacrifice a creature / pay N life") | the cast consumes the resource (a real body leaves the battlefield; life is paid) |
| "Create N …tokens" sorceries | the cast creates N token bodies (capped at 8) |

### Free mana engines and the loop cap

A zero-cost untapped non-tap activation ("{0}: Add {C}") repeats in real
Magic. The sim lets it repeat but caps the pass at 24 activations per
turn; crossing the cap flags `infinite_mana_pct` (share of games with a
suspected infinite engine) and stops the loop. No engine runs away.

### Cost reductions

Modeled as discounts on the generic part, and only for castability
(`min_cost`), never as extra mana:

| Mechanic | Approximation |
| --- | --- |
| Warp | the warp cost when cheaper |
| Improvise | −2 generic, plus 1 more per 4 artifacts on the battlefield (capped at the printed generic) |
| Affinity | −2 generic, plus 1 more per 4 artifacts on the battlefield (capped at the printed generic) |
| "costs {1} less" / "{X} less" | −2 generic |

The improvise/affinity discount grows while playing: a mid-game board casts
big improvise spells a few turns earlier than the flat floor, and such
cards are exempt from the `dead_cards` finding (their real cast time is
much earlier than the floor implies).

Hybrid pips (`{W/U}`, `{B/P}`) pay from any of their colors. {S} (snow)
pays as colorless. X-costs follow the X-cost section above.

## The turn pipeline

A best-case agent plays each turn in a fixed order:

1. **Untap** — everything untaps; summoning sickness clears; once-per-turn
   flags reset.
2. **Upkeep** — draw, mill, recursion, drain, and token engines fire (one
   firing each, per turn); saga chapters advance through their parsed
   abilities; win-threshold engines check their counter stock;
   planeswalker ultimates flag online when loyalty reaches the minus
   cost.
3. **Draw** — draw 1.
4. **Land** — play an untapped land when one is in hand, else any land
   (tapped lands wait for a better turn when possible). Fetch lands search
   up a non-fetch land, which enters tapped. An "additional land" board
   (Aesi class) plays a second land the same turn.
5. **Cast** — cheapest castable spells first, with the full pip check. A
   cast is blocked (and its color recorded for color-screw stats) when the
   pool has enough total mana but misses the pips. ETB triggers fire; ETB
   tokens join the battlefield as bodies. One-shot effects (ritual mana,
   draws, mills, scry/surveil, burn, extra turns, X-scaling, kicker)
   apply on cast. Per-cast mana engines (Vivi class) add their yield per
   spell cast.
6. **Activate** — spend leftover mana on unlocked activations (draw,
   tutor, mana, counters, loot, sacrifice outlets, planeswalker loyalty
   abilities, drain activations). Cheapest first, one activation per
   permanent per turn (free zero-cost activations repeat until the cap).
   Banked activations (Pentad Prism) consume a counter instead of
   tapping.
7. **Tap budget** — remaining untapped bodies, in priority order:
   a. tap for **mana** only while the cheapest uncast spell still needs
      mana;
   b. else tap to **station** the highest-threshold unfilled
      spacecraft/planet (counters = body power);
   c. else tap to **crew** untapped Vehicles (total body power ≥ crew
      cost; crewed vehicles count as bodies for the turn);
   d. else pay **equip** costs once per Equipment — the gear suits up
      its best untapped, unsick body and the buff joins only that
      host's attack.
7b. **Interaction readiness (measured, not forced)** — was instant-speed
   interaction in hand while spare mana covered its cost? The goldfish
   never spends it; the spare amount is the "mana held" census.
8. **Threshold** — station tiers unlock permanently when counters reach
   them; the commander spacecraft's online turn is recorded.
9. **Combat** — bodies attack; attack triggers and combat-damage
   triggers fire per connecting attacker (best case: unblocked). Buffs,
   equipped gear, double strike, prowess, and landfall join the power
   sum. Hasted creatures attack the turn they enter. Token payoffs join
   next turn's bodies.
10. **End** — crew animations expire; hand-limit discards from the end;
    queued extra turns replay a land drop (recorded in `land_drops[]`), a
    draw, and one firing of each upkeep engine.

The commander casts from the command zone with the full pip check. Its
cost is deducted (it counts in `mana_spent`), it joins the battlefield as
a permanent, and its ETB triggers fire (Infinite Guideline Station's
tokens become station fuel). There is no recast tax — a destroyed
commander stays on the board in this model.

## Metrics

All metrics aggregate over `--runs` games (default 10,000). Percentages
carry ±0.5pp at 10k runs.

| Block | Meaning |
| --- | --- |
| `opening_hand` | land distribution, mulligan rate (one free mulligan under 2 or over 6 lands in commander) |
| `land_drops` | hit-all-N rates, screw (≤2 by t4), flood (6+ lands in hand + on the battlefield at end of t4 — drops made are the wrong lens), flood expectation at the deck's actual draw volume, percentiles |
| `mana_base` | lands/rocks/dorks/ramp-spells/total sources + the bracket target band (see below) + a verdict sentence; top level `mana_base_bracket_inferred` says when the bracket was inferred |
| `commander` | castable-by-turn curve, p50/p95/avg first cast turn, on-curve share |
| `station` | commander spacecraft animated by t6 + p50 online turn (null when not a station card) |
| `bodies_by_turn` | creatures + animated spacecraft + ETB tokens in play |
| `engines_online_by_turn` | repeatable draw engines active |
| `mana` | unspent mana per turn, share of games floating 3+ by t6 |
| `draw` | games with no draw source by t6 (starvation); `pct_seen_by_turn` = share of games with a draw-role card in hand (hand visibility, not engines online) |
| `role_access` | share of games with the role seen in hand: removal by t5, draw by t6, creature by t3, wincon by t8, lock by t3 |
| `velocity` | cumulative cards seen per turn (milled and looted cards count) |
| `library_awareness_by_turn` | share of the library evaluated per turn (drawn + milled + scried/surveiled). Scry/surveil give zero draw credit — a looked-at card is not a drawn card |
| `self_milled_by_turn` / `opp_milled_by_turn` | mill split by direction: graveyard fuel vs deck-out pressure ("target player mills") |
| `library_remaining_by_turn` | average library size (deck-out proximity) |
| `combat` | attack power per turn + p90 by t8 (a power curve, never a kill estimate); attackers + evasion census (trample/flying/menace) |
| `wincons` | life drained per turn (burn/drain engines; ×3 for "each opponent" in commander, ×1 in 60-card formats), extra-turn share, win-threshold engines (Darksteel Reactor class: pct + p50 online turn), planeswalker ultimate online pct, infinite-mana suspicion pct |
| `interaction` | P(interaction in hand AND affordable with spare mana) per turn — instant-speed copies, spare mana while ready ("mana held"), instant vs sorcery by copy count. **Capacity, not events**: no opponent event is claimed |
| `color_screw` | per-color share of games with a pip-blocked cast (WUBRG) |
| `pip_blocks` | top card×color offenders: which card's cast was pip-blocked, worst 5 |
| `graveyard` | average graveyard size per turn (mill + discards − returns) |
| `card_castability` | per distinct card: share castable on-curve, avg first castable turn |
| `combo_access` | (`--combo "A + B"`) share of games with both pieces seen in hand by the pair's target turn |
| `combos` | store-backed Spellbook assembly: complete combos with assembly rates + one-card-away near-misses (`--combo-limit` caps each list, default 20) |
| `hypgeo` | (`--hypgeo`) exact hypergeometric cast-on-curve ceilings: an upper bound on the real cast rate (lands × drawn). The sim's castability is draw-agnostic, so it naturally sits at or above its ceiling — the two answer different questions, not one scale |

`mana_ready` (the castability curve) is draw-agnostic: it asks when the
board could first pay each cost, independent of whether the card was
drawn. It measures the mana base; the hand adds the draw dependency.

### Mana-base target bands

`mana_base` compares the deck's counts against research-derived bands
(EDHREC average decks, 46 average decks across 11 commanders, fetched
2026-09; Sam Black's cEDH land-count guidance in the Commander's Herald).
A deck outside its band gets a "trim/add lands" verdict — the same signal
an AI deckbuilding agent sees, so "add lands" is never the only lever.

| Bracket | Lands | Ramp (rocks + dorks + ramp spells) |
| --- | --- | --- |
| 1–2 (casual) | 34–40 | 7–12 |
| 3 (upgraded) | 33–38 | 8–11 |
| 4 (optimized) | 32–36 | 9–12 |
| 5 (cEDH) | 25–31 | 10–16 |

Without an explicit `--bracket` the bracket is inferred from the Game
Changer census (0 GC → 2, 1–3 → 3, 4+ → 4); `mana_base.bracket` reports
it and `mana_base_bracket_inferred` (top level) marks the inference.

Lands-matter decks (an extra-land-drop engine on the board) widen the
land band by 4 and the verdict says so. Calibration anchors: a 35-land
deck with 9 rocks reads "on target" at bracket 3; a 44-land deck reads
"trim 6 lands"; a 44-land deck floods in ~35% of games (expectation
~35%), while the old drops-made detector read it as 0.0%.

### Findings (exit 1)

| Kind | Trigger | Suggestion pattern |
| --- | --- | --- |
| `mana_screw` | ≥20% of games ≤2 lands by t4 | magnitude-scaled "add {N} land slots" — or "add two-mana rocks" when the deck has fewer than 6 nonland ramp sources (rock-heavy decks must not read as land-screwed) |
| `mana_flood` | rate exceeds its velocity-adjusted expectation by >10pp | magnitude-scaled "trim {N} land slots"; the expectation uses each game's actual cards seen by t4, so cantrip decks compare against their real window (a fixed 11-card window reads draw-heavy decks as floodier than they are). A lands-matter deck's note says to check the plan before trimming |
| `commander_late` | <60% castable on curve | "add 2-3 ramp sources" |
| `color_screw` | any color's pips missed in ≥10% of games | "add ~2-3 {COLOR} sources" — or, when choice lands already exist, "swap basics for lands that also tap for {COLOR}"; when the deck runs 10+ ramp sources the suggestion points at the color fixes instead of land counts |
| `draw_starvation` | ≥25% of games see no draw source by t6 | "add 2-3 draw engines" |
| `mana_unused` | ≥2.5 mana unspent on average by t6 | "add cheaper spells or more draw" |
| `dead_cards` | ≥3 distinct non-reactive cards cast on time under 60% | "cut or discount late cards, or add ramp" |
| `category_starved` | removal by t5 <40% / wincons by t8 <40% | "add 2-3 interaction pieces" |
| `interaction_unready` | instant-speed interaction ready by t5 <40% while access ≥40% | "add cheaper instant-speed answers" |

Suggestions name categories and magnitudes, never card names. Exit 0 when
no findings; exit 1 otherwise (the finding is the result, not a crash).

## Validation loop

The standard workflow for any deck edit:

```sh
stm deck simulate <name> --seed 42 --json > /tmp/base.json
# ... apply the edit ...
stm deck simulate <name> --seed 42 --baseline /tmp/base.json
# prints deltas only; exit 1 only when a problem is new
```

Identical seeds reproduce identical shuffles, so a metric delta isolates
the deck change. A batch is working when its target metric moved and no
new problems appeared. **Regenerate the baseline after upgrading `stm`**:
seed compatibility is version-local.

Add `--hypgeo` to any run for exact cast-on-curve ceilings beside the
simulated numbers, and `--combo "A + B"` to measure combo assembly
(share of games with both pieces seen in hand by the pair's target turn).

## Tests

Two test layers cover the simulator:

- `simulator/tests/*.rs` — unit tests per model shape: tap yields (choice
  vs fixed vs any vs colorless vs creature-only), station tiers (single
  tier, two tiers, planets never animate), crew (printed power), ability
  shapes (ETB, upkeep, attack, cast, activations, planeswalker loyalty,
  mill, graveyard return, wheel, loot, sacrifice, death triggers), roles
  (including Lock and Booster; reactive spells classify as Removal),
  enters-tapped (shock duals), verge gates, Leyline openers, cost
  reductions, and game-loop behavior (land drops, screw, mulligan policy,
  commander pip gating, 5c commander with/without any-color rocks,
  crew→station chains, fetch smoothing, same-seed reproducibility, pip
  blocks, graveyard census, sacrifice→death-token flow, restricted mana
  paying creature casts only). New-mechanic tests cover: whenever-ETB
  parsing, landfall trigger family, ETB scry → awareness, token counts
  (bare / word / "for each" cap 8), X-cost classes, per-cast mana,
  kicker, saga chapters, loyalty plus/minus costs, text flying,
  flashback-not-flash, extra land drops, removal-beats-draw role order,
  haste entry-turn attacks, planeswalker loyalty/ultimate flow, X drain
  totals, extra-land-drop ramp, extra-turn replay drops, per-cast mana
  velocity, the free-mana loop cap census, upkeep drain engines,
  constructed ×1 drain, kicker payment, saga chapter payoffs, and token
  body counts. `mechanic_tests.rs` pins the audit-remediation behavior:
  wheel execution, cast-trigger dedupe, commander upkeep drain + no
  double draw, combined-numeral sagas, chapter IV + leave-board, Helix
  X-sink counters, token-count bodies, once-per-turn engines (no
  infinite flag), commander ×3 vs constructed ×1 drain, additional-cost
  consumption, X-entry counters, and the Vivi no-double-count parse.
  Dedicated archetype tests live in the per-format deck test files
  (commander/standard/modern) for every fixture that was previously
  invariants-only, plus degradation-fixture problem assertions.
- `simulator/deck_tests.rs` + `simulator/tests/deck_fixtures/*.json` —
  real tournament lists. Sources: mtggoldfish metagame, cEDH Decklist
  Database, EDHREC, and topdeck.gg competitive tournament standings
  (fetched through the TopDeck.gg API with attribution; commander lists
  from cEDH/casual league events, 60-card lists from Standard/Modern
  tournaments). Fixture filenames use underscores. The sweep tests assert
  simulator consistency properties only, never deck quality: land-drop
  sanity, velocity monotonicity, castability ≥ cost floor, opener
  sanity, and archetype-specific checks (topdeck Murktide early threats,
  Eldrazi ramp never curve-ready, Boros tokens body emergence, Yawgmoth
  combo velocity, Aesi median drops > 4, superfriends walker cast rates,
  graveyard growth for recursion decks).

When you add a modeled mechanic, add both: a unit test for the parse shape
and a game-level assertion that the behavior shows up in the metrics.

## Assumptions and limits

Everything the model cannot execute is dropped at parse time, and the
`assumptions` array in every report lists the current limits:

- **No opponents, no interaction.** No countermagic fires, no removal is
  cast, no attacker is blocked. Removal and counterspells count toward
  role access and the **readiness** metric (in hand + affordable), but
  never fire. Readiness is capacity, not events.
- **Draw engines fire on a fixed delay.** An engine draws its amount once
  per turn from the turn after it enters, regardless of board state. A
  "draw for each artifact" engine draws 1. Commanders add a synthetic
  engine only for unconditional (upkeep/end-step) draw triggers; attack-
  gated and opponent-gated commander draws fire no engine.
- **Mill feeds velocity and the graveyard census, not replay quality.**
  Milled cards count as cards seen and fill the graveyard log; graveyard
  return fires once per card (no recursion chains).
- **Wheels and loot reset the hand.** A wheel discards the hand into the
  graveyard census, then draws seven (trigger wheels fire from their
  upkeep/cast trigger; plain wheel sorceries resolve on cast); loot is
  draw-n discard-n.
- **Spend-restricted mana pays creature casts only.** Secluded
  Courtyard-style lands park their mana in a separate bucket; noncreature
  spells cannot touch it.
- **Sacrifice outlets consume real bodies.** "Sacrifice a creature:
  Add {C}{C}" activations parse (Ashnod's Altar class); the activation
  removes an untapped non-token body and fires its death triggers. With
  no body available the activation does not fire (no phantom mana).
- **Blink re-fires once.** "Exile … return it to the battlefield" ETBs
  re-fire the host's OnEnter triggers the turn after; no chains.
- **Planeswalker loyalty is tracked and spent.** Starting loyalty comes
  from the card row; one loyalty ability fires per turn, gated on
  affordable loyalty — plus abilities gain loyalty, minus abilities spend
  it. Ultimates only flag `ultimate_online`; they do not resolve.
- **No commander recast tax.** A commander destroyed or countered stays
  cast; nothing re-casts it.
- **Body power uses printed power when known.** Crew and station math use
  the card's printed power; tokens and unknowns stay flat at 2.
- **Enters-tapped honored, best-case reads.** Shock duals always pay 2
  life (untapped); "enters tapped unless" clauses that self-solve early
  ("unless you control two or fewer other lands", first three turns) read
  untapped; other conditional-tap texts read tapped.
- **Verge gates approximate.** The sim checks for another land whose name
  maps to the gated basic type; exotic land names outside the known table
  satisfy nothing.
- **Cost cuts are flat at parse, board-scaled for improvise/affinity.**
  Warp uses the cheaper cost; improvise/affinity start at a flat −2 and
  gain 1 more per 4 artifacts on the battlefield, capped at the printed
  generic; they are exempt from the `dead_cards` finding.
- **Opponent-dependent production yields turn 2 on.** Fellwar Stone
  reads as any-color from turn 2, nothing on turn 1.
- **Type-granted lands read as any-color.** "This land is the chosen type"
  lands tap for one mana of any color (the choice is the player's). Static
  grants on other lands (The World Tree) do not model.
- **X-costs pay the leftover pool.** A `{X}` spell with a drain/draw/
  mill/tokens scaling effect converts the whole floatable pool into X at
  cast; the effect scales with it. Spells with unmodeled X effects
  (damage at creatures, X-counters) pay X = 1 and do nothing extra.
- **Kicker pays from spare mana.** Only drain amounts scale with the
  kick; other kicker riders are inert.
- **Per-cast mana engines fire per spell.** Vivi-class "add one mana for
  each spell you've cast this turn" joins the pool once per cast while
  the host is untapped.
- **Free mana engines cap out.** A zero-cost untapped non-tap
  activation repeats until the pass hits 24 activations, then flags
  `infinite_mana_pct` and stops. No engine runs away.
- **"For each" token counts cap at 8.** Go-wide boards stay bounded.
- **Drain resolves ×3 in commander, ×1 in 60-card formats.** "Each
  opponent" and "target player" both read through the format multiplier.
- **Extra turns replay a land drop, a draw, and upkeep engines.** No
  full-turn replay: the cast/attack phases do not run again.
- **Sagas fire parsed chapter abilities.** Chapters with no readable
  effect draw one card instead; the saga leaves the battlefield after
  its final chapter.
- **Once-per-turn activations stay once.** "Activate only once each
  turn" engines fire once per turn and never trip the infinite-mana
  census; unrestricted zero-cost engines cap and flag.
- **Equipment suits one body.** The buff joins only the equipped host's
  attack; two equipments stack on one host only when both are paid.
  Aura buffs ("enchanted creature gets +N/+N") are not modeled.
- **Commander upkeep engines fire real abilities.** Upkeep drain/mill/
  token engines register like the draw engines (a synthetic draw tier
  only fills the gap when the parse produced no upkeep draw — no
  double draws).
- **No energy, metalcraft, ascend, delirium.** Conditional producers and
  discounts keyed on game state do not fire (Mox Opal reads as dead
  colorless in the pool unless its text says otherwise).
- **Proliferate stays one-shot.** "Put N charge counters" spells,
  enter-with-counters, and combat-damage proliferate (one counter per
  fire on the source) work; chained proliferate engines do not.
- **No replay mechanics.** Flashback, rebound, splice, ninjutsu, warp
  beyond the cheaper cost, and cast-from-graveyard chains fire once.
- **Keyword support is goldfish-aligned only.** A keyword models only
  when it moves an existing metric: haste skips summoning sickness,
  combat-damage triggers fire per connecting attacker (draw/proliferate/
  drain), static "+N/+N" board buffs join the attack power, equipment
  buffs join only after the equip cost was paid (tap budget 7d),
  double strike doubles attack power, prowess adds +1 per noncreature
  spell cast that turn, landfall counter-shapes add +1 per later land
  drop while full landfall trigger lines parse as real engines, scry/
  surveil feed awareness only (surveil puts the cards in the graveyard),
  and trample/flying/menace (from keywords or text) count as an evasion
  census with no math. Vigilance is free (attackers never tap). Imprint
  is out of scope.
- **Drain is a census, not a life total.** "Each opponent loses N" /
  "deals N damage to target player" multiplies by 3 in commander (three
  opponents) and by 1 in 60-card formats. No life totals, no racing.
- **Extra turns replay a land drop, a draw, and upkeep engines.** No
  full-turn replay: cast and combat do not run again, no chaining beyond
  the queued count.
- **Win thresholds are checked at upkeep.** "N or more counters wins"
  engines record the first turn the stock reaches N; the sim does not
  declare a win.
- **Loyalty activations are once per turn.** One planeswalker ability
  fires per walker per turn, cheapest first, with no loyalty cost gating
  beyond affordability.
- **Role classification is heuristic.** Counts come from oracle-text
  matching; audit them via `deck_shape` in `--json` before trusting a
  finding.
- **Seed baselines are version-local.** An upgrade may reshuffle
  identically-seeded decks; regenerate the baseline JSON after upgrading
  before diffing.
- **London mulligan bottoms at random.** The 60-card policy redraws and
  bottoms one random card per mulligan taken; no keep-choice evaluation.
  Commander-family decks keep the single free redraw.
- **Combo assembly is a consistency diagnostic.** Store-backed Spellbook
  rows measure how often pieces reach their zones, never whether the
  combo wins; quality labels come from Spellbook data.
- **Exile-zone combo pieces are excluded.** The sim has no exile path;
  variants needing one drop out of the join (counted in
  `variants_considered`).
- **Library-zone pieces read as hand-seen.** 1.6k Spellbook rows use the
  hand-sighting as the proxy; noted, not hidden.
- **Command-zone pieces resolve through the cast.** A `mustBeCommander`
  piece assembles when the commander was cast; the deck provides the
  commander, never a format branch.
- **Solitaire.** The sim never trades resources, never faces a board
  wipe, and never races. Curve and consistency numbers transfer to real
  games; timing numbers are best-case ceilings.

## Extending the model

One documented path for a new mechanic:

1. **Model shape** in `model.rs` (Effect/Trigger/TapYield variant or
   field).
2. **Parser** in `parse.rs`: an oracle-text branch with a unit test.
3. **Game step** in `game.rs`: execute the shape in `apply_effect` or the
   turn pipeline; record census data on `GameLog`.
4. **Metric** in `aggregate.rs` (stats) + `report.rs` (JSON + human).
5. **Docs**: a row in the metrics table, a line in `assumptions`.
6. **Tests**: unit parse test + game-level assertion (see Tests).