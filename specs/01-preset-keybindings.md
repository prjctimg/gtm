# Spec 01 — Preset Shuffle Keybindings

Status: **Planned** — add keybindings for all widgets with alternate styles.

Green gate: `cargo clippy --workspace --all-targets -- -D warnings`

---

## 1.1 — KeyboardAction enum expansion

**File**: `gtm/src/keymap.rs:44-112`

Add new `KeyboardAction` variants for theme cycling, crossfade type, and preset cycling:

```
// Existing (already present)
CycleFooterPreset
CycleProgressStyle
CycleDesign
CycleVisualizerPreset

// New variants — add these after the existing ones
ToggleTheme (for theme cycling: dark/light variants)
CycleCrossfadeType (for crossfade preset cycling: auto/on/off/adaptive)
CyclePresetType (for preset cycling: visualizer + progress + footer + design types)
```

Note: All new variants go in the `KeyboardAction` enum. Existing variants like `CycleFooterPreset`, `CycleDesign`, `CycleVisualizerPreset`, `CycleProgressStyle` already exist (keymap.rs:570-660) and are used for their respective cycling contexts.

---

## 1.2 — Default keybinding additions

**File**: `gtm/src/keymap.rs:149-669`

Add the following bindings in the `default_keybindings()` function (after the existing Cycling bindings, around line 660):

```
// Theme cycling — Alt+T
(
    KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT),
    BoundCommand {
        action: KeyboardAction::ToggleTheme,
        contexts: vec![KeyContext::Normal],
    },
),

// Crossfade type cycling — Alt+S
(
    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT),
    BoundCommand {
        action: KeyboardAction::CycleCrossfadeType,
        contexts: vec![KeyContext::Normal],
    },
),

// Preset type cycling — Alt+P
(
    KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
    BoundCommand {
        action: KeyboardAction::CyclePresetType,
        contexts: vec![KeyContext::Normal],
    },
),
```

---

## 1.3 — Cycling logic (no file edit needed, describe in spec)

Each cycling action (e.g., `ToggleTheme`, `CycleCrossfadeType`, `CyclePresetType`) must be implemented in the app's `App` struct:

- `ToggleTheme`: cycles through all 12 built-in themes (`theme.rs`)
- `CycleCrossfadeType`: cycles through crossfade preset types (auto/on/off/adaptive)
- `CyclePresetType`: cycles through all preset type pickers (visualizer, progress styles, footer styles, etc.)

For `CycleProgressStyle`, `CycleDesign`, `CycleVisualizerPreset`, `CycleFooterPreset` — these already exist in keymap.rs and need implementation logic in the main app loop.

---

## 1.4 — Verification

- `cargo clippy --workspace --all-targets -- -D warnings` — no new warnings
- `cargo test` — all existing tests pass
- Each binding dispatches correctly for the current `KeyContext`
