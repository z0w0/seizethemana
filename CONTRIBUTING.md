# Contributing to `stm`

Thanks for helping out. This page tells you how to get set up and what a
change needs before it can land.

## Getting set up

You need a stable Rust toolchain (1.88 or newer) and nothing else:

```sh
git clone https://github.com/z0w0/seizethemana
cd seizethemana
cargo build
cargo nextest run
```

The test suite is self-contained: no network, no downloads, nothing to
configure. If you want to run real workloads (`stm setup`, `stm sync`)
while developing, build with `--release` first; embedding is very slow in
debug builds.

## Every change must pass these

```sh
cargo nextest run
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Run `aislop scan --changes` too; it catches common code-quality issues
that tests miss. If clippy flags something, fix the code rather than
turning the lint off; an exception is fine with a reason, written next to
the `#[allow]` attribute.

CI runs the same checks plus a release build on every pull request.

## Conventions

- **Commits** use Conventional Commits (`feat:`, `fix:`, `docs:`,
  `refactor:`, `test:`, `chore:`). Branch names can follow the same style,
  like `fix/session-timeout`.
- **Keep changes small.** One feature or one fix per pull request, with a
  test for any behavior change.
- **Write plainly.** Docs, comments, and user-facing messages use short
  sentences and common words. See the writing rules in
  [AGENTS.md](AGENTS.md).
- **Document the "why".** New public functions, types, and modules get doc
  comments explaining what they're for, not restating what the code does.
- **Don't break the machine interface.** `--json` output may gain fields
  but never lose or rename them, and exit codes (0 ok, 1 error, 2 bad
  usage, 3 nothing found) are fixed. Scripts and agents depend on both.
- **Database schema changes** currently go straight into the v1 migration
  in `src/db.rs`; users rebuild with `stm setup --force`. Once the first
  version ships, breaking schema changes must append new migrations
  instead so existing databases keep working.

## Submitting changes

1. Fork the repo and create a branch.
2. Make your change with tests.
3. Run the checks above.
4. Open a pull request against `main`.

If you're planning something large, opening an issue to talk it through
first saves time all around.

## Data source

Card data, prices, and tags all come from Scryfall's free daily bulk
exports. This project deliberately makes no per-card API requests; please
keep it that way. The project is unofficial Fan Content under Wizards of
the Coast's Fan Content Policy and isn't produced or endorsed by either.