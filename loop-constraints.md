# Loop Constraints — gtm.rs

> The `loop-triage` and `loop-verifier` skills read this file at the start of every run.
> Constraints here are **binding** — the agent MUST follow them.

## Push & Merge
- Never auto-merge to main without human approval
- Always create a draft PR first

## Paths
- Never edit `.git/`, `.gitignore`, `.cargo/`, or hidden config files
- Never auto-edit `target/`, `dist/`, or build artifacts
- Never edit `*.log` files

## Code
- Always run `cargo clippy` before proposing a change
- Always run `cargo test` before proposing a fix
- Never disable tests to make CI green
- Never refactor unrelated code — one fix per run
- Max 3 fix attempts per item; escalate after
- Follow Rust idioms and error handling patterns

## Communication
- Always tell the user what you're about to do before doing it
- Never close an issue or PR without approval

## Budget
- If token spend hits 80% of daily cap, switch to report-only
- If loop-pause-all is active, exit immediately
