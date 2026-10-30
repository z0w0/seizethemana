---
disclosure-default: ai-assisted
models-used:
  - zai-glm-5.3-flash
providers:
  - Ollama
scope: |
  All application code is AI-assisted or AI-generated and reviewed by a
  human before it lands. Architecture, data model, CLI design, and work
  order are human decisions. Documentation is AI-written and human-edited.
last-updated: 2026-09-14
---

# AI disclosure

This repository follows the
[ai-disclosure convention](https://github.com/ggfevans/ai-disclosure).
Per-file headers override the defaults below. No header means the repo
default applies.

## How this codebase is built

The maintainer's disclosure records this workflow:

- A human (Zack Corr) designs the architecture, data model, command
  surface, output style, and the order of the work.
- AI coding agents use models served by Ollama to write code. A human reviews
  it line by line before it lands and can reject or rework it.
- AI agents draft documentation. A human edits and approves it.
- Every change passes mechanical checks configured by the human:
  `cargo nextest run`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo fmt --check`. Release checks also cover commands against a live
  data store.

Files may have no header when human review has reshaped the generated code.
A missing header still uses the repository default under the convention above.
