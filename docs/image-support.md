# Image Support

GTM renders cover art using terminal image protocols via
[ratatui-image](https://github.com/lhisch/ratatui-image). The library
auto-detects the best available protocol at startup and falls back to
Unicode half-block rendering (▀ with RGB fg/bg) when no image protocol
is available.

## Supported protocols

| Protocol | Terminals | Notes |
|----------|-----------|-------|
| **Kitty** | Kitty, WezTerm, Ghostty | Best quality. Uses the Kitty graphics protocol. |
| **Sixel** | foot, mlterm, mintty, xterm (with +sixel) | Good quality. Renders inline as sixel images. |
| **iTerm2** | iTerm2, WezTerm,mintty | Uses OSC 1337 inline images. |
| **Half-block fallback** | All terminals | Unicode `▀` characters with RGB colors. Lower resolution, may appear blocky. |

## Terminal compatibility

### Kitty, WezTerm, Ghostty

Image rendering works out of the box. No configuration needed.

### foot

Sixel support must be enabled in `foot.ini`:

```ini
[main]
sixel=yes
```

### Zellij

Zellij does **not** passthrough Kitty or Sixel image protocols by default.
Cover art will fall back to the half-block renderer, which may appear
low-resolution or fail entirely depending on the underlying terminal.

**Workaround**: Run GTM outside of Zellij, or use a terminal multiplexer
that supports image passthrough (e.g., tmux with `set -g allow-passthrough on`).

### Neovim terminal (`:terminal`)

Neovim's built-in terminal emulator does not fully passthrough Kitty/Sixel
protocols. Cover art may appear blocky because the half-block fallback is
used.

**Workaround**: Use GTM in a standalone terminal window or tab instead of
an embedded Neovim terminal. If you run GTM inside Neovim via a floating
window, consider using `vim.fn.termopen()` with a passthrough-capable
terminal or run GTM in a split terminal buffer.

### tmux

 tmux does not passthrough image protocols by default.

**Workaround**:

```tmux
set -g allow-passthrough on
```

### Alacritty, Rio

No image protocol support. Half-block fallback is used.

## Half-block fallback details

When no image protocol is available, GTM renders cover art using the
Unicode character `▀` (upper half block). Each terminal cell represents
two vertically stacked pixels: the foreground color sets the top pixel
and the background color sets the bottom pixel.

This gives an effective resolution of `terminal_width × (terminal_height × 2)`
pixels, which is sufficient for recognizable album art but lacks the
sharpness of protocol-based rendering.

### Improving half-block appearance

- Use a terminal with a small font size to increase effective resolution.
- Ensure your terminal supports 24-bit (truecolor) RGB output.
- The half-block renderer resizes images using Catmull-Rom resampling,
  which provides reasonable quality at small sizes.

## Configuration

GTM auto-detects the image protocol at startup by querying the terminal
via `ratatui-image`'s `Picker`. No manual configuration is required.

If you experience issues, you can force a specific protocol by setting
the `RATATUI_IMAGE_PROTOCOL` environment variable:

```bash
# Force sixel (useful if auto-detection fails)
RATATUI_IMAGE_PROTOCOL=sixel gtm
```

Valid values: `kitty`, `sixel`, `iterm2`, `halfblocks`.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Cover art looks blocky | Half-block fallback is active | Use a terminal with Kitty/Sixel support |
| Cover art doesn't render | Terminal blocks all image output | Check terminal config; try `RATATUI_IMAGE_PROTOCOL` |
| Cover art flashes/flickers | Protocol mismatch or terminal resize | Restart GTM after terminal resize |
| No cover art at all | Track has no embedded art | Use `gtm sync-covers` to fetch missing art |
