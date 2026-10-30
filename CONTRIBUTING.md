# Contributing to `stm`

You can contribute CLI fixes, card search, collection and deck tools, simulator
rules, documentation, the assistant skill, or the website. Most changes do not
need downloaded card data, an embedding model, or an AI assistant.

## Choose a first change

Start with one behavior and a test that shows the expected result. The
[module map](docs/architecture.md#module-map) covers the repository. These paths
are useful starting points:

| Change                                 | Start here                                                                | Test or reference                                                                         |
| -------------------------------------- | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| CLI help, errors, or output            | `src/cli.rs`, `src/main.rs`, `src/output.rs`                              | `src/tests/cli_tests.rs`, `src/tests/output_commands.rs`, [CLI contracts](docs/design.md) |
| Search filters or ranking              | `src/query.rs`, `src/query/`, `src/search.rs`                             | `src/tests/query_tests.rs`, `src/query/tests/`, [search evaluation](docs/querying.md)     |
| Collection imports or ownership counts | `src/collection.rs`, `src/deck/ownership.rs`                              | `src/tests/collection_tests.rs`, `src/tests/collection_location_tests.rs`                 |
| Deck file parsing or import formats    | `src/deck/grammar.rs`, `src/deck/io_external.rs`                          | `src/deck/tests/io_tests.rs`, `src/deck/tests/io_external_tests.rs`                       |
| Simulator rules                        | `src/deck/simulator/oracle_parser.rs`, `src/deck/simulator/oracle_lower/` | [Rule tests and fixture limits](docs/simulator.md#extend-and-test-rules)                  |
| Docs or assistant workflow             | `README.md`, `docs/`, `.agents/skills/seizethemana/SKILL.md`              | Check examples against `src/cli.rs` and the command handlers                              |
| Website copy or layout                 | `site/index.html`, `site/style.css`, `site/*.test.ts`                     | [Website setup and checks](site/README.md)                                                |

For a large change, open an issue first to agree on scope and user-visible
behavior.

## Build and test without card downloads

Install Git, current stable Rust with Cargo, Clippy, and rustfmt, and
[Nextest](https://nexte.st/docs/installation/from-source/). `Cargo.toml` declares
Rust 1.88 as the minimum; CI uses current stable Rust. You also need your
platform's C/C++ build tools and linker. SQLite is bundled, so the Rust build
does not need a separate SQLite library install.

```sh
rustup component add clippy rustfmt
cargo install --locked cargo-nextest
```

On macOS and Linux, capture process tests also need Bash, Git, `jq`, `shasum`,
and standard Unix tools. They replay local files and do not contact card services.

Clone the repository and build with the checked-in dependency versions:

```sh
git clone https://github.com/z0w0/seizethemana
cd seizethemana
cargo build --locked
env -u STM_WRITE_POPULAR_INVENTORY cargo nextest run --locked
./target/debug/stm --help
```

The first build needs network access for Cargo dependencies and native ONNX
Runtime libraries. `cargo fetch` alone does not prepare those native libraries.
Complete the build and test compilation while online before working offline.
Cargo's `--offline` option is separate from `stm --offline`; it does not make a
clean checkout ready to build without downloaded dependencies.

Ordinary tests use checked-in inputs and temporary stores. They do not call
live card services, download embedding models, or read your collection. Keep
`STM_WRITE_POPULAR_INVENTORY` unset so tests cannot rewrite compiler expectations.
CI unsets it and rejects fixture changes made by tests. Keep new tests offline.

You do not need `stm setup` to run these tests. For manual data checks, use
`--data-dir <DIR>` with a disposable store. Never seed or reset a real collection
as test data. Run commands that edit the same deck one at a time. Concurrent
edits can overwrite each other's changes.

If you need a full card index, build a release binary first:

```sh
cargo build --release
./target/release/stm --data-dir ./target/stm-dev-data setup
```

Setup and sync need a network connection. Read commands can skip automatic
refreshes with `--offline`. The flag does not block explicit setup or sync, or
downloads for an uncached search model. URL imports reject offline mode. See
[storage and refresh behavior](docs/architecture.md#setup-and-sync).

## Run the required checks

Install the [aislop CLI package](https://www.npmjs.com/package/aislop) with
Node.js and npm if it is missing:

```sh
npm install -g aislop
```

Run these checks before you submit a change:

```sh
cargo nextest run
cargo nextest run --examples
cargo clippy --all-targets -- -D warnings
cargo fmt --check
aislop scan --changes
aislop scan
```

Build the CLI, then run the shell checks from the repository root:

```sh
cargo build
for script in scripts/*.sh; do
  bash -n "$script"
done
bash scripts/check-release.sh
bash scripts/check-cli-terminal.sh
```

The release check needs Bash 3.2 or later, Cargo, `jq`, standard Unix tools,
and `tar` with xz support. It tests real Unix packaging with temporary files
and a local upload stub; it does not publish a release. Windows CI checks
the contents and binary hash of its produced ZIP archive.

The terminal check needs Bash 3.2 or later, native BSD/macOS or GNU/Linux
`script`, `sqlite3`, `jq`, `stty`, and standard Unix tools. It uses a
temporary store without a model or network. By default, it checks
`target/debug/stm`. It accepts an optional binary path:

```sh
bash scripts/check-cli-terminal.sh ./target/release/stm
```

Markdown checks need Node.js and npm; CI uses Node.js 24 and Prettier 3.6.2.
The commands below download Prettier if it is not cached. Format changed Markdown
files, then check those same files:

```sh
npx --yes prettier@3.6.2 --write path/to/changed.md
npx --yes prettier@3.6.2 --check path/to/changed.md
```

Fix every Clippy finding. Do not disable a lint unless you document a specific
reason beside the `#[allow]` attribute. Both `aislop scan --changes` and a full
`aislop scan` must report no errors or warnings.

CI runs the tests, evaluation-tool tests, Clippy, formatting check, and a release build on each pull
request. It also checks pinned source/card fixtures, rejects test-written
fixtures, checks Markdown, tests and builds the website, and checks release
archives and offline capture scripts.

## Follow project conventions

- Use Conventional Commits, such as `feat:`, `fix:`, `docs:`, `refactor:`,
  `test:`, and `chore:`. Branch names can use forms such as
  `fix/session-timeout`.
- Keep each pull request focused. Add tests for behavior changes.
- Write repository scripts in Bash or PowerShell.
- Write docs, comments, and user-facing text in short, direct sentences. See
  [AGENTS.md](AGENTS.md) for the writing rules.
- Put short usage examples in `README.md`, contribution tasks here, and detailed
  contracts in `docs/`. Link to a reference instead of copying its manual.
- Use sentence case for headings and descriptive link text. Tell the reader
  what to do; remove implementation plans and narration about past changes.
- Add doc comments to new top-level functions, types, traits, and file-level
  modules. Explain their purpose and any limits callers need to know.
- Keep command names, flags, JSON keys, and exit codes stable unless the change
  requires a contract update. Keep results on stdout and progress and diagnostics
  on stderr. Errors must explain the problem and the next action. See
  [CLI contracts](docs/design.md#command-results-and-examples).
- Preserve plain output, `NO_COLOR`, `--no-color`, redirection, and terminal
  contrast when changing styling. Website colors do not override terminal
  accessibility defaults.
- Test simulator report serialization and comparisons against report schema 3,
  model version 2, and policy version 2.
- Define SQLite schema v1 in `migrations/0001_initial_schema.sql`. Test fresh
  in-memory and file-backed databases, including constraints and indexes.

**Warning:** `stm setup --force` deletes `stm.db`, its SQLite sidecars,
`vectors.bin`, and `status.json`. This loses collection records, deck assignments,
and purchase prices. Stop other `stm` processes and back up the whole data
directory first. Saved decklists and primers in `decks/` remain. Do not use
this reset as test setup for a real collection.

The database, vector index, and published status must agree on the embedding
model, dimension, and document format. Test interrupted publication and retry
without losing collection records.

## Add simulator rules and fixtures

Start with the [compiler and runtime module map](docs/simulator.md#compiler-and-runtime-boundaries)
to find the parsing, lowering, action, and report code for a rule.

Keep Oracle parsing, typed syntax, lowering, runtime actions, and reporting
separate. Derive behavior from card data, not spell-name checks. A required
unsupported cost, target, condition, or linked effect must leave that ability
inert. Document the limit in the report assumptions.

Use pinned card data and named assertions. State whether a test checks parsing,
lowering, a runtime helper, a paid action, a whole turn, or a report. A source
snapshot or a passing deck run does not certify a whole card. Include matching
and nonmatching controls. See [fixture maintenance](docs/simulator.md#card-sources-and-fixture-maintenance)
for source revisions and explicit inventory updates.

Source downloads are a separate, opt-in maintenance task. Preserve dated inputs
and review Oracle-text changes before replacing expectations. Never refresh
source data or write expected snapshots during an ordinary test run.

Commander and Vehicle fixture metadata uses a separate pinned supplement.
Metadata captures are not whole-card support evidence. See
[fixture metadata](docs/simulator.md#fixture-metadata) for the
offline builder, opt-in capture command, exact prerequisites, and output limits.
Run the offline capture tests with
`cargo nextest run --test fixture_capture`. On Unix, these offline process
tests run the capture commands with small command shims. They need Bash, Git,
`jq`, `shasum`, and standard Unix tools, but never contact card services.
Supplement construction and source validation run in ordinary Rust tests.

Keep `scripts/` for these operational and platform-specific tasks:

| Command                         | Purpose                                                                  |
| ------------------------------- | ------------------------------------------------------------------------ |
| `capture-fixture-metadata.sh`   | Capture Scryfall metadata for an explicit fixture review.                |
| `capture-commander-fixtures.sh` | Capture ordered EDHREC sources, added lands, or Behold sources.          |
| `check-cli-terminal.sh`         | Check real terminal streams, colors, narrow output, and prompt behavior. |
| `check-release.sh`              | Exercise Unix packaging and upload arguments without publishing.         |

Put fixture builders and offline assertions in Rust tests. Ordinary tests must
not download sources or overwrite pinned inputs.

## Work on the website

The website needs Node.js 24 or later and npm. From `site/`, run `npm ci`,
`npm test`, and `npm run build`. `npm run check` runs the type check alone.
Use `npm start` for a local preview. See [site/README.md](site/README.md) for
preview limits and deployment.

## Run benchmarks

Benchmarks use Criterion and release builds. The simulator suite uses checked-in
fixtures without a live store, network calls, or model downloads. For a short
measurement, run:

```sh
STM_SIM_BENCH_FIXTURES=mono-red STM_SIM_BENCH_MODE=phases \
  cargo bench --bench simulator_bench -- 'phases/mono-red' --quick
```

Do not use that smoke check to claim a speedup. Before each before/after timing
run, use `STM_SIM_BENCH_MODE=validate` with that run's exact fixture, run, turn,
worker, and batch selections. It checks indexed logs and retained and streamed
reports. `all` runs the timed modes only; it does not run `validate`.

The core pipeline is not the full CLI path. It excludes database loading,
stored combo lookup, combined pair/combo analysis, hypergeometric analysis,
and terminal output. It retains one full-log batch, but scalar percentile
storage still grows with the run count. Check outcomes and phase observation
counts with timings so that early stops do not look like optimizations.
The benchmark prints those counts outside the timed loop. State whether a
profile includes that observation run.

[Simulator benchmarks](docs/simulator.md#execution-cost-and-benchmarks) lists
matrix controls, saved-baseline commands, longer Criterion settings, and a
focused profiling command. Record fixture hashes, settings, hardware, and timed
boundaries. Do not claim a speedup from a workload that skips required rules.

With `STM_DATA_DIR` unset, the query benchmark runs only the offline reciprocal
rank fusion (RRF) workload:

```sh
env -u STM_DATA_DIR cargo bench --bench query_bench
```

Store-based groups require an explicit `STM_DATA_DIR` pointing to a disposable,
prepared store. The production-query path can write recovery files, database
state, and model caches, and can download a missing model. Do not use your
personal store. See [query-path timing](docs/querying.md#query-path-timing) for
store preparation, group selection, and repeatable comparisons.

## Submit a change

1. Fork the repository and create a branch.
2. Make the change and add tests.
3. Run the required checks above.
4. Open a pull request against `main`.

Describe the user-visible result, why it is needed, and which checks passed.
State any checks you could not run. For fixture or performance changes, include
the input identities and the evidence required by the relevant reference doc.

## Prepare a release

Keep unreleased notes in [CHANGELOG.md](CHANGELOG.md). Do not claim a release
has shipped before its tag and assets exist. The build workflow packages the
supported targets and uploads release archives on version tags such as
`v0.1.0`. Check their names
against `Cargo.toml`'s `cargo-binstall` metadata before tagging. Release review
must include current report baselines, reset warnings, and the offline fixture
checks as well as the required checks above.

## Data sources

Card data, prices, and tags come from Scryfall's daily bulk exports. Do not add
per-card API requests. This project is unofficial Fan Content permitted under
the Wizards of the Coast Fan Content Policy. Scryfall, Commander Spellbook, and
Wizards of the Coast do not produce or endorse this project.
