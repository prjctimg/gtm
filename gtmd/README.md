# gtmd

Background daemon for gtm. Owns all playback state, manages the music library via SQLite, and serves the `gtm` TUI/CLI over a Unix domain socket. Handles InnerTube-based YouTube streams, Deezer cover art, LRCLIB lyrics, crossfade/equalizer audio, and an MPRIS interface (on by default).

Install with `cargo install gtmd`.