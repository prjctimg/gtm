# Spec 01 — Preset Shuffle Keybindings

Status: **Planned** — add keybindings for all widgets with alternate styles.

Green gate: `cargo clippy --workspace --all-targets -- -D warnings`

---

## 1.1 — KeyboardAction enum expansion

**File**: `gtm/src/keymap.rs:43-112`

Add new `KeyboardAction` variants for theme cycling and crossfade type cycling:

```
// Existing (already present)
CycleFooterPreset      // keymap.rs:106
CycleProgressStyle     // keymap.rs:107
CycleDesign            // keymap.rs:108
CycleVisualizerPreset  // keymap.rs:110

// New variants — add these after the existing ones
ToggleTheme            // cycles through dark/light variants of each theme
CycleCrossfadeType     // cycles through crossfade preset types: auto/on/off/adaptive
```

Note: All new variants go in the `KeyboardAction` enum at lines 43-112. The existing `Cycle*` variants are already implemented in `app.rs:2186-2215` and need no additional logic.

---

## 1.2 — Default keybinding additions

**File**: `gtm/src/keymap.rs:149-669`

### 1.2.1 — New bindings (add after existing cycling bindings, around line 660)

Keybinding convention:
- **Lowercase keys**: open pickers (picker triggers)
- **Uppercase keys (Alt+uppercase)**: cycle through presets directly

```
// Theme cycling — Alt+T (cycles through dark/light variants of each theme)
(
    KeyEvent::new(KeyCode::Char('T'), KeyModifiers::ALT),
    BoundCommand {
        action: KeyboardAction::ToggleTheme,
        contexts: vec![KeyContext::Normal],
    },
),

// Crossfade type cycling — Alt+X
(
    KeyEvent::new(KeyCode::Char('X'), KeyModifiers::ALT),
    BoundCommand {
        action: KeyboardAction::CycleCrossfadeType,
        contexts: vec![KeyContext::Normal],
    },
),
```

### 1.2.2 — Replace existing 'P' binding with Alt+P

**Current** at `keymap.rs:637-643`:
```rust
// P — cycle progress style
(KeyCode::Char('P').into(), BoundCommand { action: KeyboardAction::CycleProgressStyle, ... })
```

**Replace with** Alt+P to match the Alt+uppercase pattern (Alt+F, Alt+D, Alt+V, Alt+P):
```rust
// Alt+P — cycle progress style
(
    KeyEvent::new(KeyCode::Char('P'), KeyModifiers::ALT),
    BoundCommand {
        action: KeyboardAction::CycleProgressStyle,
        contexts: vec![KeyContext::Normal],
    },
),
```

### 1.2.3 — Summary of all cycling keybindings

| Binding | Action | Status |
|---|---|---|
| `Alt+F` | CycleFooterPreset | Existing (keymap.rs:570) |
| `Alt+D` | CycleDesign | Existing (keymap.rs:578) |
| `Alt+V` | CycleVisualizerPreset | Existing (keymap.rs:653) |
| `Alt+P` | CycleProgressStyle | **New** (replaces plain 'P') |
| `Alt+T` | ToggleTheme | **New** |
| `Alt+X` | CycleCrossfadeType | **New** |

---

## 1.3 — Theme cycling logic

**File**: `gtm/src/app.rs` (new method `toggle_theme`)

Each theme (Chadrula, One Dark, Tokyo Night, etc.) has both a dark and light variant. `ToggleTheme` cycles through dark/light variants of each theme:

- Each built-in theme entry should have `dark_theme: AppTheme` and `light_theme: AppTheme`
- Cycling goes: Chadrula Dark → Chadrula Light → One Dark → One Dark Light → ...
- Light variants are sourced from existing NvChad palettes where available
- Themes without known light variants auto-generate one by inverting the dark palette

Implementation in `App` struct:
```rust
pub fn toggle_theme(&mut self) {
    // Cycle to next dark/light variant pair
    // Update self.current_prefs().theme with new theme name
    // Apply the theme to self.theme
    // Notify user of theme change
}
```

---

## 1.4 — Crossfade type cycling logic

**File**: `gtm/src/app.rs` (new method `cycle_crossfade_type`)

Cycles through crossfade preset types: auto → on → off → adaptive → auto.

Implementation in `App` struct:
```rust
pub fn cycle_crossfade_type(&mut self) {
    // Cycle through crossfade types
    // Update self.current_prefs().crossfade.type
    // Notify user of change
}
```

---

## 1.5 — Existing implementation note

The existing `Cycle*` variants (`CycleFooterPreset`, `CycleDesign`, `CycleVisualizerPreset`, `CycleProgressStyle`) are already fully implemented in `app.rs:2186-2215`:

```rust
Some(KeyboardAction::CycleFooterPreset) => { self.cycle_footer_preset(); }
Some(KeyboardAction::CycleProgressStyle) => { self.progress_style = self.progress_style.next(); ... }
Some(KeyboardAction::CycleDesign) => { self.cycle_design(); }
Some(KeyboardAction::CycleVisualizerPreset) => { self.visualizer.cycle_preset(); ... }
```

No additional implementation logic is needed for these existing variants.

---

## 1.6 — Verification

- `cargo clippy --workspace --all-targets -- -D warnings` — no new warnings
- `cargo test` — all existing tests pass
- Each binding dispatches correctly for the current `KeyContext`
- Alt+T cycles through dark/light variants of each theme
- Alt+X cycles through crossfade types (auto/on/off/adaptive)
- Alt+P cycles through progress styles (replaces plain 'P')
