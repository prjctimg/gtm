# gtm

TUI + CLI frontend for the gtm music player.

## Contents

- **`app.rs`** — Application state, event loop, IPC dispatch
- **`cli.rs`** — CLI subcommand parser and IPC client dispatch
- **`ui.rs`** — ratatui render layer (library, Now Playing, settings, pickers)
- **`theme.rs`** — 12 built-in NvChad-inspired themes + TOML user themes
- **`keymap.rs`** — keyboard bindings and `KeyboardAction` dispatch
- **`picker.rs`** — floating picker panel stack
- **`footer.rs`** — status bar modules and presets
- **`progress.rs`** — progress indicator styles (Braille, Gradient, etc.)
- **`visualizer.rs`** — audio visualizer presets (Braille, Blocks, Mirror, etc.)

## Building

```bash
cargo build --release -p gtm
```
