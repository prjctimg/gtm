# gtm-cli

Headless CLI controller for the gtm daemon. Connects to the Unix socket, sends JSON
request lines, and prints JSON responses. No TUI, no Ratatui.

## Usage

```
gtm play ~/Music/song.flac    # Play a file
gtm pause                     # Toggle play/pause
gtm next                      # Next track
gtm volume 80                 # Set volume
gtm queue list                # List queue
gtm library scan ~/Music      # Scan library
gtm status                    # Show daemon status (JSON)
```

## Subcommands

`play`, `pause`, `stop`, `next`, `prev`, `seek`, `volume`, `shuffle`, `repeat`,
`mute`, `status`, `now`, `queue` (list/clear/add/remove/move), `library` (scan/list/search/recent),
`favourite` (list/add/remove), `playlist` (list/create/delete/add), `crossfade`, `sleep`, `kill`, `lyrics`.

## Shell Completions

```
gtm completions bash
gtm completions zsh
gtm completions fish
```

## Dependencies

`gtm-core`, `clap`, `tokio`, `serde_json`
