# Spec 08 — TOML Based Configuration

Status: **Planned** — switch from JSON to TOML, create specification, show limitations.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 8.1 — TOML specification

Create a TOML specification file at `/workspaces/gtm.rs/specs/08-toml-config.md` describing the configuration structure.

### 8.1.1 — Configuration file format
- Use TOML format for all configuration files
- Main config: `~/.config/gtm/config.toml`
- User themes: `~/.config/gtm/themes/*.toml`
- Footer presets: `~/.config/gtm/footer.toml`

### 8.1.2 — Configuration schema
```toml
[general]
theme = "Catppuccin Mocha"        # string, built-in theme name
volume = 80                         # u8 0-100
muted = false                       # bool
auto_shuffle = true                 # bool
crossfade = { enabled = true, duration_secs = 7 }  # struct
audio_backend = "rodio"             # string: "rodio" | "pulseaudio"

[library]
library_path = "~/.config/gtm/library.db"  # string

[daemon]
socket_path = "/tmp/gtm.sock"      # string
verbose = false                      # bool
test_mode = false                    # bool

[ui]
theme_dark_light = false             # bool
monochromatic = false                 # bool
footer_style = "default"             # string: "default" | "minimal" | "full"
notification_slide = "right"         # string: "left" | "right"

[footer]
left = ["Playback", "Queue", "Repeat", "Shuffle", "Volume", "EqPreset"]
middle = ["KeyAction", "SleepTimer"]
right = ["Device"]
```

### 8.1.3 — User theme TOML schema
```toml
name = "MyTheme"
light = false
bg = "#1e1e2e"
pane_bg = "#1e1e2e"
picker_bg = "#181825"
elevated_bg = "#141422"
muted_border = "#313244"
fg = "#cdd6f4"
fg_dim = "#6c7086"
fg_bright = "#f5f5ff"
accent = "#89b4fa"
secondary_accent = "#a6e3a1"
tertiary_accent = "#fab387"
error = "#f38ba8"
warning = "#fab387"
success = "#a6e3a1"
selection_fg = "#1e1e2e"
selection_bg = "#89b4fa"
border = "#313244"
border_active = "#89b4fa"
volume_low = "#a6e3a1"
volume_medium = "#fab387"
volume_high = "#f38ba8"
sidebar_active_border = "#89b4fa"
```

### 8.1.4 — TOML 1.0 limitations and technical restrictions

The following are **actual** TOML 1.0 limitations (not inaccurate claims):

1. **No inline comments in arrays** — TOML does NOT support inline comments within array items. Comments must be on their own line with `#` prefix. Example of INVALID TOML:
   ```toml
   items = ["foo", "bar", "baz"]  # this comment is INVALID
   ```
   Must be:
   ```toml
   items = ["foo", "bar", "baz"]
   # this comment is valid
   ```

2. **Type restrictions** — TOML has limited type support (string, integer, float, boolean, array, table). No complex nested structures beyond what JSON supports. No `null` type; use empty strings or omit keys.

3. **Key naming** — TOML keys are case-sensitive; use snake_case for readability.

4. **Whitespace handling** — TOML allows both spaces and tabs for indentation. However, use consistent spacing for readability.

5. **Inline tables are limited** — Inline tables (e.g., `{ key = "value" }`) cannot span multiple lines or contain comments. Use standard tables for complex structures.

6. **Date/time types** — TOML has native date/time types (`offset-datetime`, `local-datetime`, `local-date`, `local-time`) that may not map directly to Rust types without conversion.

7. **No trailing commas** — TOML does not allow trailing commas in arrays or tables.

8. **String escaping** — TOML supports escape sequences in basic strings (`\n`, `\t`, `\"`, `\\`, etc.). Use literal strings (`'...'`) for paths with backslashes.

9. **Version compatibility** — TOML 1.0 is the minimum spec; TOML 1.1 adds new features (inline tables, dotted keys, etc.) but ensure compatibility with TOML 1.0 parsers.

---

## 8.2 — Migration from JSON

### 8.2.1 — Auto-migration strategy
- On first run with a TOML config, auto-migrate from `prefs.json` to TOML
- On subsequent runs, use TOML
- If both files exist, prefer TOML (newest wins)
- Migration script: `cargo run --bin migrate-config` or similar

### 8.2.2 — JSON fallback
- If TOML file is missing or invalid, fall back to JSON
- Warning: "Config file moved from JSON to TOML format — please run `gtm config` to migrate"

### 8.2.3 — File migration path
- `~/.config/gtm/prefs.json` → `~/.config/gtm/config.toml`
- `~/.config/gtm/user_theme.json` → `~/.config/gtm/themes/*.toml`
- `~/.config/gtm/user_presets.json` → `~/.config/gtm/footer.toml`
- Migration is done automatically on first TOML file creation

---

## 8.3 — Existing config structure

### 8.3.1 — Daemon config (already TOML-based)
- `gtmd/src/config.rs` already uses TOML for daemon configuration
- `DaemonArgs` struct is already TOML-serializable

### 8.3.2 — User theme config (currently JSON, migrating to TOML)
- Currently in `gtm/src/theme.rs:496-561` as `UserThemeFile` (deserialized from JSON)
- **Migrating to TOML** (per decision): update `UserThemeFile` struct to use TOML-compatible types
- Add `#[serde(default)]` fallbacks for backward compatibility with existing user TOML themes
- Migration helper: add `UserThemeFile` deserialization from TOML (same approach as existing `toml::from_str`)

### 8.3.3 — Footer presets (JSON currently)
- `gtm/src/footer.rs:141-198` as `UserPresetsFile` (deserialized from JSON)
- Same TOML migration needed

---

## 8.4 — Changes needed

1. Replace `serde_json` dependency in `gtm/Cargo.toml` with `toml` (already present, but check)
2. Update `UserThemeFile` struct in `theme.rs` to use TOML types
3. Add `UserPresetsFile` deserialization from TOML
4. Update `config.rs` in `gtmd` to use TOML
5. Add migration logic for `prefs.json` → `config.toml`
6. Update `main.rs` and `cli.rs` to read TOML config
7. Update `theme.rs` test to use TOML parsing instead of JSON
