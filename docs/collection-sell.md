# Sell rules and evidence

`stm collection sell` helps release collection value while preserving cards
for deckbuilding. It recommends copies, not whole card holdings.

## Protection

Use `--exclude-binder Collect` to leave collector copies out of the
deckbuilding pool. Repeat the flag for multiple binders. Names match exactly,
ignoring ASCII case. Unknown binder names are errors. Excluded copies cannot
cover spare reserves, deck shortages, or personally owned combo options.
Exclusion never edits inventory or removes deck-assigned copies.

Assigned copies never enter the sale pool. For each deck and card, protect
`max(0, demanded - assigned_to_that_deck)` binder copies. Sum these shortages
across decks. Main deck, commander, and sideboard entries count as demand.
Maybeboard entries mark interest but do not consume copies.

Keep one extra binder copy when a card is a Game Changer, has Commander rank
at most 5000, has Penny rank, completes a known deck combo, or belongs to a
fully owned combo that fits a known deck identity. Required commanders must
match an actual commander. Unknown color identities do not establish a fit.

Cards with a priced printing below $1 keep four extra binder copies instead.
The reserves do not stack. Allocate protection to cheaper known owned
printings first. Unknown-price copies sort after priced copies and are never
recommended as singles or bulk.

## Singles and review

Singles need a per-copy printing price of at least $1. Price and rarity filters
narrow actions after protection. A stack of sub-$1 copies remains bulk even
when its total retail value exceeds $1.

Move an action to review when metadata is unknown, the card is on the Reserved
List, or deck demand could not be checked. Also review final binder-copy sales
with maybeboard interest. Missing ranks, recent releases, and general combo
participation affect priority instead of blocking recommendations.
Combo coverage is unknown when the table is empty or its latest refresh is
undated or older than seven days. Positive combo evidence still protects
copies when the snapshot is stale. Review actions do not enter recommended
totals or funding.

## Priority policy

`sell_priority` is an ordering metric, not a probability or proceeds estimate.
The policy weights are not statistically calibrated.

When an action retains another binder copy, priority equals sale market value.
Otherwise, multiply sale value by `1 - strongest_demand`:

- Commander demand is 1 at rank 5000 or better. It decreases linearly to 0 at
  `--rank-floor` (default 15000). Missing Commander rank uses a 0.35 demand weight.
- Penny demand is `0.6 / (1 + max(rank, 1) / 500)` when ranked. Missing Penny
  rank adds no negative evidence.
- Combo demand is `min(0.6, 0.15 * log2(1 + distinct_piece_sets))`.
- A first known release within 90 days has a 0.15 demand weight. New printings
  of an older card do not reset its age.
- Use the largest demand value instead of adding correlated signals.

Count distinct canonical card sets and commander requirements, not stored
face rows. Report variant counts separately. Different prerequisites can
share a piece set, so this count is not a claim that those variants are
equivalent. Up to three personal combo IDs support inspection.

Sort recommended singles by priority, then sale value, then name. A general
combo listing is not a personal hold: useful reserves depend on combinations
that fit a known deck or the included collection pool.

## Human output

The default view shows 20 cards in a colored table: card, sell quantity, keep
quantity, and value in USD. Names fit the terminal width. Bulk shows binder
locations instead of individual market values. Color follows the existing TTY
and `NO_COLOR` rules.

Use `--details` for sale printings, present ranks, combo counts, and diagnostic
notes. Retained printing lists and missing-rank messages are omitted. Use
`--review` to add the review table. Short footer hints point to bulk, more rows,
and optional details. JSON retains the full evidence fields and includes
`excluded_binders`, regardless of the human display flags.

## List exports

Use `--output txt` or `--output csv` to write the sell list to stdout:

```sh
stm collection sell --exclude-binder Collect --output csv > sell.csv
stm collection sell --exclude-binder Collect --output txt > sell.txt
stm collection sell --bulk --output csv > bulk-sell.csv
stm collection sell --target 50 --output txt > funding-sell.txt
```

Export all matching recommendations, independent of `--limit`. With `--target`,
export only its selected funding allocations. Exclude retained copies, review
cards, and excluded binders. Merge identical sale printings across source
binders. Keep the ranked card order.

TXT uses the shared ManaBox format: `quantity Name (SET) number [*F*]`.
It has no section headers. Regular and etched foils both use `*F*`.
CSV preserves `normal`, `foil`, and `etched` separately. Its columns are
`Binder Name`, `Binder Type`, `Name`, `Set code`, `Collector number`, `Foil`,
and `Quantity`. Rows belong to the non-owning ManaBox list `Sell` (`list`).
Do not invent purchase prices, conditions, or print IDs.

The export contains no colors, summaries, or diagnostic notes. `--output`
conflicts with `--json`, `--details`, and `--review`. An empty TXT export is
empty; an empty CSV export contains its header. Both return exit code 3.

The formats follow ManaBox's
[collection import guide](https://manabox.app/guides/collection/import-export)
and [text format](https://manabox.app/guides/decks/import-export).

## Bulk view

`--bulk` selects priced printings below $1. Sort by excess-copy quantity, then
name. Show sale quantities, retained quantities, and binder locations.
`--details` adds exact sale printings. JSON separates normal commons/uncommons,
foils, rares/mythics, and other cards. Printing price takes precedence over rarity.

The default singles view summarizes bulk without consuming its display limit.
`--limit` applies separately to recommendations and review rows. Summaries
cover all matching actions before those limits.

Do not sum bulk retail prices as sale proceeds. `--bulk-rate USD` supplies a
uniform rate per 1,000 selected copies. Use filters to match a specific quote.
For example, 2,000 selected copies at $5 per 1,000 estimates $10 before
shipping. Bulk and review rows never enter `--target`.

## Funding and data limits

`--target USD` selects ranked singles until their market value reaches the
target or recommendations run out. Display limits do not restrict the plan.
Report the achieved value and shortfall. These values exclude fees, shipping,
condition adjustments, and replacement purchases.

`--format` scopes Spellbook legality and reports card legality. Legality does
not establish usage. The store has no Modern, Pioneer, Legacy, or Pauper usage
statistics. Missing ranks mean unknown demand, not proof of no play.

Prices are snapshots. Retaining another copy avoids losing loose-copy access,
but neither price nor popularity proves that selling matches a collector's
preferences. The command does not predict prices or reprints.

## Research sources

Reviewed on October 3, 2026:

- [Scryfall card objects](https://scryfall.com/docs/api/cards): ranks are
  nullable; legality and printing prices are separate fields.
- [EDHREC data sources](https://edhrec.com/about-us) and
  [top cards](https://edhrec.com/top): Commander decklist popularity, not
  all-format usage or measured gameplay.
- [Penny Dreadful](https://pennydreadfulmagic.com/about): a rotating Magic
  Online format based on digital card prices.
- [Commander Spellbook](https://commanderspellbook.com/about/) and its
  [search interface](https://commanderspellbook.com/search): community combo
  records distinguish variants from grouped combos.
- [Commander bracket update](https://magic.wizards.com/en/news/announcements/commander-brackets-beta-update-february-9-2026):
  Game Changers affect expected gameplay and bracket choices.
- [TCGplayer fees](https://help.tcgplayer.com/hc/en-us/articles/201357836-TCGplayer-Fees):
  market prices differ from net proceeds.
- [TCG Bulk Kings](https://tcgbulkkings.com/shop) and
  [Card Kingdom selling instructions](https://www.cardkingdom.com/purchasing/how_to_sell):
  bulk uses lot rates and buyer-specific availability. Rates must remain
  explicit inputs, not permanent assumptions.
