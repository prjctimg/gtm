# Spec 14 — Misc Changes

## Requirements

1. Instead of showing the raw hours ahead/behind, just show the current timezone string like CAT or EST etc.
2. Allow the user to hide the help row in the library view.
3. Don't make the settings menu/library view accessible by the old number keybindings. Remove all code related with that since we no longer need it.
4. On half terminal width, don't show both the left and middle pane but allow the user to toggle the views using Tab.

## Changes

### 14a. Timezone display (`gtm/src/ui.rs:2027-2030`)

Current format: `"%a,%d %B | %H:%M %Z"` — already shows timezone abbreviation. The requirement is to remove raw UTC offset hours and show only the timezone string.

- Simplify `local_time_str()` to: `format!(" {} | {} ", now.format("%H:%M"), now.format("%Z"))`
- This drops the weekday/month/day noise and shows just time + timezone (e.g. "14:30 | CAT")

### 14b. Help row toggle

- Add `hide_help_bar: bool` to `App` struct (`gtm/src/app.rs`)
- Default: `false` (help bar visible)
- Add keybinding `?` (with `KeyContext::Normal`, no overlay open) to toggle `hide_help_bar`
- Modify `render()` in `gtm/src/ui.rs:132`: `let show_help = app.current_tab == Tab::Library && !app.overlays.is_open() && !app.hide_help_bar;`
- Add the `?` key to `gtm/src/keymap.rs`
- Add `HideHelpBar` variant to `KeyboardAction` in `gtm-core/src/state.rs`
- Handle in `app.rs` dispatch

### 14c. Remove number keybindings

- Remove `1` and `2` key bindings from `gtm/src/keymap.rs:515-531`
- Remove `KeyboardAction::SwitchTab(Tab)` variant from `KeyboardAction` enum (`gtm-core/src/state.rs`)
- Remove the `SwitchTab` handler in `gtm/src/app.rs:1439-1444`
- Tab switching still available via Command Palette (`gtm/src/app.rs:2318-2326`)
- Update help overlay text (`gtm/src/ui.rs:1607`) to remove "1 / 2" line

### 14d. Half-width Tab toggle

- When `terminal_cols < 60` (narrow/half-width):
  - Show only the currently focused pane at full width
  - Tab/Shift-Tab toggles which pane is visible
- Modify `render_library()` in `gtm/src/ui.rs`:
  - If `is_narrow && library_pane_focus`: render library pane at full width of content area, hide content pane
  - If `is_narrow && !library_pane_focus`: render content pane at full width, hide library pane
- Remove the current narrow behavior where both panes are shown at 1/3 + 2/3 split

## Verification
- Timezone shows "14:30 | CAT" format, no weekday/month
- `?` key toggles help bar visibility on/off
- `1` and `2` keys no longer do anything
- Tab switching still works via Command Palette
- At terminal width < 60: only one pane visible, Tab toggles between library/content
