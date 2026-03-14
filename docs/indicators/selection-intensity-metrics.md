---
title: "Bar Selection Intensity Metrics — Technical Reference"
description: >
  Comprehensive documentation of the 7-metric intensity analysis overlay shown
  when a bar range is selected on ODB charts. Covers IWDS, Mann-Whitney AUC,
  log₂ ratio, conviction, absorption, climax concentration, and regime classification.
feature: SelectionIntensityMetrics
source_files:
  - src/chart/kline.rs # draw_bar_selection_stats()
  - data/src/aggr/ticks.rs # OdbMicrostructure.trade_intensity
key_functions:
  - "draw_bar_selection_stats(frame, palette, tick_aggr, anchor, end)"
key_types:
  - BarSample # local struct: raw: f32, is_up: bool
  - Regime # local enum: BullConviction/BearConviction/BullAbsorption/BearAbsorption/BullClimax/BearClimax/Contested
key_variables:
  - rank_norm # Vec<f32> within-selection rank normalisation
  - order # Vec<usize> sort index, reused for both rank_norm and AUC
  - iwds # f32 in [-1,+1], intensity-weighted direction score
  - auc # f32 in [0,1], Mann-Whitney P(up>dn)
  - log2_ratio # f32, log2(mean_raw_up / mean_raw_dn)
  - conviction # f32, mean_t(dominant) / mean_t(minority)
  - absorption # f32, mean_t of minority direction
  - climax_up_frac # f32 in [0,1], top-quartile up fraction
key_constants:
  - REGIME_IWDS_THRESHOLD: 0.15
  - REGIME_AUC_HIGH: 0.60
  - REGIME_AUC_LOW: 0.40
  - CLIMAX_THRESHOLD: 0.78
  - BOX_W: 215.0
nuances:
  - "rank_norm is within-selection only — two different sessions are directly comparable"
  - "AUC reuses the same sorted order[] as rank_norm — single sort pass for both"
  - "IWDS session baseline cancels: α×mean_up / α×total = mean_up/total"
  - "Regime priority: Climax checked BEFORE Conviction (climax takes precedence)"
  - "absorption shows mean_t of MINORITY bars, not of down-bars specifically"
literature:
  - "Cont, Kukanov & Stoikov (2014) — OFI formulation (IWDS analogue)"
  - "Mann-Whitney (1947) — rank-sum test (AUC)"
  - "Shinohara Intensity Ratio — cumulative intensity ratio (related)"
---

# Bar Selection Intensity Metrics — Technical Reference

## 1. What This Feature Does (Plain Terms)

When you select a range of ODB bars by Shift-clicking on the chart, a small statistics
overlay appears at the top-center of the chart pane. You already see the obvious counts
— how many bars went up, how many went down, and their percentages. But the overlay
goes much further.

It asks a more interesting question: **was the buying urgent or lazy compared to the
selling?**

A bar moving up with 200 trades per second arriving at the exchange is a very different
event from a bar moving up with 5 trades per second. The first is contested, frantic,
potentially climactic. The second is a slow drift through thin air. Both close higher —
but the microstructure story is opposite.

The overlay computes seven statistical metrics that each illuminate a different facet of
that question, then synthesises them into a single **regime label** that captures the
dominant pattern: conviction, absorption, climax, or contested.

**Example read**: You select 20 bars following a sharp move up. The overlay shows
`BULL ABSORPTION`. This means: yes, more bars closed up, but the intensity was actually
loading into the down-bars — sellers were fighting more urgently than buyers.
Classic divergence before a reversal.

---

## 2. Data Source and Prerequisites

### Where `trade_intensity` Lives

Every completed ODB bar carries an `OdbMicrostructure` struct embedded in its
`TickAccumulation`:

```rust
// data/src/aggr/ticks.rs
pub struct OdbMicrostructure {
    pub trade_intensity: f32,  // trades / second over this bar's duration
    pub ofi: f32,              // order-flow imbalance [-1, +1]
}
```

`trade_intensity` is the number of individual Binance `@aggTrade` events that arrived
during the bar, divided by the bar's duration in seconds. A bar that lasted 3 seconds
and contained 600 trades has intensity 200.0 t/s.

The raw value originates in ClickHouse — it is a column in
`opendeviationbar_cache.open_deviation_bars` precomputed by the opendeviationbar-py
pipeline. It is deserialized through `ChMicrostructure.trade_intensity: Option<f64>`
and stored as `f32` on the Rust side.

Bars without microstructure (e.g. the live forming bar, or bars loaded before the
pipeline ran) have `microstructure = None`. The collection step maps these to
`raw = 0.0` via `map_or(0.0, |m| m.trade_intensity)`.

### What "t/s" Means in Context

`trade_intensity` is an **absolute** rate. It is session-dependent by construction:
during the New York open, even slow moves can show 80 t/s because the market is busy.
During the Asian session, a fast move might only show 15 t/s. The overlay's design
accounts for this wherever possible — rank-normalized metrics (`↑t`, `↓t`, conviction,
absorption) are session-independent because they normalize within the selection. Metrics
that use raw values (`log₂` ratio, IWDS) also cancel session baseline naturally, as
explained in §5.

### ODB-Only Feature

This overlay only renders when the chart's `PlotData` is `TickBased` (i.e. when the
chart is an ODB pane, `Basis::Odb(_)` or `Basis::Tick(_)`). Time-series charts
(`TimeBased`) do not have per-bar `OdbMicrostructure` and the function is not called
for them.

---

## 3. Triggering: Bar Range Selection

### User Interaction

The selection is created using **Shift+Left Click** on the chart canvas:

- **First Shift-click**: sets the anchor bar.
- **Second Shift-click** (different bar): sets the end bar. The overlay appears.
- **Third Shift-click**: resets — the clicked bar becomes the new anchor.
- **Click on the outermost bar of an existing selection** (the "brim"): drag mode —
  move that boundary while keeping the other fixed.

The anchor and end are stored in `BarSelectionState` (wrapped in `RefCell<T>` for
interior mutability, since `canvas::Program::update()` takes `&self`). Both are
`usize` visual indices (newest = 0).

### Function Signature and Coordinate Conversion

```rust
fn draw_bar_selection_stats(
    frame: &mut canvas::Frame,
    palette: &Extended,
    tick_aggr: &data::aggr::ticks::TickAggr,
    anchor: usize,
    end: usize,
)
```

`anchor` and `end` are visual indices (newest bar = 0). The `tick_aggr.datapoints`
Vec uses **storage indices** (oldest bar = 0). Conversion is:

```
storage_idx = len - 1 - visual_idx
```

The function computes `(lo, hi) = (anchor.min(end), anchor.max(end))` so that the
range is always valid regardless of which end the user clicked first. `hi` is then
clamped to `len - 1`.

`distance = hi - lo` is the number of bar-gaps between the two endpoints. A selection
of a single bar has `distance = 0`; two adjacent bars have `distance = 1`.

### Render Layer

The function is called inside the **`legend` cache layer** in `kline.rs`. This layer
is screen-space (no chart translation or scaling applied), and is invalidated on every
cursor move — making it efficient for interactive overlays that update with drag.
The heavy `main` cache (candle bodies) is not touched during selection.

---

## 4. Data Collection: `BarSample`

The first step is collecting the raw data for all bars in the selection:

```rust
struct BarSample {
    raw: f32,    // trade_intensity in t/s (0.0 if no microstructure)
    is_up: bool, // close >= open
}

let bars: Vec<BarSample> = (lo..=hi)
    .map(|vi| {
        let si = len - 1 - vi;
        let dp = &tick_aggr.datapoints[si];
        BarSample {
            raw: dp.microstructure.map_or(0.0, |m| m.trade_intensity),
            is_up: dp.kline.close >= dp.kline.open,
        }
    })
    .collect();
```

After collection:

```rust
let n = bars.len();
let n_up = bars.iter().filter(|b| b.is_up).count();
let n_dn = n - n_up;
let up_pct = n_up as f32 / n as f32 * 100.0;
let dn_pct = n_dn as f32 / n as f32 * 100.0;
```

"Up bar" is defined as `close >= open` — a doji (open == close) counts as up. This
matches the candle body color convention used throughout the chart.

---

## 5. The Seven Metrics

### 5.1 — Within-Selection Rank Normalization (`↑t` / `↓t`)

**Plain explanation**: On a scale of 0 to 1, how intense were the up-bars compared to
the down-bars, relative to the other bars in this specific selection? A value of 0.68
for `↑t` means up-bars were, on average, in the top 32% of intensity within this window.
You do not need to memorize any absolute trade rate — the comparison is always to the
other bars you selected.

**Technical specification**:

A sorted index `order` is computed once and reused by both rank normalization and the
AUC calculation (§5.3):

```rust
let mut order: Vec<usize> = (0..n).collect();
order.sort_unstable_by(|&a, &b| {
    bars[a].raw.partial_cmp(&bars[b].raw).unwrap_or(std::cmp::Ordering::Equal)
});
```

Rank normalization with tie handling:

```rust
let mut rank_norm = vec![0.5_f32; n];
if n > 1 {
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && (bars[order[j + 1]].raw - bars[order[i]].raw).abs() < 1e-6 {
            j += 1;
        }
        // Ties get average rank
        let avg = (i + j) as f32 * 0.5 / (n - 1) as f32;
        for k in i..=j {
            rank_norm[order[k]] = avg;
        }
        i = j + 1;
    }
}
```

- Bars are ranked 0.0 (coldest in selection) to 1.0 (hottest in selection).
- Ties receive the average of their position range: e.g. if positions 3 and 4 (0-indexed)
  both have the same raw intensity and n=10, both get `(3+4)/2 / 9 = 0.389`.
- With n=1, `rank_norm` defaults to `[0.5]` — the sole bar is at the midpoint.

Per-direction means of rank_norm:

```rust
let mean_t_up = if n_up > 0 { sum_t_up / n_up as f32 } else { f32::NAN };
let mean_t_dn = if n_dn > 0 { sum_t_dn / n_dn as f32 } else { f32::NAN };
```

**Display format**: `↑t 0.68  ↑ 124 t/s` — the rank-normalized mean followed by the raw
trades-per-second mean in parentheses. Both numbers on the same line.

**Session independence**: because normalization reference is the selection itself,
a Monday NY-session selection and a Sunday overnight selection are directly comparable.
`↑t = 0.68` in both cases means "up-bars were in the 68th percentile of intensity
within their own window" — regardless of whether that was 200 t/s or 15 t/s in absolute terms.

**Edge cases**:

- `n_up = 0`: `mean_t_up = NaN` → displayed as `—`
- `n_dn = 0`: `mean_t_dn = NaN` → displayed as `—`
- `n = 1`: `rank_norm[0] = 0.5`, so `↑t` or `↓t` shows `0.50`

---

### 5.2 — IWDS: Intensity-Weighted Direction Score (`flow`)

**Plain explanation**: Imagine giving each bar a vote of +1 (up) or -1 (down), but
weighting heavier (more intense) bars more. What is the net weighted score? A score of
+0.42 means 42% more of the total trading urgency was on buy-side bars. 0.0 is perfectly
neutral; +1.0 means every single trade happened during up-bars; -1.0 means every trade
happened during down-bars.

**Technical specification**:

```rust
let total_raw: f32 = bars.iter().map(|b| b.raw).sum();
let iwds = if total_raw > 0.0 {
    bars.iter()
        .map(|b| b.raw * if b.is_up { 1.0 } else { -1.0 })
        .sum::<f32>() / total_raw
} else {
    0.0
};
```

Expanded: `iwds = (Σ raw_i for up-bars − Σ raw_i for dn-bars) / Σ raw_i`

Range: `[-1.0, +1.0]`.

- `+1.0`: all trade intensity on up-bars (100% bull urgency)
- `-1.0`: all trade intensity on down-bars (100% bear urgency)
- `0.0`: equal intensity on both sides, or all bars have intensity 0.0

**Session baseline cancellation**: if all bars are in the same session and every bar's
intensity is scaled by a common factor α (e.g. the NY open multiplier), the IWDS is
unchanged: `α × sum_up / (α × sum_up + α × sum_dn) = sum_up / (sum_up + sum_dn)`.
Cross-session comparisons of IWDS are therefore meaningful.

**Relation to per-bar OFI**: the per-bar `OdbMicrostructure.ofi` field is
`(buy_vol − sell_vol) / total_vol` — order-flow imbalance at the trade level within
a single bar. IWDS is the analogous computation at the bar level across a selection:
it aggregates the directional urgency of bars themselves rather than of individual trades
within a bar.

**Literature**: the IWDS formulation is analogous to the order-flow imbalance metric
in Cont, Kukanov & Stoikov (2014), aggregated at bar granularity.

**Display**: `[██████░░░░]  flow: +0.42` — the ASCII bar visualizes the IWDS value
(see §9 for details), followed by the numeric value with sign.

---

### 5.3 — Mann-Whitney AUC: `P(↑>↓)`

**Plain explanation**: If you randomly picked one up-bar and one down-bar from your
selection and compared their trade intensities, what is the probability that the up-bar
would have higher intensity? 73% means up-bars won 73% of head-to-head matchups.
50% means no directional edge at all — up and down bars are equally intense.
This is the most outlier-resistant of all the metrics: a single frenzied 500 t/s bar
counts as rank N, not as 500 times more weight.

**Technical specification**:

The Mann-Whitney U statistic is computed from the `order` sorted index that was already
computed for rank normalization — no second sort is needed:

```rust
let auc: f32 = if n_up > 0 && n_dn > 0 {
    // Sum 1-indexed ranks of up-bars in the sorted order
    let r_up: f32 = order.iter().enumerate()
        .filter(|(_, orig)| bars[**orig].is_up)
        .map(|(rank_0, _)| rank_0 as f32 + 1.0)
        .sum();
    let u_up = r_up - n_up as f32 * (n_up as f32 + 1.0) / 2.0;
    u_up / (n_up as f32 * n_dn as f32)
} else {
    f32::NAN
};
```

The rank-sum `r_up` is the sum of 1-indexed ranks of all up-bars in the globally sorted
order. `u_up = r_up − n_up(n_up+1)/2` is the Mann-Whitney U statistic. Dividing by
`n_up × n_dn` normalizes to the AUC interpretation: the probability that a randomly
chosen up-bar has higher intensity than a randomly chosen down-bar.

**Properties**:

- Range: `[0.0, 1.0]`. 0.5 = no directional edge.
- `AUC = 1.0`: every up-bar was more intense than every down-bar.
- `AUC = 0.0`: every down-bar was more intense than every up-bar.
- **Outlier resistant**: extreme values contribute only their rank, not their magnitude.
  A 500 t/s bar counts as rank N whether it's 200 t/s or 2000 t/s above the next bar.
- **Distribution-free**: no assumption about normality or any other distribution.
  Trade intensity is right-skewed (skewness ≈ 322 on BPR25); rank-based tests are
  robust to this.

**Complexity**: O(N log N) for the initial sort, then O(N) for the rank-sum. No
additional sort pass because `order` is shared with rank normalization.

**Literature**: Mann & Whitney (1947). The AUC interpretation of the U statistic
is described in Hanley & McNeil (1982).

**Edge cases**:

- `n_up = 0` or `n_dn = 0`: `auc = f32::NAN` → displayed as `—`

**Display format**: `P(↑>↓): 73%` (formatted as `pct_s(auc)` = `format!("{:.0}%", v * 100.0)`)

---

### 5.4 — Log₂ Ratio: `log₂(↑/↓)`

**Plain explanation**: How many doublings of intensity advantage do up-bars have over
down-bars, in terms of raw trades per second? +1.0 means buyers were exactly 2× more
intense. -1.0 means sellers were 2× more intense. 0.0 means equal. +2.0 would mean
buyers were 4× more intense. The symmetric scale makes it easy to compare: +1.5 bears
is the same "distance from neutral" as +1.5 bulls.

**Technical specification**:

```rust
let log2_ratio = if mean_raw_up > 0.0 && mean_raw_dn > 0.0 {
    (mean_raw_up / mean_raw_dn).log2()
} else {
    f32::NAN
};
```

where `mean_raw_up` and `mean_raw_dn` are the arithmetic means of raw `trade_intensity`
(in t/s) for each direction's bars.

**Properties**:

- Range: `(-∞, +∞)`. Typical observed range: approximately `[-3, +3]`.
- Symmetric: `log₂(2/1) = +1.0` and `log₂(1/2) = -1.0` have equal magnitude.
  Compare to raw ratio: 2.0 vs 0.5 appear very different in scale.
- **Session baseline cancels in the ratio**: if all bars' intensities are α× higher
  (e.g. during NY open), `α × mean_up / α × mean_dn = mean_up / mean_dn`.
  The log₂ ratio is therefore session-independent.
- Uses raw means (not rank-normalized), allowing the metric to reflect true magnitude
  differences, not just rank order.

**Edge cases**:

- `mean_raw_up = 0.0` or `mean_raw_dn = 0.0` (bars with no microstructure data):
  `log2_ratio = f32::NAN` → displayed as `—`

**Display format**: `log₂(↑/↓): +0.74` (formatted as `lg2_s(v)` = `format!("{v:+.2}")`)

---

### 5.5 — Conviction Ratio (`conv`)

**Plain explanation**: Is the "winning direction" (the one with more bars) also the
more urgently traded direction? Conviction > 1.0 means yes — the majority direction
was also more intense, suggesting the move has real urgency behind it. Conviction < 1.0
is an early exhaustion warning — the majority direction actually had _lower_ intensity
than the minority, meaning the market is moving more bars in one direction but the
urgency is loading into the other side.

**Technical specification**:

```rust
let dominant_up = n_up >= n_dn;
let conviction = if dominant_up {
    if !mean_t_dn.is_nan() && mean_t_dn > 0.0 { mean_t_up / mean_t_dn } else { f32::NAN }
} else {
    if !mean_t_up.is_nan() && mean_t_up > 0.0 { mean_t_dn / mean_t_up } else { f32::NAN }
};
```

`conviction` is always `mean_t(dominant_direction) / mean_t(minority_direction)`,
using rank-normalized t values.

**Interpretation**:

- `conv > 1.0`: dominant direction is more intense. "Trend has intensity fuel."
- `conv = 1.0`: both directions equally intense.
- `conv < 1.0`: minority direction is more intense. "Intensity loading against trend."
  Combined with `BULL ABSORPTION` or `BEAR ABSORPTION` regime, this is a classic
  exhaustion / divergence signal.
- `dominant_up = true` if `n_up >= n_dn` (ties go to up).

**Why rank-normalized?**: uses `mean_t_up` / `mean_t_dn` (rank-normalized) rather than
`mean_raw_up` / `mean_raw_dn` (absolute), making the conviction ratio session-independent.
A conviction of 1.85 means the same structural relationship at 2 AM and 9:30 AM.

**Edge cases**:

- Only one direction present: `conviction = f32::NAN` → displayed as `—`
- `mean_t_dn = 0.0` (all down-bars have zero intensity): `conviction = f32::NAN`

**Display format**: `conv: 1.85×` (formatted as `con_s(v)` = `format!("{v:.2}×")`)

---

### 5.6 — Absorption (`absorp`)

**Plain explanation**: How hard was the losing side fighting? This metric shows the
average intensity rank of the _minority-direction_ bars — the side that closed fewer
bars. High absorption (> 0.6) means the losing side was urgently contested — it did
not give up easily. This pattern is associated with absorption of dominant flow by
large participants (icebergs, stop-hunt defense). Low absorption (< 0.3) means the
minority bars were lazy — the market moved through a vacuum with little opposition,
which often means the trend is sustainable.

**Technical specification**:

```rust
let absorption = if dominant_up { mean_t_dn } else { mean_t_up };
```

`absorption` is simply `mean_t` of the minority-direction bars. It is the rank-normalized
mean intensity of bars that closed against the dominant trend.

**Important nuance**: absorption is the mean_t of whichever direction has _fewer bars_,
not necessarily the down-bars. In a selection with more down-bars than up-bars, absorption
shows the mean_t of the up-bars (the minority).

**Interpretation by threshold**:

- `> 0.6`: high absorption — minority bars were in the top 40% of intensity.
  Suggests active defense, absorption of flow, iceberg orders, or stop-hunting.
- `0.3 – 0.6`: moderate absorption — mixed signal.
- `< 0.3`: low absorption — minority bars drifted. The move has little opposition;
  the trend may continue.

**Edge cases**:

- Only one direction present: `absorption = f32::NAN` (because the relevant `mean_t` is NaN)
  → displayed as `—`

**Display format**: `absorp: 0.41` (formatted as `t_s(v)` = `format!("{v:.2}")`)

---

### 5.7 — Climax Concentration (`◈ climax`)

**Plain explanation**: Of the most frenzied bars in your selection — the top 25% by
intensity within the selection — what fraction went in the up direction? 80% ↑ means
that buyers dominated the most urgent moments of this window. This is the "climax"
metric because concentrating the most intense action in one direction is a classic
climactic pattern: the move has exhausted its most urgent participants, and reversals
often follow. An even split (near 50%) means the most frenzied moments were balanced.

**Technical specification**:

```rust
let (top_n, top_up) = bars.iter().enumerate()
    .filter(|(i, _)| rank_norm[*i] > 0.75)
    .fold((0_usize, 0_usize), |(t, u), (_, b)| {
        (t + 1, if b.is_up { u + 1 } else { u })
    });
let climax_up_frac = if top_n > 0 { top_up as f32 / top_n as f32 } else { f32::NAN };
```

The top quartile is defined as `rank_norm[i] > 0.75` (strict inequality — the 75th
percentile threshold, within-selection). `climax_up_frac` is the fraction of those
top-quartile bars that are up-bars.

**Display logic**: the displayed direction always shows the dominant direction in the tail:

```rust
let (frac, dir) = if climax_up_frac >= 0.5 {
    (climax_up_frac, "↑")
} else {
    (1.0 - climax_up_frac, "↓")
};
format!("◈ climax: {:.0}% {dir}  (of top-25% bars)", frac * 100.0)
```

So `climax_up_frac = 0.20` displays as `◈ climax: 80% ↓  (of top-25% bars)`.

**Regime thresholds**:

- `≥ 0.78` (78% of top-quartile bars are up): `BULL CLIMAX ◈`
- `≤ 0.22` (78% of top-quartile bars are down): `BEAR CLIMAX ◈`
- Otherwise: no climax regime triggered from this metric alone

**Why 75th percentile?**: the top quartile by within-selection rank gives a stable
"tail" sample. Using the top decile (90th) would leave too few bars in short selections.
Using the top half would dilute the signal into the median.

**Edge cases**:

- No bars with `rank_norm > 0.75`: can occur when n is very small (e.g. 2-3 bars
  with all having the same intensity, so all ranks are equal). Displays as:
  `◈ climax: — (no top-25% bars yet)`
- `n = 1`: `rank_norm[0] = 0.5`, so it does not exceed 0.75. Displays as `—`.

---

## 6. Regime Classification

After computing all seven metrics, a single `Regime` enum is determined. This is the
synthesized "verdict" that combines multiple metrics into a human-readable label.

### Priority Order

Regime classification uses a priority-ordered `if / else if` chain. **Climax is checked
first** — it overrides conviction signals because climax is a forward-looking reversal
warning that supersedes the current directional story.

```rust
enum Regime {
    BullConviction, BearConviction,
    BullAbsorption, BearAbsorption,
    BullClimax, BearClimax,
    Contested
}

let regime = if !climax_up_frac.is_nan() && climax_up_frac >= 0.78 {
    Regime::BullClimax
} else if !climax_up_frac.is_nan() && climax_up_frac <= 0.22 {
    Regime::BearClimax
} else if iwds > 0.15 && !auc.is_nan() && auc >= 0.60 {
    Regime::BullConviction
} else if iwds < -0.15 && !auc.is_nan() && auc <= 0.40 {
    Regime::BearConviction
} else if iwds > 0.15 && !auc.is_nan() && auc < 0.50 {
    Regime::BullAbsorption
} else if iwds < -0.15 && !auc.is_nan() && auc > 0.50 {
    Regime::BearAbsorption
} else {
    Regime::Contested
}
```

### Regime Table

| Regime            | Condition                   | Color                     | Border         | Microstructure Meaning                                                                        |
| ----------------- | --------------------------- | ------------------------- | -------------- | --------------------------------------------------------------------------------------------- |
| `BULL CONVICTION` | IWDS > 0.15 AND AUC ≥ 0.60  | `palette.success` (green) | green (0.65a)  | Both metrics agree: up-bars have more urgency AND more head-to-head wins. Strong bull signal. |
| `BEAR CONVICTION` | IWDS < -0.15 AND AUC ≤ 0.40 | `palette.danger` (red)    | red (0.65a)    | Both metrics agree: down-bars dominate intensity. Strong bear signal.                         |
| `BULL ABSORPTION` | IWDS > 0.15 AND AUC < 0.50  | orange (0.95,0.55,0.10)   | orange(0.65a)  | IWDS says net bullish, but AUC says dn-bars win more matchups. Divergence — suspect the move. |
| `BEAR ABSORPTION` | IWDS < -0.15 AND AUC > 0.50 | orange (0.95,0.55,0.10)   | orange(0.65a)  | IWDS says net bearish, but AUC says up-bars win more matchups. Divergence — suspect the move. |
| `BULL CLIMAX ◈`   | climax_up_frac ≥ 0.78       | magenta (0.90,0.25,0.80)  | magenta(0.65a) | Top-quartile intensity concentrated in up-bars. Classic buying exhaustion/climax setup.       |
| `BEAR CLIMAX ◈`   | climax_up_frac ≤ 0.22       | magenta (0.90,0.25,0.80)  | magenta(0.65a) | Top-quartile intensity concentrated in down-bars. Selling climax/exhaustion setup.            |
| `CONTESTED`       | All others                  | dim grey (0.50,0.50,0.50) | none           | Mixed or weak signals. No clear intensity edge from this selection.                           |

### Regime Deep-Dive: Conviction

`BULL CONVICTION` requires IWDS > 0.15 (net intensity loading onto up-bars) **and**
AUC ≥ 0.60 (up-bars win at least 60% of head-to-head matchups). Both metrics must
agree. This double confirmation reduces false positives: IWDS can be gamed by a single
massive bar, but AUC measures rank-order consistency across all bars.

A conviction reading does _not_ predict the future — it describes what just happened.
A selection of strong conviction bars during a trend could be mid-trend (consistent
with continuation) or at the end of a leg (the strongest bars often mark exhaustion).
Use conviction in conjunction with the chart pattern.

### Regime Deep-Dive: Absorption

`BULL ABSORPTION` means: despite more net intensity on up-bars (IWDS > 0.15), a random
down-bar is actually _more likely_ to beat a random up-bar in intensity (AUC < 0.50).
This is the hallmark of absorption: the dominant direction (by bar count or by total
intensity) is facing concentrated resistance from the minority.

In orderbook microstructure terms: a large participant may be absorbing the up-side
flow — taking the other side of every up-bar with equal or greater urgency, and closing
small down-bars in the process. The market appears to be going up but the intensity
tells a different story.

### Regime Deep-Dive: Climax

`BULL CLIMAX ◈` fires when 78% or more of the top-quartile-intensity bars within the
selection are up-bars. This means the most frenzied moments of the window were dominated
by buying. Climax takes priority over conviction because even a strongly "convincing"
move can be at its exhaustion point if all the urgency is in one direction at the tail.

The `◈` symbol (used in the regime label and the climax line) is a diamond-shaped
indicator borrowing from Japanese candlestick tradition where concentrated "doji" activity
at a peak signals reversal potential.

### Regime Deep-Dive: Contested

`CONTESTED` is the null hypothesis — no strong reading from intensity alone. This covers
selections where:

- IWDS is between -0.15 and +0.15 (nearly neutral)
- AUC was NaN (only one direction present)
- AUC is between 0.40 and 0.60 (no directional intensity edge)
- The climax fraction was between 0.22 and 0.78

A `CONTESTED` selection does not have a colored border — the box is drawn without a
stroke, making it visually quieter. This is a deliberate design choice: when there is
no signal, the UI should not shout.

---

## 7. ASCII Intensity Bar

The intensity bar is a 10-character ASCII visualization of IWDS:

```rust
let fill = ((5.0 + iwds * 5.0).round() as usize).clamp(0, 10);
let bar_str = format!("[{}{}]", "█".repeat(fill), "░".repeat(10 - fill));
```

| IWDS | `fill` | Display        |
| ---- | ------ | -------------- |
| +1.0 | 10     | `[██████████]` |
| +0.6 | 8      | `[████████░░]` |
| +0.2 | 6      | `[██████░░░░]` |
| 0.0  | 5      | `[█████░░░░░]` |
| -0.2 | 4      | `[████░░░░░░]` |
| -0.6 | 2      | `[██░░░░░░░░]` |
| -1.0 | 0      | `[░░░░░░░░░░]` |

The center (5 filled blocks) corresponds to IWDS = 0. Left-heavy → sell dominant.
Right-heavy → buy dominant. This gives an immediate visual before reading any number.
It appears on the same line as the IWDS numeric value: `[██████░░░░]  flow: +0.22`.

---

## 8. Plain-English Caption

Below the regime label, a one-line human-readable caption provides a direct verbal
summary:

```rust
let caption = if !mean_raw_up.is_nan() && !mean_raw_dn.is_nan()
    && mean_raw_dn > 0.0 && mean_raw_up > 0.0
{
    let (dom, min_raw) = if mean_raw_up >= mean_raw_dn {
        (mean_raw_up, mean_raw_dn)
    } else {
        (mean_raw_dn, mean_raw_up)
    };
    let side = if mean_raw_up >= mean_raw_dn { "buyers" } else { "sellers" };
    format!("{side} {:.1}× more urgent", dom / min_raw)
} else if n_dn == 0 {
    "all bars bullish — no dn comparison".to_string()
} else {
    "all bars bearish — no up comparison".to_string()
};
```

Examples:

- `"buyers 1.8× more urgent"` — mean raw intensity of up-bars is 1.8× that of down-bars
- `"sellers 2.3× more urgent"` — down-bars averaged 2.3× more t/s than up-bars
- `"all bars bullish — no dn comparison"` — the selection contains only up-bars
- `"all bars bearish — no up comparison"` — the selection contains only down-bars

The caption uses raw means (not rank-normalized) so the multiple is in the same units as
the t/s values shown in `↑t` / `↓t` lines. Color: `dim_white` (0.75, 0.75, 0.75, 0.65).

---

## 9. Visual Design and Layout

### Color Palette

| Name        | RGBA                             | Used For                                          |
| ----------- | -------------------------------- | ------------------------------------------------- |
| `amber`     | (0.85, 0.65, 0.15, 1.00)         | ASCII bar + IWDS numeric, primary intensity color |
| `amber_dim` | (0.85, 0.65, 0.15, 0.55)         | `↑t`/`↓t` lines, P(↑>↓), log₂, conv, absorp       |
| `orange`    | (0.95, 0.55, 0.10, 1.00)         | Absorption regime border/label; climax line color |
| `magenta`   | (0.90, 0.25, 0.80, 1.00)         | Climax regime border/label                        |
| `dim`       | (0.50, 0.50, 0.50, 0.65)         | Separator line; CONTESTED regime label            |
| `dim_white` | (0.75, 0.75, 0.75, 0.65)         | Plain-English caption                             |
| `neutral`   | `palette.background.strong.text` | Bar count line (`N bars`)                         |
| `success`   | `palette.success.base.color`     | Up-bar count; BULL CONVICTION label/border        |
| `danger`    | `palette.danger.base.color`      | Down-bar count; BEAR CONVICTION label/border      |

### Box Geometry

```
Box width:  box_w = 215.0 px
Box x:      frame.width() / 2.0 - box_w / 2.0  (horizontally centered)
Box y:      10.0 px from top
Padding:    x - 8.0 (left), y - 4.0 (top), box_w + 16.0 (total width)
Background: (0.07, 0.07, 0.07, 0.92)  — near-black, near-opaque
Border:     1.5 px stroke, border_col at alpha 0.65 (only when regime != CONTESTED)
```

Height is computed dynamically from line heights:

```rust
let lh_main = ts + 5.0;  // ts=13.0 → 18.0 px per main-size line
let lh_sm   = sm + 4.0;  // sm=11.0 → 15.0 px per small-size line
let total_h: f32 = lines.iter()
    .map(|(_, _, sz)| if (*sz - ts).abs() < 0.5 { lh_main } else { lh_sm })
    .sum::<f32>() + 12.0;  // +12.0 for top/bottom padding
```

### The 12-Line Display (Annotated)

| Line # | Font Size | Content Example                      | Color        | Section                          |
| ------ | --------- | ------------------------------------ | ------------ | -------------------------------- |
| 1      | 13 px     | `15 bars`                            | neutral      | Bar count (distance)             |
| 2      | 13 px     | `↑ 9  (60%)`                         | success      | Up-bar count + %                 |
| 3      | 13 px     | `↓ 6  (40%)`                         | danger       | Down-bar count + %               |
| 4      | 11 px     | `─────────────────────────────`      | dim          | Separator                        |
| 5      | 11 px     | `↑t 0.68  ↑ 124 t/s`                 | amber_dim    | Up intensity (§5.1)              |
| 6      | 11 px     | `↓t 0.41  ↓ 82 t/s`                  | amber_dim    | Down intensity (§5.1)            |
| 7      | 13 px     | `[██████░░░░]  flow: +0.22`          | amber        | ASCII bar + IWDS (§5.2)          |
| 8      | 13 px     | `BULL CONVICTION`                    | regime_color | Regime label (§6)                |
| 9      | 11 px     | `buyers 1.8× more urgent`            | dim_white    | Plain-English caption (§8)       |
| 10     | 11 px     | `P(↑>↓): 73%   log₂(↑/↓): +0.74`     | amber_dim    | AUC + log₂ ratio (§5.3/4)        |
| 11     | 11 px     | `conv: 1.85×   absorp: 0.41`         | amber_dim    | Conviction + Absorption (§5.5/6) |
| 12     | 11 px     | `◈ climax: 80% ↑  (of top-25% bars)` | climax_color | Climax concentration (§5.7)      |

Line 4 is a Unicode box-drawing separator. Lines 5-6 show the rank-normalized t values
and raw t/s side by side. Line 7 is the widest element (ASCII bar + numeric). Lines 10-11
are the dense data rows. Line 12 uses `orange` color when the regime has a signal
(border is showing), or `amber_dim` when the regime is CONTESTED — so climax context
is always visually prominent.

---

## 10. Algorithm Complexity

The function processes N bars (where N = `distance + 1` = the selection size):

| Step                     | Complexity | Notes                                         |
| ------------------------ | ---------- | --------------------------------------------- |
| Collect `bars` Vec       | O(N)       | One pass over `tick_aggr.datapoints`          |
| Sort `order` (unstable)  | O(N log N) | Single sort pass, reused by rank_norm AND AUC |
| Rank normalization       | O(N)       | Linear scan over `order` with tie grouping    |
| Per-direction aggregates | O(N)       | Single fold over `bars.iter().enumerate()`    |
| IWDS                     | O(N)       | Two sums over `bars`                          |
| AUC rank-sum             | O(N)       | Single filtered iteration over `order`        |
| Log₂ ratio               | O(1)       | Computed from already-computed means          |
| Conviction + Absorption  | O(1)       | Computed from already-computed means          |
| Climax filter            | O(N)       | Single scan with `rank_norm[i] > 0.75` filter |
| Rendering                | O(1)       | Fixed 12 lines, fixed box geometry            |

**Total: O(N log N)** dominated by the sort.

N is typically 5–200 bars for practical selections; maximum possible is the total loaded
bar count (~13,000–20,000), but selecting thousands of bars is not a typical use case.
At N=200, the sort is negligible (<1 µs on aarch64).

The key optimization is the **single sort pass**: `order` is sorted once and used by
both the rank normalization algorithm and the AUC calculation. Without this sharing,
a naive implementation would require two sort passes, or keeping a separate sorted
structure for AUC computation.

---

## 11. Worked Example: 5-Bar Selection

Suppose a user selects 5 consecutive bars with the following properties:

| Bar | Direction | Raw intensity (t/s) |
| --- | --------- | ------------------- |
| 0   | Up        | 120                 |
| 1   | Up        | 45                  |
| 2   | Down      | 180                 |
| 3   | Up        | 30                  |
| 4   | Down      | 90                  |

So `n = 5`, `n_up = 3`, `n_dn = 2`.

### Step 1: Sort

Sorting by raw intensity ascending: `[30, 45, 90, 120, 180]`
→ `order = [3, 1, 4, 0, 2]` (bar indices in sorted order)

### Step 2: Rank Normalization

n = 5, so rank positions are divided by (n - 1) = 4:

| Sorted pos | Bar idx | Raw | rank_norm       |
| ---------- | ------- | --- | --------------- |
| 0          | 3       | 30  | 0.0 / 4 = 0.000 |
| 1          | 1       | 45  | 1.0 / 4 = 0.250 |
| 2          | 4       | 90  | 2.0 / 4 = 0.500 |
| 3          | 0       | 120 | 3.0 / 4 = 0.750 |
| 4          | 2       | 180 | 4.0 / 4 = 1.000 |

No ties in this example. `rank_norm = [0.750, 0.250, 1.000, 0.000, 0.500]`
(indexed by bar, not sorted position).

### Step 3: Per-Direction Aggregates

Up-bars: bars 0, 1, 3 → rank_norm = [0.750, 0.250, 0.000], raw = [120, 45, 30]

- `sum_t_up = 0.750 + 0.250 + 0.000 = 1.000` → `mean_t_up = 1.000 / 3 = 0.333`
- `sum_raw_up = 120 + 45 + 30 = 195` → `mean_raw_up = 65.0 t/s`

Down-bars: bars 2, 4 → rank_norm = [1.000, 0.500], raw = [180, 90]

- `sum_t_dn = 1.000 + 0.500 = 1.500` → `mean_t_dn = 1.500 / 2 = 0.750`
- `sum_raw_dn = 180 + 90 = 270` → `mean_raw_dn = 135.0 t/s`

### Step 4: IWDS

`total_raw = 120 + 45 + 180 + 30 + 90 = 465`

Signed sum: `120×(+1) + 45×(+1) + 180×(-1) + 30×(+1) + 90×(-1)`
= `120 + 45 - 180 + 30 - 90 = -75`

`iwds = -75 / 465 = -0.161`

→ Slightly net bearish by intensity. IWDS is just below the -0.15 threshold.

### Step 5: Mann-Whitney AUC

1-indexed ranks of up-bars in `order = [3, 1, 4, 0, 2]`:

- Bar 3: position 0 → rank 1
- Bar 1: position 1 → rank 2
- Bar 0: position 3 → rank 4

`r_up = 1 + 2 + 4 = 7`
`u_up = 7 - 3 × (3 + 1) / 2 = 7 - 6 = 1`
`auc = 1 / (3 × 2) = 0.167`

Interpretation: up-bars won only 16.7% of head-to-head matchups. Down-bars were
consistently more intense in direct comparison.

### Step 6: Log₂ Ratio

`log₂(65.0 / 135.0) = log₂(0.481) = -1.056`

Interpretation: down-bars averaged about 2× more intensity than up-bars (more precisely,
2^1.056 ≈ 2.08×).

### Step 7: Conviction and Absorption

`dominant_up = (n_up >= n_dn) = (3 >= 2) = true` (more up-bars by count)

`conviction = mean_t_up / mean_t_dn = 0.333 / 0.750 = 0.444`

Interpretation: conviction < 1.0 — the majority direction (up) has _lower_ intensity
rank than the minority (down). Exhaustion signal.

`absorption = mean_t_dn = 0.750`

Interpretation: absorption is high (0.75 > 0.6). The down-bars — even though fewer —
were fighting hard.

### Step 8: Climax

Top quartile: bars where `rank_norm > 0.75`:

- Bar 2: rank_norm = 1.000 > 0.75 ✓ (is_up = false)

`top_n = 1`, `top_up = 0`
`climax_up_frac = 0 / 1 = 0.00`

The sole top-quartile bar was a down-bar. `climax_up_frac ≤ 0.22` → `BEAR CLIMAX ◈`

### Step 9: Regime Classification

1. `climax_up_frac = 0.00 ≤ 0.22` → **`BEAR CLIMAX ◈`** (fired first, climax takes priority)

### Step 10: ASCII Bar

`fill = round(5.0 + (-0.161) × 5.0) = round(5.0 - 0.805) = round(4.195) = 4`

`[████░░░░░░]  flow: -0.16`

### Final Display

```
5 bars
↑ 3  (60%)
↓ 2  (40%)
─────────────────────────────
↑t 0.33  ↑ 65 t/s
↓t 0.75  ↓ 135 t/s
[████░░░░░░]  flow: -0.16
BEAR CLIMAX ◈
sellers 2.1× more urgent
P(↑>↓): 17%   log₂(↑/↓): -1.06
conv: 0.44×   absorp: 0.75
◈ climax: 100% ↓  (of top-25% bars)
```

Box has a magenta border. The pattern: 3 up-bars, 2 down-bars, but the down-bars were
far more intense. The most frenzied bar was a down-bar. Despite a bar count majority,
the buying looks exhausted: low conviction (0.44), high seller absorption (0.75), and
the only top-quartile bar going down. Classic microstructure picture of a top with
absorption into the rally.

---

## 12. Integration Points in the Codebase

### Call Site

```rust
// src/chart/kline.rs — inside legend cache closure
if let (Some(anchor), Some(end)) = (state.anchor, state.end) {
    if let PlotData::TickBased(tick_aggr) = &self.data_source {
        draw_bar_selection_stats(frame, palette, tick_aggr, anchor, end);
    }
}
```

### Interaction with `BarSelectionState`

`BarSelectionState` is held in a `RefCell<BarSelectionState>` inside `KlineChart`. The
legend layer borrows the state immutably to read `anchor` and `end`, then calls
`draw_bar_selection_stats`. The selection highlight (the yellow brim handles and the
translucent region between them) is also drawn in the `crosshair` layer, not in `main`.

Both `legend` and `crosshair` layers are invalidated together on `CursorMoved` events
(via `clear_crosshair()` which calls both). The `main` (candle bodies) cache is never
touched during selection interaction.

### Sentinel Guard

`snap_x_to_index()` returns `u64::MAX` when the cursor is in the forming-bar zone.
Selection anchors and endpoints are stored as `usize` visual indices. If `anchor` or
`end` was derived from a `u64::MAX` snap position, the code ignores it (the forming
bar has no completed microstructure). The guard in `BarSelectionState::try_set_end()`
rejects `u64::MAX` as an endpoint.

---

## 13. Design Rationale and Limitations

### Why Seven Metrics?

Each metric answers a different question that the others cannot:

| Metric     | Question                                         | Blind Spots of the Others                           |
| ---------- | ------------------------------------------------ | --------------------------------------------------- |
| `↑t / ↓t`  | "How hot were each direction's bars, on scale?"  | Does not say who won head-to-head                   |
| IWDS       | "Where did the total urgency flow?"              | One outlier bar dominates; AUC is robust to this    |
| AUC        | "Did up-bars consistently beat dn-bars?"         | Says nothing about magnitude of the edge            |
| log₂       | "How large is the raw rate advantage?"           | Not session-independent in isolation (but ratio is) |
| Conviction | "Is the majority direction also the urgent one?" | Does not tell us who the majority is by count       |
| Absorption | "How hard was the minority fighting?"            | Does not say who the minority is                    |
| Climax     | "Where was the most frenzied action?"            | Does not describe the overall distribution          |

Together they form a complete microstructure picture that no single metric can achieve.

### Limitations

1. **No statistical significance testing**: with N=5–20 bars, the Mann-Whitney p-value
   will rarely cross conventional thresholds. The metrics are descriptive, not inferential.
   Do not interpret `BULL CONVICTION` as "statistically proven bull signal".

2. **Bars with missing microstructure**: bars without microstructure data get `raw = 0.0`,
   which anchors them to rank_norm = 0.0 in the sort. If many bars lack microstructure
   (e.g. selecting across a data gap), the metrics will be misleading. Check the
   `↑t / ↓t` raw t/s values: if they show `0 t/s` for many bars, the data is sparse.

3. **Direction definition**: `is_up = close >= open` means a doji (open == close) counts
   as up. In selections with many dojis, the up/down split may not reflect what a trader
   would call "bullish" or "bearish". ODB bars rarely produce dojis due to the deviation
   threshold mechanic, but it is possible.

4. **No temporal ordering**: the metrics treat the selection as an unordered bag of bars.
   A selection where the first half is bear-conviction and second half is bull-conviction
   would show `CONTESTED`. Use the visual chart pattern alongside the overlay.

5. **Short selections (n ≤ 3)**: climax can only trigger if at least one bar exceeds the
   75th percentile rank. With n=1, `rank_norm = [0.5]`, so climax always shows `—`.
   With n=2, one bar will be at 0.0 and one at 1.0; the 1.0 bar exceeds the 0.75 threshold,
   so climax is based on just one bar. Interpret with caution for very short selections.

---

## 14. Relationship to the Trade Intensity Heatmap Indicator

The bar selection overlay and the `TradeIntensityHeatmap` indicator read from the same
underlying data — `OdbMicrostructure.trade_intensity: f32` stored in
`TickAccumulation.microstructure` — but use it in fundamentally different ways:

| Aspect              | Trade Intensity Heatmap                               | Bar Selection Overlay                                  |
| ------------------- | ----------------------------------------------------- | ------------------------------------------------------ |
| Reference window    | Rolling lookback (100–7000 bars), session-aware       | Within-selection only (completely session-independent) |
| Normalization       | Log-quantile percentile → bin 1..=K                   | Linear rank normalization → [0, 1] continuous          |
| Purpose             | Color individual bars relative to recent history      | Compute statistics across a user-defined range         |
| State               | Stored `HeatmapPoint` per bar, persisted              | Computed fresh each render from raw microstructure     |
| Visualization       | Candle body and wick color                            | Stats overlay box (top-center of chart)                |
| Session sensitivity | Affected by session (rolling window crosses sessions) | Session-independent (normalizes within selection)      |

The heatmap's `t` value (`(bin - 1) / (k_actual - 1)`) and the overlay's `rank_norm[i]`
are both normalized intensity scores in `[0, 1]`, but they are computed against different
reference populations and are not interchangeable.

A bar that appears green (cold, K3) on the heatmap might still have the highest
`rank_norm` in a short selection of similarly cold bars — and vice versa, a hot-colored
bar might be rank_norm = 0.5 in a selection that includes even hotter bars.

---

## 15. Quick Reference Card

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  Bar Selection Intensity Metrics — Quick Reference                          │
│                                                                             │
│  TRIGGER      Shift+Click (anchor) → Shift+Click (end)                     │
│  DATA         OdbMicrostructure.trade_intensity (t/s) per bar               │
│  LAYER        legend cache (screen-space, cleared on cursor move)           │
│  COMPLEXITY   O(N log N) sort + O(N) all metrics                            │
│                                                                             │
│  ── METRICS ────────────────────────────────────────────────────────────── │
│  ↑t / ↓t      rank_norm mean per direction, within-selection [0,1]         │
│               + raw t/s mean in parentheses                                │
│  flow (IWDS)  Σ(raw × ±1) / Σ(raw) ∈ [-1,+1]                             │
│  P(↑>↓)       Mann-Whitney AUC ∈ [0,1]  (0.5 = no edge)                  │
│  log₂(↑/↓)   log₂(mean_raw_up / mean_raw_dn)  (0 = equal)                │
│  conv         mean_t(dominant) / mean_t(minority)  (>1 = fuel, <1 = exhaust)│
│  absorp       mean_t of minority direction  (>0.6 = contested, <0.3 = vacuum)│
│  ◈ climax     fraction of top-25%-rank bars that are up                    │
│                                                                             │
│  ── REGIMES ─────────────────────────────────────────────────────────────  │
│  BULL CONVICTION  IWDS>0.15 AND AUC≥0.60   green  — both agree, bullish   │
│  BEAR CONVICTION  IWDS<-0.15 AND AUC≤0.40  red    — both agree, bearish   │
│  BULL ABSORPTION  IWDS>0.15 AND AUC<0.50   orange — divergence, suspect   │
│  BEAR ABSORPTION  IWDS<-0.15 AND AUC>0.50  orange — divergence, suspect   │
│  BULL CLIMAX ◈    climax_up_frac≥0.78      magenta — buying exhaustion?    │
│  BEAR CLIMAX ◈    climax_up_frac≤0.22      magenta — selling exhaustion?   │
│  CONTESTED        all others               grey   — no clear edge          │
│                                                                             │
│  ── PRIORITY ─────────────────────────────────────────────────────────── │
│  Climax checked FIRST (overrides conviction)                               │
│  Conviction checked before Absorption                                      │
│  Contested is the fallthrough                                               │
└─────────────────────────────────────────────────────────────────────────────┘
```
