# LOOP.md — gtm.rs

Rust implementation of GTM — modular crate suite (gtm-core, gtm-audio, gtmd, gtm, gtm-mpris).

## Active Loops

### Daily Triage (L1 — report only)
- Cadence: 1d weekdays
- Skill: `loop-triage`
- State: STATE.md
- Phase: Report-only initially. L2 after trust established.
- Handoff: Design decisions, architectural changes, crate restructuring.

### PR Review (L2 — assisted)
- Cadence: on PR creation
- Skill: `loop-triage` + `loop-verifier`
- State: STATE.md
- Phase: Assisted — verifier runs `cargo test` + `cargo clippy` in worktree.
- Handoff: Anything touching gtm-core/, gtm-audio/, or gtmd/.

## Worktrees

- Use isolated git worktrees for any L2 code changes.
- One worktree per fix attempt; discard after verifier REJECT or escalation.

## Budget & Observability

- Token caps: `loop-budget.md`
- Run history: `loop-run-log.md`
- Kill switch: `loop-pause-all` label in STATE.md

## Safety

- Never auto-merge changes to `gtm-core/` or `gtm-audio/`.
- All Rust changes must pass `cargo clippy` + `cargo test` before merge.
- MPRIS/D-Bus changes require manual review.
