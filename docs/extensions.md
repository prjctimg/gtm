# Extensions

The playback engine, DSP, library, IPC and the event bus live in `shared`
and are always on. A few **optional user-facing surfaces** of the TUI are
registered as *extensions* and can be switched off per session through the
`[extensions]` table in the TUI config (`prefs.toml`-style file written to
`~/.config/gtm/prefs.toml` on Linux):

```toml
[extensions]
visualizer = true
floating_notifications = true
notifications_overlay = true
```

Every extension defaults to **enabled**, so an absent table — or a config
file written before the registry existed — behaves exactly like the current
codebase.

## Registered extensions

| Key                     | Surface                                          | When disabled                                                          |
| ----------------------- | ------------------------------------------------ | ---------------------------------------------------------------------- |
| `visualizer`            | Spectrum analyzer + visualizer presets           | Spectrum is zeroed on the client (no spectrum applied to live state); `Ctrl+V` and the visualizer preset picker report the extension as disabled. |
| `floating_notifications`| Floating notification cards + notification history overlay (`Alt+N`) | Floating events are demoted to the footer instead of showing cards; the history overlay is not opened. |
| `notifications_overlay` | Notification-settings overlay (reached from the settings picker)    | The settings picker entry reports the overlay as disabled.              |

## Gating points

All of the pickers route through one gate (`overlay_extension` in
`gtm/src/app.rs`), which maps a `PickerId` to the `ExtensionId` that owns it.
Keybinding and palette paths that cross this gate print a short footer /
history notification instead of opening a disabled surface. The registry
itself is `ExtensionId` + `ExtensionsConfig` in `gtm/src/extensions.rs`.

## Core stays in core

The following are intentionally NOT extensions:

- playback transport + DSP (EQ / reverb / speed / crossfade / loudness /
  gapless)
- queue, library scan/sync/metadata/covers/lyrics, favourites
- volume / mute / repeat / shuffle / seek / mono / low-power / sleep timer
- audio device handling, MPRIS, daemon IPC + CLI
- the TUI event loop, keybindings, picker infrastructure and mouse handling

Extensions only *choose whether a surface renders*, reusing the existing
`DaemonEvent` / `IpcResult` seams and picker framework. Nothing is removed
from the wire; disabled extensions simply stop consuming it.