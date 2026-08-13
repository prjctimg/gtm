# gtm.rs Iteration Specs

Iteration specs for the gtm.rs launch-prep + feature + TUI redesign work.
Each file is self-contained and lists concrete `file:line` targets so the work
can be picked up and completed in a later session.

| Spec | Status |
|---|---|
| [`00-repo-launch.md`](./00-repo-launch.md) — repository/launch readiness (audit fixes) | Planned |
| [`01-feature-improvements.md`](./01-feature-improvements.md) — playback/library feature work | Planned |
| [`02-ui-redesign.md`](./02-ui-redesign.md) — borderless TUI structural redesign | Planned |

## Committed decisions

Decisions locked in during planning (session 2026-08-08):

- **tachyonfx** `0.22` (ratatui-0.30 compatible) for the `evolve_into` animation.
- **Titles-only library rows everywhere**, including drill-down detail views
  (`ui.rs:838-842`, `ui.rs:1019-1031`, Spotify rows `ui.rs:753-806`). The Now
  Playing pane keeps its separate `Artist:` line; the footer status bar keeps
  its `artist – title` display.
- **New theme fields** `elevated_bg` + `muted_border` added to `AppTheme`
  (`gtm/src/theme.rs`), all 12 built-in themes, and the TOML `UserThemeFile`
  parse. `picker_bg` stays the picker scrim; `elevated_bg` is the opaque popup
  fill; `muted_border` is the subtle pane separator color.
- **Crossfade**: lower the trigger margin at `gtmd/src/daemon.rs:1367` from
  `+ 0.5` to `+ 0.15`, AND make the mixer's `step_crossfade` swap EOF-aware
  (let the outgoing track play to its real end instead of hard-stopping at
  `progress >= 1.0`).

## Worktree policy

- Never commit `PROMPT.md` or the `Screenshot *.png` files (they are dev-local).
- Green gates after every sub-step:
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
