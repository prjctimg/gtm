# Spec 13 — Footer Layout Reorder

## Requirement

> The footer module is laid out to show 7 elements:
> 1. a,b,c - Playback status (a), repeat/shuffle/eq preset (b) and tracklist count (c)
> 2. j,k - Next track,last pressed keybinding command (k)
> 3. x,y - Sleep timer/volume level (x), date/time (y)
>
> The 'y' module must be at the very start from the right of the footer, it must, the 'j' and 'k' modules in the middle and the remaining ones at the extreme left.

## Current State

The footer (`gtm/src/footer.rs`) renders 5 background-colored groups left-to-right:
- Group 1: Playback + Volume + EqPreset (status)
- Group 2: Repeat + Shuffle (mode)
- Group 3: Queue
- Group 4: KeyAction + SleepTimer (misc)
- Group 5: Clock (far right)

## Target State

Three groups, left-to-right:
1. **Left**: Playback + Volume + EqPreset + Queue + Repeat + Shuffle + Progress (all status/info modules)
2. **Middle**: KeyAction + SleepTimer (j,k modules)
3. **Right**: Clock (y module — date/time)

## Changes

### `gtm/src/footer.rs`
- Modify `build_groups()` to produce exactly 3 groups:
  - Group 1 (left, status_bg): All status modules — Playback, Volume, EqPreset, Queue, Repeat, Shuffle, Progress
  - Group 2 (middle, fg_dim_bg): KeyAction, SleepTimer
  - Group 3 (right, fg_dim_bg): Clock
- Remove the old 5-group categorization logic
- The `render_preset()` function needs no structural changes — it already renders groups left-to-right with a fill spacer

## Verification
- Visual inspection: footer shows [status modules] ... [keyaction + sleeptimer] ... [clock]
- Clock is at the far right edge
- Middle group sits between status and clock
- No content clipping or overlap
