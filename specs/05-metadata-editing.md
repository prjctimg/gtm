## Spec: Metadata Editing Window Fixes

### Problem
The metadata editing floating window was missing help text that guided the
user on available actions (Tab, Enter, Ctrl+S, Esc).

### Changes

#### Help Text (`ui.rs`)
- Added two help lines at the bottom of the metadata editor:
  - `Tab: next field   Enter: save   Ctrl+S: sync cover`
  - `Esc: cancel       Type to edit the active field`

### Verification
- Open the metadata editor, verify help text is visible at the bottom
- All listed keybindings work correctly
