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

# AI Disclosure

This repo follows the
[ai-disclosure convention](https://github.com/ggfevans/ai-disclosure).
Per-file headers override the defaults below. No header means the repo
default applies.

## How this codebase is built

`stm` is built in a human/agent loop:

- A human (Zack Corr) designs the architecture, data model, command
  surface, output style, and the order of the work.
- Code is written with AI coding agents (models served by Ollama), then
  **reviewed line-by-line by a human before it lands**. The agents draft.
  The human decides, rejects, and reworks.
- Documentation is AI-written and human-edited. The agent drafts; the human
  corrects, cuts, and approves.
- Every change passes mechanical checks configured by the human:
  `cargo nextest run`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo fmt --check`. Milestones also pass a manual command matrix against
  a live data store.

Disclosure reflects the current state of the code. Where review reshaped
generated code into something the maintainer fully owns, files may carry no
header at all.