# gtm-rs: Specification Index

Based on PROMPT.md feature requirements.

| # | File | Priority | Description |
|---|------|----------|-------------|
| 00 | [00-regressions.md](00-regressions.md) | 🔴 High | Fix 7 critical bugs: seeking, TUI crashes, track advancement, metadata, thumbnails, now playing sync, lyrics |
| 01 | [01-customizable-progress-indicators.md](01-customizable-progress-indicators.md) | 🔴 High | Move indicators to struct, add settings, create new styles (waveform, analog, dots, etc.) |
| 02 | [02-responsive-tui.md](02-responsive-tui.md) | 🔴 High | Single pane mode, tab navigation, adaptive footer for narrow terminals |
| 03 | [03-visualizer.md](03-visualizer.md) | 🔴 High | Live audio visualizer (1/4 width), toggle keybinding, lyrics pane extension |
| 04 | [04-daemon-stability.md](04-daemon-stability.md) | 🔴 High | Fix command handling, idle state, error notifications |
| 05 | [05-command-palette.md](05-command-palette.md) | 🔴 High | Fix command execution, reduce width, add icons |
| 06 | [06-audio-transitions.md](06-audio-transitions.md) | 🟡 Medium | Research and implement creative audio transition options |
| 07 | [07-sleep-timer.md](07-sleep-timer.md) | 🟡 Medium | Slider overlay, h/l keys, manual input, max 3h, persistence |
| 08 | [08-misc.md](08-misc.md) | 🟡 Medium | Settings keybinding, hide help row, remove version, fix About overlay, adjust layout |

## Legend

- 🔴 **High** — Critical for functionality or stability
- 🟡 **Medium** — Important for user experience

## Implementation Order

1. **00-regressions.md** — Fix critical bugs first
2. **04-daemon-stability.md** — Ensure daemon is stable
3. **02-responsive-tui.md** — Fix TUI crashes
4. **05-command-palette.md** — Fix command execution
5. **08-misc.md** — Quick UI fixes
6. **01-customizable-progress-indicators.md** — Add new features
7. **03-visualizer.md** — Add visualizer
8. **06-audio-transitions.md** — Add transition options
9. **07-sleep-timer.md** — Improve sleep timer

## Quick Reference

```
gtm-rs/
├── Cargo.toml                   # workspace root
├── gtm-core/                    # shared types, IPC, wire protocol
├── gtm-audio/                   # audio decode + output abstraction
├── gtmd/                        # gtmd binary + daemon-lib
├── gtm/                         # gtm binary (Ratatui TUI + CLI)
├── gtm-mpris/                   # MPRIS D-Bus server (optional)
├── specs/                       # ← you are here
└── PROMPT.md                    # feature requirements (drives specs)
```

## Spec Format

Each spec file follows this structure:

1. **Goal** — Clear objective
2. **Current State** — What exists now
3. **Required Changes** — Detailed implementation steps
4. **Files to Modify** — Specific file paths
5. **Implementation Details** — Code examples and patterns
6. **Checklist** — Verification items
7. **Visual Design** — ASCII mockups of UI changes