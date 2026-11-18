# Repository Guidelines

Moiren is a Rust 2024 workspace for Windows real-time audio. This file holds only the summary and the rules that must never be violated. The full rules live in [docs/development/README.md](docs/development/README.md), one topic per document.

## Non-negotiable rules

1. **Real-time safety.** No heap allocation/deallocation, blocking, locking, or COM rebuild on the audio thread. Prepare graphs, buffers, and signal bindings on the control side; reclaim retired plans off RT.
2. **Three-layer separation.** Keep logical topology (`LogicalGraph`), prepared execution plans (`compiler`), and platform transports (`moiren-windows-audio`) separate. `NodeKind` stays a closed enum; follow the [node-kind checklist](docs/development/architecture.md) for every new variant.
3. **Truthful state.** The candidate graph is the only routing truth; runtime facts come from engine observations. Show requested → accepted → applied, use Unknown/`—` instead of fake data, and never label a candidate as active.
4. **One owner per rule.** UI/design rules live in [FRONTEND.md](FRONTEND.md); crate contracts in [docs/development/crates/](docs/development/crates/); workflow, verification, architecture and documentation rules in `docs/development/`. Designs and audits are records, not current verification.
5. **User-facing text** in `moiren-app` goes through `t!` with [locales/zh-CN.yml](crates/moiren-app/locales/zh-CN.yml); ledger and guard tests reject drift.
6. **Verify before claiming.** Run the gate for the touched surface ([verification.md](docs/development/verification.md)); `cargo test --workspace` alone is not the full gate. Hardware acceptance is user-run and stays separate from portable tests.
7. **Safe collaboration.** Re-check `git branch`/`git status` before staging; never touch `.git/` or another session's WIP; one task = one branch/worktree. Ask the user for unresolved decisions instead of guessing.
8. **Commit/PR discipline.** Conventional commits, atomic slices, squash-merged PRs; details and exceptions in [workflow.md](docs/development/workflow.md).

## Start here

- [docs/development/README.md](docs/development/README.md): index and precedence of all development rules.
- [README.md](README.md): project overview, current boundaries, docs index.
- [FRONTEND.md](FRONTEND.md): UI design and collaboration rules.
- [docs/TODO.md](docs/TODO.md): active design decisions and the work queue.
- `docs/designs/NN-*.md`, `docs/audit/YYYY-MM-DD/`, `docs/validation/`, `docs/experiments/`: records, not current truth.
- Each crate has a thin `AGENTS.md` that forwards to [docs/development/crates/](docs/development/crates/).

## Repository at a glance

`core` + `signal` + `plugins` are leaves → `engine` → `windows-audio` → `app` (gpui-kit UI). The C++ VAD driver lives in `driver/`, the rt-check harness and audio-hack-lab in `tools/`, support scripts in `scripts/`, shared logos in `assets/`. Detailed layout: [repository.md](docs/development/repository.md).

## Change flow

Plan → worktree/branch → implement → verify → update docs → commit/PR → merge → sync main. The details and exceptions live in [workflow.md](docs/development/workflow.md) and [verification.md](docs/development/verification.md).
