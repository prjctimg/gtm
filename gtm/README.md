# gtm

TUI + CLI frontend for the gtm music player. Renders the library, now-playing pane, settings, and pickers (ratatui), ships 12 themes plus TOML user themes, several progress and visualizer styles, and shell completions for bash, zsh, fish, elvish, and powershell.

Install with `cargo install gtm`, and run the daemon with `cargo install gtmd`.

On Linux, building needs a C compiler, `cmake`, `pkg-config`, and the ALSA development headers (`libasound2-dev` on Debian/Ubuntu).