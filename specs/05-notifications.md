# Spec 05 — Better UI Notifications

Status: **Planned** — new floating notification window replacing legacy approach.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 5.1 — Design requirements

### 5.1.1 — Floating window properties
- Slide from right or left (configurable, default right)
- Use `tachyonfx` for easing animation (`evolve_into` with customizable timing)
- 3px left border that respects theme coloring for the fill
- Replaces legacy notification row approach (the simple highlighted row)
- Notification is a full floating widget, not a row in the TUI

### 5.1.2 — Animation details
- Use `tachyonfx` `evolve_into` with custom timing function
- Animation duration: 250ms (configurable)
- Easing: configurable (defaults to `evolve_into` with tachyonfx)
- The notification slides from the configured direction (left or right) into the screen

### 5.1.3 — Border design
- 3px thick left border that respects theme: `border_left` = current theme's accent color
- The border fills the notification's fill with theme-aware colors

### 5.1.4 — Notification content
- Title (short, bold)
- Message body (single line)
- Optional: icon or color indicator
- Auto-dismiss after 3 seconds
- Configurable slide direction (left or right)

---

## 5.2 — Implementation

**File**: `gtm/src/ui.rs`

### 5.2.1 — New `Notification` widget
```rust
pub struct Notification {
    pub title: String,
    pub message: String,
    pub direction: SlideDirection,  // Left or Right
    pub duration: Duration,
    pub accent_color: Color,
}
```

### 5.2.2 — New render function
- `render_notification(f: &mut Frame, area: Rect, notification: &Notification) -> Option<Rect>`
- Uses `tachyonfx::fx::evolve_into` with `TimingFunction::QuadInOut`
- Slides notification in from left or right (configurable via `direction` field)
- Applies 3px left border with theme's accent color

### 5.2.3 — Legacy notification removal
- Remove the old notification row rendering code in `ui.rs` (the simple highlighted row approach)
- All notification rendering is now done by the new floating widget

### 5.2.4 — Notification scheduling
- Notifications are pushed via `App` method: `push_notification(title, message, direction, duration)`
- Each notification uses `tachyonfx::evolve_into` for the animation
- Notification is rendered as a floating widget overlay

---

## 5.3 — Notification types to be used

- **Playback status**: "▶ Playing" / "⏸ Paused" / "⏹ Stopped"
- **Volume change**: Volume updated (e.g., "Volume: 75%")
- **Queue update**: Track added to queue
- **Track change**: New track playing
- **Error**: Error messages
- **Info**: General info (e.g., "Library updated")
- **Theme change**: Theme applied
- **Metadata sync**: Metadata synced

---

## 5.4 — Verification

- New notification widget renders correctly with `tachyonfx` animation
- Left border 3px thick and theme-aware
- Notification slides from configured direction
- Legacy notification row code is removed
- `cargo test` passes