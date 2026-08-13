# Spec 04 — Better Theming

Status: **Planned** — add secondary/tertiary accent colors, footer section colors, monochromatic theme, gradient progress fill.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 4.1 — Secondary and tertiary accent colors

**File**: `gtm/src/theme.rs:13-38`

### 4.1.1 — Add `secondary_accent` and `tertiary_accent` fields

Add to `AppTheme` struct:

```rust
#[derive(Clone, Copy)]
pub struct AppTheme {
    pub bg: Color,
    pub pane_bg: Color,
    pub picker_bg: Color,
    pub elevated_bg: Color,
    pub muted_border: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_bright: Color,
    pub accent: Color,
    pub secondary_accent: Color,  // new
    pub tertiary_accent: Color,   // new
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub border: Color,
    pub border_active: Color,
    pub volume_low: Color,
    pub volume_medium: Color,
    pub volume_high: Color,
    pub sidebar_active_border: Color,
}
```

### 4.1.2 — Update all 12 built-in themes

Each theme needs secondary and tertiary accent colors. For themes that already have an accent, the secondary is a darker/complementary variant and tertiary is an even lighter/sharper variant.

**Chadrula** (current: accent=0x7aa2f7): secondary=0x5c6b9a, tertiary=0x9ece6a
**One Dark** (current: accent=0x61afef): secondary=0x4a7ab8, tertiary=0x98c379
**Tokyo Night** (current: accent=0x7aa2f7): secondary=0x5c6b9a, tertiary=0x9ece6a
**Tokyo Night Storm** (current: accent=0xff9e64): secondary=0xd08770, tertiary=0xfab387
**Catppuccin Mocha** (current: accent=0x89b4fa): secondary=0x89b4fa, tertiary=0xa6e3a1
**Catppuccin Latte** (current: accent=0x1e66f5): secondary=0x1e66f5, tertiary=0x40a02b
**Gruvbox Dark** (current: accent=0xd3869b): secondary=0xfb4934, tertiary=0x9ccfd8
**Nord** (current: accent=0x88c0d0): secondary=0x88c0d0, tertiary=0xa3be8c
**Rose Pine** (current: accent=0xc4a7e7): secondary=0xc4a7e7, tertiary=0x9ccfd8
**Everforest** (current: accent=0xa7c080): secondary=0xa7c080, tertiary=0xe69875
**Kanagawa** (current: accent=0x7e9cd8): secondary=0x7e9cd8, tertiary=0x98bb6c
**Kanagawa Lotus** (current: accent=0x2d6a9f): secondary=0x2d6a9f, tertiary=0x6a9589

For each, the secondary is a shade darker than the accent (dark theme variant), and tertiary is a lighter shade.

---

## 4.2 — Footer accent color per section

**File**: `gtm/src/footer.rs:325-344`

Apply accent colors per footer section:
- **Left**: `accent` (primary accent)
- **Middle**: `secondary_accent` (secondary accent)  
- **Right**: `tertiary_accent` (tertiary accent)

This ensures the footer feels colorful when possible, with each section's background showing its assigned color.

---

## 4.3 — Monochromatic theme support

**File**: `gtm/src/theme.rs`, new module `monochrome.rs`

Add a monochromatic theme variant that:
- Uses only `accent` color for all elements
- Background and foreground are derived from the accent color (grayscale variant)
- All colors are derived from a single base accent color

The monochromatic theme should be a separate theme entry that inherits from the base theme but applies:
- `bg` = `accent` (or a dark variant)
- `pane_bg` = darker version of `accent`
- `picker_bg` = even darker
- `elevated_bg` = darkest
- `fg` = `accent` (bright)
- `fg_dim` = dim version of `accent`
- `fg_bright` = bright version of `accent`
- `accent` = original
- `secondary_accent` = `accent` (same as primary)
- `tertiary_accent` = `accent` (same as primary)

All other colors derived from the single `accent` color.

---

## 4.4 — Progress indicator gradient fill

**File**: `gtm/src/progress.rs`

The `ProgressStyle::Gradient` already exists and renders █/▓/▒ characters. For theme-aware gradient fill:

Instead of hardcoded █/▓/▒ colors, the gradient should use theme colors:
- The gradient should be computed per-theme using `theme.accent` as the starting color
- The gradient should use `theme.secondary_accent` as the middle
- The gradient should use `theme.tertiary_accent` as the end

Update the `render_progress` function to accept a `Color` parameter for each gradient step:
```rust
pub fn render_progress(ratio: f64, width: usize, style: ProgressStyle, accent: Color, secondary: Color, tertiary: Color) -> String {
```

The gradient colors should be computed using the theme's color values, not hardcoded.

---

## 4.5 — Gradient implementation detail

**File**: `gtm/src/theme.rs`

Add helper function to generate gradient colors from theme:
```rust
fn gradient_colors(theme: &AppTheme) -> (Color, Color, Color) {
    let accent = theme.accent;
    let secondary = theme.secondary_accent;
    let tertiary = theme.tertiary_accent;
    // Use these 3 colors for the 3-step gradient
}
```

The gradient has 3 steps:
- Step 0 (left): `accent` (bright)
- Step 1 (middle): `secondary_accent` (medium)
- Step 2 (right): `tertiary_accent` (dim)

---

## 4.6 — Verification

- All 12 built-in themes render with new accent colors
- Footer sections display with correct colors
- Monochromatic theme can be selected and renders correctly
- Progress gradient uses theme colors, not hardcoded
- `cargo clippy` and `cargo test` pass