---
title: "Trade Intensity Heatmap — Complete Technical Reference"
description: >
  Exhaustive documentation of the rolling log-quantile percentile heatmap indicator
  for ODB trade intensity in Flowsurface. Covers every algorithm step, data structure,
  colour mapping, legend layout, render pipeline integration, and diagnostic subsystem.
indicator_type: TradeIntensityHeatmap
source_files:
  - src/chart/indicator/kline/trade_intensity_heatmap.rs # Algorithm, colour, legend
  - data/src/chart/kline.rs # adaptive_k(), Config struct
  - src/chart/kline.rs # Render pipeline integration
key_functions:
  - adaptive_k(n: usize) -> u8
  - thermal_color(t: f32) -> Color
  - process_one(idx, intensity, bullish)
  - "rebuild_from_source(source: &PlotData<KlineDataPoint>)"
  - "on_insert_trades(_trades: &[Trade], old_dp_len: usize, source: &PlotData<KlineDataPoint>)"
  - thermal_body_color(storage_idx: u64) -> Option<Color>
  - draw_heatmap_legend(frame, k_actual: u8)
  - draw_screen_legend(frame)
  - log_oracle_spectrum()
key_types:
  - HeatmapPoint # Per-bar data point: intensity, bin, k_actual, bullish
  - TradeIntensityHeatmapIndicator # Indicator struct with rolling state
  - Config # Serialisable chart config (intensity_lookback, thermal_wicks)
key_constants:
  - SWATCH_W: f32 = 10.0
  - SWATCH_H: f32 = 9.0
  - ROW_H: f32 = 11.0
  - PAD: f32 = 4.0
  - GAP: f32 = 3.0
  - FONT_SIZE: f32 = 9.0
  - TEXT_OFFSET_X: f32 = SWATCH_W + GAP # = 13.0
  - LEGEND_W: f32 = PAD + TEXT_OFFSET_X + 7.0 * 5.5 + PAD # ≈ 59.5 px
  - K_MINIMUM: u8 = 5 # hardcoded literal floor in adaptive_k() — not a named constant
  - K_SATURATION_THRESHOLD: usize = 6332 # derived threshold (cbrt(n).round()==19); not a named constant
config_fields:
  - Config.intensity_lookback: usize # default 2000; range 100..=7000 (UI slider)
  - Config.thermal_wicks: bool # default true; wicks match body thermal colour
nuances:
  - "bin=0 is a sentinel meaning 'no microstructure data for this bar' — not a real bin"
  - "rank is computed BEFORE pushing current bar — zero look-ahead bias by construction"
  - "legend always shows adaptive_k(lookback), NOT the current window fill level"
  - "draw_screen_legend fires in the legend cache layer; thermal_body_color fires in klines layer"
  - "sorted Vec is O(N) insert but cache-friendly; outperforms BTreeMap range scan at 30K+ bars"
  - "VecDeque ring buffer: O(1) push_back + pop_front for eviction"
  - "storage_idx (oldest=0) ≠ visual_idx (newest=0); caller converts before calling thermal_body_color"
git_issue: "https://github.com/terrylica/rangebar-py/issues/97"
---

# Trade Intensity Heatmap — Complete Technical Reference

## 1. What This Indicator Does (In Plain Terms)

Every ODB bar carries a **trade intensity** value — the number of individual Binance
`@aggTrade` events that fired within that bar, divided by the bar's duration in seconds.
A bar that closes in 10 seconds and contains 400 trades has intensity 40 t/s.

Raw intensity has a ferocious power-law distribution on BPR25: skewness ≈ 322, and the
maximum observed value is ~700,000× the median. Mapping raw values to colour directly
would paint 99 % of bars the same shade and make outlier bars blindingly bright.

This indicator solves that with **rolling log-quantile percentile binning**:

1. Take `log10(intensity)` — compresses the power-law tail into a roughly uniform-looking distribution.
2. Compare the current bar's log value against a **rolling window** of the previous `lookback` bars.
3. Find what **percentile** the current bar sits at (0 % = coolest ever seen, 100 % = hottest ever seen).
4. Map that percentile to one of **K bins** (K is adaptive — see §4).
5. Map the bin to a **thermal colour** via a 300° HSV sweep (blue → magenta).

The result: every bar's colour conveys how _relatively_ active it was compared to recent
market history, not an absolute number you need to memorise.

---

## 2. Data Source

### Where `trade_intensity` Comes From

`trade_intensity` is a column in `opendeviationbar_cache.open_deviation_bars` computed
by the **opendeviationbar-py** pipeline (Python/DuckDB). It is fetched into Rust as part
of `ChMicrostructure`:

```rust
// exchange/src/adapter/clickhouse.rs
pub struct ChMicrostructure {
    pub individual_trade_count: Option<u32>,
    pub ofi: Option<f64>,
    pub trade_intensity: Option<f64>,   // ← unit: trades / second
}
```

This is threaded into `TickAccumulation.microstructure` as `OdbMicrostructure.trade_intensity: f32`
in `data/src/aggr/ticks.rs`. The heatmap indicator reads it via:

```rust
// In process_one(), called during rebuild_from_source() and on_insert_trades()
let intensity = dp.microstructure.map(|m| m.trade_intensity).unwrap_or(0.0);
```

Bars with no microstructure (live forming bar, or bars loaded before the microstructure
pipeline ran) get `intensity = 0.0` and are stored with `bin = 0` (the **sentinel** value
that means "skip colouring this bar").

---

## 3. The Per-Bar Data Point: `HeatmapPoint`

```rust
// src/chart/indicator/kline/trade_intensity_heatmap.rs:51
struct HeatmapPoint {
    intensity: f32,    // raw t/s (stored for tooltip display)
    bin: u8,           // 1..=k_actual, or 0 if no microstructure (sentinel)
    k_actual: u8,      // the K value at the time this bar was processed
    bullish: bool,     // close >= open (used for candle direction colouring)
}
```

The method `HeatmapPoint::t(self) -> f32` normalises `bin` to `[0, 1]`:

```rust
fn t(self) -> f32 {
    if self.k_actual <= 1 { return 0.0; }
    (self.bin - 1) as f32 / (self.k_actual - 1) as f32
}
```

`t = 0.0` → coldest blue; `t = 1.0` → hottest magenta.

---

## 4. The `TradeIntensityHeatmapIndicator` Struct

```rust
// src/chart/indicator/kline/trade_intensity_heatmap.rs:119
pub struct TradeIntensityHeatmapIndicator {
    cache: Caches,              // iced geometry caches (main, crosshair, legend, etc.)
    data: Vec<HeatmapPoint>,    // forward-indexed: data[0] = oldest bar, data[N-1] = newest
    lookback: usize,            // rolling window capacity (user-configurable, default 2000)
    ring: VecDeque<f32>,        // sliding window of log10(intensity) values, O(1) eviction
    sorted: Vec<f32>,           // same values as ring, kept sorted for O(log N) rank queries
    next_idx: usize,            // global index of the next datapoint to process (incremental path)
}
```

### Key Invariants

| Invariant                                               | Description                                     |
| ------------------------------------------------------- | ----------------------------------------------- |
| `data.len() == tickseries.datapoints.len()`             | One `HeatmapPoint` per completed bar            |
| `ring.len() == sorted.len() <= lookback`                | Rolling window is always bounded                |
| `data[i].bin == 0 iff data[i].intensity == 0.0`         | Sentinel is exclusive to no-microstructure bars |
| `data[i].k_actual = adaptive_k(ring.len() before push)` | K is frozen per bar at time of processing       |

### Construction

```rust
// Default lookback = 2000
TradeIntensityHeatmapIndicator::new()             // lookback = 2000

// User-configured via Config.intensity_lookback
TradeIntensityHeatmapIndicator::with_lookback(lookback)
```

`with_lookback` pre-allocates `sorted` with capacity `lookback + 1` to avoid reallocation.

---

## 5. `adaptive_k(n)` — The Cube-Root Rule

```rust
// data/src/chart/kline.rs:380
pub fn adaptive_k(n: usize) -> u8 {
    ((n as f32).cbrt().round() as u8).max(5)
}
```

`n` is the **current window size** (`sorted.len()` = number of bars in the rolling
window at the time a bar is being processed), NOT the lookback setting directly.

### How It Works

- **Cube root of window size**: a classical histogram rule (Sturges, Scott, Rice) adapted
  for uniformly-distributed quantile bins. With N samples, cube-root gives ≈ N^(1/3) bins.
- **Floor of 5**: prevents degenerate single-bin behaviour on the very first bars.
- **No ceiling**: K grows unboundedly as the window fills. The effective ceiling is imposed
  by the lookback setting (once `window.len() == lookback` it stops growing).

### Saturation Table

| `lookback` | Bars to saturate window | `adaptive_k(lookback)` | Max K reached              |
| ---------- | ----------------------- | ---------------------- | -------------------------- |
| 100        | 100                     | 5                      | 5                          |
| 500        | 500                     | 8                      | 8                          |
| 1000       | 1000                    | 10                     | 10                         |
| 2000       | 2000                    | 13                     | 13 (default)               |
| 3375       | 3375                    | 15                     | 15                         |
| 7000       | 6332                    | 19                     | 19 (max at K=19 threshold) |

The key inflection: **`lookback = 2000` → K = 13** (default out-of-box). Setting
`lookback = 7000` unlocks K = 19 with 19 visually distinct colour bins.

### Critical Dependency: ClickHouse Fetch Strategy

`adaptive_k` can only reach K = 19 if the `TickAggr` actually has ≥ 6332 bars loaded.
This requires the **full-reload path** in `missing_data_task()`:

```rust
// src/chart/kline.rs — initial fetch and sentinel refetch MUST use u64::MAX
FetchRange::Kline(0, u64::MAX)   // ✅ full-reload path → adaptive limit (13K–20K bars)
FetchRange::Kline(0, now_ms)     // ❌ hits LIMIT 2000 → K forever capped at ≈13
```

See `exchange/CLAUDE.md` for the `u64::MAX` sentinel pattern.

---

## 6. `process_one()` — The Core Algorithm (Step by Step)

```rust
fn process_one(&mut self, idx: usize, intensity: f32, bullish: bool)
```

Called once per bar, **in ascending order** (oldest first). `idx` must equal
`self.data.len()` at the time of the call (sequential-only).

### Step 1: Log Transform

```rust
let log_val = intensity.log10().max(0.0);
```

- `intensity.log10()` compresses the power-law range into a near-uniform distribution.
- `.max(0.0)` clamps values below 1.0 t/s to 0 (log10(1) = 0; values < 1 give negative logs
  which would be meaningless in a percentile of always-positive intensities).

### Step 2: Sample `adaptive_k` Before Push

```rust
let n = self.sorted.len();   // current window size (BEFORE adding current bar)
let k_actual = if n == 0 { 5u8 } else { adaptive_k(n) };
```

`n == 0` on the very first bar → K is seeded at 5 (the minimum).

### Step 3: Rank Query (No Look-Ahead)

```rust
let bin = if n == 0 {
    1u8    // first bar: no history → coldest bin by definition
} else {
    let rank_count = self.sorted.partition_point(|&v| v <= log_val);
    let rank = rank_count as f32 / n as f32;
    ((rank * k_actual as f32).ceil() as u8).clamp(1, k_actual)
};
```

- `partition_point(|&v| v <= log_val)` — binary search on the sorted Vec. Returns the
  count of values ≤ `log_val`, i.e. the number of historical bars less active than
  (or equally active as) the current bar. O(log N).
- `rank = rank_count / n` — normalises to [0, 1]. rank = 0.0 means coldest bar ever;
  rank = 1.0 means at least as hot as every bar in the window.
- `(rank × k_actual).ceil()` — maps rank to bin in 1..=K. `.ceil()` means the lowest
  rank > 0 lands in bin 1 (not bin 0). `.clamp(1, k_actual)` handles the edge case
  where `rank = 0.0` exactly (→ ceil(0) = 0, clamped to 1).

> **Zero look-ahead guarantee**: the rank is computed against `self.sorted` which contains
> only bars _prior_ to the current one. The current bar is pushed into `self.sorted` only
> _after_ `bin` is determined (Step 4 below).

### Step 4: Store Result

```rust
self.data.push(HeatmapPoint { intensity, bin, k_actual, bullish });
```

If `idx > self.data.len()` (gap in indices — bars without microstructure in the middle),
sentinel `HeatmapPoint { bin: 0, k_actual: 0, ... }` entries are inserted to keep the
Vec densely packed.

### Step 5: Push Into Rolling Window

```rust
let ins_pos = self.sorted.partition_point(|&v| v < log_val);
self.sorted.insert(ins_pos, log_val);   // maintain sorted order: O(N) shift
self.ring.push_back(log_val);           // maintain FIFO order for eviction: O(1)
```

Both `ring` and `sorted` always hold the same set of values. `ring` tracks insertion order
(for FIFO eviction); `sorted` tracks sorted order (for binary search).

### Step 6: Evict Oldest If Over Capacity

```rust
if self.ring.len() > self.lookback {
    let old = self.ring.pop_front().unwrap();      // FIFO eviction: O(1)
    let pos = self.sorted.partition_point(|&v| v < old);
    if pos < self.sorted.len() {
        self.sorted.remove(pos);                    // remove exactly one: O(N) shift
    }
}
```

Eviction keeps `ring.len() == min(bars_processed, lookback)` at all times.

---

## 7. Full Rebuild vs Incremental Update

### Full Rebuild: `rebuild_from_source()`

```rust
fn rebuild_from_source(&mut self, source: &PlotData<KlineDataPoint>)
```

Called when:

- A new `TickAggr` is loaded (initial CH fetch, sentinel refetch)
- Tick size changes (`on_ticksize_change`)
- Basis changes (`on_basis_change`)
- `next_idx != old_dp_len` (state mismatch detected in `on_insert_trades`)

**Procedure**: calls `reset_state()` (clears `ring`, `sorted`, `data`, `next_idx` —
allocations kept for reuse), then iterates all `tickseries.datapoints` calling
`process_one()` for each.

**Performance**: O(N log N) due to sorted inserts, but cache-friendly Vec operations.
At 20K bars this takes ~80ms on aarch64 (vs ~2ms for incremental). The `data` and
`sorted` Vecs retain their allocations across resets — no realloc cost.

### Incremental Update: `on_insert_trades()`

```rust
fn on_insert_trades(&mut self, _trades: &[Trade], old_dp_len: usize, source: &PlotData<KlineDataPoint>)
```

Called whenever `TickAggr` completes new bars. Guard condition:

```rust
if self.next_idx == old_dp_len {
    // Only process bars from old_dp_len..new_len — previously unseen bars.
    for idx in old_dp_len..new_len { self.process_one(idx, ...) }
    self.next_idx = new_len;
} else {
    // State mismatch → full rebuild
    self.rebuild_from_source(source);
}
```

`next_idx` is the global counter of how many datapoints have been processed. If it
matches `old_dp_len` exactly, only the N newly completed bars need processing — O(N log W)
where W is the window size and N is typically 1.

---

## 8. `thermal_color(t)` — The 300° HSV Colour Sweep

```rust
fn thermal_color(t: f32) -> Color
```

`t` is in `[0.0, 1.0]` where `t = (bin - 1) / (k_actual - 1)`.

### The Formula

```rust
let hue_deg = (240.0 - t.clamp(0.0, 1.0) * 300.0).rem_euclid(360.0);
let s = 0.95_f32;
let v = 0.92_f32;
// Standard HSV → RGB conversion (6-sector)
```

| Parameter              | Value          | Meaning                                                            |
| ---------------------- | -------------- | ------------------------------------------------------------------ |
| `s` (saturation)       | 0.95           | Near-maximum saturation — highly vivid colours on dark backgrounds |
| `v` (value/brightness) | 0.92           | Near-maximum brightness — bright but not blown-out                 |
| Hue start              | 240° (blue)    | coldest bin (K1, t=0)                                              |
| Hue sweep              | 300° backward  | total arc across the colour wheel                                  |
| Hue end                | 300° (magenta) | hottest bin (K_max, t=1.0), wrapping via `rem_euclid`              |

### Hue Anchors at K=19 (≈16.7°/bin)

| Bin       | t     | Hue  | Colour       |
| --------- | ----- | ---- | ------------ |
| K1 (Calm) | 0.000 | 240° | Blue         |
| K2        | 0.056 | 223° | Indigo-blue  |
| K3        | 0.111 | 207° | Azure        |
| K4        | 0.167 | 190° | Cyan-blue    |
| K5        | 0.222 | 173° | Cyan         |
| K6        | 0.278 | 157° | Teal         |
| K7        | 0.333 | 140° | Green-teal   |
| K8        | 0.389 | 123° | Green        |
| K9        | 0.444 | 107° | Yellow-green |
| K10       | 0.500 | 90°  | Lime         |
| K11       | 0.556 | 73°  | Yellow-lime  |
| K12       | 0.611 | 57°  | Yellow       |
| K13       | 0.667 | 40°  | Amber        |
| K14       | 0.722 | 23°  | Orange       |
| K15       | 0.778 | 7°   | Red-orange   |
| K16       | 0.833 | 350° | Red          |
| K17       | 0.889 | 333° | Crimson      |
| K18       | 0.944 | 317° | Hot pink     |
| K19 (Max) | 1.000 | 300° | Magenta      |

### Why Not Turbo / Viridis?

Turbo was evaluated first. Its hot zone compresses bins 13–19 into ~30° of hue change
(~5°/bin ≈ 2× just-noticeable-difference threshold). This caused the observed
"K stops diversifying at 13" problem — bins 14–19 were visually indistinguishable red
variants. The 300° HSV sweep continues through red into magenta, giving ≈16.7°/bin —
**8× the ~2° JND threshold** — so all 19 bins are perceptually distinct.

---

## 9. Candle Body (and Wick) Colouring

### `thermal_body_color(storage_idx)` — Called by the Render Pipeline

```rust
// src/chart/indicator/kline/trade_intensity_heatmap.rs:653
fn thermal_body_color(&self, storage_idx: u64) -> Option<Color> {
    self.data
        .get(storage_idx as usize)
        .filter(|p| p.bin != 0)   // sentinel guard: bin=0 → no colour override
        .map(|p| thermal_color(p.t()))
}
```

`storage_idx` uses the **oldest-first** index (same as `data` Vec). The caller
in `kline.rs` converts from `visual_idx` (newest = 0) to `storage_idx`:

```rust
// src/chart/kline.rs:3305-3328
let storage_idx = total_len.saturating_sub(1 + visual_idx);
let thermal_color = heatmap_indi.and_then(|h| h.thermal_body_color(storage_idx as u64));
```

### Wick Colouring

Controlled by `Config.thermal_wicks: bool` (default `true`):

```rust
let thermal_wicks = self.kline_config.thermal_wicks;
let wick_color = if thermal_wicks { thermal_color } else { None };
// → None causes draw_candle_dp() to fall back to green/red direction colouring
```

Both `thermal_color` and `wick_color` are passed to `draw_candle_dp()` in `kline.rs:3321`.

---

## 10. The Colour-Scale Legend

### `draw_screen_legend()` — Entry Point from Render Pipeline

```rust
// Called in kline.rs legend cache layer:
// src/chart/kline.rs:3401-3405
let legend = chart.cache.legend.draw(renderer, bounds_size, |frame| {
    if let Some(heatmap) = self.indicators[KlineIndicator::TradeIntensityHeatmap].as_deref() {
        heatmap.draw_screen_legend(frame);
    }
    // ...
});
```

`draw_screen_legend` delegates:

```rust
fn draw_screen_legend(&self, frame: &mut canvas::Frame) {
    if self.data.is_empty() { return; }
    // NOTE: uses adaptive_k(self.lookback), NOT adaptive_k(self.ring.len())
    // → legend always shows the *configured* max K, not the current window fill
    draw_heatmap_legend(frame, adaptive_k(self.lookback));
}
```

**Key nuance**: the legend always shows `adaptive_k(self.lookback)` — the K value that
will be reached once the window is fully saturated. If you just loaded the chart and
only 200 bars are in the window, `adaptive_k(200)` ≈ 6 while the legend still shows
K=13 (for lookback=2000). This is intentional: the legend represents the _full scale_
the user has configured, so colours in the UI always map to the same legend labels.

### `draw_heatmap_legend(frame, k_actual)` — Layout Internals

```rust
fn draw_heatmap_legend(frame: &mut canvas::Frame, k_actual: u8)
```

#### Layout Constants

| Constant        | Value                                             | Purpose                                     |
| --------------- | ------------------------------------------------- | ------------------------------------------- |
| `SWATCH_W`      | `10.0` px                                         | Width of the colour patch square            |
| `SWATCH_H`      | `9.0` px                                          | Height of the colour patch square           |
| `ROW_H`         | `11.0` px                                         | Total row height (patch + vertical spacing) |
| `PAD`           | `4.0` px                                          | Outer padding on all sides of the panel     |
| `GAP`           | `3.0` px                                          | Horizontal gap between patch and label text |
| `FONT_SIZE`     | `9.0` px                                          | Label font size in iced Pixels              |
| `TEXT_OFFSET_X` | `SWATCH_W + GAP = 13.0` px                        | X offset from panel left to label start     |
| `LEGEND_W`      | `PAD + TEXT_OFFSET_X + 7.0 × 5.5 + PAD ≈ 59.5` px | Total panel width                           |

#### Position Calculation

```rust
let legend_h = k_actual as f32 * ROW_H + PAD * 2.0;
// = k_actual × 11.0 + 8.0 px

let origin_x = frame.width() - LEGEND_W - PAD;
// = frame.width() - 63.5 px   (right-anchored)

let origin_y = (frame.height() - legend_h - PAD).max(PAD);
// Bottom-anchored; .max(PAD) prevents clipping if legend is taller than the frame
```

At K=13: `legend_h = 13 × 11 + 8 = 151 px`
At K=19: `legend_h = 19 × 11 + 8 = 217 px`

#### Row Drawing (Hottest at Top)

```rust
for bin in (1..=k_actual).rev() {         // K → 1 top-to-bottom
    let row_idx = (k_actual - bin) as f32; // 0 at top, K-1 at bottom
    let row_y = origin_y + PAD + row_idx * ROW_H;

    let t = (bin - 1) as f32 / (k_actual - 1) as f32;
    let color = thermal_color(t);

    // Colour swatch: vertically centred within the row
    frame.fill_rectangle(
        Point::new(origin_x + PAD, row_y + (ROW_H - SWATCH_H) * 0.5),
        Size::new(SWATCH_W, SWATCH_H),
        color,
    );

    // Label: "K{n} Max" for top bin, "K1 Calm" for bottom, "K{n}" for middle
    let label_text = match bin {
        b if b == k_actual => format!("K{bin} Max"),
        1 => "K1 Calm".to_string(),
        _ => format!("K{bin}"),
    };
}
```

#### Background Panel

```rust
frame.fill_rectangle(
    Point::new(origin_x, origin_y),
    Size::new(LEGEND_W, legend_h),
    Color::from_rgba(0.0, 0.0, 0.0, 0.65),   // semi-transparent black
);
```

#### Position Relative to Other UI Elements

The legend anchors to `frame.width()` (right edge of the chart canvas). The crosshair
tooltip (OHLC + timing + agg_trade_id) is positioned with a right margin of `72.0` px
to avoid overlapping:

```rust
// src/chart/kline.rs:4523 — crosshair tooltip right margin
frame.width() - bg_width - 72.0
// = frame.width() - bg_width - (LEGEND_W + PAD + gap)
//                             ≈ 59.5   + 4   + 8   = 71.5 → 72.0
```

---

## 11. The Oracle Diagnostic Subsystem

`log_oracle_spectrum()` fires at the end of every `rebuild_from_source()` call. It writes
to both `log::warn!` and `/tmp/flowsurface-oracle.log`.

### What the Oracle Reports

**Section 1 — Colour table**: one row per bin from 1 to `adaptive_k(lookback)`, showing:

- `bin` number
- `t` value (normalized position in [0,1])
- hex colour `#RRGGBB` computed from `thermal_color(t)`

**Section 2a — All-bars histogram**: bin distribution across the last 500 bars, regardless
of what `k_actual` was when each bar was processed (relevant during window warm-up).

**Section 2b — Filtered histogram**: same 500 bars, but **only those where
`p.k_actual == adaptive_k(lookback)`** (i.e. bars processed at full window capacity).
The ideal distribution is flat across all K bins (each gets ≈ 1/K of bars).

### Oracle Log Tags

| Tag                     | When                    | Meaning                                   |
| ----------------------- | ----------------------- | ----------------------------------------- | ----- | ---- |
| `[oracle-spectrum]`     | Every rebuild           | Full colour table + histograms            |
| `[oracle-rebuild-tail]` | Every rebuild           | Last bar's bin/k_actual/t                 |
| `[oracle-incr-tail]`    | Every incremental batch | Last newly-added bar                      |
| `[oracle-FAIL]`         | Error                   | Bar has microstructure but `bin == 0`     |
| `[oracle-bin]`          | trace level             | Per-bar bin assignment (verbose)          |
| `[intensity-rebuild]`   | Every rebuild           | old/new data len + tail sample            |
| `[intensity-incr]`      | Incremental path        | new bars count + last bar                 |
| `[intensity-mismatch]`  | State diverge           | next_idx ≠ old_dp_len → rebuild triggered |
| `[intensity-diverge]`   | kline.rs diverge check  | heatmap_len ≠ dp_count (                  | delta | > 1) |

The `bin=0` oracle FAIL triggers a Telegram critical alert via:

```rust
exchange::tg_alert!(exchange::telegram::Severity::Critical, "oracle", "...");
```

---

## 12. Integration in the Render Pipeline (`kline.rs`)

The heatmap plugs into the `kline.rs` render pipeline at three distinct points:

### 12.1 Lookup (once per draw frame)

```rust
// src/chart/kline.rs:3277-3281 (inside klines cache closure)
let heatmap_indi =
    self.indicators[KlineIndicator::TradeIntensityHeatmap].as_deref();
let total_len = if let PlotData::TickBased(t) = &self.data_source {
    t.datapoints.len()
} else { 0 };
```

`KlineIndicator::TradeIntensityHeatmap` is an `EnumMap` key — O(1) lookup.

### 12.2 Divergence Check (once per draw frame)

```rust
// src/chart/kline.rs:3287-3293
if let Some(h) = heatmap_indi {
    let delta = h.data_len() as isize - total_len as isize;
    if delta.unsigned_abs() > 1 {
        log::warn!("[intensity-diverge] ...");
        exchange::tg_alert!(Warning, "intensity", "...");
    }
}
```

`|delta| == 1` is normal (forming bar has no completed microstructure yet). `|delta| > 1`
indicates a sync bug.

### 12.3 Per-Candle Colour Lookup (inside `render_data_source` loop)

```rust
// src/chart/kline.rs:3305-3330 (inside render_data_source closure)
let thermal_color = heatmap_indi.and_then(|h| {
    let storage_idx = total_len.saturating_sub(1 + visual_idx);
    h.thermal_body_color(storage_idx as u64)
});
let wick_color = if thermal_wicks { thermal_color } else { None };
draw_candle_dp(frame, price_to_y, candle_width, palette, x_position, kline,
               thermal_color, wick_color);
```

`visual_idx` runs 0 (newest) to N-1 (oldest); `storage_idx` inverts this to the
`data` Vec ordering.

### 12.4 Screen-Space Legend (legend cache layer)

```rust
// src/chart/kline.rs:3401-3405
let legend = chart.cache.legend.draw(renderer, bounds_size, |frame| {
    if let Some(heatmap) = self.indicators[KlineIndicator::TradeIntensityHeatmap].as_deref() {
        heatmap.draw_screen_legend(frame);   // fires draw_heatmap_legend(frame, K)
    }
    // ...bar selection stats overlay...
});
```

This is in the **legend cache** (screen-space, no frame transforms). Cleared on every
cursor move. The `klines` (main) cache is unaffected — candle body colours are baked
into the main cache geometry and only re-drawn when bars change.

---

## 13. Configuration and Persistence

### `Config` Struct (serialisable)

```rust
// data/src/chart/kline.rs:351
pub struct Config {
    pub ofi_ema_period: usize,        // default 20
    pub intensity_lookback: usize,    // default 2000; drives adaptive_k → K
    pub thermal_wicks: bool,          // default true
    pub show_sessions: bool,          // default false
}
```

Serialised to `~/Library/Application Support/flowsurface/saved-state.json` as part of
pane state.

### Changing Lookback at Runtime

```rust
// src/chart/kline.rs:1727
pub fn set_intensity_lookback(&mut self, lookback: usize) {
    self.kline_config.intensity_lookback = lookback;
    if self.indicators[KlineIndicator::TradeIntensityHeatmap].is_some() {
        let mut new_indi = TradeIntensityHeatmapIndicator::with_lookback(lookback);
        new_indi.rebuild_from_source(&self.data_source);   // full O(N log N) rebuild
        self.indicators[KlineIndicator::TradeIntensityHeatmap] = Some(new_indi);
    }
}
```

Triggered by the lookback slider in the UI (range `100..=7000`, default `2000`).
The indicator is rebuilt from scratch because the window capacity change invalidates
all stored `k_actual` values and the eviction order.

---

## 14. What You See in the UI — End-to-End Walk-Through

1. **User has `lookback = 2000`** → `adaptive_k(2000) = 13` (K=13 legend shows in bottom-right)
2. **BPR25 chart loads 13,000 bars** → first 2000 bars fill the window, then it stays saturated
3. **Bar 1**: `ring.len() = 0` → K forced to 5, `bin = 1` (coldest). After push: `ring.len() = 1`
4. **Bar 5**: `ring.len() = 4` → `adaptive_k(4) = max(round(cbrt(4)), 5) = 5`. rank computed vs 4 bars.
5. **Bar 13**: `adaptive_k(12) = max(round(2.29), 5) = 5`. Still K=5.
6. **Bar 126**: `adaptive_k(125) = max(round(5), 5) = 5`. Still K=5. (cbrt(125)=5 exactly)
7. **Bar 127**: `adaptive_k(126) = max(round(cbrt(126)), 5) = max(5,5) = 5`. Stays 5.
8. **Bar 216+**: `adaptive_k(216) = max(round(6), 5) = 6`. K upgrades to 6.
9. **Bar 2001**: `ring.len() = 2000` → `adaptive_k(2000) = 13`. Window saturated. K=13 thereafter.
10. **Crosshair hover**: tooltip shows `"Intensity: {p.intensity:.1} t/s (bin {p.bin}/{p.k_actual})"`
11. **Legend**: shows K13 Max (magenta) at top → K1 Calm (blue) at bottom, regardless of current fill.

---

## 15. Data-Flow Summary Diagram

```
ClickHouse opendeviationbar_cache
  └── ChMicrostructure.trade_intensity (f64, trades/second)
      └── OdbMicrostructure.trade_intensity (f32) in TickAccumulation.microstructure
          └── process_one(idx, intensity, bullish)
              ├── log_val = log10(intensity).max(0.0)
              ├── k_actual = adaptive_k(sorted.len())
              ├── rank_count = sorted.partition_point(|&v| v <= log_val)
              ├── rank = rank_count / sorted.len()
              ├── bin = ceil(rank × k_actual).clamp(1, k_actual)
              ├── data.push(HeatmapPoint { intensity, bin, k_actual, bullish })
              ├── sorted.insert(partition_point for log_val, log_val)  ← maintain sorted order
              ├── ring.push_back(log_val)                              ← maintain FIFO order
              └── if ring.len() > lookback: evict oldest from ring + sorted
                  └─►  HeatmapPoint.t() = (bin-1)/(k_actual-1)
                           └─► thermal_color(t) → HSV(240°-300°t, 0.95, 0.92) → Color
                                   ├── thermal_body_color(storage_idx) → candle body colour
                                   ├── wick_color (if thermal_wicks=true)
                                   └── draw_heatmap_legend(frame, adaptive_k(lookback))
                                           └── Legend panel: K bins top→bottom, hottest first
```
