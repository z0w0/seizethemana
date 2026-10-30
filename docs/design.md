# CLI design

Use these rules for CLI text, reports, and terminal styling. `stm` is a plain
command-line tool, not a full-screen interface. Its pink accent and terminal art
match the website.

## Voice

- Use Simplified Technical English at a 10th-grade reading level or below.
- Lead with the result, problem, required action, or next step.
- Use short sentences, active voice, and familiar words. Explain needed jargon.
- Use sentence case for headings, labels, and table columns. Use terms
  consistently across help, output, docs, and the site.
- Remove filler, inflated claims, fake urgency, and generic success messages.
  Avoid em dashes, en dashes, decorative emoji, and excess bold text.
- State costs, risks, failures, and required actions directly, especially before
  destructive actions.
- Preserve card names, paths, user data, and quoted source text exactly.
- Support price, performance, card-support, and simulator claims with evidence.
- Keep task instructions separate from factual references. Link to related
  details instead of repeating them.

For general writing guidance, see
[Google's developer documentation style guide](https://developers.google.com/style/highlights)
and [Diátaxis's documentation types](https://diataxis.fr/).

## Terminology

| Term                              | Use                                                        | Do not imply                                               |
| --------------------------------- | ---------------------------------------------------------- | ---------------------------------------------------------- |
| Collection                        | Owned cards and their locations                            | A saved decklist                                           |
| Decklist                          | Saved intended deck contents                               | Ownership of every listed card                             |
| Owned                             | Copies owned within the command's stated scope             | Copies available without taking cards from another deck    |
| Available                         | Copies eligible within the stated binder or deck scope     | Every owned copy                                           |
| Price                             | The stated market-price basis in USD                       | Guaranteed sale proceeds or a fixed checkout price         |
| Unpriced                          | Price is unknown                                           | Free or zero dollars                                       |
| Mana readiness                    | Modeled payment capacity                                   | Hand access, a legal target, or an actual cast             |
| Card access                       | Arrival in a stated zone                                   | An active engine or resolved effect                        |
| Potential combat damage           | Unblocked damage under the stated model                    | Damage applied to an opponent or a win                     |
| Potential commander combat damage | Unblocked combat damage from one commander                 | Combined partner damage, spell damage, or an opponent loss |
| Finding                           | A diagnostic result with evidence and limits               | A required deck edit                                       |
| Finished game                     | A game that reached the horizon or a supported player loss | A win or full-horizon survival                             |
| Incomplete game                   | Execution stopped at a safety limit                        | A normal loss, success, or proven infinite combo           |
| Unsupported ability               | Required behavior cannot execute                           | A free cost, partial reward, or fully supported card       |
| Verified scenario                 | A stated source revision passed named assertions           | General correctness for the whole card                     |

Keep public command names, flags, exit codes, JSON keys, enum values, finding
identities, and metric paths stable. Review interface changes separately from
wording edits. Human labels may use "games" without changing `--runs`.

## Messages

Name the affected object and give a valid next step when one exists.

| Message type       | Order                                        | Guidance                                                                               |
| ------------------ | -------------------------------------------- | -------------------------------------------------------------------------------------- |
| Error              | Problem, then recovery action                | Name the failing object or path when it helps. Add a valid next step when one exists.  |
| Warning            | Consequence, then required action            | State what may be lost or changed before a destructive action.                         |
| Advisory           | Observation, scope, then next check          | Do not present advice as an error or a required deck edit.                             |
| Finding            | Observation, scope, then limit               | Say what the measurement shows and what it does not show.                              |
| Success            | Completed action and useful count            | Report the result instead of a generic "Success!"                                      |
| Progress           | Task, object, and unit                       | Use consistent verbs for downloading, embedding, saving, and simulation.               |
| Unavailable value  | Reason and observation scope                 | Distinguish outside the horizon, not applicable, no observations, and incomplete data. |
| Destructive action | Deleted data, retained data, and backup step | State what is lost, what remains, and what to back up.                                 |

For example, forced setup must warn that it deletes collection records but
keeps deck files and primers. See the [reset warning](../README.md#install).

### Setup progress

- Keep the Cargo-style, right-aligned 12-column verb field. Start task names in
  the same column for active work and completed results.
- Name each download: Scryfall cards, Scryfall tags, or Commander Spellbook combos.
  Use that name in progress, cache reuse, completion, and failure messages.
- Put a compact bar below the task name. Use byte units for downloads and item
  counts for index work. A wide terminal must not produce a screen-wide bar.
- Use a spinner when the total is unknown. Do not show a fake count, percentage,
  or estimated finish time. Model loading uses this display for both missing-file
  downloads and local initialization; dependency-owned bars stay hidden.
- Replace transient activity with a named completion line after the step succeeds.
  Keep received byte counts and useful result counts in that line. Clear progress
  before reporting an error. Do not label a failed step as complete.
- Redirected output gets plain start, bounded checkpoint, and completion lines on
  stderr. JSON output stays free of status text.

## Help text

- Keep command options together. Put inherited flags under "Global options"
  and search filters under "Filters".
- Show common tasks as complete commands. Quote card names, card instructions,
  and shell comparisons. Use full card names in deck edits.
- Let the argument parser show fixed defaults once. Explain inferred defaults
  where users choose the value.
- State scope beside the flag. For example, `collection query --deck` adds deck
  copies to the binder pool; it does not search only that deck.
- Put preview and backup steps beside write commands. Explain what the command
  changes and what it leaves alone.
- Keep long role lists and model details out of short option descriptions.
  Use examples and a short explanation after the options.

## Output streams and color

- Put results on stdout and progress and diagnostics on stderr. Keep human
  status text out of JSON.
- Detect terminal capability separately for each stream. Redirected output,
  JSON, `NO_COLOR`, `--no-color`, and `TERM=dumb` must remain plain.
- Pair color with clear status words or symbols. Keep essential text readable
  in monochrome and do not make it too dim.
- Use the terminal's default body text and background. Preserve the meaning of
  Magic's white, blue, black, red, green, and colorless mana symbols.
- Use vertical layouts when tables do not fit. Do not truncate critical error
  details. Align by visible character width, not byte count or ANSI codes.
- Test long names and paths, Unicode names, and combining marks.
- Hide decorative website art from assistive technology. Label informative art
  and respect reduced-motion preferences. Animation must not block actions,
  corrupt text, or be the only explanation of a feature.

### Color and layout roles

| Role                        | Direction                                                  | Constraint                                                        |
| --------------------------- | ---------------------------------------------------------- | ----------------------------------------------------------------- |
| Brand accent                | Flamingo pink `#ff5c9a`, with terminal shades chosen below | Use for selected headings, links, and emphasis, not every number. |
| Website text and borders    | Dark ink, based on `#211820`                               | Check contrast against the actual background.                     |
| Terminal body text          | Terminal default foreground                                | Support light and dark backgrounds.                               |
| Success, warning, and error | Existing semantic color roles plus explicit words          | Pink must not blur the difference between warning and failure.    |
| Mana colors                 | Preserve WUBRG and colorless meaning                       | Do not recolor every mana symbol pink.                            |
| Layout                      | Monospace alignment, short headings, light rules           | Avoid a decorative box around every output block.                 |
| Art                         | Existing ASCII or block style at deliberate entry points   | Do not add a banner or animation to every command.                |

Use Dwarf Fortress as a reference for a clear character grid and useful density,
not for wording or command names.

### Terminal accent policy

Human output and Clap help share one accent policy. Use RGB only when
`COLORTERM` is `truecolor` or `24bit` and a valid, semicolon-separated
`COLORFGBG` ends with a known standard background:

- `0`: bright pink `#ff5c9a` for a standard dark background.
- `7` or `15`: dark pink `#97174d` for standard light backgrounds.

Otherwise, use ANSI magenta from the user's palette. Do not assume an unknown
background is dark. This policy has no public flags or theme configuration.

Dark pink has at least 4.5:1 contrast against standard white and silver. Bright
pink meets that ratio against black and `#211820`. These pairs do not guarantee
contrast for custom palettes or remapped colors. The user controls body text
and background colors.

## Reports and evidence

- Keep machine-readable names and units stable. Format human percentages with
  two decimal places on the report's 0 to 100 scale.
- Show human USD amounts with two decimal places and state the valuation basis.
  Market value is not guaranteed sale proceeds. Unknown prices are not `$0.00`.
- State the denominator for each reported rate. For incomplete games, show the
  known-outcome denominator, incomplete count, and risk of selection bias.
  Distinguish outside the horizon, not applicable, no observations, and
  incomplete data. An unavailable observation is not `0%`.
- Distinguish compiler evidence from verified runtime behavior and report
  evidence. A parsed or lowered ability does not prove that a full card works.
- Describe combat metrics as potential unblocked damage. Do not imply that this
  damage reduced opponent life.
- Track potential commander combat damage for each commander. Do not combine
  partners or include noncombat damage toward 21.
- Describe simulator output as consistency diagnostics, not a win-rate estimate
  or a full multiplayer rules engine.
- An unsupported ability must not appear to have a free cost or partial reward.

See [simulator metrics and limits](simulator.md) for the report contract.

## Command results and examples

Keep these exit meanings in scripts and help:

| Code | Meaning                                                                                            |
| ---- | -------------------------------------------------------------------------------------------------- |
| `0`  | Success.                                                                                           |
| `1`  | Runtime error, legality violations, simulation findings, or new findings in a baseline comparison. |
| `2`  | Invalid usage.                                                                                     |
| `3`  | No results or a missing card.                                                                      |

Interactive choices require a terminal. Do not promise a prompt in redirected
output.

Use the command's `--help` for its exact options. Prefer JSON for programs:

```sh
set -o pipefail
stm --offline card "Sol Ring" --json | jq -r '.name, .price'
stm --offline deck show Froggy --json | jq '{cards, missing_cost}'
stm --offline deck simulate Froggy --seed 42 --json > before.json
```

`jq` is optional. A pipeline normally returns the last command's status, so a
successful `jq` can hide an error or finding status from `stm`. Use
`set -o pipefail` in Bash or Zsh to expose upstream failures. A simulation can
write a valid report and exit `1` for findings. Keep stderr separate from saved
result data.

Use explicit units for durations and counts. Turn numbers start at one.
Preserve each JSON field's units. Quote paths with spaces in shell examples.
