# gtm.rs Iteration Specs

Iteration specs for the gtm.rs launch-prep + feature + TUI redesign work.
Each file is self-contained and lists concrete `file:line` targets so the work
can be picked up and completed in a later session.

| Spec | Status |
|---|---|
| [`00-repo-launch.md`](./00-repo-launch.md) — repository/launch readiness (audit fixes) | Done |
| [`01-feature-improvements.md`](./01-feature-improvements.md) — playback/library feature work | Done |
| [`01-preset-keybindings.md`](./01-preset-keybindings.md) — preset shuffle keybindings (dark/light cycling, Alt+key pattern) | Done |
| [`02-pickers.md`](./02-pickers.md) — better pickers and floating windows (cover art, swatches, fuzzy finder) | Planned |
| [`02-ui-redesign.md`](./02-ui-redesign.md) — borderless TUI structural redesign | Done |
| [`03-command-palette.md`](./03-command-palette.md) — command palette reliability and coverage | Planned |
| [`04-theming.md`](./04-theming.md) — secondary/tertiary accents, footer colors, monochromatic, gradient fill | Planned |
| [`05-notifications.md`](./05-notifications.md) — floating notifications and vertical volume window | Planned |
| [`06-improvements.md`](./06-improvements.md) — UI improvements, help text, CLI logging, READMEs | Planned |
| [`07-cli-logging.md`](./07-cli-logging.md) — verbose CLI output, --stream flag | Planned |
| [`08-toml-config.md`](./08-toml-config.md) — TOML configuration format migration | Planned |
| [`09-bugs.md`](./09-bugs.md) — crossfade, track title, master volume fixes | Planned |

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

## Decisions from spec verification (session 2026-08-12):

- **Theme cycling**: Every theme has both dark and light variants. Use existing NvChad palettes where available.
- **Keybinding convention**: Lowercase keys for pickers, uppercase keys (Alt+uppercase) for cycling presets.
- **TOML limitations**: Rewritten with accurate TOML 1.0 technical restrictions.
- **Help text**: Decision deferred to user input per prompt requirements.
- **User theme config**: TOML format (reconciled with spec 02 C0).
- **Volume notification**: Vertical floating window with gradient progress bar, configurable slide direction.
- **Equalizer descriptions**: Subjective only style.
- **Fuzzy finder**: Side-by-side layout (cover left, meta right).
- **Progress gradient**: Left to right (accent → secondary → tertiary).
- **CyclePresetType removed**: Alt+P replaces 'P' for CycleProgressStyle.

## Worktree policy

- Never commit `PROMPT.md` or the `Screenshot *.png` files (they are dev-local).
- Green gates after every sub-step:
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
