# Project Instructions

## Validation commands

Run all of these after any change; all must pass before you stop.

```sh
cargo nextest run      # tests (use nextest, not `cargo test`)
cargo clippy --all-targets -- -D warnings   # fix ALL clippy findings, zero warnings
cargo fmt --check      # formatting
```

Rules:

- `cargo clippy` findings are not optional. Fix the code, don't silence the lint
  unless there is a concrete reason (then document it in a comment next to the
  attribute).
- `aislop scan --changes` must end at zero fixable findings. Fix every
  finding — never assume a warning is pre-existing or acceptable. Split
  oversized functions/files, remove decorative comments, and replace
  `.unwrap()`s in production code with `?` or `.expect("reason")` before
  stopping. A 100/100 score is the goal for every change.
- New top-level functions, types, traits, and file-level modules get doc
  comments (`///`). Say what they are for, not what the code mechanically does.
- Keep it simple, stupid. Smallest correct change. No speculative abstractions,
  no fallbacks for impossible states, no cleverness.
- Large test modules live in their own file, next to the code they test.
  When a module file nears the 1000-line limit, move its `mod tests` into a
  co-located `tests/` **folder** (not a flat sibling file): module
  `src/deck/suggest.rs` → `src/deck/tests/suggest_tests.rs`; module
  `src/deck/simulator/game.rs` → `src/deck/simulator/tests/game_tests.rs`.
  Include it with `#[path]`:
  ```rust
  #[cfg(test)]
  #[path = "tests/suggest_tests.rs"]
  mod suggest_tests;
  ```
  Rules:
  - The file must live under a `tests/` subfolder next to the module's
    parent, named `<module>_tests.rs`. Never a bare `<module>_tests.rs`
    file floating beside the implementation.
  - The folder is shared by sibling modules in the same directory
    (`src/deck/tests/` holds `suggest_tests.rs`, `legal_tests.rs`, …).
  - The test file starts at module level: `use super::*;` at the top, no
    leftover `mod tests {` wrapper, no stray indentation from the old
    inline module.
  - Split very large test modules thematically if the file itself would
    exceed the 1000-line limit (`suggest_tests.rs`, `suggest_combo_tests.rs`).

## Running the binary

Embedding is CPU-heavy and unusably slow in a debug build. When you need to
run real workloads (`stm setup`, `stm sync`, re-embedding after a doc-layout
change), build with `--release` first:

```sh
cargo build --release && ./target/release/stm setup
```

Debug builds are fine for output smoke tests that do not embed (`stm card`,
`stm collection`, `stm deck show` on a seeded store).

## Writing

Write all docs, comments, and user-facing text in Simplified Technical
English at a 10th grade reading level:

- Short, direct sentences. One idea per sentence where you can.
- Common words. No jargon you don't need; expand the acronym on first use.
- Active voice. Say who does what.
- No filler. Cut every sentence that does not change what the reader does.
- Keep the same rules for README, docs/, SKILL.md, and error messages.
- Format every changed `.md` file with Prettier. Run `prettier --write` on
  changed Markdown files, then run `prettier --check` on those same files.

## Reference documents

| Document               | What it covers                                                                                                                                      |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/architecture.md` | Data schema, vector index + query path, sync pipeline, deck module, migration policy                                                                |
| `docs/querying.md`     | Production search, embedding documents, baseline, model bake-off, and ranking sweep                                                                 |
| `docs/design.md`       | CLI design: command surface, output style, human vs agent-friendly modes                                                                            |
| `docs/simulator.md`    | How the goldfish simulation works: intent, card model, turn pipeline, metrics, assumptions, limits — the living reference for `src/deck/simulator/` |
| `README.md`            | Install, usage examples, ManaBox guide                                                                                                              |
