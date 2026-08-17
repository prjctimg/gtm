## Spec: Visualizer & Progress Bar Cycling

### Problem
There was no quick way to cycle through visualizer presets and progress bar
styles. Scrolling through the picker list didn't preview changes live.

### Changes

#### Keybindings (`keymap.rs`, `app.rs`)
- `Alt+V` cycles visualizer presets via `KeyboardAction::CycleVisualizerPreset`
- `Alt+P` cycles progress bar styles via `KeyboardAction::CycleProgressStyle`
- Both apply immediately and persist via `save_prefs`
- `Alt+P` removed from CommandPalette (`:` key remains)

#### Live Preview on Scroll (`app.rs`)
- Added `apply_preset_preview()` called after Up/Down navigation
- Sets `self.visualizer.preset` or `self.progress_style` to the highlighted item
- Picker UI shows `(current)` marker on the active preset

### Verification
- `Alt+V` cycles through visualizer presets (Notifier, Spectrum, Bars, etc.)
- `Alt+P` cycles through progress styles (Block, Bar, Line, Dots, etc.)
- Scrolling in the picker shows live preview in the bottom pane
- Changes persist across restarts via prefs
