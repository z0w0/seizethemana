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

- Daily data files (cards and tags) re-download when older than a day, so
  price and tag refreshes actually pick up new data instead of reusing
  yesterday's file.
- Deck card counts separate the sideboard. `stm deck show` and
  `stm deck list` now report the maindeck count in `cards` plus a new
  `sideboard_cards` field, and `stm deck simulate` no longer shuffles
  sideboard cards into the library (`deck_shape.sideboard_cards` counts
  them). Human output reads `100 cards + 15 sideboard`.

### Fixed

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