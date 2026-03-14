---
title: "ODB Bar Range Selection — Technical Reference"
description: >
  Complete technical reference for the bar range selection tool on ODB charts.
  Covers the Shift+Click UX, brim drag mechanics, canvas coordinate system hit
  detection, render layer architecture, and stats overlay with intensity analytics.
feature: BarRangeSelection
source_files:
  - src/chart/kline.rs
  - src/chart.rs
key_types:
  - BarSelectionState
  - BrimSide
key_functions:
  - "snap_x_to_index(x: f32, bounds_size: Size, region: Rectangle) -> (u64, f32)"
  - "draw_selection_highlight(frame, chart, bounds_size, lo, hi)"
  - "draw_bar_selection_stats(frame, palette, tick_aggr, anchor, end)"
  - "brim_screen_xs(chart, bounds_size, lo, hi) -> (f32, f32)"
key_constants:
  - "HIT_ZONE: abs_diff <= 1 bar (snap_x_to_index ±1)"
  - "FILL_ALPHA: 0.02"
  - "HANDLE_ALPHA: 0.22"
  - "U64_MAX_SENTINEL: forming-bar zone marker"
nuances:
  - "visual_idx=0 is the NEWEST bar (rightmost); anchor/end are stored as visual indices"
  - "snap_x_to_index is the canonical hit test — never compute screen_x manually for hit detection"
  - "brim handles are exactly one cell_width*scaling px wide to match the ±1 bar hit zone"
  - "RefCell needed because canvas::Program::update() takes &self"
  - "BrimSide::Lo is the RIGHT (newer) brim despite the name suggesting 'low'"
  - "Ties in brim proximity always resolve to Lo (right/newer brim)"
---

# ODB Bar Range Selection — Technical Reference

## Table of Contents

1. [Feature Overview](#feature-overview)
2. [Data Structures](#data-structures)
   - [BrimSide Enum](#brimside-enum)
   - [BarSelectionState Struct](#barselectionstate-struct)
   - [Visual Index Orientation](#visual-index-orientation)
   - [Interior Mutability via RefCell](#interior-mutability-via-refcell)
3. [Hit Detection: Anti-Pattern and Correct Pattern](#hit-detection-anti-pattern-and-correct-pattern)
   - [The Broken Screen-Space Approach](#the-broken-screen-space-approach)
   - [The Correct snap_x_to_index Approach](#the-correct-snap_x_to_index-approach)
   - [The u64::MAX Forming-Bar Sentinel](#the-u64max-forming-bar-sentinel)
4. [Canvas Rendering Layers](#canvas-rendering-layers)
5. [Interaction State Machine](#interaction-state-machine)
   - [Shift+Left Click Flow](#shiftleft-click-flow)
   - [Brim Drag Flow](#brim-drag-flow)
   - [Mouse Cursor Feedback](#mouse-cursor-feedback)
6. [Visual Highlight — draw_selection_highlight](#visual-highlight--draw_selection_highlight)
   - [brim_screen_xs — Coordinate Conversion](#brim_screen_xs--coordinate-conversion)
   - [Fill Rectangle](#fill-rectangle)
   - [Brim Handle Strips](#brim-handle-strips)
   - [Why Handle Width Matches the Hit Zone](#why-handle-width-matches-the-hit-zone)
7. [Stats Overlay — draw_bar_selection_stats](#stats-overlay--draw_bar_selection_stats)
   - [Basic Counting Metrics](#basic-counting-metrics)
   - [Intensity Metrics](#intensity-metrics)
   - [Regime Classification](#regime-classification)
   - [Box Layout and Color Scheme](#box-layout-and-color-scheme)
8. [Key Constants and Thresholds Summary](#key-constants-and-thresholds-summary)
9. [Coordination with Other Systems](#coordination-with-other-systems)

---

## Feature Overview

**In plain terms**: The bar range selection lets you draw a yellow band across any span of ODB bars. Once you mark a range with two Shift+Clicks, a stats panel appears at the top center of the chart telling you how many bars are in the range, how many closed up versus down, and a set of microstructure intensity metrics characterizing the trading behavior inside that window. You can drag either edge (brim) of the selection to adjust boundaries without redrawing from scratch.

**Technically**: The feature is an interactive overlay exclusive to `Basis::Odb(_)` charts. State is stored in a `RefCell<BarSelectionState>` inside `KlineChart`. Hit detection is performed entirely through `snap_x_to_index()` — the same ratio-based mapping used for the crosshair and Shift+Click bar lookup — which makes it immune to the coordinate-offset discrepancies that plague manual screen-space formula approaches. Rendering is split across two lightweight canvas cache layers (`crosshair` and `legend`) so that dragging a brim never invalidates the expensive kline (candle) geometry.

---

## Data Structures

### BrimSide Enum

The selection has two boundaries, called **brims** by analogy with a container's rim. A brim is the outermost bar on one side of the highlighted region.

```rust
// kline.rs:78-85
/// Which brim of the selection range is being dragged.
#[derive(Clone, Copy)]
enum BrimSide {
    /// Right (newer) brim — the lower visual_idx boundary.
    Lo,
    /// Left (older) brim — the higher visual_idx boundary.
    Hi,
}
```

**Naming convention note**: `Lo` refers to the _lower_ visual index (newer bars have smaller indices — index 0 is always the rightmost/newest bar), which places it on the _right_ side of the screen. Likewise, `Hi` is the higher visual index, meaning the _older_ brim on the _left_ side. This is counterintuitive at first glance, so treat the names as index-space labels, not screen-position labels.

| Variant | visual_idx       | Screen position | Temporal position |
| ------- | ---------------- | --------------- | ----------------- |
| `Lo`    | smaller (e.g. 2) | right           | newer             |
| `Hi`    | larger (e.g. 17) | left            | older             |

### BarSelectionState Struct

```rust
// kline.rs:87-100
/// State for interactive bar-range selection on ODB charts.
/// Shift+Left Click: 1st = set anchor, 2nd = set end, 3rd = reset anchor.
/// Left-drag near a brim: relocates that boundary in real-time.
#[derive(Default)]
struct BarSelectionState {
    /// Visual index of the anchor bar (0 = newest/rightmost).
    anchor: Option<usize>,
    /// Visual index of the end bar (set on second Shift+Click).
    end: Option<usize>,
    /// Whether the Shift key is currently held (tracked via ModifiersChanged).
    shift_held: bool,
    /// Which brim is currently being dragged (None when idle).
    dragging_brim: Option<BrimSide>,
}
```

Both `anchor` and `end` are `Option<usize>` so the struct can represent every phase of the selection lifecycle:

| `anchor`  | `end`     | Meaning                            |
| --------- | --------- | ---------------------------------- |
| `None`    | `None`    | No selection active                |
| `Some(i)` | `None`    | Anchor set, awaiting second click  |
| `Some(i)` | `Some(j)` | Full selection, both brims defined |

The ordering of `anchor` and `end` is arbitrary — the user may click left-to-right or right-to-left. All rendering and statistics code normalizes via `anchor.min(end)` (`lo`) and `anchor.max(end)` (`hi`) before use.

### Visual Index Orientation

The ODB chart stores bars in `TickAggr.datapoints` oldest-first (ascending storage index). On screen, bars are laid out newest-right. The visual index (`visual_idx`) inverts this:

```
visual_idx  =  0      1      2      3      4  …  N-1
screen pos  = [right (newest) ────────────── left (oldest)]
storage_idx = [N-1   N-2    N-3    N-4    N-5 …  0       ]
```

Converting between the two:

```rust
// Used inside draw_bar_selection_stats (kline.rs:4318-4319)
let si = len - 1 - vi;   // visual_idx vi → storage index si
let dp = &tick_aggr.datapoints[si];
```

### Interior Mutability via RefCell

`canvas::Program` in iced requires `update()` to take `&self` (shared reference), even though updating selection state requires mutation. This is resolved with `RefCell<BarSelectionState>`:

```rust
// kline.rs:364-367
/// Bar range selection state (ODB charts only).
/// RefCell: `canvas::Program::update()` takes `&self`, interior mutability needed.
bar_selection: RefCell<BarSelectionState>,
```

The canonical borrow discipline used throughout:

1. Borrow immutably to extract values into local variables.
2. Drop the immutable borrow (let the binding go out of scope or use a block).
3. Borrow mutably to update.

Never hold an immutable borrow (`sel.borrow()`) across a `borrow_mut()` call — iced's borrow checker will panic at runtime.

---

## Hit Detection: Anti-Pattern and Correct Pattern

This is the most critical correctness property of the entire feature. Getting hit detection wrong produces a brim that is visually centered on one bar but activates when clicking a bar one or two positions away, making the UI feel broken.

### The Broken Screen-Space Approach

The natural impulse is to convert each brim's bar position to a screen x-coordinate using the canvas coordinate formula, then check if the cursor is close enough:

```rust
// ANTI-PATTERN — do NOT use this approach
let chart_x = -(lo as f32) * cell_width;
let screen_x = (chart_x + translation.x) * scaling + bounds.width / 2.0;
if (cursor.x - screen_x).abs() < HIT_PIXELS {
    // start dragging
}
```

**Why this fails**: `cursor.position_in(bounds)` returns cursor coordinates in a frame of reference that may differ subtly from the manually computed `screen_x`. The iced canvas coordinate system applies transforms in a specific order, and reproducing that transform outside the canvas can accumulate floating-point discrepancies — particularly near edges — that shift the apparent hit position by 1-3 pixels. At high zoom (large `cell_width * scaling`), this is invisible, but at low zoom it causes complete misses where the clickable region does not visually overlap the handle strip.

### The Correct snap_x_to_index Approach

Instead of computing a screen position and checking pixel distance, map the cursor x-coordinate into a bar index using the same function the crosshair uses, then compare indices:

```rust
// kline.rs:3076-3092 (brim drag start)
let region = self.chart.visible_region(bounds_size);
let (visual_idx, _) =
    self.chart.snap_x_to_index(cursor_pos.x, bounds_size, region);
if visual_idx != u64::MAX {
    let snapped = visual_idx as usize;
    let lo_dist = snapped.abs_diff(lo);
    let hi_dist = snapped.abs_diff(hi);
    // Within ±1 bar of a brim → drag it. Ties go to Lo (right/newer brim).
    let side = if lo_dist <= 1 && lo_dist <= hi_dist {
        Some(BrimSide::Lo)
    } else if hi_dist <= 1 {
        Some(BrimSide::Hi)
    } else {
        None
    };
    ...
}
```

**Why this works**: `snap_x_to_index` for `Basis::Odb` uses ratio-based mapping that is purely relative to `bounds.width` and the visible chart region — there are no absolute coordinate offsets involved:

```rust
// src/chart.rs:1270-1289
Basis::Odb(_) => {
    let (chart_x_min, chart_x_max) = (region.x, region.x + region.width);
    let chart_x = chart_x_min + x_ratio * (chart_x_max - chart_x_min);
    //            ^^^^^^^^^^^^ x_ratio = cursor.x / bounds.width

    let cell_index = (chart_x / self.cell_width).round();
    ...
    let rounded_index = if cell_index > 0.0 {
        u64::MAX // forming-bar sentinel
    } else {
        (-cell_index) as u64
    };
    (rounded_index, snap_ratio)
}
```

The function maps `cursor.x → x_ratio → chart_x → cell_index → visual_idx`. Every step is a proportional computation with no dependency on the absolute translation of the canvas frame. The same computation is used for:

- Drawing the crosshair vertical line
- Shift+Click bar selection
- Brim hit detection (drag start)
- Brim drag position update
- Mouse cursor feedback (`mouse_interaction`)

Using a single canonical function guarantees that "the bar the crosshair snaps to" equals "the bar the brim will snap to" equals "the bar that counts as a hit zone boundary" — all by construction.

**Comparison table**:

|                             | Anti-pattern (screen formula)        | Correct (snap_x_to_index)        |
| --------------------------- | ------------------------------------ | -------------------------------- |
| Hit criterion               | `\|cursor.x − screen_x\| < N pixels` | `visual_idx.abs_diff(brim) <= 1` |
| Coordinate frame            | Absolute screen pixels               | Ratio-relative bar indices       |
| Immune to translation drift | No                                   | Yes                              |
| Consistent with crosshair   | No                                   | Yes                              |
| Consistent with Shift+Click | No                                   | Yes                              |
| Zoom-invariant              | No (needs pixel threshold tuning)    | Yes                              |

### The u64::MAX Forming-Bar Sentinel

`snap_x_to_index` returns `u64::MAX` when the cursor is to the right of bar 0 — in the zone that corresponds to the forming (in-progress) bar. ODB charts always show the forming bar at position 0, but it should not be selectable as a range boundary because its data is incomplete and changes every second.

```rust
// src/chart.rs:1283-1287
let rounded_index = if cell_index > 0.0 {
    u64::MAX // sentinel: cursor is in forming bar territory
} else {
    (-cell_index) as u64
};
```

Every call site guards against this sentinel:

```rust
// kline.rs:3038, 3079, 3106
if visual_idx != u64::MAX {
    // safe to cast and use as a bar index
}
```

This means all three interaction paths — brim drag update, brim drag start, and Shift+Click — silently do nothing when the cursor is in the forming-bar zone.

---

## Canvas Rendering Layers

The iced canvas in `KlineChart::draw()` produces four `Geometry` objects stacked in draw order:

```rust
// kline.rs:3459
vec![klines, watermark, legend, crosshair]
```

| Layer       | Cache field             | Cleared when        | Contents                        |
| ----------- | ----------------------- | ------------------- | ------------------------------- |
| `klines`    | `chart.cache.main`      | Pan, zoom, new bars | Candlestick/ODB bar geometry    |
| `watermark` | `chart.cache.watermark` | Rarely              | Exchange/symbol label           |
| `legend`    | `chart.cache.legend`    | Every cursor move   | Stats overlay, intensity legend |
| `crosshair` | `chart.cache.crosshair` | Every cursor move   | Crosshair, selection highlight  |

**Selection highlight** is drawn in the `crosshair` layer (kline.rs:3417-3427):

```rust
let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
    if chart.basis.is_odb() {
        let sel = self.bar_selection.borrow();
        if let Some(anchor) = sel.anchor {
            let end = sel.end.unwrap_or(anchor);
            let (lo, hi) = (anchor.min(end), anchor.max(end));
            draw_selection_highlight(frame, chart, bounds_size, lo, hi);
        }
    }
    // ... crosshair tooltip follows
});
```

**Stats overlay** is drawn in the `legend` layer (kline.rs:3406-3414):

```rust
let legend = chart.cache.legend.draw(renderer, bounds_size, |frame| {
    // ... intensity heatmap spectrum legend first ...
    if chart.basis.is_odb() {
        let sel = self.bar_selection.borrow();
        if let (Some(anchor), Some(end)) = (sel.anchor, sel.end)
            && let PlotData::TickBased(tick_aggr) = &self.data_source
        {
            draw_bar_selection_stats(frame, palette, tick_aggr, anchor, end);
        }
    }
});
```

**Why this split matters for performance**: During brim drag, the update handler clears only the two lightweight caches:

```rust
// kline.rs:3058-3061 (inside brim drag CursorMoved handler)
self.chart.cache.clear_crosshair();
self.chart.cache.legend.clear();
return Some(canvas::Action::request_redraw().and_capture());
```

The `klines` cache is never invalidated during drag. This means every drag event only redraws:

1. The yellow highlight rectangle and brim handles (crosshair layer)
2. The stats text box (legend layer)

The O(N) candlestick geometry (potentially thousands of bars) stays baked in GPU memory and is composited without re-generation.

---

## Interaction State Machine

### Shift+Left Click Flow

```
┌────────────────────────────────────────────────────────────────┐
│                    BarSelectionState                           │
│                                                                │
│  anchor=None, end=None                                         │
│         │                                                      │
│         │  Shift+Click on completed bar (visual_idx != MAX)    │
│         ▼                                                      │
│  anchor=Some(i), end=None                                      │
│    [yellow highlight: single bar width, lo=hi=i]              │
│         │                                                      │
│         │  Shift+Click on any completed bar                    │
│         ▼                                                      │
│  anchor=Some(i), end=Some(j)                                   │
│    [yellow band spans lo..=hi, stats overlay appears]         │
│         │                                                      │
│         │  Shift+Click (THIRD click) — restart                 │
│         ▼                                                      │
│  anchor=Some(new_k), end=None                                  │
│    [previous selection cleared, new anchor set]               │
└────────────────────────────────────────────────────────────────┘
```

Source (kline.rs:3098-3121):

```rust
// ── Shift+Left Click: set anchor / end / restart ───────────────
if shift_held
    && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
{
    if let Some(cursor_pos) = cursor.position_in(bounds) {
        let region = self.chart.visible_region(bounds_size);
        let (visual_idx, _) =
            self.chart.snap_x_to_index(cursor_pos.x, bounds_size, region);
        if visual_idx != u64::MAX {
            let mut sel = self.bar_selection.borrow_mut();
            match (sel.anchor, sel.end) {
                (None, _) => sel.anchor = Some(visual_idx as usize),
                (Some(_), None) => sel.end = Some(visual_idx as usize),
                // Third Shift+Click: restart from new anchor.
                (Some(_), Some(_)) => {
                    sel.anchor = Some(visual_idx as usize);
                    sel.end = None;
                }
            }
            self.chart.cache.clear_all();
        }
    }
    return Some(canvas::Action::request_redraw().and_capture());
}
```

Note that the third click calls `clear_all()`, which invalidates all four caches including `klines`. This is correct because resetting the anchor is an infrequent operation and the full redraw budget is acceptable.

### Brim Drag Flow

Brim dragging is a three-phase gesture: detect → update → release.

```
┌─────────────────────────────────────────────────────────────────┐
│ Phase 1: DETECT (ButtonPressed, no Shift)                       │
│                                                                 │
│  Preconditions:                                                 │
│  - shift_held == false                                          │
│  - anchor.is_some() && end.is_some()                           │
│  - anchor != end  (selection has width)                        │
│                                                                 │
│  snap_x_to_index(cursor.x) → snapped                          │
│  lo_dist = snapped.abs_diff(lo)                                │
│  hi_dist = snapped.abs_diff(hi)                                │
│                                                                 │
│  if lo_dist <= 1 && lo_dist <= hi_dist → dragging_brim = Lo   │
│  elif hi_dist <= 1                     → dragging_brim = Hi   │
│  else                                  → no drag starts        │
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│ Phase 2: UPDATE (CursorMoved while dragging_brim.is_some())     │
│                                                                 │
│  snap_x_to_index(cursor.x) → new_idx                          │
│  if new_idx == u64::MAX → ignore (forming-bar zone)           │
│                                                                 │
│  BrimSide::Lo: update whichever of anchor/end is smaller       │
│    (Some(a), Some(e)) if a <= e → anchor = new_idx            │
│    (Some(_), Some(_))           → end    = new_idx            │
│                                                                 │
│  BrimSide::Hi: update whichever of anchor/end is larger        │
│    (Some(a), Some(e)) if a >= e → anchor = new_idx            │
│    (Some(_), Some(_))           → end    = new_idx            │
│                                                                 │
│  clear_crosshair() + legend.clear() → redraw lightweight caches│
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│ Phase 3: RELEASE (ButtonReleased)                               │
│                                                                 │
│  dragging_brim = None                                           │
│  request_redraw()                                               │
└─────────────────────────────────────────────────────────────────┘
```

**Brim identity tracking**: The drag handler tracks which field (`anchor` or `end`) to update by examining which one holds `lo` or `hi` at the time of the drag update. This means the user can drag the Lo brim past the Hi brim — at which point the roles invert because `anchor.min(end)` recalculates dynamically. The selection cannot be "inverted" from the user's perspective; the highlight always covers the full span between the two stored points.

### Mouse Cursor Feedback

The `mouse_interaction` method (kline.rs:3462-3490) returns `ResizingHorizontally` in two cases:

1. **While dragging**: `sel.dragging_brim.is_some()` — unconditionally show the resize cursor to indicate an active drag operation.
2. **While hovering near a brim**: The same `snap_x_to_index` → `abs_diff <= 1` check used for drag initiation. This provides visual affordance before the user clicks.

```rust
// kline.rs:3468-3488
if self.chart.basis.is_odb() {
    let sel = self.bar_selection.borrow();
    if sel.dragging_brim.is_some() {
        return mouse::Interaction::ResizingHorizontally;
    }
    if let (Some(anchor), Some(end)) = (sel.anchor, sel.end)
        && anchor != end
        && let Some(cursor_pos) = cursor.position_in(bounds)
    {
        let lo = anchor.min(end);
        let hi = anchor.max(end);
        let region = self.chart.visible_region(bounds.size());
        let (visual_idx, _) =
            self.chart.snap_x_to_index(cursor_pos.x, bounds.size(), region);
        if visual_idx != u64::MAX {
            let snapped = visual_idx as usize;
            if snapped.abs_diff(lo) <= 1 || snapped.abs_diff(hi) <= 1 {
                return mouse::Interaction::ResizingHorizontally;
            }
        }
    }
}
```

---

## Visual Highlight — draw_selection_highlight

The yellow overlay is drawn in screen space, meaning all coordinates are in pixels relative to the canvas frame. The function takes bar indices (`lo`, `hi`) in visual-index space and converts them to pixel x-coordinates before drawing.

### brim_screen_xs — Coordinate Conversion

```rust
// kline.rs:4227-4238
/// Converts left and right brim bar positions to screen-space x coordinates.
///
/// `lo` = right brim (newer, lower visual_idx), `hi` = left brim (older).
/// Screen formula: `screen_x = (chart_x + translation.x) * scaling + bounds_width / 2`
fn brim_screen_xs(chart: &ViewState, bounds_size: Size, lo: usize, hi: usize) -> (f32, f32) {
    let to_screen = |chart_x: f32| {
        (chart_x + chart.translation.x) * chart.scaling + bounds_size.width / 2.0
    };
    // ODB: interval_to_x(idx) = -(idx as f32) * cell_width
    let right_chart_x = -(lo as f32) * chart.cell_width + chart.cell_width / 2.0;
    let left_chart_x  = -(hi as f32) * chart.cell_width - chart.cell_width / 2.0;
    (to_screen(left_chart_x), to_screen(right_chart_x))
}
```

The ODB chart-space convention places bar 0 (newest) near x=0, with older bars at negative x:

- `chart_x(visual_idx) = -(visual_idx as f32) * cell_width`

The brim positions add/subtract `cell_width / 2.0` to encompass the full bar body:

- Right edge of `lo` bar: `-(lo) * cell_width + cell_width/2` (the right half of bar lo)
- Left edge of `hi` bar: `-(hi) * cell_width - cell_width/2` (the left half of bar hi)

**Important**: `brim_screen_xs` is used exclusively for _drawing_ the highlight, not for hit detection. Hit detection always goes through `snap_x_to_index`. This separation is intentional — the visual helper and the interaction helper solve different problems and the formula used for drawing does not need to be the formula used for clicking.

### Fill Rectangle

```rust
// kline.rs:4255-4259
frame.fill_rectangle(
    Point::new(left_sx, 0.0),
    Size::new(w, bounds_size.height),
    iced::Color { r: 1.0, g: 1.0, b: 0.3, a: 0.02 },
);
```

The fill spans the full chart height (`bounds_size.height`) and the full horizontal extent from the left edge of the `hi` bar to the right edge of the `lo` bar.

Alpha `0.02` (2%) is deliberately very low. At this opacity, the yellow tint is barely perceptible on a dark background — the candle bodies, wicks, and heatmap colors beneath remain fully readable. The overlay is recognizable as a selection not through its fill but through its brim handles.

### Brim Handle Strips

```rust
// kline.rs:4261-4273
let handle_w = (chart.cell_width * chart.scaling).clamp(3.0, 60.0);
let handle_color = iced::Color { r: 1.0, g: 1.0, b: 0.3, a: 0.22 };
frame.fill_rectangle(
    Point::new(left_sx, 0.0),
    Size::new(handle_w, bounds_size.height),
    handle_color,
);
frame.fill_rectangle(
    Point::new(right_sx - handle_w, 0.0),
    Size::new(handle_w, bounds_size.height),
    handle_color,
);
```

Two full-height rectangles are drawn at each edge of the selection, with alpha `0.22` (22%) — about 11x more opaque than the fill. This makes them visible as distinct handles while remaining translucent enough to show what is behind them.

The left handle starts at `left_sx` (leftmost pixel of the hi bar) and extends rightward `handle_w` pixels. The right handle ends at `right_sx` (rightmost pixel of the lo bar) and extends leftward `handle_w` pixels. Neither handle overlaps the interior fill area as long as the selection spans more than one bar.

### Why Handle Width Matches the Hit Zone

```rust
let handle_w = (chart.cell_width * chart.scaling).clamp(3.0, 60.0);
```

`cell_width * scaling` is the width of exactly one bar in screen pixels at the current zoom level. The `±1 bar` hit zone used in `snap_x_to_index` corresponds visually to one bar-width on each side of the brim bar center. This means:

- The drawn handle strip covers exactly the region where a click will register as "on the brim"
- At any zoom level, the visual affordance matches the interactive behavior
- The `clamp(3.0, 60.0)` prevents the handle from disappearing at extreme zoom-out (minimum 3px visible) or becoming too dominant at extreme zoom-in (maximum 60px per side)

---

## Stats Overlay — draw_bar_selection_stats

The stats overlay appears in the top-center of the chart when both `anchor` and `end` are set. It provides a microstructure analysis of the selected bar range, drawing from the `trade_intensity` field in `OdbMicrostructure` attached to each bar.

### Basic Counting Metrics

```rust
// kline.rs:4303-4332
let len = tick_aggr.datapoints.len();
let (lo, hi) = (anchor.min(end), anchor.max(end));
let hi = hi.min(len - 1);
let lo = lo.min(len - 1);
let distance = hi - lo;   // 0 for same bar, 1 for adjacent, N for N+1 bars span

let bars: Vec<BarSample> = (lo..=hi)
    .map(|vi| {
        let si = len - 1 - vi;   // visual → storage index
        let dp = &tick_aggr.datapoints[si];
        BarSample {
            raw: dp.microstructure.map_or(0.0, |m| m.trade_intensity),
            is_up: dp.kline.close >= dp.kline.open,
        }
    })
    .collect();

let n = bars.len();
let n_up = bars.iter().filter(|b| b.is_up).count();
let n_dn = n - n_up;
let up_pct = n_up as f32 / n as f32 * 100.0;
let dn_pct = n_dn as f32 / n as f32 * 100.0;
```

`distance = hi - lo`:

- 0 when anchor and end are the same bar (a degenerate single-bar selection)
- 1 when they are adjacent bars
- The displayed line reads e.g. "15 bars" for a selection covering visual indices `lo=3` to `hi=17` (`distance = 14`, `examined = 15`)

A bar is "up" when `close >= open` — this matches the candle coloring convention.

### Intensity Metrics

The overlay computes seven microstructure statistics from within-selection `trade_intensity` values (trades per second, as computed by the heatmap indicator).

#### Within-Selection Rank Normalization

```rust
// kline.rs:4334-4353
let mut order: Vec<usize> = (0..n).collect();
order.sort_unstable_by(|&a, &b| {
    bars[a].raw.partial_cmp(&bars[b].raw).unwrap_or(std::cmp::Ordering::Equal)
});
let mut rank_norm = vec![0.5_f32; n];
if n > 1 {
    // Fractional rank (0=coldest, 1=hottest in this window), ties averaged
    ...
}
```

Rank normalization maps each bar's raw intensity to a [0, 1] value within the selected window. A bar with `rank_norm = 0.0` had the lowest intensity of all bars in the selection; `1.0` had the highest. Ties receive the average of the tied ranks.

This normalization is used for `↑t` / `↓t` (mean rank of up/down bars), `P(↑>↓)` (AUC), conviction, and absorption. It cancels out session-level intensity baseline, making metrics comparable across different market regimes.

#### IWDS (Intensity-Weighted Direction Score)

```rust
// kline.rs:4366-4372
let iwds = if total_raw > 0.0 {
    bars.iter().map(|b| b.raw * if b.is_up { 1.0 } else { -1.0 }).sum::<f32>() / total_raw
} else { 0.0 };
```

`IWDS = Σ(intensity × sign) / Σ(intensity)` where sign = +1 for up bars, -1 for down bars.

Range: [-1, +1]. +1 means all trading urgency occurred on up bars; -1 means all urgency was on down bars; 0 means urgency was evenly split or balanced by direction. This is displayed as an ASCII bar chart `[████████░░]` followed by `flow: +0.42`.

#### Mann-Whitney AUC

```rust
// kline.rs:4374-4384
let auc: f32 = if n_up > 0 && n_dn > 0 {
    let r_up: f32 = order.iter().enumerate()
        .filter(|(_, orig)| bars[**orig].is_up)
        .map(|(rank_0, _)| rank_0 as f32 + 1.0)
        .sum();
    let u_up = r_up - n_up as f32 * (n_up as f32 + 1.0) / 2.0;
    u_up / (n_up as f32 * n_dn as f32)
} else { f32::NAN };
```

`P(↑>↓)` is the Mann-Whitney U test probability that a randomly chosen up-bar has higher intensity than a randomly chosen down-bar. 0.5 = no systematic edge; >0.5 = up-bars tend to be more intense; <0.5 = down-bars tend to be more intense. Computed in O(N log N) using the rank-sum formulation.

#### Log₂ Ratio of Raw Means

```rust
// kline.rs:4386-4391
let log2_ratio = if mean_raw_up > 0.0 && mean_raw_dn > 0.0 {
    (mean_raw_up / mean_raw_dn).log2()
} else { f32::NAN };
```

`log₂(mean_up_intensity / mean_dn_intensity)`. A ratio in log space is additive across sessions, making it more stable than the raw ratio. +1.0 means up-bars were 2× more intense on average; -1.0 means down-bars were 2× more intense.

#### Conviction and Absorption

```rust
// kline.rs:4393-4400
let dominant_up = n_up >= n_dn;
let conviction = if dominant_up {
    if !mean_t_dn.is_nan() && mean_t_dn > 0.0 { mean_t_up / mean_t_dn } else { f32::NAN }
} else {
    if !mean_t_up.is_nan() && mean_t_up > 0.0 { mean_t_dn / mean_t_up } else { f32::NAN }
};
let absorption = if dominant_up { mean_t_dn } else { mean_t_up };
```

**Conviction** (`conv`): rank-normalized intensity ratio of dominant direction to minority direction. `2.0×` means the dominant side's bars were twice as intense as the losing side's bars. Higher = trend had fuel.

**Absorption** (`absorp`): mean rank-normalized intensity of the minority direction. High absorption means the losing side fought hard — potential exhaustion signal.

#### Climax Concentration

```rust
// kline.rs:4402-4406
let (top_n, top_up) = bars.iter().enumerate()
    .filter(|(i, _)| rank_norm[*i] > 0.75)
    .fold((0_usize, 0_usize), |(t, u), (_, b)| (t + 1, if b.is_up { u + 1 } else { u }));
let climax_up_frac = if top_n > 0 { top_up as f32 / top_n as f32 } else { f32::NAN };
```

`climax` is the fraction of top-25%-intensity bars (by rank) that closed up. `>=0.78` → bull climax (intense buying tail); `<=0.22` → bear climax. NaN if fewer than 4 bars in selection (no top-25% subset exists).

### Regime Classification

Seven regimes, checked in priority order (kline.rs:4413-4428):

```
climax_up_frac >= 0.78            → BULL CLIMAX ◈   (magenta)
climax_up_frac <= 0.22            → BEAR CLIMAX ◈   (magenta)
iwds >  0.15 && auc >= 0.60       → BULL CONVICTION  (green)
iwds < -0.15 && auc <= 0.40       → BEAR CONVICTION  (red)
iwds >  0.15 && auc <  0.50       → BULL ABSORPTION  (orange)
iwds < -0.15 && auc >  0.50       → BEAR ABSORPTION  (orange)
else                               → CONTESTED        (dim)
```

**Conviction**: IWDS and AUC agree — dominant direction was both more frequent and more intense. Strong directional signal.

**Absorption**: IWDS says one side was more intense but AUC says the other side's bars ranked higher. Divergence: the "winning" side by intensity was actually trading against the dominant count direction. Often seen near turning points.

**Climax**: The top-intensity events are concentrated ≥78% in one direction, regardless of count. Associated with blow-off tops/bottoms.

**Contested**: No clear edge detected.

Regimes with signal (all except CONTESTED) render a colored border on the stats box at 65% opacity.

### Box Layout and Color Scheme

```rust
// kline.rs:4488-4503
let ts = 13.0_f32;   // main font size
let sm = 11.0_f32;   // small font size

let lines: &[(String, iced::Color, f32)] = &[
    (format!("{distance} bars"),                                    neutral,  ts),
    (format!("↑ {n_up}  ({up_pct:.0}%)"),                          success,  ts),
    (format!("↓ {n_dn}  ({dn_pct:.0}%)"),                          danger,   ts),
    ("─────────────────────────────".to_string(),                   dim,      sm),
    (format!("↑t {}  ↑ {} t/s", ...),                              amber_dim, sm),
    (format!("↓t {}  ↓ {} t/s", ...),                              amber_dim, sm),
    (format!("{bar_str}  flow: {:+.2}", iwds),                      amber,    ts),
    (regime_label.to_string(),                                      regime_color, ts),
    (caption,                                                       dim_white, sm),
    (format!("P(↑>↓): {}   log₂(↑/↓): {}", ...),                  amber_dim, sm),
    (format!("conv: {}   absorp: {}", ...),                         amber_dim, sm),
    (climax_line,                                                   climax_color, sm),
];
```

Box dimensions (kline.rs:4506-4520):

```rust
let box_w = 215.0_f32;
let x = frame.width() / 2.0 - box_w / 2.0;   // horizontally centered
let y = 10.0_f32;                               // 10px from top
```

Background color: `Color { r: 0.07, g: 0.07, b: 0.07, a: 0.92 }` — near-black with 8% transparency so the chart context remains faintly visible.

Line height: `18px` for main-size lines (`ts=13` + 5px gap), `15px` for small lines (`sm=11` + 4px gap).

Color assignments:

| Element                      | Color                              |
| ---------------------------- | ---------------------------------- |
| Bar count                    | `neutral` (theme foreground)       |
| Up count/%                   | `success` (green)                  |
| Down count/%                 | `danger` (red)                     |
| Intensity lines (`↑t`, `↓t`) | `amber_dim` (0.85,0.65,0.15 @ 55%) |
| IWDS bar + flow              | `amber` (0.85,0.65,0.15 @ 100%)    |
| Regime label                 | Regime-specific (see table above)  |
| Caption                      | `dim_white` (0.75,0.75,0.75 @ 65%) |
| AUC, log₂, conv, absorp      | `amber_dim`                        |
| Climax line (no signal)      | `amber_dim`                        |
| Climax line (signal)         | `orange` (0.95,0.55,0.10)          |

---

## Key Constants and Thresholds Summary

| Constant            | Value                   | Location           | Purpose                             |
| ------------------- | ----------------------- | ------------------ | ----------------------------------- |
| Hit zone            | `abs_diff <= 1` bar     | kline.rs:3081-3086 | Brim drag start + hover cursor      |
| Fill alpha          | `0.02`                  | kline.rs:4258      | Selection area transparency         |
| Handle alpha        | `0.22`                  | kline.rs:4263      | Brim strip visibility               |
| Handle width min    | `3.0 px`                | kline.rs:4262      | Visible at extreme zoom-out         |
| Handle width max    | `60.0 px`               | kline.rs:4262      | Not overwhelming at extreme zoom-in |
| u64::MAX sentinel   | `u64::MAX`              | chart.rs:1284      | Forming-bar zone marker             |
| Stats box width     | `215.0 px`              | kline.rs:4509      | Horizontal extent of overlay        |
| Stats x position    | `frame.width/2 - 107.5` | kline.rs:4510      | Centered on chart                   |
| Stats y position    | `10.0 px`               | kline.rs:4511      | Near top of chart area              |
| Background alpha    | `0.92`                  | kline.rs:4520      | Stats box legibility                |
| Border width        | `1.5 px`                | kline.rs:4531      | Regime-colored border stroke        |
| Climax threshold    | `0.78 / 0.22`           | kline.rs:4414-4417 | ≥78% top-intensity bars one dir     |
| Conviction IWDS     | `\|iwds\| > 0.15`       | kline.rs:4418-4425 | Minimum directional flow            |
| Conviction AUC      | `auc >= 0.60 / <= 0.40` | kline.rs:4418-4421 | Minimum rank separation             |
| Top-quartile cutoff | `rank_norm > 0.75`      | kline.rs:4404      | Climax analysis top-25%             |

---

## Coordination with Other Systems

### ODB-Only Guard

The entire feature is guarded by `chart.basis.is_odb()` at every call site. It does not activate for `Basis::Time` or `Basis::Tick` charts. The `bar_selection` field exists in `KlineChart` for all basis types but remains in the `Default` (empty) state for non-ODB charts.

### PlotData Requirement for Stats

The stats overlay requires `PlotData::TickBased(tick_aggr)`:

```rust
// kline.rs:3410-3413
if let (Some(anchor), Some(end)) = (sel.anchor, sel.end)
    && let PlotData::TickBased(tick_aggr) = &self.data_source
{
    draw_bar_selection_stats(frame, palette, tick_aggr, anchor, end);
}
```

ODB charts always use `PlotData::TickBased`, so this match always succeeds. The guard exists to satisfy the Rust type system and to make the dependency explicit.

### Intensity Heatmap Coexistence

The stats overlay and the intensity heatmap spectrum legend both draw in the `legend` layer. They do not conflict spatially:

- The intensity heatmap spectrum legend draws at the far right edge of the chart
- The stats overlay draws centered at the top

The draw order within the `legend` closure is: heatmap legend first, then bar selection stats. The stats box may overlap the heatmap spectrum at narrow window widths, but at typical chart widths they are in separate horizontal zones.

### Sentinel Audits

The bar continuity sentinel (`audit_bar_continuity`) runs on a 60-second timer in `invalidate()`. It is independent of the bar selection state — a sentinel audit that triggers a re-fetch does not clear the selection. After the re-fetch completes and bars are replaced, the stored `anchor` and `end` indices remain valid as long as they fall within the new `tick_aggr.datapoints.len()`. The `hi.min(len - 1)` and `lo.min(len - 1)` clamps inside `draw_bar_selection_stats` ensure no out-of-bounds access if the selection extends beyond the loaded history.

### Cache Invalidation on Shift+Click

Third Shift+Click (restart) calls `self.chart.cache.clear_all()` (kline.rs:3117), which clears all four layers including `klines`. This is the only selection interaction that invalidates the expensive klines cache. The rationale: the anchor-set operation (1st click) also calls `clear_all()`, ensuring that once an anchor is placed, subsequent pan/zoom events regenerate the klines frame with the highlight visible. Brim drag only clears `crosshair` and `legend` because the klines geometry does not change during drag.
