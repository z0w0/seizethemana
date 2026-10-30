# Commander source fixtures

This bundle contains 1,208 unique Oracle identities captured on October 30, 2026.
It is separate from the historical 250-card review and 356-identity source
registry in `../popular_card_fixtures/`. Do not replace those source fixtures when
maintaining this bundle. Review compiler-pin changes as a separate step.

Source presence, compiler records, executed runtime branches, approximations,
and exclusions are different claims. No source count proves complete card
support. See [simulator rules and limits](../../../../../docs/simulator.md) before
using these fixtures to describe coverage.

Use [selection and saved evidence](#selection-and-saved-evidence) to trace inputs,
[test scope](#test-scope) to check claims, and [capture and review](#capture-and-review)
to maintain the bundle.

## Selection and saved evidence

`sources/2026-10-30/` contains 1,185 sources:

- 1,000 EDHREC cards in "Past 2 Years" site order. The saved responses cover
  `top/year.json` and `top/year-past2years-1.json` through
  `top/year-past2years-9.json`, with 100 cards per page. The capture stops after
  the tenth page; it is not the complete EDHREC list.
- 185 distinct added lands selected by [`land_cycles.json`](land_cycles.json).
  Complete ten-card cycles cover shock, Verge, pain, slow, surveil, check, fetch,
  bond, triome, fast, filter, bounce, Pathway, and original dual lands. The groups
  also include all five Neon Dynasty Channel lands, all five original Kamigawa
  legendary lands, basics, and the listed fetch and utility lands.

`sources/2026-10-30-behold/` contains 23 additional sources from the complete
saved Scryfall query `keyword:behold legal:commander`. Elven Passage belongs to
this result and is also an explicit utility selection. It counts once.

The two captures keep ten raw EDHREC responses, one raw Behold query response,
and 17 Scryfall collection request/response pairs. Each `capture.json` records
source URLs, capture times, and SHA-256 hashes of original file bytes.
`selection.json` records requested identities. `cards.json` keeps simulator
input metadata; the response files keep the full API objects. The base capture
also preserves `popularity.json` and its copy of `land_groups.json`.

EDHREC source position is not Scryfall `edhrec_rank`. The saved "Past 2 Years"
window does not give exact date bounds. Some selected printings can be future
reprints of released cards. Keep print IDs, Oracle IDs, and source revisions
separate.

The exact EDHREC selection includes One, Borg Evolved with
captured Commander legality `not_legal`. Do not label the whole bundle
Commander-legal. The Behold supplement's legality filter does not apply to the
base selection.

## Test scope

[`../commander_source_tests.rs`](../commander_source_tests.rs) checks saved-byte
hashes, continuation links, site order, complete land groups, request/response
identity, and production face flattening. These are source checks, not runtime
proof.

[`../commander_corpus_tests.rs`](../commander_corpus_tests.rs) compiles every
source and checks distinct identities and stable compiler records. Its runtime
smoke check starts each source in hand on empty and established boards, executes
one full turn, checks physical-card conservation, and constructs a report with
outcomes and compiler evidence. It does not assert every printed cost, target,
condition, event, or effect.

Named tests provide narrower evidence. Examples include
[`../commander_land_entry_tests.rs`](../commander_land_entry_tests.rs) for
checkland and Verge conditions and
[`../commander_behold_tests.rs`](../commander_behold_tests.rs) for Behold choices,
costs, and the found land's identity. Native mana on a captured utility land can
work while its Channel, combat, or opponent-facing effects remain unsupported.
Read the [supported mechanics and fixed choices](../../../../../docs/simulator.md#supported-mechanics-and-fixed-choices)
with the report's assumptions. Broad corpus execution is not whole-card
certification.

## Capture and review

Ordinary tests use saved files and do not access the network. Capture is an
explicit maintenance action. From the repository root, use Bash 3.2 or later,
`git`, `jq`, `shasum`, `curl` with `--fail-with-body`, and standard Unix tools.
Use a new destination with physical parent paths, not symlinks.

These commands contact EDHREC and Scryfall. They keep the base and Behold
captures separate:

```bash
capture_review=$(mktemp -d)
capture_review=$(cd "$capture_review" && pwd -P)
bash scripts/capture-commander-fixtures.sh \
  src/deck/simulator/tests/commander_card_fixtures/land_cycles.json \
  "$capture_review/commander"
bash scripts/capture-commander-fixtures.sh --behold "$capture_review/behold"
```

The script checks page continuity, requested identities, and complete responses.
It spaces requests by at least 150 milliseconds. It refuses existing outputs,
dangling symlinks, and symlink parents. On handled failure, it removes only its
own incomplete capture. It does not remove an existing bundle.

Review source hashes, selected identities, legality, metadata, and Oracle text
before registering a new dated capture. Keep older captures unchanged. A source
refresh does not approve compiler changes or runtime claims.

Run the capture tests without network access:

```bash
cargo nextest run --test fixture_capture
```

The Rust tests replay local responses and check request pacing, failed captures,
identity mismatches, output safety, and unchanged pinned bytes. These capture
checks do not certify card behavior.
