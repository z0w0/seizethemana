# Sell rules and evidence

`stm collection sell` recommends surplus binder copies after deck needs and
spare reserves. It does not edit collection records, decklists, or locations.

```sh
stm collection sell --exclude-binder Collect
stm collection sell --details --review
```

Check quantities before exporting. Market value is an estimate, not a buyer's
offer or proceeds after costs. Run `stm sync` when you need a fresh snapshot;
the sell command does not start an automatic refresh.

Use [list exports](#list-exports) to create a ManaBox sell list. Read
[protection rules](#protection) and [review rules](#singles-and-review) before
acting on a recommendation.

## Options

| Option                               | Scope                                                                                                      |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `--exclude-binder NAME`              | Exclude a whole binder from sales and deckbuilding supply. Repeat for multiple binders.                    |
| `--details`                          | Show sale printings and demand evidence.                                                                   |
| `--review`                           | Show candidates that need a personal decision or more data.                                                |
| `--bulk`                             | Select priced copies below $1 instead of singles. Cannot combine with `--target`.                          |
| `--bulk-rate USD`                    | Estimate bulk proceeds per 1,000 selected copies. Requires `--bulk` and a positive rate.                   |
| `--min-price USD`, `--max-price USD` | Filter sale printings after protection. Singles still require a price of at least $1 per copy.             |
| `--rarity RARITY`                    | Select `common`, `uncommon`, `rare`, or `mythic`.                                                          |
| `--format FMT`                       | Scope combo evidence and report card legality. Defaults to Commander combo evidence.                       |
| `--rank-floor N`                     | Set the low-demand Commander boundary. Defaults to 15000; must exceed 5000.                                |
| `--target USD`                       | Select ranked singles toward a positive market-value target.                                               |
| `--limit N`                          | Cap rows in each section, including JSON. Defaults to 20; maximum 500. Totals and funding remain complete. |
| `--json`                             | Include evidence, summaries, excluded binders, and printing allocations.                                   |
| `--output txt`, `--output csv`       | Export all matching sale allocations. Cannot combine with `--json`, `--details`, or `--review`.            |

## Protection

Binder names match as whole names, ignoring ASCII case. An unknown excluded
binder is an error. Excluded copies cannot cover reserves, deck shortages, or
personally owned combo options. Exclusion does not move or delete copies.

Assigned copies never enter the sale pool. For each saved deck and card, protect
`max(0, demanded - assigned_to_that_deck)` binder copies. Add these shortages
across decks. Main deck, commander, and sideboard entries count as demand.
Maybeboard entries show interest but do not use copies. Assignments without a
saved decklist remain protected, but the command cannot check that deck's unmet
demand. Plains, Island, Swamp, Mountain, and Forest are excluded from sales.

Keep one extra binder copy for a Game Changer, a Commander rank of 5000 or
better, a Penny rank, or a personally useful combo piece. A combo is useful when
the card completes it in a saved deck or all its pieces are owned. Commander
combos must fit a saved deck's known color identity, and required commanders
must match actual commanders. An unknown identity does not establish a fit.

If any included owned printing has a price below $1, keep four extra binder
copies instead, even without a usefulness signal. Reserves do not stack.
Protect cheaper known owned printings first. Unpriced copies sort last and
are never recommended as singles or bulk.

## Singles and review

Singles need a printing price of at least $1 per copy. Copies below $1 remain
bulk even when their total retail value exceeds $1.

Sales need review when metadata is unknown, the card is on the Reserved List,
or a saved decklist could not be checked. A sale also needs review if it leaves
no binder copy of a maybeboard card. Review candidates stay outside totals,
funding, and exports. Missing ranks, recent releases, and general combo
participation affect priority but do not block sales by themselves.

Combo coverage is unknown when the table is empty or its latest refresh is
undated or over seven days old. Positive combo evidence still protects copies
in stale snapshots. No matches in stale data do not establish zero demand.

## Priority policy

`sell_priority` sorts recommendations. It is not a probability or proceeds
estimate, and its weights are not statistically calibrated.

When an action keeps another binder copy, priority equals its sale market value.
Otherwise, the command multiplies sale value by `1 - strongest_demand`:

- Commander demand is 1 at rank 5000 or better. It decreases linearly to 0 at
  `--rank-floor` (default 15000). A missing Commander rank uses a 0.35 demand
  weight.
- Penny demand is `0.6 / (1 + max(rank, 1) / 500)` when ranked. A missing Penny
  rank adds no negative evidence.
- Combo demand is `min(0.6, 0.15 * log2(1 + distinct_piece_sets))`.
- A card whose first known release was within 90 days has a 0.15 demand weight.
  New printings of an older card do not reset its age.
- Use the largest demand value. Do not add correlated signals.

Combo counts use distinct canonical card sets and commander requirements, not
stored face rows. Variant counts are separate. Different prerequisites can
share a piece set, so the variants need not be equivalent. The report includes
up to three personal combo IDs for inspection.

Singles sort by priority, then sale value, then name. General combo participation
alone does not create a personal reserve.

## Human output

The default table shows card, sale quantity, retained quantity, and value in
USD. Bulk shows binder locations instead of market values. Color follows
terminal capability, `NO_COLOR`, and `--no-color`.

`--details` shows sale printings, available ranks, combo counts, and diagnostic
notes. It omits retained printing lists and missing-rank messages. `--review`
adds the review table. JSON includes evidence regardless of these human display
flags. It separates `singles`, `bulk`, `protected`, `rows`, and `review`;
printings include sale and keep quantities.

## List exports

Use `--output txt` or `--output csv` to write the sell list to stdout:

```sh
stm collection sell --exclude-binder Collect --output csv > sell.csv
stm collection sell --exclude-binder Collect --output txt > sell.txt
stm collection sell --bulk --output csv > bulk-sell.csv
stm collection sell --target 50 --output txt > funding-sell.txt
```

Exports ignore `--limit`. With `--target`, they contain only funding picks.
They omit retained copies, review cards, and excluded binders. Identical sale
printings merge across source binders; cards keep their ranked order.

TXT uses the shared ManaBox format: `quantity Name (SET) number [*F*]`.
It has no section headers. Regular and etched foils both use `*F*`.
CSV keeps `normal`, `foil`, and `etched` separate. Its columns are
`Binder Name`, `Binder Type`, `Name`, `Set code`, `Collector number`, `Foil`,
and `Quantity`. Rows belong to the non-owning ManaBox list `Sell` (`list`).
Exports do not add purchase prices, conditions, or print IDs.

Exports contain no colors, summaries, or diagnostic notes. Empty TXT has no
content; empty CSV contains its header. Both return exit code `3`.

The formats follow ManaBox's
[collection import guide](https://manabox.app/guides/collection/import-export)
and [text format](https://manabox.app/guides/decks/import-export).

## Bulk view

Bulk sorts by excess-copy quantity, then name. `--details` adds exact sale
printings. JSON separates normal commons/uncommons, foils, rares/mythics, and
other cards. Printing price takes precedence over rarity.

The singles view summarizes bulk. Summaries cover all matching actions before
row limits.

Bulk has no proceeds estimate without `--bulk-rate USD`. The rate applies to
every selected copy, so use rarity and price filters to match a buyer's quote.
For example, 2,000 copies at $5 per 1,000 estimates $10 before shipping. Bulk
never enters `--target`.

## Funding and data limits

`--target USD` selects ranked singles until their market value reaches the
target or recommendations run out. It selects all sale copies of each chosen
card, so it can exceed the target. The report shows achieved value and shortfall
before fees, shipping, condition adjustments, and replacement purchases.

`--format` scopes Spellbook legality and reports card legality. Legality does
not establish usage. The store has no Modern, Pioneer, Legacy, or Pauper usage
statistics. A missing rank means demand is unknown, not that no one plays the
card.

Prices are snapshots. Price and popularity do not show whether a sale fits your
collection goals. The command does not predict prices or reprints.

## Data sources

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
