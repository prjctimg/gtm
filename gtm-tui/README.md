# gtm-tui

Terminal UI for gtm, built with Ratatui and Crossterm.

## Architecture

```
loop {
    tokio::select! {
        events = client.poll_events() => process_event(ev),
        key = crossterm::event::poll(10ms) => handle_key(key),
        _ = render_throttle.tick() => render(),
    }
}
```

## Layout

```
┌──────────────────────────────────────────────────┐
│  ▶ Now Playing │ Library │ Queue │ YouTube │ ... │  ← TabBar
├──────────────────────────────────────────────────┤
│                                                    │
│              Active Tab Content                     │
│                                                    │
├──────────────────────────────────────────────────┤
│  ⏸ 2:34 / 4:20  ████████░░  Vol: 75%  🔀  🔁 All │  ← Footer
└──────────────────────────────────────────────────┘
```

## Tabs

| Tab | Description |
|-----|-------------|
| NowPlaying | Album art, progress, controls, synced lyrics |
| Library | Track list, playlists, favourites, search |
| Queue | Now playing + up next, move mode |
| YouTube | yt-dlp search, results, stream play |
| Settings | All config options, theme picker |
| Help | Keybinding reference |

## Overlays

Command palette (`:`), fuzzy finder, queue picker, theme picker, confirm dialog, track detail.

## Dependencies

`gtm-core`, `gtm-audio`, `ratatui`, `crossterm`, `tokio`, `clap`, `color-eyre`, `image`, `base64`
