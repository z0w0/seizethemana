# Simulator cleanup plan

This plan makes the goldfish simulator clean and closer to the real
Magic rules. It has six phases. Each phase ends with the full
validation suite: `cargo nextest run`, `cargo clippy --all-targets --
-D warnings`, `cargo fmt --check`, and `aislop scan --changes`.

Work in one branch per phase is not required, but commit after each
checklist item so changes stay reviewable.

## Rules reference

All rule numbers come from the official Comprehensive Rules,
effective September 25, 2026. Key citations used in this plan:

- 113.3: the four ability categories (spell, activated, triggered,
  static).
- 603.4: the intervening "if" clause of a triggered ability.
- 118: costs. 606.3: one loyalty activation per permanent per turn.
- 500.7: extra turns. 505.1: the precombat main phase, also called
  the first main phase.
- 714.3c: Saga lore counters are added in the precombat main phase.
  714.4: a finished Saga is sacrificed.
- 714.2: a chapter is a triggered ability.
- 725.2: the Monarch draws at the beginning of their end step.
- 702.184 and 721: Station, charge counters, and the `{N+}` station
  symbol.
- 122.2: counters and other state do not follow a card between zones.
- 509.1h: an attacking creature with no blockers is an unblocked
  creature.

## Phase 1: bug fixes

These are correctness bugs in the game engine. Fix them first so
later refactors sit on correct behavior.

### 1.1 Cap the sacrifice loop in the graveyard exchange

- [x] `src/deck/simulator/cast_phase.rs`, `graveyard_creature_exchange`
      block (around line 637): the `while` loop sacrifices creatures
      until none remain. Death triggers on the board can create new
      creature tokens each iteration, so the loop can run forever.
      Cap the loop at 24 iterations, the same bound the other
      repeatable-effect loops use.
- [x] Add a regression test: a deck with a `graveyard_creature_exchange`
      card plus a permanent whose OnDeath trigger creates tokens.
      Assert the game finishes.

### 1.2 Route opponent mills away from the player's library

- [x] `src/deck/simulator/game_effects.rs`, `mill_library_card`
      (around line 49): when `mill_opponent` is true, the code still
      pops the player's library and fills the player's graveyard.
      Only the census counter changes. This is wrong per the rules:
      milling the opponent must not touch the player's own zones.
      Change the `mill_opponent` path to bump `st.milled_opp` and
      return without touching `st.library`, `st.graveyard`, or
      `st.seen`.
- [x] `CardZone::OpponentLibrary` in `game_effects.rs` becomes
      unused. Delete it.
- [x] Update tests that rely on opponent mills filling the player's
      graveyard (search for `mills_opponent` and `milled_opp`).
- [x] Update `docs/simulator.md` wherever it describes opponent mills
      as self-mill fuel.

### 1.3 Fix landfall power on the final turn

- [x] `src/deck/simulator/game_combat.rs` (around line 122): the
      clamp `perm.entered_turn.min(turns - 1)` makes a landfall body
      that enters on the last turn count that turn's own land drops.
      Every other turn excludes them. Drop the clamp so the slice is
      `[entered_turn..turn]`, empty for same-turn entrants.

### 1.4 Gate graveyard casts on additional life costs

- [x] `src/deck/simulator/cast_phase.rs`, `cast_graveyard_spells`
      (around line 300): the candidate check tests mana only. Hand
      casts also gate on `st.life <= card.additional_cost_life`.
      Add the same life gate to the graveyard path so an escaped or
      flashback-cast spell cannot drive life negative.
- [x] Add a test: a graveyard-castable card with an additional life
      cost while life is too low. Assert it is skipped.

### 1.5 Stop double-counting `seen` on graveyard returns

- [x] `src/deck/simulator/game_effects.rs`, the `ReturnFromGraveyard`
      block (around line 204): a card returned to hand bumps `st.seen`
      even though the card was already counted when it was milled,
      cycled, or discarded. Remove the `st.seen += 1` on the to-hand
      path. Cards already in hand that get "returned" by a loot effect
      stay counted.
- [x] Check the flood metric tests for baseline shifts and update
      them.

### 1.6 Move the Monarch draw to the end step

- [x] `src/deck/simulator/game_run.rs` (around line 138): the Monarch
      extra draw fires at the draw step. Per rule 725.2 it fires at
      the beginning of the Monarch's end step. Move it to phase 10,
      before `fire_triggers(Trigger::OnEndStep)`.
- [x] Update `docs/simulator.md` lines that say the Monarch draws at
      upkeep (around lines 286, 402, 806).

### 1.7 Move Saga chapter advancement to the precombat main phase

- [x] `src/deck/simulator/game_run.rs`, `run_turn`: `run_sagas`
      currently runs in phase 2 (upkeep). Per rule 714.3c, lore
      counters are added in the precombat main phase, after the draw
      step. Move the `run_sagas` call to the start of phase 5, before
      `build_pool`.
- [x] `src/deck/simulator/game.rs`, `fire_on_enter` (around line 529):
      the entry chapter fires at cast time. Keep that, since the
      entry chapter is the Saga's own first lore counter, placed in
      the main phase where the cast happened.
- [x] Expect baseline shifts in saga fixture tests. Update them and
      note the change in `docs/simulator.md` (turn pipeline section
      and any saga assumption text).
- [x] Update `docs/simulator.md` line that says chapters advance at
      upkeep (the phase-2 comment in `game_run.rs` too).

### 1.8 Fix the cascade reinsertion comment

- [x] `src/deck/simulator/cast_phase.rs` (around line 916): the
      comment says misses return to the bottom in reveal order, but
      repeated `insert(0, index)` reverses them. Either reinsert in
      reverse iteration order so the comment becomes true (preferred:
      the last-revealed miss ends up deepest is wrong; the reveal
      order should be preserved), or fix the comment. Prefer the code
      fix: iterate `exposed.iter().rev()` so the first-revealed card
      ends up deepest.

## Phase 2: dead code removal

Delete code that no production path reads. Every deletion below was
verified against the full source tree.

- [x] Delete the `source` fields on `StaticAbility`, `SpellAbility`,
      and `SagaChapter` in `oracle_ast.rs`, plus their initializers in
      `oracle_parser/` files.
- [x] `Equipment.death_draws` (`model.rs` field, the parse block in
      `parse_keywords.rs` around line 60): parsed but never fired.
      Delete the field and the parse block. If Skullclamp-style death
      draws are wanted later, add them with a runtime effect and a
      test, not a dead field.
- [x] `TriggeredAbility.condition`: an intervening "if" clause is
      parsed but never read. Keep the parse (it is the right model per
      rule 603.4) and wire it in phase 4 (see 4.4). If 4.4 is not
      done, delete the field instead. Do not leave it dead.
- [x] `KeywordAbility.source` and the `KeywordSource` enum: read only
      by tests. Delete both and drop the `keyword_source` parameter
      from `parse_oracle_keyword` and its callers.
- [x] `KeywordArgument::ManaCost` and `KeywordArgument::Text`: never
      read in production. Keep `Number`. Also delete the mana-symbol
      collection loop in `oracle_parser/keywords.rs` (lines 17-38)
      that feeds `ManaCost` — it is dead work per keyword.
- [x] `OracleCard`: remove the unused `Default` derive.
- [x] `game.rs`, `token_body()`: one caller (`token_body_card`).
      Inline it.
- [x] `mod.rs`, `store_has_combos_pub`: a thin `pub` wrapper. Make
      `store_has_combos` `pub(crate)` and delete the wrapper.
- [x] `hypgeo.rs`, `cards_seen_by`: used only by tests. Make it
      private or move its test use into the test module.
- [x] `game_run.rs`, `expire_crew`: no external callers. Tighten to
      `pub(super)`.
- [x] Audit `pub(crate)` module declarations in `mod.rs` with no
      external users and tighten them.

## Phase 3: dead AST variants wired into lowering

The point of the AST layer is to be the single parse of a card. Today
four `StaticEffect` variants are parsed but never consumed; parallel
raw-text scans produce the real flags. Wire the AST into lowering and
delete the duplicate text scans. This is the "lean into the AST"
direction.

- [x] `StaticEffect::AdditionalLandDrop`: `static_flags.rs` sets
      `extra_land_drops` from `text.contains("additional land")`.
      Change it to read the AST variant (already parsed by
      `parse_oracle_static`). Delete the text scan.
- [x] `StaticEffect::CostReduction`: `parse_min_cost` scans
      "cost {N} less to cast". Move the reduction to the AST variant
      and lower it into the cost fields in `card_lower.rs`. Delete
      the scan in `parse_min_cost`.
- [x] `StaticEffect::KeywordGrant`: `static_flags.rs` reads evasion
      and haste partly from raw text. Change the keyword grant path to
      read `KeywordGrant` from the AST (grant of Flying, Trample,
      Menace sets the evasion census on the granter's board presence;
      grant of Haste to the card itself only when the grant targets
      the source — keep the existing "granter does not attack on its
      entry turn" behavior). Delete the matching text scans.
- [x] `StaticEffect::Unsupported`: keep as the catch-all. Confirm no
      lowering path matches on it.
- [x] Add a test per consumed variant: a card whose only relevant
      text is the static line, asserted through the lowered
      `SimCard` fields, with a matching and a nonmatching case.

## Phase 4: domain model restructure

These changes make the model match the rules vocabulary and remove
the remaining modeling mismatches. Do them in this order; each step
compiles and passes tests before the next.

### 4.1 Saga chapters as first-class data

Chapters are stored today as a synthetic `Tier { at: 0 }` holding
`Trigger::Activated` abilities, and `chapter_count()` counts
Activated abilities. Per rule 714.2, chapters are triggered
abilities, and a real activated ability on a Saga would corrupt the
count.

- [x] Add `chapters: Vec<Effect>` to `SimCard` (suggested: a
      `SagaData { chapters: Vec<Effect> }` struct if more saga state
      appears later).
- [x] `oracle_lower.rs`: lower `SagaChapter` AST nodes into
      `chapters` in play order (sorted by chapter number).
- [x] `InPlay.saga_step` becomes the index into `chapters` (keep it
      a plain counter; the vec provides the count).
- [x] `game_run.rs`, `run_sagas`: read `card.chapters` directly.
      Delete the tier/Activated re-derivation.
- [x] `game.rs`, `fire_on_enter`: fire `chapters[0]` at entry.
      Delete the `Trigger::Activated` lookup.
- [x] `station_tiers` returns to Station cards only.
- [x] Update `docs/simulator.md` and `docs/architecture.md` where
      sagas are described as tiers.

### 4.2 `CardRef` on the battlefield; `CardIdx` newtype for zones

The sentinel values `usize::MAX` (commander) and `usize::MAX - 1`
(token) are scattered across comparisons in five files. Make the
distinction explicit in one type.

- [x] New type in `model.rs` or `game.rs`:
      `CardIdx(pub u32)` — index into `SimDeck.cards`.
- [x] New enum `CardRef { Deck(CardIdx), Commander { slot: usize },
Token }`.
- [x] `InPlay.card` becomes `CardRef`. `is_commander` becomes
      redundant (the variant carries it) — remove the field and match
      on `CardRef::Commander` where it is read.
- [x] Zone vectors (`library`, `hand`, `graveyard`, `exile`,
      `flashback_permissions`, seen maps) become
      `Vec<CardIdx>`/`HashMap<CardIdx, u32>`.
- [x] `card_of` becomes a single match on `CardRef` with no range
      checks. Delete every `perm.card < usize::MAX - 1` comparison
      (search for `usize::MAX` across `src/deck/simulator/`).
- [x] Token sentinel constants in `cast_phase.rs` and elsewhere get
      replaced by `CardRef::Token` tokens pushed onto the battlefield.
- [x] Keep `usize` conversions at the CLI/report boundary only.

### 4.3 Rename `Trigger` to `AbilityTiming`

Per rule 113.3, activated abilities are not triggered abilities. The
runtime enum mixes the two. Rename to `AbilityTiming` and, where it
improves call sites: `OnFirstMainPhase` → `PrecombatMainPhase`
(rule 505.1). Purely mechanical; do it with a single rename commit.

### 4.4 Consume the intervening "if" condition

Per rule 603.4 the condition is checked when the trigger fires. The
sim's goldfish board makes most conditions trivially true, but
wiring the parsed condition lets future support check it.

- [x] Lower `TriggeredAbility.condition` into the runtime `Ability`
      (a new `condition: Option<String>` field).
- [x] Where triggers fire (`fire_triggers`, `fire_on_enter`), skip
      firing when a supported condition shape is parsed and fails the
      check; unsupported condition strings stay inert (document in
      the assumptions list). Start with the only shapes the runtime
      can evaluate: none. If no shape can be evaluated yet, do not
      add dead matching code — instead lower the condition and
      document that evaluation is future work, or delete the field
      (see phase 2). Decide during implementation; the rule is: no
      dead branches.
      Result: the parser captures the if-clause text and the lowering
      carries it into `Ability.condition`. No condition shape is
      evaluated yet, so nothing skips firing; every lowered condition
      fires as if true. Documented in `assumptions` and on both
      fields; no dead branches.

### 4.5 Rename `InPlay` to `Permanent`

"In play" left the rules vocabulary; rule 403 says battlefield
permanent. Mechanical rename.

### 4.6 Group the flat `SimCard` flag fields

`SimCard` has roughly 70 flat fields. Group the ones that travel
together. Keep field grouping shallow — one level, no behavior.

- [x] Move the `CastRiders` struct from `oracle_parse/cast_riders.rs`
      into `model.rs` and make it one `SimCard` field
      (`riders: CastRiders`). Delete the 26 copy-paste assignments in
      `lower_cast_fields`.
- [x] Same for the `StaticFlags` bools that survive phase 3
      (group into one `CombatFlags` or similar struct).
- [x] Document every field of `CastRiders`, `StaticFlags`, and
      `TurnCensus` (0/26, 0/15, 0/30 documented today).
- [x] Do not group fields that lowerings or reports read one at a
      time — stop when grouping stops paying for itself.

### 4.7 Keyword table

- [x] In `oracle_parser/keywords.rs`, replace the two hand-synced
      30-entry keyword lists (`keyword_name` and
      `known_keyword_head_end`) with one
      `const KEYWORDS: &[(&str, KeywordName)]` that drives both.
- [x] Give `KeywordName` a category tag (keyword ability, keyword
      action, ability word) as data. `Transform` is a keyword action
      (701.27); Landfall-class ability words never appear as
      `KeywordName` variants — confirm the parser routes them through
      `strip_ability_word` instead.

### 4.8 Model cleanups

- [x] `Effect::WheelSkip(usize)`: a hand index inside the shared
      model enum. Move the skip detail into the wheel resolution
      path (a small private type in `game_effects.rs`).
- [x] `TriggerEvent::TappedForMana { nonland, player }`: both fields
      are constant at every construction site. Simplify to a unit
      variant unless a new card shape needs them.
- [x] Move the text helpers `draw_amount` and `amount_after` from
      `model.rs` into the parser layer where they are used.
- [x] Fix the stale header comment in `model.rs` that says parsing
      lives in `parse` (a deleted module).

## Phase 5: wrappers and performance

- [ ] `game_mana.rs`: `add_yield` → `add_yield_turns_empty_board` →
      `add_yield_turns(&empty_deck(), ...)` — the tap budget builds a
      throwaway `SimDeck` (two Vec allocations plus a format lookup)
      per tap, per turn, per game, across 10,000 runs. Change
      `add_yield_turns` to take `Option<&SimDeck>` or split a
      non-scaling fast path, and delete `empty_deck()`.
- [ ] `mod.rs`: `sim_report_for` (lines 269-293) and `simulate`
      (lines 371-395) duplicate a 25-line stats/classify block.
      Extract one helper.
- [ ] `parse_keywords.rs` → rename to `parse_equipment.rs`: the file
      parses equipment and creature buffs, not keywords. The name
      collides with the real keyword parser.

## Phase 6: doc comments and documentation

### 6.1 Rust doc comments

- [ ] Convert the plain `//` header above each `mod` declaration in
      `mod.rs` (28 mods) to per-mod `///` doc comments.
- [ ] Add `//!` headers to the 24 files that have plain `//` headers
      (add the `!`; the text is already good).
- [ ] Document the four undocumented functions:
      `report_view.rs` `print_hypgeo`, `mod.rs` `store_has_combos`,
      `report.rs` `round2`, `combos.rs` `zone_times`.
- [ ] Document the child-mod declarations in `cast_phase.rs` and
      `game_run.rs`.

### 6.2 Document the state-representation contract

- [ ] Add module docs to `game.rs` stating the contract: zones hold
      indexes into the immutable `SimDeck.cards` table; per-instance
      state lives only on the battlefield `Permanent` and resets on
      zone change (rule 122.2); a card re-entering a zone is the same
      index, with no incarnation counter.
- [ ] Add one line to `docs/simulator.md` assumptions about the
      index-aliasing approximation (a card milled, returned, and
      discarded again is one index).

### 6.3 `docs/simulator.md` corrections

- [ ] Remove the false proliferate-on-combat-damage claim (around
      lines 837-839, 848) and align with the runtime assumptions
      string (proliferate is not modeled).
- [ ] Fix "phases 1-11" → 10 phases (around line 49); fix the
      `game_*.rs` parenthetical (around line 50) — cast lives in
      `cast_phase.rs`, extra turns in `game_run.rs`.
- [ ] Fix the fetch-targets-enter-tapped claim (around line 405) —
      tapped entry is derived from Oracle text now.
- [ ] Add `cast_phase/cast_sweep.rs` and `game_run/census.rs` to the
      module tree.
- [ ] Fix the saga chapter timing description (phase 4 result) and
      the Monarch draw step (phase 1.6 result).
- [ ] Fix: `card_castability` rows are per copy, not per distinct
      card; the fixture-naming sentence (hyphens and underscores are
      both used — make the sentence match reality); draw-starvation
      is turn 5 for a 60-card deck; `explain` → `explain_with_threshold`.
- [ ] Add a note that milestone output nests per card under the
      velocity key, if the JSON does so.

### 6.4 Other docs

- [ ] `docs/architecture.md`: replace the stale `parse` module name
      (around lines 390, 394, 409-410, 422-423); fix the "flat cuts"
      claim (board-scaled improvise/affinity); fix the stale
      "X-costs pay for one" and "flat body power (2)" claims where
      they disagree with code.
- [ ] `.agents/skills/seizethemana/SKILL.md`: fix the extra-turn
      description (around line 475, pre-refactor behavior); add
      `milestones` and `colored_sources` to the JSON shape list
      (around lines 707-713).
- [ ] Simplified Technical English pass on the worst sentences: the
      60-word Karsten sentence in `docs/simulator.md`, the lethal
      table cell, the 120-word test list, the architecture bullet.
      Expand "MDFC" on first use.
- [ ] Run `prettier --write` on every changed `.md` file, then
      `prettier --check` on the same files.

## Validation

After every checklist item:

```sh
cargo nextest run
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

After each phase, additionally:

```sh
aislop scan --changes
```

Embedding-heavy commands (`stm setup`, `stm sync`) are not affected
by this plan. If a doc-layout change requires re-embedding, build
with `cargo build --release` first per the project instructions.

## What this plan deliberately does not do

- No per-instance objects outside the battlefield, no ECS, no
  heavyweight card object graph. Research on Forge, Magarena, XMage,
  Phase, and Landlord shows flat-index zones plus a battlefield
  permanent struct is the right design for a Monte Carlo goldfish.
- No rule-fidelity work beyond the items listed: the stack, layers,
  targets, and opponent interaction stay out of scope, per the
  simulator's assumptions.
