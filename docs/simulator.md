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
├── mod.rs                         CLI entry point and report assembly
├── format.rs                      Format and mulligan policy
├── model.rs, model/                Executable card and game data
├── oracle_ast.rs                   Typed Oracle syntax tree
├── oracle_parser.rs, oracle_parser/ Oracle grammar and clause parsing
├── oracle_lower/, role_classify.rs AST and card metadata to SimCard
├── deck.rs, deal.rs                Deck construction and seeded draws
├── game*.rs, game_run/, cast_pass/  Game state, turn stages, and casting
├── game_effects/, game_mana.rs      Effect resolution and cost payment
├── aggregate.rs                     Game logs to statistics
├── findings.rs, findings_detail.rs  Typed findings and evidence
├── combos.rs, hypgeo.rs             Combo access and exact ceilings
├── report_schema.rs                 Serialized report types
├── report.rs, report_view.rs        Report construction and rendering
└── tests/                           Co-located tests and deck fixtures
```

The pipeline is Oracle wording → typed AST → executable `SimCard` model →
game execution → typed `SimReport`. Role and interaction classification
is a diagnostic exception: it uses card text to measure access and
readiness, but does not create game effects. Unsupported wording, costs,
or conditions remain inert. `run_game` is pure and driven by one seeded
`ChaCha8Rng`; the same deck and seed produce the same games. Aggregation,
findings, and rendering are separate from game execution.

## Format rules

Per-format behavior lives in one table (`format.rs`), never in branches.
`FormatRules` carries the format key, the library shape
(command-zone singleton vs 60-card), the default turn count, and the
mulligan policy. Commander-family formats redraw once when the opener's
land count leaves the 2–6 band (Karsten's commander model redraws 0–2
and 6–7; the 2–6 band is close and changing it would churn commander
baselines for little gain). 60-card formats play Karsten's London
mulligan: a 7-card opener with 0, 1, 6, or 7 lands redraws once, then
the redrawn hand bottoms one card toward 3 lands (a land when holding
4+ lands — shed the flood — else a spell). Kept hands stay at 7 cards;
only the redrawn hand bottoms. The keep choice is not evaluated; the
bottomed card goes to the bottom of the library. Commander features
(cast loop, engine tier) gate on the deck having commanders, not on the
format enum. An unknown `--format` key plays as generic constructed.

## The card model

Cards are data, not rules. `oracle_parser.rs` reads each card's Oracle text
once, at deck build time, and produces typed nodes in `oracle_ast.rs`.
`oracle_lower/` converts supported nodes into simulator abilities and
effects. The turn loop executes that data. Unsupported wording stays in the
AST and has no game effect.

This is a strict design rule: do not add custom mechanic handling based on
the card name. Parse supported Oracle wording into model data and execute
that data. An unknown card with the same Oracle text should get the same
behavior. Unsupported text stays inert and the model's limits appear in
runtime assumptions. Name-based exceptions are limited to basic-land
identity and fetch-land target mapping when the target pair is absent from
the type line. Fetch activation, life payment, sacrifice, and tapped-entry
rules should come from Oracle text. Prefer parsing the fetch's target types
from Oracle text too; a small name map is only a fallback for target pairs
that cannot otherwise be recovered. Never use a card name to implement a
spell-specific rule.

### Tap yields

A permanent taps once per turn. What one tap yields splits into four
shapes, keyed off the "or" in the text:

| Oracle shape           | Example                                             | Model                             |
| ---------------------- | --------------------------------------------------- | --------------------------------- |
| "or" choice            | `Add {G} or {U}` (shock duals, Verge lands)         | one tap, one mana of either color |
| any-color prose        | `Add one mana of any color` (Command Tower, Signet) | one tap, any color                |
| fixed simultaneous set | `Add {W}{U}{B}{R}{G}` (Jegantha)                    | one tap, all five at once         |
| colorless              | `Add {C}{C}` (Sol Ring)                             | one tap, N colorless              |

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
  produce" (Fellwar Stone) is an intentional goldfish approximation. It
  produces one colorless-only pip from turn 2 on and does not tap on turn
  1. This lets it pay generic costs without inventing an opponent's lands
     or claiming access to colors the simulated player may not have.
- **Scaling.** "For each color among permanents you control" (Faeburrow
  Elder) counts the colors actually on the board; "for each charge
  counter" (Astral Cornucopia) reads one pip per counter.

Two banked shapes:

- **Charge-counter banks.** "Remove a charge counter: add one mana of
  any color" (Pentad Prism) is a banked activation: the source stays
  untapped, fires once per turn while counters last, and one counter
  buys one pip. Sunburst enters with one counter per color paid
  (best-case 2).
- **Treasures.** "Create a Treasure token" effects on the _acting card_
  bank one flexible pip per token (sacrificed to use) instead of
  creating a body. Only Treasure-labeled effects convert; other token
  spells in the same deck still create bodies, and the deck-wide
  fallback (any unsourced token converting when any deck card makes
  Treasures) is gone. Smothering Tithe stays inert — it needs
  opponents.

Static mana grants ("creatures you control have {T}: add one mana of
any color" — Enduring Vitality; "lands you control have…" — Chromatic
Lantern) add one flexible pip per matching permanent per turn, capped
at two per grant, while the source is on the battlefield.

### Station (CR 702.184, 721)

Spacecraft and Planets carry one or two `{N+}` striations. Each striation
means "as long as this permanent has N or more charge counters, it has
these abilities" (CR 721.2a). Only the striation with a printed P/T box
animates the card as a creature; Planets never animate. The parser finds
the animation threshold from the reminder text ("It's an artifact
creature at 12+") and attaches the striation abilities from the
`N+ | ...` segments. The station keyword ability itself (CR 702.184a)
is an activated ability the card always has: any creature may tap to add
charge counters, no matter the current counter count.

Charge counters come from creature taps (the tap budget, below), from
"put N charge counters" spells (Drill Too Deep), and from cards that enter
with counters (Reckoner Bankbuster). Counters persist across turns.
Animation is permanent: once a spacecraft is a creature, it stays one.

### Crew

Vehicles crew with bodies: total untapped body power ≥ the crew cost.
Crew is an activated keyword ability (CR 702.122a): it taps other
creatures, so summoning-sick bodies may crew and station — sickness
blocks only the body's own tap abilities and attacking (CR 302.6). A
Vehicle crewed the turn it entered cannot attack that turn; it becomes a
creature only during its main phase. Crew animation lasts until end of
turn. Bodies are creatures, animated spacecraft, and ETB tokens. Body
power uses the card's printed power when the row carries one; tokens and
unknowns stay at 2.

### Roles

Every card classifies into one role (first match wins):
`Land`, `Rock`, `Dork`, `RampSpell`, `Draw`, `Removal` (also fogs and
regeneration — reactive spells), `Lock` (tax/restrict pieces),
`Booster` (equipment, auras, pump), `Wincon`, `Other`. Protection grants
("gains hexproof") are not Removal: they answer nothing in solitaire.
Roles and interaction census are diagnostics derived from text; they do not enable card mechanics.
Reactive spells (removal, fogs) never fire in solitaire: `role_access`
judges them by hand visibility, and they are exempt from the
`low_castability` finding.

The removal census splits targeted from sweeps. A card with `wipe = true`
("destroy all", "exile all", "return all", "sacrifice all", "-X/-X to each
creature") reports in `deck_shape.removal_wipes`; everything else targeted
reports in `removal_targeted`. Wipes count toward the interaction metric
too: a Wrath in hand with mana is readable capacity.

Interaction readiness matches any instant/sorcery with a destroy/exile/
counter/bounce target shape, any "deals N damage to target …" with N ≥ 2
(player-targeted burn is drain, not capacity), or a wipe. Ability-word
prefixes ("Constellation —", "Raid —", …) strip before trigger matching,
so they no longer hide the trigger underneath.

### Abilities

Each ability has a trigger, an optional cost, and an effect.

Triggers modeled: `Activated` (pay cost, consumes a tap or loyalty),
`OnEnter`, `OnUpkeep`, `OnFirstMainPhase`, `OnEndStep`, `OnLandfall`,
`OnAttack`, `OnCombatDamage`, `OnCastSpell`, and `OnDeath` (dies or is
sacrificed). Opponent-scoped triggers stay inert.

Effects modeled, by variant:
`Draw(n)` — one draw operation, including dredge replacement.
Filtered `Search` — by type, color, mana value, and hand or
battlefield destination.
`Mana(ManaYield)` — unlocked mana activations feed the pool.
`ManaPerCounter(ManaYield)` — first-main release of banked mana:
Coalition Relic's "remove all charge counters, add that many mana".
`Tokens(n)` — become battlefield bodies; "for each" shapes cap at 8.
`Counters(n)` — charge the host.
`ExtraLand`, `Mill(n)` — library top → graveyard census.
`Surveil(n)` — library top → graveyard, with library-mill triggers.
`ReturnFromGraveyard { to_hand, count }` — hand-return = draw credit;
battlefield-return = a body once per card.
`Wheel` — hand → graveyard, draw seven.
`Loot(n)` — draw n, discard n.
`LoseLife(n)` and `Damage(n)` retain the target scope: each-opponent
effects apply once per opponent, target-player effects apply once, and
each-player effects also affect the goldfish. Dredge cards replace a
draw only when the full dredge amount remains in the library. The sim uses
the largest available dredge value, mills exactly that many cards, then
returns that dredger to hand. Each new draw checks the graveyard again.

Library-to-graveyard movement carries its source zone. Oracle triggers that
require a card to enter the graveyard from the library resolve only for that
transition, not for discards, sacrifices, or resolving spells. Parsed
returns put the card onto the battlefield; parsed drain-and-gain effects
exile the card and record damage and player life gain separately.

Activations carry optional costs beyond mana: `sacrifice_bodies` consumes
an untapped body and fires its death triggers (aristocrats outlets);
`loyalty_cost` spends planeswalker loyalty instead of mana;
`loyalty_gain` adds loyalty (planeswalker plus abilities). Planeswalker
abilities fire through the same activation pass: one loyalty ability per
turn, gated on affordability, never on mana. Ultimates stay flags — they
mark `ultimate_online` when loyalty covers the minus cost, they do not
resolve.

Commanders get one extra synthetic ability: when the commander's oracle
shows an _unconditional_ repeatable draw (upkeep or end-step triggers),
the command zone registers a draw-1-per-turn engine from the turn after it
is cast. Attack-gated draws ("Whenever N attacks, draw…") never become
synthetic engines: they need the permanent to animate and attack, which
the normal combat path models. Opponent-gated draw triggers
("whenever an opponent…") register no engine — a goldfish has no
opponents.

Tap yields that reference opponents ("Add one mana of any color that a
land an opponent controls could produce") produce one colorless-only pip
from turn 2. This intentional constraint grants generic mana without
assuming an opponent's board or colors. Kinnan's trigger adds one
colorless pip when this approximate source is tapped. Lands whose oracle
grants them a basic type ("This land is the chosen type") tap for one
mana of any color — the chosen type is
the player's choice each game. Static type-granting on _other_ lands (The
World Tree's "lands you control have {T}: …") is not modeled.

Sagas resolve chapter I when they enter. Later chapters advance one per
turn as the precombat main phase begins (CR 714.3c — before the main-phase
triggers and land drops), so chapter II resolves on the turn after entry. Chapter lines ("I — Draw a card.",
"II — Mill three.") parse into a per-chapter effect list on the card
(`SimCard.saga.chapters`, in play order — chapters are triggered
abilities per CR 714.2, not activations); a chapter with no readable
effect stays unsupported and does nothing. Combined numeral lines ("I, II, III — Create a token")
give every listed chapter the same effect. A four-chapter saga runs its
fourth chapter; once the final chapter resolves, the permanent leaves
the battlefield (it sacrificed in real Magic).

Blink-shaped ETBs ("exile … return it to the battlefield") re-fire the
host's OnEnter triggers once, the turn after (Skyskipper Duo, flicker
engines). Land-search ETBs and Monarch acquisition parse to their own
effects and never arm the blink re-fire: a fetch-style ETB searches
once, and the Monarch is an engine, not a blink. Monarch
acquisition ("you become the Monarch") draws one extra card per turn
from the turn after acquisition (the sim draws it at the end phase,
close enough to the CR 725.2 end-step timing for a goldfish).
Search and extra-land effects add cards to the hand but give no
awareness credit (the card was searched, not seen from the library).

### Keywords modeled (goldfish-aligned)

A keyword models only when it moves a metric:

| Keyword                   | Model                                                                                                                                                           |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Haste                     | hasted creatures skip summoning sickness: they attack, tap, and crew the turn they enter                                                                        |
| Landfall                  | "+1/+1 counter" shapes feed the combat power sum; full trigger lines ("Whenever a land you control enters, draw/create/add") parse as real OnEnter engines      |
| Double strike             | attack power ×2 (no blockers exist)                                                                                                                             |
| Prowess                   | +1 power per noncreature spell cast that turn                                                                                                                   |
| Flying / Trample / Menace | evasion census (no math)                                                                                                                                        |
| Flash                     | instant-speed flag (feeds interaction readiness); "flashback" does not count as flash                                                                           |
| Cascade                   | reveal from the library top until the first nonland card with lower printed mana value; cast it free once, no chaining; misses go to the bottom in reveal order |
| Mobilize N                | attack trigger creates N tapped-and-attacking 1/1 Warriors; they join the swing and are sacrificed at the next end step (death triggers fire)                   |
| Amass N                   | creates or grows one Army body with N +1/+1 counters; Army attack power is its counters                                                                         |
| The Ring                  | temptations raise the emblem level; the Ring-bearer is the strongest body. Level 2 attack-loots; level 4 combat damage drains each opponent for 3               |
| Empower Jace N            | creates or grows a Jace planeswalker token ("[-1]: Surveil 1", "[-3]: Draw a card")                                                                             |
| Saddle N                  | taps bodies with total power N to mark a Mount saddled; a "while saddled" +N/+N buff joins the attack that turn                                                 |
| Power-up / Exhaust        | once-per-game activation; Power-up's cost is reduced by the permanent's mana cost the turn it entered                                                           |
| Teamwork N                | optional additional cost that taps bodies with total power N                                                                                                    |
| Storm                     | the spell copies once per other spell cast this turn; copies resolve the same modeled riders                                                                    |
| Convoke / Delve           | convoke taps creatures for {1} each (or a matching pip); delve exiles graveyard cards for {1} generic each                                                      |
| Offspring / Plot          | offspring pays an extra cost for a 1/1 token copy on entry; plot exiles now for a free cast on a later turn                                                     |
| Explore / Connive         | explore draws a land off the top or adds a +1/+1 counter; connive loots and adds a counter on a nonland discard                                                 |
| Living metal              | during your turn the Vehicle is an artifact creature without crew (it attacks uncrewed)                                                                         |
| Afterlife N               | death trigger creates N Spirit bodies                                                                                                                           |
| Read ahead                | the Saga starts at its first chapter with a parsed effect (best case)                                                                                           |

"Whenever … enters" ETBs parse like "When … enters". Landfall trigger
lines form their own family (draw / tokens / mana-adds read as an
extra-land-style ramp credit rather than a mana tap — the effect lands
in `cards_seen`, not the turn's mana pool; search-for-land reads as
extra land). "You may play an additional land" (Aesi class) grants one
extra land drop per turn while on the battlefield, recorded in
`land_drops[]`.

### X-costs, kicker, and per-cast mana

| Shape                                                                             | Model                                                                                                                                            |
| --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `{X}` spells with a scaling effect (drain/draw/mill/tokens)                       | the cast pays the whole leftover pool as X; the effect scales with it (drain ×N, draw ×N, mill ×N, up to 8 tokens)                               |
| "Reveal the top X cards … put any number of permanent cards onto the battlefield" | the cast converts the leftover pool into X battlefield entries (capped at 8; every library card reads as a permanent)                            |
| "Enters with X +1/+1 counters"                                                    | the entered X counters join the body's attack power (the leftover pool converts at entry)                                                        |
| Kicker / multikicker                                                              | paid from spare mana when affordable; drain amounts scale with the kick; multikicker parses as one kick                                          |
| "Add one mana … for each spell you've cast this turn" (Vivi class)                | the yield joins the pool once per spell cast every turn the host is on the battlefield; the same clause never also reads as a plain one-mana tap |
| Additional costs ("as an additional cost …, sacrifice a creature / pay N life")   | the cast consumes the resource (a real body leaves the battlefield; life is paid)                                                                |
| "Create N …tokens" sorceries                                                      | the cast creates N token bodies (capped at 8)                                                                                                    |

### Turn actions and the loop cap

After each draw or mana action, the sim checks the current hand again.
Newly drawn spells can be cast in the same main phase. Newly cast
noncreature mana rocks tap before the next cast choice. The fixed order is:
land play, commander, cheapest cast, needed creature mana, new mana rocks,
another cast pass, commander if newly payable, then leftover activations.
Casts get another pass after an activation adds mana or cards. When a
cascade spell and a graveyard-creature exchange are available, payable
cycling and landcycling creatures cycle before the cascade choice. Each
permanent taps at most once; a creature tapped for mana cannot station or
crew. The player taps dorks only when the current pool cannot cover the
cheapest spell.

Each main phase runs at most 24 action passes, and each activation pass
runs at most 24 activations. A capped activation pass sets
`win_conditions.percent_games_with_suspected_infinite_mana` only if it added mana. A free draw or other bounded
action does not count as infinite mana. The loop cap protects the simulator
from unsupported repeated effects.

Every draw source uses the same single-card operation. It applies dredge
before drawing, then updates hand visibility and library awareness once.
A dredge replacement counts its milled cards as evaluated and does not
count the returned dredger as a new card seen. Mill effects use one
library-to-graveyard move; Oracle triggers for cards moved from the library
resolve from that event only. Discarding an identical card from hand does
not trigger them.

Cycling pays its parsed mana or life cost, discards that card, then draws
one card through the shared operation. Landcycling pays its cost, discards
that card, and searches for a land with the named basic land type. An alternate cast cost can exile
two green cards from hand. A sacrifice search can find a creature with a
mana value one higher than the sacrificed creature and put it onto the
battlefield with a +1/+1 counter. A failed search does not undo the cast
or sacrifice.

Cascade uses printed mana value and the actual library order. The free hit
uses the normal cast-resolution path, so on-cast effects and enter triggers
resolve. A spell without a mana cost can be a cascade hit, but cannot be
cast normally from hand. A graveyard-creature exchange affects only the
player's cards: exile creature cards in the graveyard, sacrifice battlefield
creatures, then return the exiled creatures.

### Cost reductions

Modeled as discounts on the generic part, and only for castability
(`min_cost`), never as extra mana:

| Mechanic         | Approximation                                                 |
| ---------------- | ------------------------------------------------------------- |
| Warp             | the warp cost when cheaper                                    |
| Improvise        | −1 generic per artifact on the battlefield (never below zero) |
| Affinity         | −1 generic per artifact on the battlefield (never below zero) |
| "costs {1} less" | −N generic from the printed digit                             |

The "{X} less" wording stays inert: `{X}` is not a readable amount.

The improvise/affinity discount grows with the artifact board at the
rule rate (one generic per artifact, CR 702.126a/702.41a): a mid-game
board casts big improvise spells early, and such cards are exempt from
the `low_castability` finding.

Hybrid pips (`{W/U}`) pay from either color (CR 107.4e). Phyrexian pips
(`{B/P}`) pay their color or 2 life (CR 107.4f); the goldfish pays life
when it has room. Convoke (CR 702.51) taps untapped creatures for {1}
each, or a matching colored pip. Delve (CR 702.66) exiles graveyard
cards for {1} generic each. Twobrid `{2/W}` reads as 2 generic. Snow
`{S}` pays as colorless (no snow-source tracking). X-costs follow the
X-cost section above.

## The turn pipeline

A best-case agent follows all five CR 500.1 phases in order. The model
executes actions in the precombat main phase; the postcombat main phase is
an explicit no-op stage.

1. **Beginning** — untap, upkeep, and draw. Everything untaps except permanents whose card says it
   doesn't untap during the untap step (they stay tapped until an untap
   activation clears them); summoning sickness clears; once-per-turn
   flags reset; crew animations expire. Blink-armed permanents re-fire
   their OnEnter triggers once (the blink re-fire pass).
   During upkeep, draw, mill, recursion, drain, and token engines fire (one
   firing each, per turn); win-threshold engines check their
   counter stock; planeswalker ultimates flag online when loyalty reaches the
   minus cost.
   Then draw. Commander games draw on turn 1; constructed games on the
   play skip the turn-1 draw.
2. **Precombat main** — first-main triggers resolve, then play
   an untapped land when one is in hand, else any land (tapped lands wait
   for a better turn when possible). Fetch lands search up a non-fetch
   land; tapped entry follows the found land's Oracle text (many modern
   duals enter untapped). Landfall triggers resolve for each land entry. An
   "additional land" board (Aesi class) plays a second land the same turn.
   Cast the cheapest payable spells first, with the full
   pip check (lore counters were already added above; CR 714.3c puts
   lore counters at the start of the precombat main phase). A
   cast is blocked (and its color recorded for color-screw stats) when the
   pool has enough total mana but misses the pips. ETB triggers fire —
   every parsed clause of a multi-effect trigger lowers separately, so
   "draw a card and create a Treasure token" does both; ETB
   tokens join the battlefield as bodies. One-shot effects (ritual mana,
   draws, mills, scry/surveil, burn, extra turns, X-scaling, kicker)
   apply on cast. Per-cast mana engines (Vivi class) add their yield per
   spell cast. Recheck the hand after each productive cast sweep, so
   cantrips and rituals can enable same-phase follow-up casts.
   Spend leftover mana on unlocked activations (draw,
   tutor, mana, counters, loot, sacrifice outlets, planeswalker loyalty
   abilities, drain activations). Cheapest first, one activation per
   permanent per turn (free zero-cost activations repeat until the cap).
   Mana activations and new cards feed another cast pass. Banked
   activations (Pentad Prism) consume a counter instead of tapping.
   Use remaining untapped bodies in this priority order:
   a. tap for **mana** only while the cheapest uncast spell still needs
   mana;
   b. else tap to **station** the highest-threshold unfilled
   spacecraft/planet (counters = body power);
   c. else tap to **crew** untapped Vehicles (total body power ≥ crew
   cost; crewed vehicles count as bodies for the turn);
   d. else pay **equip** costs once per Equipment — the gear suits up
   its best untapped, unsick body and the buff joins only that
   host's attack.
   Measure interaction readiness; the goldfish never spends answers.
3. **Combat** —
   Attack power and triggers follow the combat policy. The goldfish assumes
   attacks connect. Haste allows attacks on the entry turn. The unused
   postcombat main phase remains an explicit no-op.
4. **Ending** —
   The Monarch draws at the beginning of the end step, end-step
   triggers resolve, then the hand-limit policy discards. Extra turns run
   through the same five phases within the configured turn limit.

The model adds Saga lore counters as the precombat main phase begins
(CR 714.3c). It does not model stack timing or other lore-counter
placements. Additional land plays increase the per-turn allowance
(CR 305.2); the simulator may use a later land play after one becomes
available.

The commander casts from the command zone with the full pip check. Its
cost is paid, it joins the battlefield as
a permanent, and its ETB triggers fire (Infinite Guideline Station's
tokens become station fuel). There is no recast tax — a destroyed
commander stays on the board in this model.

## Metrics

All metrics aggregate over `--runs` games (default 10,000). Percentages
carry ±0.5pp at 10k runs.

| Top-level key                                                                                                               | Meaning                                                                             |
| --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `deck_shape`                                                                                                                | Deck census, including lands, ramp, roles, and mana-value curve.                    |
| `opening_hand`, `land_drops`, `mana_base`                                                                                   | Opening lands, land-play rates, screw/flood measures, and target bands.             |
| `commander`, `station`, `companion`                                                                                         | Optional timing blocks; `null` when not applicable.                                 |
| `creatures_by_turn`, `repeatable_sources_by_turn`, `mana`, `draw`, `role_access`                                            | Board presence, engine availability, unused mana, card access, and role visibility. |
| `velocity`, `combat`, `win_conditions`, `interaction`, `color_mana_shortage`, `color_pip_blocks`, `graveyard`, `milestones` | Turn-indexed measures. Interaction is readiness capacity, not resolved events.      |
| `color_sources`, `card_castability`, `findings`, `summary`, `assumptions`                                                   | Source census, spell timing, typed diagnoses, summary, and model limits.            |
| `combo_access`, `colored_sources`, `combos`, `win_paths`, `hypgeo`                                                          | Optional typed blocks included when requested or available.                         |

Percent values are typed at the serialization boundary and emitted from
0 to 100, rounded to two decimals. A field named `percent_*` always
measures a percentage; mana, counts, and averages remain numeric amounts.
The exact fields are defined by `report_schema.rs`.

`mana_ready` (the castability curve) is draw-agnostic: it asks when the
board could first pay each cost, independent of whether the card was
drawn. It measures the mana base; the hand adds the draw dependency.

### Mana-base target bands

`mana_base` compares the deck's counts against research-derived bands
(EDHREC average decks, 46 average decks across 11 commanders, fetched
2026-09; Sam Black's cEDH land-count guidance in the Commander's Herald;
Frank Karsten's 60-card land-count method). A deck outside its band gets
a "trim/add lands" verdict — the same signal an AI deckbuilding agent
sees, so "add lands" is never the only lever.

Commander-family decks (a COMMANDER section) use bracket bands:

| Bracket       | Lands | Ramp (rocks + dorks + ramp spells) |
| ------------- | ----- | ---------------------------------- |
| 1–2 (casual)  | 34–40 | 7–12                               |
| 3 (upgraded)  | 33–38 | 8–11                               |
| 4 (optimized) | 32–36 | 9–12                               |
| 5 (cEDH)      | 25–31 | 10–16                              |

60-card constructed decks use bands derived from the deck's average
mana value (Karsten's method):

| Average nonland MV         | Lands |
| -------------------------- | ----- |
| < 2.0 (aggro)              | 20–22 |
| 2.0–3.0 (midrange)         | 22–25 |
| ≥ 3.0 (control / big mana) | 25–28 |

Every four cheap (cost ≤ 2) draw or cantrip spells count as one land,
capped at 2. 60-card decks report `bracket: null`, skip bracket
inference, and carry `bracket_target_ramp: [0, 0]` (no separate ramp
verdict).

Without an explicit `--bracket` the bracket is inferred from the Game
Changer census (0 GC → 2, 1–3 → 3, 4+ → 4). The `mana_base` object
reports `bracket` and `bracket_inferred_from_game_changers`.

Lands-matter decks (an extra-land-drop engine on the board) widen the
land band by 4 and the verdict says so. Calibration anchors: a 35-land
deck with 9 rocks reads "on target" at bracket 3; a 44-land deck reads
"trim 6 lands"; a 44-land deck floods in ~35% of games (expectation
~35%), while the old drops-made detector read it as 0.0%.

### Colored sources (static audit)

The `colored_sources` JSON block (and the standalone `stm deck mana`
command, which runs the same math with no simulation) audits the mana
base against Karsten's requirement floors. It is static math on the
deck's census: weighted sources per color, per-card requirements, and
deficits. Source: Frank Karsten, "How Many Sources Do You Need to
Consistently Cast Your Spells? A 2022 Update".

Weights: lands 1.0 per producible color (fetches credit their fetchable
basics; any-color lands credit every deck color), mana dorks 0.5, mana
rocks 0.75, cheap cantrips 0.25 per effect (capped at 10 effects).
Land/spell MDFCs (modal double-faced cards) count 0.4 land each (0.75
mythic, by the rarity
column — shared with the sim's mana-base block) in the land count.
Tap lands count as sources but not as untapped turn-1 sources; the block
reports both (`sources`, `untapped_t1_sources`, `tapland_count`).

Requirements: commander decks use the 99-card floors — 12/17/21 sources
for 1/2/3 pips of one color. 60-card decks use the Karsten 2022
pip-shape table keyed on (generic pips, total colored pips, max
same-color pips): 1 pip 13, CC 21, 1CC 18, CCC 23, 2CCC 22, CCCC 24.
The floor is corrected for the deck's land count (±1 near 20 lands, +2
at 28+).
Unlisted shapes fall back to the same-pip floor (14/13/21/23/24) less
one source per generic pip. Gold cards add +1 per
additional color requirement. Each nonland card with colored pips gets a
`requirements` row (`needs`/`have`/`deficit`/`ok`); `worst_deficits`
formats the top lines ("Wrath of God: need 16 W, have 14.0") and the
human view suggests categories ("add 2-3 more white sources"), never
card names.

### Findings (exit 1)

| Kind                            | What it measures                                                                              |
| ------------------------------- | --------------------------------------------------------------------------------------------- |
| `insufficient_land_drops`       | Games with two or fewer lands by turn four.                                                   |
| `excess_lands_seen`             | Games seeing six or more lands by turn four, compared with the draw-adjusted expectation.     |
| `late_commander_cast`           | Commander not castable by its mana-value turn in at least 40% of games.                       |
| `insufficient_color_mana`       | Games with enough total mana but missing a required color pip. `identity` includes the color. |
| `limited_draw_access`           | No draw source seen by the format's target turn.                                              |
| `unused_mana`                   | Average unused mana reaches the threshold by turn six.                                        |
| `low_castability`               | At least three distinct nonreactive cards are castable on curve in too few games.             |
| `low_removal_access`            | Removal not seen by turn five often enough.                                                   |
| `low_win_condition_access`      | Win condition not seen by turn eight often enough.                                            |
| `limited_interaction_readiness` | Instant-speed interaction is seen but not affordable with spare mana often enough.            |

Each `findings` row has `kind`, stable `identity`, `severity`, optional
`percent_of_games` and `color`, `explanation`, `suggestion`, and `evidence`.
Evidence rows have `subject`, `explanation`, and optional
`percent_of_games`. Suggestions use categories and quantities, not card
names. The command exits 0 when there are no findings and 1 when findings
are present; this is a result, not a runtime failure.

## Validation loop

The standard workflow for any deck edit:

```sh
stm deck simulate <name> --seed 42 --json > /tmp/base.json
# ... apply the edit ...
stm deck simulate <name> --seed 42 --baseline /tmp/base.json
# prints typed metric, finding, and deck-shape deltas
```

Identical seeds reproduce identical shuffles, so a metric delta isolates
the deck change. A batch is working when its target metric moved and no
new findings appeared. **Regenerate the baseline after upgrading `stm`**:
seed compatibility is version-local.

Add `--hypgeo` to any run for exact cast-on-curve ceilings beside the
simulated numbers, and `--combo "A + B"` to measure combo assembly
(share of games with both pieces seen in hand by the pair's target turn).

## Problems and explainability

Every finding carries a stable identity and typed evidence:

1. **`explanation`** — a plain-English sentence: what is wrong, how often.
   Short sentences, no jargon ("you run out of lands often: 22.4% of
   games had 2 or fewer lands by turn 4"). The raw `kind` stays a stable
   JSON key; the stable `kind` identifies the measured deficit.
2. **`evidence`** — the cards or counts behind the finding, from the
   same aggregated stats: pip blocks name the color-screw cards, per-card
   castability names the slow cards, the land/rock census backs screw and
   flood, the commander's cost explains a late cast. Assembled in
   `findings_detail.rs` (`explain_with_threshold`), called from
   `analyze_findings` so every consumer gets it automatically.
3. **`suggestion`** — the fix. Cause-specific where the data supports it
   (color screw names the worst blockers and the color to add);
   category-level elsewhere ("add 2-3 draw engines" — never card names).

The human `Findings` block prints the finding, evidence, and suggestion.
`--json` carries the same evidence rows. Human output follows one policy:
**what is wrong → how often → which cards → what to do**, simplified
English, no "pip-block"/"dead card"/"mana screw" jargon in user-facing
strings.

## Tests

Two test layers cover the simulator:

- `simulator/tests/*.rs` — unit tests per model shape:
  - Tap yields: choice, fixed, any, colorless, creature-only.
  - Station tiers: single tier, two tiers, planets never animate. Crew
    uses printed power.
  - Ability shapes: ETB, upkeep, attack, cast, activations, planeswalker
    loyalty, mill, graveyard return, wheel, loot, sacrifice, death
    triggers.
  - Roles: including Lock and Booster; reactive spells classify as
    Removal.
  - Land parsing: enters-tapped (shock duals), verge gates, Leyline
    openers, cost reductions.
  - Game loop: land drops, screw, mulligan policy, commander pip
    gating, 5c commander with/without any-color rocks, crew→station
    chains, fetch smoothing, same-seed reproducibility, pip blocks,
    graveyard census, sacrifice→death-token flow, restricted mana
    paying creature casts only.
  - New mechanics: whenever-ETB parsing, landfall trigger family, ETB
    scry → awareness, token counts (bare / word / "for each" cap 8),
    X-cost classes, per-cast mana, kicker, saga chapters, loyalty
    plus/minus costs, text flying, flashback-not-flash, extra land
    drops, removal-beats-draw role order, haste entry-turn attacks,
    planeswalker loyalty/ultimate flow, X drain totals, extra-land-drop
    ramp, extra-turn replay drops, per-cast mana velocity, the free-mana
    loop cap census, upkeep drain engines, constructed ×1 drain, kicker
    payment, saga chapter payoffs, token body counts, dredge replacement
    and short-library fallback, chained dredgers during one resolving
    draw effect, library-only graveyard triggers, separate drain/life-gain
    accounting.
  - `mechanic_tests.rs` pins the audit-remediation behavior: wheel
    execution, cast-trigger dedupe, commander upkeep drain + no double
    draw, combined-numeral sagas, chapter IV + leave-board, Helix
    X-sink counters, token-count bodies, once-per-turn engines (no
    infinite flag), commander ×3 vs constructed ×1 drain,
    additional-cost consumption, X-entry counters, and the Vivi
    no-double-count parse.
  - `mechanics_tests.rs` pins the strong-mechanic batch: mobilize's
    attack tokens and their end-step sacrifice, amass Army growth, the
    Ring's temptation triggers, empower Jace's loyalty token, saddle
    payoffs, power-up's entry-turn discount and once-per-game gate,
    exhaust, teamwork, storm copies, convoke, delve, offspring, plot,
    explore, living metal, afterlife, connive, read ahead, burden
    counters (The One Ring), and the descend/raid intervening-if
    conditions.
  - Truth-fix tests pin: ETB draws never double count as cast riders,
    protection spells stay out of Removal, wipes count as interaction
    with the targeted/wipes deck-shape split, 2-damage burn and bounce
    read as interaction, the modern "triggers only once each turn"
    phrasing bounds engines, Treasure banking requires the Treasure
    clause on the token-creating card, "draws X" and the
    reveal-permanents and counter-power X classes, the one-shot +X/+X
    board buff, ability-word prefix stripping, board-count scaling draw
    engines, and split-card on-cast-credit suppression.
  - Dedicated archetype tests live in the per-format deck test files
    (commander/standard/modern) for every fixture that was previously
    invariants-only, plus degradation-fixture problem assertions.
  - `oracle_ast_tests.rs` covers every AST ability, cost, effect,
    keyword, trigger, static target, and search field.
    `oracle_runtime_tests.rs` checks exact zones and turn timing for
    surveil, landfall, combat, compound spells, unsupported text, and
    the Kinnan/Basalt follow-up line.
- `simulator/tests/deck_tests.rs` + `simulator/tests/deck_fixtures/*.json` —
  real tournament lists. Sources: mtggoldfish metagame, cEDH Decklist
  Database, EDHREC, and topdeck.gg competitive tournament standings
  (fetched through the TopDeck.gg API with attribution; commander lists
  from cEDH/casual league events, 60-card lists from Standard/Modern
  tournaments). Fixture filenames mix underscores and hyphens. The sweep tests assert
  simulator consistency properties only, never deck quality: land-drop
  sanity, velocity monotonicity, castability ≥ cost floor, opener
  sanity, and archetype-specific checks (topdeck Murktide early threats,
  Eldrazi ramp never curve-ready, Boros tokens body emergence, Yawgmoth
  combo velocity, Aesi median drops > 4, superfriends walker cast rates,
  graveyard growth for recursion decks, Shorikai Vehicles crewing,
  Infinite Guideline Station animating from station fuel, Zurgo
  mobilize attack power, and the Sauron ring/amass drain).

When you add a modeled mechanic, add both: a unit test for the parse shape
and a game-level assertion that the behavior shows up in the metrics.

## Assumptions and limits

Unsupported wording stays in the AST and is not lowered to a game effect.
The `assumptions` array in every report lists the current limits:

- **No opponents, no interaction.** No countermagic fires, no removal is
  cast, no attacker is blocked. Removal and counterspells count toward
  role access and the **readiness** metric (in hand + affordable), but
  never fire. Readiness is capacity, not events.
- **Draw engines fire on a fixed delay.** An engine draws its amount once
  per turn from the turn after it enters, regardless of board state. A
  "draw a card for each enchantment/artifact/land/creature you control"
  engine draws the matching permanent count instead (capped at 8;
  uncountable shapes fall back to 1). ETB draws parse as triggers only:
  an enter-trigger draw never also reads as a cast rider, so the
  double-count is gone. Commanders add a synthetic engine only for
  unconditional (upkeep/end-step) draw triggers; attack-gated and
  opponent-gated commander draws fire no engine.
- **Mill feeds velocity and the graveyard census, not replay quality.**
  Milled cards count as cards seen and fill the graveyard log; graveyard
  return fires once per card (no recursion chains). Zones hold card
  indexes, not instances: a card that is milled, returned to hand, and
  discarded again is the same index throughout, so per-card timelines
  merge repeat visits (the index-aliasing approximation).
- **Wheels and loot reset the hand.** A wheel discards the hand into the
  graveyard census, then draws seven (trigger wheels fire from their
  upkeep/cast trigger; plain wheel sorceries resolve on cast); loot is
  draw-n discard-n.
- **Spend-restricted mana pays only its own cast class.** Secluded
  Courtyard-style lands park their mana in a class bucket (creature,
  legendary, artifact, instant/sorcery). A cast reaches the general pool
  plus its own bucket; other buckets and unrestricted casts cannot touch
  it. The bucket pays the cast's generic and pips as chosen-color mana.
- **Twobrid symbols cost generic only.** "{2/W}" reads as 2 generic: the
  total is exact for a pool of two or more mana, but a pool of exactly
  one {W} cannot pay it in the sim even though the rules allow the
  one-white-mana option.
- **Drain targets follow the target shape.** "Each opponent loses N"
  counts N per opponent (three in the commander family); "target player
  loses N" counts once; "each player loses N" counts per opponent and
  costs the goldfish N. This is the CR 119.3 reading, not a table
  multiplier for every drain.
- **Delve, convoke, and power-up consume real resources.** Delve exiles
  cards from the graveyard, convoke taps untapped creatures, and
  Power-up's entry-turn discount cuts the activation by the permanent's
  mana cost (CR 702.193b). Exhaust and Power-up abilities fire once per
  game.
- **Storm copies the spell's modeled riders.** A storm spell resolves
  its draw, drain, and token riders once per spell cast before it that
  turn (CR 702.40a).
- **End-step discard keeps the cheapest cards.** The best-case agent
  discards the highest-mana-value cards down to seven. A board
  permanent with "You have no maximum hand size" skips the discard.
- **Mobilize, amass, and afterlife are modeled.** Mobilize creates
  tapped-and-attacking Warriors that join the swing and are sacrificed
  at the end step; amass grows one Army body with +1/+1 counters;
  afterlife creates Spirit bodies on death. Ring temptations raise the
  emblem: level 2 attack-loots, level 4 combat damage drains each
  opponent for 3 (level 3 needs blockers and stays inert). Empower Jace
  creates a Jace token with "[-1]: Surveil 1" and "[-3]: Draw a card".
- **Plot and offspring resolve.** Plot exiles a card now and casts it
  free on a later turn; offspring pays its extra cost from leftover mana
  and adds a 1/1 body on entry.
- **Supported intervening-if conditions evaluate.** Descend, threshold,
  raid, ferocious, and metalcraft gate their triggers against the game
  state. Unsupported condition shapes keep the trigger inert.
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
- **Land types come from card data.** Verge gates read the basic land
  subtypes from type lines. Fetch activation, life costs, tapped entry,
  and named target types come from Oracle text. A small fetch-name table
  remains only as a fallback when Oracle text does not name the target
  pair.
- **Cost cuts are flat at parse, board-scaled for improvise/affinity.**
  Warp uses the cheaper cost; improvise/affinity cut the printed generic
  by one per artifact on the battlefield at resolve time (capped at the
  printed generic); they are exempt from the `low_castability` finding.
- **Opponent-dependent production is generic-only.** Fellwar Stone
  yields one colorless-only pip from turn 2 on and does not tap on turn 1.
  Kinnan's mana trigger adds one additional colorless pip from this source.
  This avoids assumptions about an opponent's lands and colors.
- **Type-granted lands read as any-color.** "This land is the chosen type"
  lands tap for one mana of any color (the choice is the player's). Static
  grants on other lands (The World Tree) do not model.
- **X-costs pay the leftover pool.** A `{X}` spell with a drain/draw/
  mill/tokens/reveal-permanents/counter-power scaling effect converts the
  whole floatable pool into X at cast; the effect scales with it. Spells
  with unmodeled X effects pay X = 1 and do nothing extra.
- **Split cards cast one face.** A "Fire // Ice" card pays the cheaper
  face; the union text feeds roles, triggers, and interaction, but
  one-shot on-cast credits (draw, mana, tokens, drain) do not fire — the
  face that made them was not cast.
- **Land/spell MDFCs (modal double-faced cards) have two modes.** One face is a Land, the other a
  castable spell. The card is a `SimCard` with both: the spell face
  carries the role and cast cost; the land face carries the tap yield
  and enters-tapped state. Play rule (deliberately simple): during the
  land phase, an MDFC plays as the land only when the hand holds no
  other land; otherwise it stays a castable spell. The mulligan policy
  counts an MDFC as land-able. In the mana base an MDFC counts 0.4 land
  (0.75 mythic); the colored-source audit counts the land face as 0.8
  source of its color.
- **Cascade follows the library order.** Reveal from the top until the
  first nonland card with lower printed mana value. Cast that card for
  free through the normal resolution path. Misses go to the library
  bottom in reveal order. The effect resolves once and does not chain.
- **Searches use supported filters.** Parsed type, color, mana value,
  optional-find, and destination limits select an eligible card instead
  of taking the library top. Search does not grant cards-seen awareness.
- **Cycling pays and moves its card.** Cycling pays its mana or life cost,
  discards the card, then draws. Landcycling finds a land with its named
  basic land type. Only these fixture-relevant forms are modeled.
- **Graveyard casts use instance permissions, printed costs, and real
  resources.** A card's own printed flashback cost casts it from the
  graveyard, then the spell exiles; a card's own printed escape cost
  casts it by paying that cost plus exiling three other graveyard
  instances. Granted shapes work too: Past in Flames grants flashback to
  instant and sorcery instances already in the graveyard when it
  resolves (the permission ends with the turn), and Underworld Breach
  grants escape to graveyard nonland cards while it remains on the
  battlefield. Replays consume mana and fuel; the action cap does not
  claim that repeated casts are infinite.
- **Life-funded actions use current life and visible cards.** Life costs
  must leave the player above zero. Griselbrand-style activations repeat
  only while life can pay, and draws use the shared draw path. Ad Nauseam
  reveals in library order, adds each card to hand, and loses its printed
  mana value in life. The current policy continues until life is zero or
  the library is empty; it does not inspect hidden cards to avoid a lethal
  reveal. Oracle shapes outside these parsed forms stay inert.
- **The graveyard-creature exchange is player-only.** It exiles the
  player's graveyard creatures, sacrifices the player's battlefield
  creatures, and returns the exiled creatures. It does not process
  opposing cards.
- **"+X/+X where X is the number of creatures you control" is a one-shot
  entering board buff.** On the turn the buff body enters, each attacker
  gets +X (X = the body count, capped at 20). The buff does not persist.
- **Kicker pays from spare mana.** Only drain amounts scale with the
  kick; other kicker riders are inert.
- **Per-cast mana engines fire per spell.** Vivi-class "add one mana for
  each spell you've cast this turn" joins the pool once per cast while
  the host is untapped.
- **Actions are bounded.** A turn runs at most 24 action passes and an
  activation pass at most 24 activations. Only a capped activation pass
  that added mana flags `win_conditions.percent_games_with_suspected_infinite_mana`; a free draw does not.
- **"For each" token counts cap at 8.** Go-wide boards stay bounded.
- **Life loss follows its target.** "Each opponent" applies once per
  opponent; "target player" applies once; "each player" also affects the
  goldfish. Commander has three opponents and constructed has one.
- **Extra turns run the full player turn.** They untap, resolve upkeep,
  draw, play lands, cast spells, activate abilities, and attack. Each extra
  turn uses the next configured turn slot; chained turns stop at the
  configured `--turns` horizon. Per-turn indexes include extra-turn slots.
- **Sagas fire chapter I on entry and later chapters in the precombat
  main phase.** Unsupported
  chapters have no effect and are not replaced with an invented draw. The
  saga leaves the battlefield after its final chapter.
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
- **Metalcraft is modeled for mana sources.** A source that requires
  metalcraft is available only while its controller has three artifacts.
  Other metalcraft bonuses and ascend and delirium are not modeled.
- **Counters are split by kind.** +1/+1, -1/-1, charge, and loyalty
  counters each feed their own mechanic; +1/+1 and -1/-1 counters
  annihilate in pairs (CR 122.3). Charge counters drive station tiers,
  banked mana, and win thresholds; loyalty drives planeswalker
  activations. Unmodeled counter kinds (time, stun, oil, experience,
  lore) stay inert.
- **Energy counters are player counters.** Parsed "you get {E}" clauses
  accrue energy and parsed activation costs spend it; the stored amount
  is not surfaced in the default report. Energy sinks that are not
  activation costs stay inert.
- **Proliferate adds one counter of each kind already present.**
  Every counter-bearing permanent and the player's energy gain one more
  counter. Keyword counters do not stack, so they are unchanged.
- **Keyword counters grant their keyword.** A flying/deathtouch/lifelink
  counter parses as a permanent keyword grant; temporary keyword loss is
  not modeled.
- **A companion is the first bench card with the Companion keyword.**
  Its deck condition is assumed legal. It starts outside the game and
  never shuffles into the library. Once per game, in a main phase,
  paying {3} puts it into hand; the report shows the turn it arrived.
- **Reconfigure gear counts as a body until it attaches.** When it
  attaches it stops being a creature (CR 702.151b); the generic mana
  option is the modeled cost.
- **Morph, megamorph, and disguise cast face down.** When the face-up
  cost is unaffordable but {3} is, the card enters as a 2/2 face-down
  body with no abilities. It turns face up in a later main phase when
  the pool covers the morph cost; turning up does not re-fire enter
  triggers (CR 702.37e). Manifest text itself stays inert.
- **Other replay mechanics are unsupported.** Rebound, splice, ninjutsu,
  and cast-from-graveyard rules outside the supported flashback and escape
  shapes stay inert.
- **Unparsed conditions stay inert.** The AST retains activation conditions
  and opponent-scoped trigger events, but lowering does not execute them.
  Intervening-if trigger conditions evaluate for descend, threshold, raid,
  ferocious, and metalcraft; unsupported shapes keep the trigger inert.
  Metalcraft has its own supported mana-source gate.
- **Keyword support is goldfish-aligned only.** A keyword models only
  when it moves an existing metric:
  - Haste skips summoning sickness.
  - Combat-damage triggers fire per connecting attacker
    (draw/proliferate/drain).
  - Static "+N/+N" board buffs join the attack power.
  - Equipment buffs join only after the equip cost was paid
    (tap budget 7d).
  - Double strike doubles attack power.
  - Prowess adds +1 per noncreature spell cast that turn.
  - Landfall counter-shapes add +1 per later land drop while supported
    landfall draw, token, drain, and search effects fire when a land
    enters. Landfall mana triggers are parsed but do not add
    to the mana pool.
  - Scry/surveil feed awareness only (surveil puts cards in
    the graveyard and resolves library-to-graveyard triggers).
  - Trample/flying/menace (from keywords or text) count as an evasion
    census with no math. Vigilance is free (attackers never tap). Imprint
    is out of scope.
  - Lifelink adds the attacker's power to life gained. Deathtouch, reach,
    first strike, hexproof, and indestructible are parsed but inert
    (no blockers and no opponent interaction).
  - Keyword grants ("creatures you control have flying") and keyword
    counters merge with printed keywords at combat time.
- **Drain is a census, not a life total.** "Each opponent loses N" /
  "deals N damage to target player" multiplies by 3 in commander (three
  opponents) and by 1 in 60-card formats. No life totals, no racing.
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
- **London mulligan is a single redraw.** Karsten's model redraws 0/1/
  6/7-land 7-card hands once and the redrawn hand bottoms one card
  toward 3 lands (a land at 4+ lands, else a spell); kept hands stay at
  7 cards. No keep-choice evaluation and no second redraw.
  Commander-family decks keep the single free redraw.
- **Combo assembly is a consistency diagnostic.** Store-backed Spellbook
  rows measure how often pieces reach their zones, never whether the
  combo wins; quality labels come from Spellbook data.
- **Exile-zone combo pieces are excluded.** The game tracks exile, but
  store-backed combo assembly does not evaluate exile-zone requirements.
  Variants needing one drop out of the join (counted in
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

One documented path for a new mechanic. Layering is strict: the parser
builds the AST first, then lowering maps nodes to runtime data, then the
game loop executes them. A mechanic must not be recognized by raw string
scan after the AST stage.

1. **AST** in `oracle_ast.rs` and `oracle_parser.rs`: add a typed node
   (keyword entry, effect, trigger event, or restriction) and an
   Oracle-text parse test. Keyword abilities that the CR defines as
   triggered abilities (mobilize, afterlife) are synthesized into the
   ability list at this stage, so they run the printed-ability pipeline.
2. **Model shape** in `model.rs` (SimEffect/SimTrigger/ManaYield variant
   or field) for the runtime data the AST lowers onto.
3. **Lowering** in `oracle_lower/`: map supported nodes to runtime data.
4. **Game step** in `game_effects.rs` or `game_run.rs`: execute the shape and
   record census data on `GameLog`.
5. **Metric** in `aggregate.rs` (stats) + `report.rs` (JSON + human).
6. **Docs**: a row in the metrics table, a line in `assumptions`.
7. **Tests**: parser test + game-level assertion (see Tests).
