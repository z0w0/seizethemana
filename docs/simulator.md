# Deck simulation

`stm deck simulate` measures consistency under a fixed play policy. It checks
land drops, mana payments, card access, commander timing, and supported engines.
It does not predict win rates or set an upper bound on deck speed. Unsupported
rules can make a deck look faster or slower than it is.

Commander-shaped decks use three synthetic opponent seats by default. Other
library shapes use inactive external activity. These seats provide bounded
events and resources, not opponent deck AI. They do not block, counter spells,
or cast removal.

- [Run a simulation](#run-a-simulation).
- [Check supported mechanics and fixed choices](#supported-mechanics-and-fixed-choices).
- [Check outcomes and denominators](#outcomes-and-denominators), then
  [read the report](#read-the-report).
- [Compare runs](#repeatability-and-comparisons) or
  [measure execution cost](#execution-cost-and-benchmarks).
- [Extend rules](#extend-and-test-rules) or
  [maintain source fixtures](#card-sources-and-fixture-maintenance).

## Run a simulation

```sh
stm deck simulate Froggy --runs 10000 --seed 42
stm deck simulate Froggy --turns 12 --threads 4 --batch-size 256 --json
stm deck simulate Froggy --opponents inactive --seed 42
stm deck simulate Froggy --opponent-scenario opponents.json --seed 42
stm deck simulate Froggy --bracket 3 --hypgeo --combo "Card A + Card B"
```

| Option                | Meaning                                                                                                                                  |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `--runs`              | Independent games. Default 10,000; range 100 to 1,000,000.                                                                               |
| `--turns`             | Player turn slots per game. Default 10 for commander-shaped decks and 8 for constructed; range 1 to 30. Extra turns consume these slots. |
| `--seed`              | Master seed. Omission selects a random seed, which the report records.                                                                   |
| `--threads`           | Positive worker limit. `1` runs serially; omission uses the shared Rayon pool.                                                           |
| `--batch-size`        | Positive bound on full game logs delivered at once. Default 256.                                                                         |
| `--opponents`         | `synthetic` or `inactive`. Cannot combine with `--opponent-scenario`.                                                                    |
| `--opponent-scenario` | JSON settings for the external table.                                                                                                    |
| `--format`            | Override the format inferred from deck sections.                                                                                         |
| `--bracket`           | Commander land and ramp target band. Without it, the Game Changer count supplies an inferred bracket.                                    |
| `--baseline`          | Compare against a complete JSON report from a compatible run.                                                                            |
| `--hypgeo`            | Add joint spell-and-land access probabilities under a uniform sampling model.                                                            |
| `--combo`             | Measure access to an explicit pair of card names. Repeat for more pairs.                                                                 |
| `--combo-limit`       | Limit discovered combo rows. Default 20; range 1 to 200.                                                                                 |

Without a baseline, exit `1` means findings exist. With a compatible baseline,
exit `1` means a new finding exists. Neither result is a runtime failure.

## Compiler and runtime boundaries

Rules come from card types, keywords, and Oracle text. Parsing, lowering, and
runtime execution are separate claims:

```text
CardRow: Oracle text, keywords, types, costs, printed stats
    -> oracle_parser -> OracleCard: typed syntax and source spans
    -> oracle_lower -> SimCard: runtime data and ability support evidence
    -> SimDeck + format rules + RunConfig + master seed + game ID
    -> BatchRunner -> run_game_with_config -> GameLog
    -> SimAccumulator + ComboAccumulator + PairAccumulator
    -> SimStats + combo access -> findings -> SimReport -> text / JSON / diff
```

Database reads and terminal output stay outside workers. Each game owns its
mutable state, including `GameState` and `OpponentTable`. The runner delivers
batches in ascending game-ID order, regardless of worker completion order.

| Code                                             | Responsibility                                                          |
| ------------------------------------------------ | ----------------------------------------------------------------------- |
| `oracle_ast.rs`, `oracle_ast/`, `oracle_parser/` | Typed syntax and original wording.                                      |
| `oracle_lower/`, `support.rs`                    | Runtime data and ability-level compiler evidence.                       |
| `model.rs`, `model/`, `deck.rs`                  | Compiled cards, costs, effects, and deck shape.                         |
| `format.rs`, `deal.rs`, `rng.rs`, `runner.rs`    | Format policy, opening hands, seeded sampling, and ordered execution.   |
| `game*.rs`, `game_run/`, `cast_pass/`            | Player state, actions, payments, and spell resolution.                  |
| `resolution.rs`, `opponents.rs`, `opponents/`    | Bounded trigger settlement and shared external resources.               |
| `aggregate/`, `combos.rs`, `pair_access.rs`      | Streaming statistics and card-access measures.                          |
| `report_schema.rs`, `report/`, `report_view/`    | Public fields, provenance, sampling intervals, output, and comparisons. |

Do not implement spells by name. Equal supported Oracle wording must behave
equally after source-name normalization. The narrow name-based exceptions are
basic-land identity and fetch target mapping when card data does not identify
the target pair. Prefer Oracle text for fetch restrictions, costs, and entry.

### Support evidence

`card_support` groups physical copies by name. Ability records retain source
wording, a source byte range when available, and one status:

| Status          | Meaning                                                                                            |
| --------------- | -------------------------------------------------------------------------------------------------- |
| `supported`     | Complete typed lowering, not proof of exact runtime rules.                                         |
| `compiler_only` | Lowering emitted effects, but the ordinary runtime path is not verified.                           |
| `approximated`  | A named simplification applies, such as generic creature tokens or synthetic opponent events.      |
| `unsupported`   | A required cost, condition, event, target, payment, or effect did not lower as a complete ability. |

These records do not cover every static field or interaction. An empty list is
not full support. Read them with `assumptions` and named runtime assertions.
Source snapshots prove inputs; parser tests prove recognition. Neither proves
whole-card execution. Missing card data does not mean a playable zero-cost card;
its source hash and the report's card-data fingerprint can be unavailable.

Typed token syntax does not guarantee printed token behavior. Lowering that
replaces token colors, types, stats, or abilities with a generic body uses the
`generic_creature_tokens` approximation, including for tribal-token syntax.

## Opponent scenarios

The default active configuration is illustrative, not calibrated from real games:

```json
{
  "active": true,
  "land_target": 6,
  "spell_costs": [2, 2, 3, 3, 4, 4, 5],
  "tax_policy": { "capped_spare": 4 }
}
```

A commander-shaped library has three seats; constructed has one. Scenarios do
not change this count. `--opponents inactive` disables scheduled events, not the
table used for player-directed life loss.

Each seat starts with a seven-card abstract hand and a finite library. It tracks
life, lands, creature and artifact counts, unspent sources, and tax spending.
The model does not invent colors or creature subtypes.

Between normal player turns, each active seat:

1. Untaps its sources and draws one card.
2. Reserves resources for its scheduled spell when possible.
3. Spends one hand card to play a land until it reaches `land_target`.
4. From its second turn, attempts one spell at the configured cost. Later turns
   repeat the last entry in `spell_costs`.
5. Emits a cast event only after paying a card and the source cost. Cast triggers
   resolve before the scheduled permanent increases a board count.

The staggered schedule alternates creature spells, noncreature artifact spells,
and other noncreature spells. Card or source shortages skip casts. Extra player
turns delay the next opponent round. Seats at zero life or with empty libraries
stop generating events.

Tax engines share a per-seat source ledger. `"spare"` pays from sources left
after reservations. `{"capped_spare": 4}` also limits optional tax spending
between untaps to four. `"decline"` refuses optional payments. Engines cannot
reuse spent sources. Supported mana cumulative upkeep uses the player's real
sources; failure sacrifices the source.

The parser rejects unknown fields and empty `spell_costs`. To declare land
colors, add `"land_colors": ["WU", "BR", "G"]` for three seats. Use uppercase
`WUBRG` letters and one string per seat. An empty string declares no eligible
color; omission leaves colors unknown. These are production capabilities, not
floating or spare mana.

Fellwar Stone-style production requires an active seat. Declared colors supply
the union from active seats that control lands. Unknown colors supply only
generic-cost credit from turn 2, even with inactive scheduled activity or no
opponent lands. That credit and supported bonuses to it cannot pay colored or
true colorless `{C}` pips. Do not count it as color fixing.

## Player policy and runtime limits

### Opening hands and turns

Commander-family decks redraw once for fewer than two or more than six
land-capable cards, then keep. Constructed decks redraw full seven-card hands
outside the two-to-five-land band, up to six times. They then bottom one card
per redraw: a land at four or more lands, otherwise a spell when available.
A hand kept without a redraw stays at seven. This is a land-count policy.

The turn pipeline is:

1. Untap, resolve supported upkeep effects, then draw. Commander-shaped games
   draw on turn 1; constructed games skip that draw. Upkeep checks all live
   physical sources, not only sources recorded during a cast.
2. In the precombat main phase, advance Saga chapters, play lands, try the
   commander, cast cheap payable spells, and use supported activations. New
   cards or mana can enable another cast pass. Creature taps compete for mana,
   station, and crew. Equipment costs must be paid for its modeled buff.
3. Measure combat potential as if attacks connect. Ordinary combat potential
   does not reduce opponent life. The postcombat main phase has no actions.
4. Resolve end-step effects, then discard high-mana-value cards first to meet
   the hand limit.

Command-zone casts pay colored pips and add `{2}` per prior cast from that zone.
This is a cost rule, not an opponent removal or recast policy. Zero life or a
failed draw from an empty library ends the game at a supported resolution
boundary. An empty library alone, mill, or a failed search does not cause loss.
Damage and counter thresholds are diagnostics, not wins.

### Supported mechanics and fixed choices

Support depends on exact wording, costs, and conditions. The following describes
runtime paths and their limits, not a whole-card support inventory.

| Area             | Runtime behavior and limits                                                                                                                                                                                                                                                                                                                                                               |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Mana             | Choice, fixed, flexible, and colorless production; separate paid modes and supported spend restrictions. Modes share tap state; an explicit untap can allow another activation. Unknown mandatory costs disable that mode.                                                                                                                                                                |
| Costs            | Colored and colorless pips, hybrid and Phyrexian payment, supported additional costs, convoke, improvise, delve, affinity, and flat spell-cost reducers. Twobrid uses one matching colored mana or two generic mana. Snow payments and variable or conditional static cost reducers stay unsupported.                                                                                     |
| Lands            | Supported entry conditions, life payment, fetch targets, landfall, and additional land plays. Stated extra-land counts stack across live sources and effects for the current turn. Landfall mana depends on a live resolution pool.                                                                                                                                                       |
| Draw and library | Shared draw operations, dredge replacement, mill, filtered search, cycling, cascade, loot, and wheels. Board-scaled draw controls use synthetic runtime data; they do not establish support for the corresponding printed wording. Selection remains a fixed policy. Cascades do not chain; wheels draw seven.                                                                            |
| Graveyard        | Narrow targeted returns, flashback, and the three-card Escape subset move cards and spend resources. Other printed Escape counts stay inert. Filtered and forced-tapped return wording remains limited. Access telemetry records first visits rather than full zone histories.                                                                                                            |
| Tokens           | Treasures use a payment bank, not battlefield artifacts. They do not count for artifact totals, costs that require a battlefield artifact, or token entry and sacrifice events. A committed mana payment consumes them. Most creature tokens use generic bodies, commonly 2/2, with capped counts. Exact token colors, subtypes, stats, and abilities are not general runtime guarantees. |
| Permanents       | Supported entry, upkeep, cast, landfall, attack, combat-damage, death, and opponent events; typed counters; station and crew; Saga chapters; loyalty activations. Unsupported conditions stay inert. Ultimates report affordability rather than resolving.                                                                                                                                |
| Damage           | Player-directed damage and life loss retain supported target scopes. Creature damage, ambiguous targets, prevention, and replacement effects remain limited or inert. Combat has no blockers.                                                                                                                                                                                             |

Additional payment and choice rules:

- Simple one-mana land taps keep painless `{C}` separate from painful colored
  modes. The pilot checks complete visible commander and hand mana payments,
  including eligible ready one-mana creatures. For that payment, it prefers less
  damage, then fewer new mana activations. It rechecks the real pool after each
  planned painless activation before choosing painful mana. It applies damage
  only when it uses a painful mode. Convoke,
  improvise, and delve share the actual cast resource checks. One permanent
  cannot supply mana and a virtual tap payment in the same plan. Eligible pitch
  alternatives include mandatory mana without consuming hand cards during
  planning. The ordinary mana route remains available. The plan does not reserve
  a cast or change already-floated mana.
- For supported any-color grants, the pilot chooses the unrestricted one-mana
  tap ability. It shares native modes' tap state, but not their extra costs,
  conditions, or spend restrictions. Creature tap rules still apply. Convoke
  matches each eligible creature once against generic or allowed colored symbols,
  including hybrid. Convoke and improvise cannot spend the same creature twice;
  improvise pays only generic costs. Affinity does not tap artifacts.
- Live flat spell-cost reducers can match all spells, a union of card types, or
  a union of colors. Matching uses front-face data; hidden faces and face-down
  sources do not grant their printed reductions. Reductions stack and remove
  only generic mana after increases, including commander tax and the chosen X.
  They do not pay colored or true colorless pips, or replace nonmana costs.
- Supported mana-paid X spells and charge-counter X activations choose the
  largest payable X before payment. Every repeated X uses that value and adds
  to the cost. X can be zero; casting without paying the mana cost uses X = 0.
  Unsupported variable effects and unknown counter kinds remain inert. Warp's
  cheaper cast is disabled because its linked exile and later cast rules are
  unsupported; normal printed-cost casting remains available.
- Landfall grants no automatic power bonus. Pump needs a supported printed effect.
- A land/spell double-faced card uses its land face when no ordinary land is
  available. Entry checks use that chosen face. Other-land thresholds exclude
  the entering land and hidden faces; opponent thresholds use active seats.
  Forced-tapped entry does not pay optional life for untapped entry.
- Checkland entry and Verge mana gates use basic land types on exposed physical
  battlefield lands. Either named type can qualify, including on a tapped
  nonbasic land. Hand cards and hidden faces do not qualify. The entering land
  cannot satisfy its own entry check. A Verge can use its own types after entry.
- Reveal-to-battlefield and Descend checks use the complete front-face type.
  Hidden faces do not supply eligibility, buffs, or hand-limit flags. Face-down
  permanents do not grant printed hand-limit flags. Sea Gate Restoration's
  lasting no-maximum-hand-size effect remains unsupported.
- Life-funded cycling earns draw credit only for a successful draw, not a
  terminal payment, failed draw, land search, or dredge replacement.
- Scry and surveil choose only among exposed cards. Scry reorders kept cards and
  bottoms rejected cards. Surveil mills rejected cards and keeps the order of
  the rest. Supported Ponder wording reorders its exposed cards and shuffles
  when all are rejected. Supported Brainstorm wording draws before returning
  known hand cards to the library top. These returns are not draws or discards.
  The policy prefers lands until hand and battlefield lands total five, then
  favors cheap spells. Its rough budget is battlefield lands plus one, not an
  exact mana-payment check. It does not inspect hidden library order.
- Loot completes all draws before choosing any discards. Unequal draw and
  discard counts remain separate. Supported draw-and-self-life-loss effects
  preserve both effects and their order; the life loss is not a casting cost.
- Searches use the player's library and supported destination-controller rules.
  Filters can use front-face types, basic-land restrictions, and subtypes. Color
  filters use raw stored colors from single-face rows and reject merged
  multiface candidates. Printed shuffles use the library random stream, even
  after failed searches. Supported single-result top tutors place the chosen card
  after their shuffle.
  Multiple-result searches retain typed filters, result counts, and destination
  capacities. They select distinct physical cards in reverse library order,
  taking the first eligible cards without ranking quality. Results fill printed
  destination capacities in order, then apply a printed trailing shuffle. Cards
  returned to the library join that shuffle. All battlefield results use the
  pre-entry board and enter in one batch before entry triggers are captured.
  Foreign sources, extra source zones, changed controllers, and bounded searches
  with unmodeled bottom placement stay inert.
  Only named land-type cycling, such as Forestcycling, uses shared search and
  shuffle. Basic landcycling and untyped landcycling stay inert.
- Activated graveyard targets bind before payment. A creature sacrificed as a
  cost cannot become the action's new target. Targets retain their graveyard-entry
  generation, so leaving and reentering does not substitute a new target.
  Supported returns can put creatures onto the battlefield under normal entry
  rules or return any card, including lands, to hand. The pilot chooses the
  newest eligible entry. Other filters, forced-tapped returns, and nontargeted
  resolution-time choices remain limited or inert.
- Temporary flashback grants bind to a card index and graveyard-entry generation
  and expire at the next turn. Supported Escape pays mana and exiles three other
  graveyard cards; other printed exile counts stay inert. Escape uses the normal
  resolution destination, not flashback's exile destination.
- Supported sacrifice costs preserve "another" restrictions and can use
  eligible tapped or untapped creatures, including tokens. A source can pay its
  own cost only when allowed, and only once. Missing required targets or eligible
  creatures prevent payment.
- Lathril's damage-scaled Elf tokens are not exact, and its tap-ten-Elves
  activation is unsupported.
- Sagas use their parsed chapter count; unknown chapters stay inert without
  automatic sacrifice. Zero-loyalty activations share the per-turn loyalty
  guard; a zero mana cost alone does not create that restriction.
- Prowess adds a separate temporary power bonus to each permanent with the
  keyword when a supported noncreature spell is cast. Combat, Ferocious, crew,
  and station use the shared current-power value. Later sources get no bonus for
  earlier casts. Bonuses expire after successful cleanup, before opponents act.
- The life-loss reveal policy makes its first reveal when a library card exists,
  then repeats only while visible life exceeds the deck's largest mana
  value. It does not inspect the next hidden card; the first reveal can be lethal.

Synthetic runtime tests for board-scaled draw do not establish executable
lowering for the printed wording. That executor accepts configured counts,
including zero, without an eight-card ceiling. Likewise, a supported search
selector or destination does not certify Kinnan's full search instruction.

### Behold

The rules reference is rule 701.4 in the
[September 25, 2026 Comprehensive Rules](https://media.wizards.com/2026/downloads/MagicCompRules%2020260925.txt).
Supported singular choices reveal a matching hand card or choose a matching
controlled permanent. The typed quality retains its required types and subtype.
The choice does not consume, tap, or exile that object. Tapped permanents can
qualify. A pending casting card cannot reveal itself.

Supported Behold-or-pay costs add the printed generic mana when no matching
choice exists. Supported optional Treasure rewards remain conditional on a
legal choice and use the Treasure payment bank, not physical artifact tokens.
Elven Passage's supported search and untap instruction keeps the exact found
permanent identity. It cannot untap a different land if that result leaves the
battlefield. Theorist's Sanctum checks its supported Behold entry choice before
entering; a forced-tapped destination still takes priority.

Counted Behold choices, Champions of the Perfect's mandatory exile cost, unknown
conditional rewards, and Kindle the Inner Flame's compound flashback cost stay
inert. Source presence does not imply that these clauses execute. Native mana
can still work on a utility land whose Channel, combat, or opponent-facing
effects remain unsupported.

### Safety limits and trigger settlement

An action sequence runs at most 24 passes; an activation pass runs at most 24
activations. Hand and graveyard cast passes stop at 24 committed casts. These
limits mark a game incomplete only when supported work remains. One token effect
creates at most eight objects; uncreated requested objects also mean incomplete.
A capped positive-mana path can set a suspected-infinite-mana diagnostic. Neither
the cap nor the diagnostic proves a legal infinite combo.

Supported triggers capture source identity, counters, and the ability's position
in the compiled definition. They settle at the boundary of the causing action,
not through a full stack model. Each settlement stops after 1,024 queued actions.
If work remains, the game becomes incomplete and pending work is cleared.

## Outcomes and denominators

`outcomes` reconciles every requested game:

| Field                     | Meaning                                                                              |
| ------------------------- | ------------------------------------------------------------------------------------ |
| `requested_games`         | Games scheduled for the run.                                                         |
| `finished_games`          | Games reaching the horizon or a supported player-loss boundary.                      |
| `player_loss_games`       | Supported losses, included in `finished_games`.                                      |
| `incomplete_games`        | Games stopped by a safety limit with unresolved work.                                |
| `incomplete_reasons`      | Counts by limit category.                                                            |
| `incomplete_observations` | Grouped evidence by typed reason, turn, phase, configured limit, and remaining work. |

`finished_games + incomplete_games = requested_games`. Finished does not mean
won or survived the horizon. Public reports aggregate terminal evidence rather
than expose every game's terminal log.

Summary categories are `cast_pass_limit`, `graveyard_cast_pass_limit`,
`effect_work_limit`, `trigger_settlement_limit`, `turn_action_limit`, and
`activation_pass_limit`. Nested observation reasons have `kind`,
`configured_limit`, and `remaining_work`. Their typed names, such as `cast_pass`
and `effect_work`, omit `_limit`. Remaining work can be exact or a lower bound.
A capped effect is incomplete, not a completed smaller effect.

Primary event rates use finished games, including losses, and exclude incomplete
games. This selection can bias results in either direction; more runs do not
remove it. Compare incomplete counts and reasons before comparing rates.

Per-turn means use finished games that completed the relevant phase.
`phase_observations` provides `draw_by_turn`, `main_by_turn`, `combat_by_turn`,
`end_step_by_turn`, and `end_turn_by_turn`. Main observations support mana and
readiness; combat observations support attack and commander potential; end-turn
observations support board and library measures. Losses can leave earlier phases
observed and later phases unobserved. The engine does not repeat states or pad
later turns with zeroes. Mean maps omit unobserved keys; fixed-turn optional
values can be null. Neither means an observed zero.

Whole-game access and threshold rates still divide by finished games. A loss
before an event is a non-event, not incomplete. Suspected-infinite-mana and
positive-loop diagnostics cover all requested games, including incomplete games.
Use each rate's `sampling.observed_games`, not an assumed `runs` denominator.

## Read the report

`report_schema.rs` defines JSON schema version 3. Percentages use a 0 to 100
scale. Counts and mana amounts are not percentages. Optional mechanic blocks
are null or omitted when not applicable.

| Measure                                                       | Interpretation                                                                                                                                                                                    |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `deck_shape`, `mana_base`, `colored_sources`, `color_sources` | Static roles, source counts, target bands, and weighted source audits.                                                                                                                            |
| `outcomes`, `phase_observations`                              | Finished, lost, incomplete, and completed-phase counts. Check before reading rates.                                                                                                               |
| `opening_hand`, `land_drops`                                  | Opening lands, redraws, land plays, and flood diagnostics. Flood expectation is a uniform-sample reference, not an exact prediction after selection or draw-dependent sampling.                   |
| `commander`, `card_castability`                               | Mana readiness and timing within each field's observation window. Payable does not mean drawn or cast.                                                                                            |
| `draw`, `role_access`                                         | Card access, including first hand arrival. Access to a draw card does not mean an active engine.                                                                                                  |
| `velocity.avg_cards_seen`                                     | Kept opening hand plus draws, self-mill, searches, and modeled reveals. Repeat visits can count again. Scry adds awareness, not draw or hand-arrival credit. This is not hand size or just draws. |
| `repeatable_sources_by_turn`                                  | Sources counted online under runtime timing and conditions.                                                                                                                                       |
| `mana`                                                        | Pre-action capacity, cast payments, and remaining pool. Cast payments exclude activations and non-mana resources. Eligible Treasures add capacity; unspent Treasures are not unused floated mana. |
| `interaction`                                                 | An instant-speed answer still in hand and payable after main actions. This is capacity, not resolved removal or a decision to hold mana.                                                          |
| `combat`, `win_conditions`                                    | Potential unblocked combat, supported player-directed effects, and threshold access. `p50_lethal_turn` is conditional on reaching the diagnostic threshold, not a win rate.                       |
| `combos`, `win_paths`, `combo_access`                         | Piece access, not legal execution. First zone visits need not coincide. Exile requirements and untracked command-zone movements are excluded.                                                     |
| `findings`                                                    | Typed diagnoses with stable identities, evidence, and category-level suggestions. Check support for key engines before acting.                                                                    |

### Potential and applied damage

Ordinary combat potential does not reduce opponent life, eliminate seats, or
declare wins. Supported noncombat damage and direct life loss can change seat
resources. The table-life threshold combines modeled damage and direct life
loss against starting table life; it does not prove legal lethal assignment to
each opponent. Read `p50_lethal_turn` with `damage_threshold_games` and its share.

`combat.commander_potential` has one row per commander slot, or an empty list
without commanders. Partners stay separate. Each row contains:

- `slot` and `name` for identity.
- `damage_by_turn` and `cumulative_damage_by_turn`: mean potential for games
  completing combat on that turn.
- `games_reaching_21` and `observed_games`: events and finished-game denominator.
- `percent_games_reaching_21`: event share, or null without finished games.
- `median_turn_to_21`: median among games reaching 21, or null if none did.

Commander potential uses supported current power, counters, buffs, equipment,
and double strike. It excludes noncombat damage, attack-trigger damage, and
direct life loss. There is no blocker choice, defending-player assignment, or
per-opponent commander-damage ledger. Reaching 21 is potential, not opponent loss.

### Provenance and uncertainty

`provenance` records schema, model, policy, random-number, and engine versions.
The schema version is 3; the model and policy versions are 2.
It records deck contents separately from execution order. A null card-data
fingerprint does not verify equal Oracle inputs.

`opponent_model` records settings, seat count, and schedule version. Its
`observed_activity` reports draws, casts, successful optional tax payments, and
skipped casts from the shared ledger. Payments count decisions, not mana spent
or per-engine benefits. These are not estimates of real-world activity.
`attempted_games` and `raw_totals` include all requested games; `observed_games`,
`totals`, and `per_game` use finished games, including losses.

`sampling` gives event counts and 95% Wilson score intervals for selected rates.
Statuses distinguish `observed`, `outside_horizon`, `not_applicable`, and
`no_observations`. These records do not cover every value. Intervals use the
stated denominator and exclude error from unsupported rules, policy, and
opponent assumptions. More runs reduce sampling noise, not model error.

### Hypergeometric access baseline

`--hypgeo` computes the joint probability of at least one spell copy and enough
lands in a uniform sample without replacement. Required lands use modeled
`card.cost.total()`, not printed mana value. Unresolved X is zero; split cards
can use their modeled cheaper face. The sample is seven cards plus one draw
per turn. Target turns outside the horizon are omitted.

This excludes mulligans, selection, tutors, extra draws, ramp, colored pips,
land timing, and land faces of double-faced spells. JSON
`percent_modeled_access` is this joint access probability, not a general
cast-rate or mana-readiness ceiling.

## Repeatability and comparisons

Each game has separate ChaCha8 `Opening` and `Library` streams derived from the
master seed and game ID. Initial shuffles and mulligans use `Opening`; runtime
library shuffles use `Library`. Worker count and batch size do not affect seeds.
SHA-256 seed derivation and shuffling have a versioned contract. Pilot and
opponent choices are deterministic, not random domain actions.

`stm deck hand --seed 42` shares the simulator's game-ID opening streams under
the same deck and format rules. Its advice checks individual spell payments
from known lands by turn 2, including supported colors and entry timing. It
does not plan ramp, fetches, future draws, or extra spell costs. Individually
payable spells need not be payable together. Hand JSON records `redraws` and
`bottomed_cards`.

Save a complete report before changing a deck:

```sh
stm deck simulate Froggy --seed 42 --json > before.json
# Edit the deck, then compare under the same settings.
stm deck simulate Froggy --seed 42 --baseline before.json --json
```

Compare complete reports from the current simulator with seed, runs, horizon,
format, and scenario fixed. Baseline comparisons require matching schema,
model, policy, RNG, engine, format, and opponent scenario. The same seed does not
give different decks identical card sequences; deck order also matters.

## Execution cost and benchmarks

Production accumulates each batch, then releases full logs. Batch size bounds
full-log retention, but exact percentiles retain scalar samples proportional to
`runs`. Total memory is not constant. Log-collecting benchmark callers retain
the whole workload.

Use release builds for timing. `simulator_bench` uses local fixtures and separates
compilation, dealing, aggregation, pair and combo access, report construction,
serialization, retained-log games, and the core streaming report pipeline.

```sh
STM_SIM_BENCH_FIXTURES=mono-red STM_SIM_BENCH_MODE=phases \
  cargo bench --bench simulator_bench -- 'phases/mono-red' --quick
STM_SIM_BENCH_FIXTURES=mono-red STM_SIM_BENCH_MODE=scaling \
  STM_SIM_BENCH_RUNS=1000 STM_SIM_BENCH_TURNS=12 \
  cargo bench --bench simulator_bench -- 'scaling/mono-red/t12/1000' --quick
```

These smoke runs can reveal obvious cost changes, not establish speed claims.
Select the matrix with:

| Variable                 | Values and default                                                                                     |
| ------------------------ | ------------------------------------------------------------------------------------------------------ |
| `STM_SIM_BENCH_MODE`     | `all` (default), `phases`, `scaling`, `pipeline`, or `validate`.                                       |
| `STM_SIM_BENCH_FIXTURES` | Comma-separated `lathril`, `mono-red`, `ezuri`, `tokens`, `storm`, `graveyard`, `combo`. Default: all. |
| `STM_SIM_BENCH_RUNS`     | Positive comma-separated counts. Default: `100,1000,10000`.                                            |
| `STM_SIM_BENCH_TURNS`    | Positive comma-separated horizons. Default: `12,20`.                                                   |
| `STM_SIM_BENCH_WORKERS`  | Positive comma-separated counts. Default: available values from `1,2,4` plus available parallelism.    |
| `STM_SIM_BENCH_BATCHES`  | Positive comma-separated sizes. Default: `256`.                                                        |

The `storm` selection loads `ruby-storm.json` as a ritual and cantrip workload.
That fixture has no spell with the Storm keyword. It does not exercise Storm-copy
loops.

`phases` uses 100 games, 12 turns, and the first selected worker count and batch
size. The run/turn matrix applies to `scaling`, `pipeline`, and `validate`.
`scaling` retains logs; `pipeline` consumes batches and builds a report. Both
include local worker-pool setup. `all` runs `phases`, `scaling`, and `pipeline`,
not `validate`.

The core pipeline excludes database loading, stored Spellbook combo lookup,
combined explicit-pair and stored-combo analysis, `--hypgeo`, and terminal
rendering. Pair and synthetic combo access have separate phase benchmarks.
Neither the core pipeline nor `end_to_end_100` measures the full CLI.

Run validation separately. It compares every indexed log, retained-log report,
and core streamed report across selected workers and batches:

```sh
STM_SIM_BENCH_MODE=validate STM_SIM_BENCH_RUNS=100 \
  STM_SIM_BENCH_TURNS=12 STM_SIM_BENCH_WORKERS=1,2 \
  STM_SIM_BENCH_BATCHES=64,256 cargo bench --bench simulator_bench
```

Before each before/after timing run, validation must pass with that run's exact
fixture, run, turn, worker, and batch selections. The smoke example does not
validate a larger timing matrix. For longer comparisons:

```sh
STM_SIM_BENCH_MODE=all cargo bench --bench simulator_bench -- \
  --save-baseline before --sample-size 50 --warm-up-time 3 --measurement-time 10
STM_SIM_BENCH_MODE=all cargo bench --bench simulator_bench -- \
  --baseline before --sample-size 50 --warm-up-time 3 --measurement-time 10
```

Keep Criterion output under the same target directory. Do not run before/after
workloads concurrently. Repeat noisy or borderline results. Profile a focused
pipeline separately:

```sh
STM_SIM_BENCH_MODE=pipeline STM_SIM_BENCH_FIXTURES=lathril \
  STM_SIM_BENCH_RUNS=1000 STM_SIM_BENCH_TURNS=12 \
  STM_SIM_BENCH_WORKERS=1 STM_SIM_BENCH_BATCHES=256 \
  cargo bench --bench simulator_bench -- \
  'pipeline/lathril/t12/1000/workers1/batch256' --profile-time 10
```

Criterion profiling supplies a repeated workload. Use a platform profiler for
CPU stacks or peak memory. Record revision, fixture hashes, seed, horizon, runs,
workers, batch size, scenario, build profile, hardware, and timed boundary.
The suite prints fixture identity and uses seed 42 with default format
opponents. Its untimed observation run prints `outcomes` and
`phase_observations`; process-wide profiling can include that work.

Synthetic adjacent-name pairs measure access, not verified combos. Compare
finished, lost, incomplete, and phase counts as well as time. Earlier stops
mean less work, not necessarily an optimization. Speed claims require equal
rules and inputs.

## Extend and test rules

Add typed syntax and source evidence, lower it to explicit data, then execute
that data. Keep Oracle parsing out of the turn loop. Leave abilities inert when
a required cost or condition cannot execute. Update support evidence and report
assumptions with the rule.

Tests must separate recognition from execution. Add parser and lowering tests,
then assert payments, zones, timing, and effects through production actions.
Include equal wording under a renamed source and a nonmatching control.
Report claims must follow executed actions through census, aggregation, and
report construction. Direct helpers and hand-built logs can test isolated logic,
but do not prove legal casts or whole-card execution.

After changes, run `cargo nextest run`,
`cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`.
For supported code changes, run `aislop scan --changes` and a full `aislop scan`;
both must report no errors or warnings.

## Card sources and fixture maintenance

### Expanded Commander sources

[`commander_card_fixtures/`](../src/deck/simulator/tests/commander_card_fixtures/README.md)
holds separate October 30, 2026 captures with 1,208 unique Oracle identities:

- The first 1,000 EDHREC "Past 2 Years" cards in site order, from `top/year.json`
  and `top/year-past2years-1.json` through `top/year-past2years-9.json`.
- 185 distinct added lands from the complete cycles and utility groups in
  `land_cycles.json`, including all five Neon Dynasty Channel lands and all five
  original Kamigawa legendary lands.
- 23 additional cards from `keyword:behold legal:commander`, including Elven
  Passage, in a separate Behold supplement.

The two captures preserve ten EDHREC responses, the Behold query response, and
17 Scryfall collection request/response pairs. Manifests record URLs, capture
times, and raw-byte SHA-256 hashes. Source gates check those hashes, pagination,
selection order, and returned identities. Site order is not Scryfall
`edhrec_rank`, and the saved window has no exact published date bounds.

The exact EDHREC selection includes One, Borg Evolved, whose
captured Commander legality is `not_legal`. Do not describe all 1,208 sources as
Commander-legal. The 23-card supplement has its own explicit legality filter.
Neither count measures runtime support.

[`commander_corpus_tests.rs`](../src/deck/simulator/tests/commander_corpus_tests.rs)
checks compilation, source identity, physical-card conservation, one full turn
from empty and established boards, and report construction. This is a broad
smoke check, not whole-card certification. Keep source presence, compiler
records, named runtime assertions, approximations, and exclusions separate.
Focused rule tests, such as
[`commander_land_entry_tests.rs`](../src/deck/simulator/tests/commander_land_entry_tests.rs)
and [`commander_behold_tests.rs`](../src/deck/simulator/tests/commander_behold_tests.rs),
state narrower executed branches and controls.

Use `scripts/capture-commander-fixtures.sh` only for an explicit network capture
into a new directory. Its land-selection and `--behold` modes remain separate.
The `fixture_capture` Rust test target replays saved responses offline to check
capture behavior and destination safety. See the
[fixture README](../src/deck/simulator/tests/commander_card_fixtures/README.md)
for commands and review rules. These captures do not replace historical pins.

### Popular-card sources

`src/deck/simulator/tests/popular_card_fixtures/` holds the historical source
bundle.
Its October 23, 2026 EDHREC capture selects 250 cards in source order under
"Past 2 Years". This is a dated selection, not today's top 250. Source position
is not an explicit rank; the saved page does not establish exact window dates.
Historical deck inputs, mechanic targets, and separate October 29 captures
extend its source registry to 356 identities. The 250-card review, historical
bundle, and compiler pins remain separate from the expanded Commander captures.
Source identity counts are not supported-card counts.

The bundle keeps distinct evidence:

- `sources.json`, `sources/`, and `integrity.json` preserve responses, URLs,
  capture dates, and declared hashes. `provenance.json` records current raw-byte,
  canonical JSON, metadata, Oracle-text, and flattened-input hashes. Offline
  source gates check links, Oracle IDs, pagination, and first-250 selection.
- `corpus.json` pins metadata and Oracle-text hashes. Compilation uses production
  face flattening. `lathril.txt`, `lathril.json`, and `lathril_rows.json` preserve
  the selected deck, physical order, and stored rows separately from later captures.
- `compiler_inventory.json` records syntax and lowered data, not execution.
  Audit files and named tests track rule obligations, approximations, controls,
  production actions, and executed-turn reports. Read each assertion's evidence
  type and scope. Neither a source sweep nor a test registration certifies a
  whole card.

The claim gate executes callback bindings and writes scoped evidence:

```sh
STM_CLAIM_INVENTORY=target/card-claims.json cargo nextest run \
  -E 'test(maintained_claim_inventory_preserves_published_branches_and_regression_links)' \
  --success-output=immediate
```

This artifact is a test result, not a source fixture or compiler pin. Keep
source-bound assertions, synthetic controls, and unsupported-safety cases
separate. Use named passing assertions to describe coverage.

### Update fixtures

1. Identify cards by Oracle ID and source revision hashes. Keep the name, face,
   source URL, and capture date readable.
2. Preserve older bundles. Add a dated capture for changed wording. Network
   capture is opt-in, never part of ordinary tests.
3. Review metadata and Oracle changes. Keep raw-byte, canonical JSON,
   Oracle-text, and production flattened-input hashes distinct.
4. Add named assertions for each claim about costs, targets, conditions,
   payments, zones, and effects. Add near misses, renamed-source controls, and
   same-name changed-text controls where useful. Synthetic controls do not add
   source identities to coverage totals.
5. Recheck audit findings against the current engine. State remaining gaps and
   approximations; do not promote compiler snapshots to whole-card support.

Ordinary tests compare compiler inventory without writing it. After reviewing
an intended compiler change, update only that snapshot:

```sh
STM_WRITE_POPULAR_INVENTORY=1 cargo nextest run \
  -E 'test(popular_compiler_inventory_matches_pinned_rows)'
```

Review the diff, then rerun without the variable. This does not refresh sources,
integrity manifests, audit conclusions, or runtime evidence. Do not use it only
to hide a failure.

After reviewing a dated source capture, rebuild provenance from saved local files:

```sh
cargo nextest run --run-ignored only -E 'test(write_card_provenance)'
```

Review `provenance.json`, then run `card_source_tests` without ignored tests.
The write does not fetch data or approve wording changes. Register additional
captures in the source helper's `ADDITIONAL_SOURCES`, preserving older captures.

### Fixture metadata

`src/deck/simulator/tests/fixture_metadata/supplement.json` uses pinned sources
and stays separate from historical deck snapshots. Eligible loaders fill only
absent metadata. Explicit empty identities, `*` stats, and other present values
stay unchanged. Stats use the front face; missing front stats do not inherit
back-face values. Metadata records are not execution coverage.

Tests, benchmarks, and supplement construction are offline. Rust tests rebuild
the supplement in memory from a fixed pinned-source list and compare its records
with the saved supplement. They reject missing or conflicting metadata and
preserve the raw source hashes. Ordinary tests never write fixture files.
The builder lives beside the fixture loader in
`src/deck/simulator/tests/fixture_metadata/build.rs`.

To create a separate review candidate, run the ignored maintenance test:

```bash
cargo nextest run --run-ignored only \
  -E 'test(write_fixture_metadata_review)' --no-capture
```

The test prints a new temporary file path and leaves that file for review. It
never replaces the pinned supplement. Review records, provenance, and source
hashes before accepting changes. Never overwrite historical fixtures to hide a
failed comparison.

Capture is a separate network operation. The capture command needs Bash 3.2 or
later in a Git checkout, `git`, `jq`, `shasum`, standard Unix tools, and `curl`
with `--fail-with-body`. Provide a JSON array of 1 to 75 nonempty card names and
a new destination directory with no symlink parents. This example contacts Scryfall:

```bash
capture_review=$(mktemp -d)
capture_review=$(cd "$capture_review" && pwd -P)
printf '%s\n' '["Shorikai, Genesis Engine"]' > "$capture_review/names.json"
bash scripts/capture-fixture-metadata.sh \
  "$capture_review/names.json" "$capture_review/capture"
```

Capture writes `request.json`, `response.json`, and `capture.json`, including
request and response hashes. Inspect `returned` and `not_found` before accepting
contents. Capture rejects existing outputs, dangling symlinks, and symlink
parents. Exclusive creation prevents concurrent replacement. On handled failure,
it removes only its own incomplete output; parent directories can remain.
New captures do not enter the pinned-source list automatically or certify execution.

Run capture regression tests without network access:

```bash
cargo nextest run --test fixture_capture
```

These Unix process tests need Bash 3.2 or later, `git`, `jq`, `shasum`, and standard
Unix tools. Small command shims supply saved responses and controlled failures.
Rust tests check destination safety, retries, exclusive capture creation, exact
source bytes, manifest hashes, and request pacing in temporary directories.
