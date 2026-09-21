# Changelog

All notable changes to `stm` get documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

First public state. No version has shipped yet; while that's true,
breaking changes land in this section.

### Added

- Semantic search over every Magic card. Searching mixes two approaches
  (keyword matching and a small on-device AI model), so "sacrifice outlet"
  finds cards whose rules text never says those words.
- Collection tracking from ManaBox CSV exports: binders, per-print
  quantities, purchase prices, and an overview with collection value,
  mana curve, and color breakdown. Wishlist rows are ignored.
- Deck files in ManaBox's txt format, each with a strategy-notes
  (primer) file. Create, edit, import, and export decks; `stm deck show`
  reports what you own versus what you'd need to buy, with prices.
- `stm deck legal`: checks deck size, copy limits, commander rules and
  color identity, per-card format legality, and the Game Changer allowance
  for Commander brackets. Judgment-call bracket rules come back as a
  review checklist instead of a guess. Exits 0 when legal, 1 when not.
- Community role labels from Scryfall's Tagger project: shown on
  `stm card` and used by `stm card similar` to find cards that play like a
  given card (`--owned` limits the search to your collection).
- Deck pricing in `stm deck show`: per-card prices, owned value, and what
  buying the rest would cost. Basic lands are treated as free and
  unlimited.
- An agent skill for coding assistants that use `stm`, covering the tool
  itself plus a step-by-step, budget-first deckbuilding workflow.

### Changed

- Simulator numbers changed; regenerate baseline reports after
  upgrading. The 60-card London mulligan now follows Karsten's model
  (redraw 0/1/6/7-land openers once; only the redrawn hand bottoms one
  card toward 3 lands — a land at 4+ lands, else a spell — so kept hands
  stay at 7 cards), so screw, flood, and velocity rates shift from saved
  baselines. Land/spell MDFCs (Valakut Awakening class) play as a land
  only when the hand holds no other land and count 0.4 land (0.75
  mythic, by the rarity column) in the mana base. Cascade casts the
  cheapest cheaper card from the library for free, once. The simulate
  report gains a best-case kill-turn census (`wincons.p50_lethal_turn`);
  combat damage there is cumulative and life the goldfish pays itself
  (additional costs) no longer counts as damage dealt. The 60-card mana
  audit's pip table is keyed on the full cost shape (2CCC now reads its
  Karsten floor of 22 instead of the unkeyed row).
- Daily data files (cards and tags) re-download when older than a day, so
  price and tag refreshes actually pick up new data instead of reusing
  yesterday's file.
- Deck card counts separate the sideboard. `stm deck show` and
  `stm deck list` now report the maindeck count in `cards` plus a new
  `sideboard_cards` field, and `stm deck simulate` no longer shuffles
  sideboard cards into the library (`deck_shape.sideboard_cards` counts
  them). Human output reads `100 cards + 15 sideboard`.

### Fixed

- `--max-price` applied after the result cut, so a tight cap returned
  fewer rows than `--limit` even when affordable candidates existed. The
  cap is now a store-level filter on `query` and `collection query` and
  over-fetches then caps before the limit cut on `deck suggest`; unpriced
  and above-cap cards are excluded in all paths. `--max-price` rejects
  negative values at parse time.
- The curve histogram collapsed zero-mana and one-mana cards into one
  slot; the JSON `curve.histogram` is now 7 slots indexed MV 0..6+ and
  the human line reads the same shape. Commander curve targets follow
  the average MV (three bands) instead of one fixed sentence, and the
  `deck show --json` `curve.target` matches the human line's format.

- Legendary Vehicles and Spacecraft with a printed power/toughness box are
  accepted as commanders, per the Edge of Eternities Commander rules
  change (2025). Color identity still applies.

- Cards you own in a different printing than the one Scryfall highlights
  showed as missing (`own 0/N`) even though you had them.
- Cards banned in a format slipped past the `--format` filter and showed
  as legal in the card view.
- `deck update` accepted made-up card names; unknown names now fail with
  "did you mean" suggestions.
- Wishlist rows were counted wrong in `collection import` summaries.
- Ramp statistics missed cards that produce mana in words rather than
  mana symbols (Birds of Paradise).
- `card similar --owned` could return fewer results than requested when
  owned matches ranked below an internal cutoff.