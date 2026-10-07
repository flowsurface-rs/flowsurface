# Indicator language

A `.ind` file defines an indicator's inputs, calculations, and presentation.
See [installation](../indicators/README.md) and the [CLI](README.md#cli) for
file naming, collections, and validation.

```text
language(1)
indicator("Moving average", id = "community.example", sourceable = true)
format(precision = "base_ticker_min_tick")
period = input.int(20, "Period", min = 1, max = 200)
average = ta.sma(chart.close, period)
plot(average, id = "average", title = "Average")
```

## Execution and syntax

The host evaluates loaded closed bars in order, committing state after each
bar. The open bar is a preview from the last committed state. Missing data and
non-finite numeric results become `na`. Execution, heap, collection, and output
budgets are enforced; collection nesting is limited to 256 levels. Runtime
failures quarantine the indicator.

- `=` binds a local; `state` or `var` creates persistent state; `:=` updates it.
  Types remain compatible across updates: integers and floats can widen to
  float, and `na` is compatible with any type.
- A state initializer retries until it returns a non-`na` value. An explicit
  `:= na` update stays authoritative.
- `if`, `switch`, bounded `for`/`while`, numeric non-recursive functions,
  tuples, arrays, records, destructuring, and named arguments are supported.
  Function parameters, defaults, and returns must be numeric or `na`.
- Integer `+`, `-`, `*`, and `%` use checked 64-bit arithmetic; `/` returns a
  float. Mixed arithmetic promotes to float. Integer overflow and division or
  remainder by zero return `na`. Integer comparisons remain exact, including
  comparisons with floats. Integer/time input defaults require integer literals.
- Arithmetic requires numbers; ordering accepts numbers or times; logical
  operators require booleans. Equality involving `na` returns `na`: use
  `na(value)` to test missingness. An `na` condition does not match a switch
  branch. Result branches in `select`, `coalesce`, and `switch` must have
  compatible types.
- `series[n]` reads `n` bars back from an immutable retained series. Bind a
  calculation to an immutable top-level local before indexing it; mutable
  locals, function parameters, and direct function results cannot retain
  history. Offset zero needs no retention. Dynamic offsets require an explicit
  `max_bars_back`.
- Inputs and ordinary output/presentation declarations must be top-level.
  `region` and `region_remove` may occur in bounded control-flow blocks outside
  functions. At most one `display` and one `format` are allowed, before outputs.

Records support dot access and `record.get`:

```text
zone = { upper: chart.high, lower: chart.low }
top = zone.upper
```

User-record fields have dynamic type `Any`; absent fields or non-record values
return `na`. Built-in chart fields are checked at compile time.

## Host data

Every indicator reads the base ticker's chart frame. Comparison tickers are
presentation-only.

| Fields | Meaning |
| --- | --- |
| `chart.time`, `bar_index` | Bar timestamp and zero-based index |
| `chart.open`, `chart.high`, `chart.low`, `chart.close`, `chart.volume` | OHLCV |
| `chart.hl2`, `chart.hlc3`, `chart.ohlc4`, `chart.hlcc4` | Derived price averages |
| `chart.directional_volume_available`, `chart.buy_qty`, `chart.sell_qty` | Candle directional volume |
| `chart.trade.available`, `chart.trade.buy_qty`, `chart.trade.sell_qty` | Trade coverage and quantities |
| `chart.trade.buy_count`, `chart.trade.sell_count`, `chart.trade.count` | Trade counts |
| `open_interest` | Latest finite sample at or before the bar timestamp; `na` before the first sample |
| `instrument.min_tick` | Minimum price increment; simple metadata, `na` if unavailable |
| `barstate.isfirst`, `barstate.islast`, `barstate.isconfirmed`, `barstate.isrealtime` | Evaluation flags |

Candle directional volume does not substitute for missing trade aggregates.
Raw trade arrays are unavailable. Plot offsets change presentation only.

There is no arbitrary I/O, remote symbol/timeframe access, strategy or alert
API, `varip`, correction replay, map type, or mutable drawing/table handle.

## Inputs and source composition

```text
length = input.int(14, "Length", min = 1, max = 200, step = 1)
multiplier = input.float(2.0, "Multiplier", min = 0.1, max = 10.0)
enabled = input.bool(true, "Enabled")
method = input.string("Sma", "Method", options = ["Sma", "Ema"])
anchor = input.time(0, "Anchor")
color = input.color()
```

Defaults and labels occupy the first two arguments (or `defval` and `title`).
Metadata is literal: `min`, `max`, `step`, `group`, `inline`, `tooltip`,
`visible_when`, and `requires`. String `options` must be a literal string array.
Configured values must match the type, bounds, and options. Color defaults to
white; `int` and `time` settings preserve signed 64-bit values.

`visible_when = "enabled"` references a boolean input. `requires` names
`klines`, `directional_volume`, `trades`, or `open_interest`; inputs and styles
with that requirement hide when coverage is unavailable. Empty coverage is
pending. Hidden settings remain persisted.

A source input can consume chart data or another indicator's numeric output:

```text
language(1)
indicator("Smoothed source", id = "community.smoothed")
display(attachment = "overlay")
format(precision = "base_ticker_min_tick")
source = input.source(chart.close, "Source")
plot(ta.sma(source, 20), id = "average")
```

`input.source` defaults must name an OHLC/derived-price field, `chart.volume`,
or `open_interest`. `allow_indicator` defaults to `true`; such inputs require
view `plot`, attachment `overlay` or `panel`, and a visible numeric plot layer.
Use `allow_indicator = false` for a base-only input in any view.

Bindings consume same-index visible numeric plots of active indicators with
`sourceable = true`, within the same Kline pane and base series/timeframe.
Invalid or unavailable indicator bindings yield `na`, without falling back.
Unbound inputs use their declared default or a host-selected base price source
(Open, High, Low, Close, HL2, HLC3, OHLC4, HLCC4). Source inputs support history
and numeric TA functions. Multiple inputs work; one-click attachment requires
exactly one indicator-enabled input.

## Metadata

Arguments below are compile-time literals. Optional settings can be omitted.

### `indicator`

`indicator("Title", id = "namespace.name")`

| Option | Default / rule |
| --- | --- |
| `sourceable` | `false`; permits visible numeric plots as dependency sources |
| `optional_trades` | `false`; makes inferred directional-volume requirements optional, not `chart.trade.*` requirements |
| `max_bars_back` | `5000`; retained-series history, positive and at most `50000` |
| `bases` | Unrestricted; non-empty list of `"time"` and/or `"tick"` |
| `author`, `author_url` | Optional pair; URL must be absolute HTTP(S) |
| `description`, `source` | Optional strings; source must be absolute HTTP(S) |

Data requirements are inferred separately from `bases`. `ta.avwap` requires
complete anchor history and is unavailable on the host's live-only tick charts.

### `display` and `format`

`display(view = "plot", attachment = "panel")`

Views: `plot` (default), `primary_overlay`, `table`. Attachment `overlay` shares
the source panel's scale; `panel` keeps standalone placement and scale.
`panel_title` and `layer_name` default to the indicator title. Attachment affects
placement, not dependency evaluation.

`format(precision = "fixed_step:0.01")`

Numeric plot views require panel precision; table and primary-overlay views do
not. Panel precision controls axes, quantization, and crosshair values.

| Option | Values (default first where applicable) |
| --- | --- |
| `precision` | `base_ticker_min_tick`, `base_ticker_min_qty`, `fixed_step:<positive number>` |
| `axis_labels` | `compact`, `commas`, `abbreviated` |
| `header_labels` | `commas`, `compact`, `abbreviated` |
| `scale` | `fit_visible`, `absolute`, `fit_visible_include_zero`, `logarithmic`, `percent_from_base` |
| `max_decimals` | Optional integer, 0–18 |

## Outputs and presentation

`source`, `upper`, `lower`, and `style_by` strings in presentation declarations
reference output IDs, not local variables or tickers. Explicit color slots
reference declared styles; true/false slots must be supplied together.

### Plots

`plot(value, id = "value", title = "Value")`

Only the value is required. ID defaults to its source identifier or a generated
ID; title defaults to `Plot`. Numeric plots with visible readouts receive an
automatic color slot when none is specified. Boolean plots are output channels
without visual layers; readouts default to hidden.

| Option | Default / values |
| --- | --- |
| `pane` | `indicator` (`main` in primary overlays); also `named:<number>` |
| `scale` | `price`; also `independent` |
| `style` | `line`; also `step`, `histogram`, `area` |
| `display` | `value` (`none` for booleans) |
| `precision` | Optional: `instrument`, `decimals:<0–18>`, `step:<positive number>` |
| `width`, `offset` | `1`, `0`; width is positive and at most 64, offset is a signed 32-bit integer |
| `color_slot`, `color` | Static style slot or runtime color |
| `style_by` | Boolean output ID; requires `true_color_slot` and `false_color_slot` |
| `layer_title`, `overlay`, `layer` | Plot title, `true`, `true` |
| `data_kind` | Inferred; `scalar`, `histogram`, `ohlc`, `range` |
| `mark`, `axis` | Inferred; axis is `primary` or `secondary` |

Dynamic values are checked when emitted: numeric channels accept numbers or
`na`, boolean channels accept booleans or `na`, and scene fields require their
declared kinds. Wrong kinds fail with an output ID and field diagnostic; the
host quarantines the indicator.

`layer = false` keeps a private channel for presentation wiring; it cannot be a
dependency source. Numeric plot layers must share one pane. Primary overlays
require `main`.

Structured outputs use constructors, not generic plot styles:

```text
plotcandle(chart.open, chart.high, chart.low, chart.close, id = "candles")
plotohlc(chart.open, chart.high, chart.low, chart.close, id = "ohlc")
plotrange(chart.high, chart.low, id = "range")
broken_cloud(chart.high, chart.low, id = "broken")
```

### Styles and alternate marks

`style("bull", title = "Bullish", role = "success", shade = "base")`

Title defaults to the ID, role to `neutral`, shade to `base`. Roles: `neutral`,
`secondary`, `primary`, `success`, `danger`, `warning`. Shades: `faint`, `soft`,
`base`, `strong`, `vivid`. Optional `requires` uses the coverage rules above.

`mark(title = "Line", source = "value", kind = "line", color_slot = "main")`

All four arguments are required. Marks offer alternate rendering of an output.
Kinds: `line`, `step`, `area`, `point`, `cross`, `segment`, `horizontal_level`,
`regular_bar`, `histogram`, `histogram_signed_overlay`, `candle`; compatibility
depends on output type. Optional settings:

- `true_color_slot` / `false_color_slot`: conditional colors.
- `highlight_by` / `highlight_color_slot`: boolean output and outline style for
  regular or histogram bar marks.
- `requires` / `fallback`: required coverage and another declared mark kind.
  Fallback must differ from the current kind, identify exactly one mark, and
  have no coverage requirement.

### Fills, readouts, and tables

`fill(upper = "upper", lower = "lower", color_slot = "band")`

All three arguments are required. `geometry` is `panel_band` (default) or
`series_band`; `opacity` defaults to `0.2`; `upper_offset` and `lower_offset`
default to `0`. Optional `style_by` references a boolean output and requires
paired true/false color slots.

`readout(title = "Signal", source = "signal", color_slot = "main")`

All three arguments are required. Optional `mark` restricts it to that rendering.

A minimal table:

```text
language(1)
indicator("Status", id = "community.status")
display(view = "table")
plot(chart.close > chart.open, id = "trend")
column(title = "Trend", source = "trend", format = "boolean")
```

`column` requires title and source. Formats: `quantity` (default), `integer`,
`percent`, `boolean`. Static coloring uses `color_slot`, defaulting to an
automatic `main` style. `color_mode = "signed"` requires numeric output and
`positive_color_slot` / `negative_color_slot`, with no `color_slot`. Zero uses
the positive slot; unavailable values are blank.

### Segments and regions

`segment(start_value, end_value, start_index, end_index, id = "projection")`

Values are numeric, indices integer. Optional `visible` and `color` are runtime
expressions; `text` is a literal. ID defaults from title or is generated, title
to ID, `color_slot` to `main`, and `width` to `1`.

```text
region(key = bar_index, upper = chart.high, lower = chart.low,
       left_index = bar_index, right_index = bar_index,
       id = "zones", extend_right = true)
```

Regions are non-interactive price overlays. Geometry, integer key, `visible`,
and `extend_right` are runtime expressions; ID, positive `width` (default `1`,
maximum `64`), and `persistent` (default `false`) are literal metadata.
Color each edge with runtime `border_color` / `fill_color` or declared
`border_color_slot` / `fill_color_slot`, never both for the same edge.
Alternatively, a boolean `style_by` expression with paired true/false slots
colors both edges (`na` selects false); this cannot mix with other region colors.

Default emissions form a complete active-set snapshot: omitted `(id, key)`
pairs disappear on the next closed bar. `extend_right` reaches the plot clip.
With `persistent = true`, emissions upsert regions until explicitly removed or
rebuilt; omitted keys remain:

```text
region_remove(id = "zones", key = bar_index)
```

Within a bar, the last update/removal wins. Open previews are transactional.
Persistent regions exceeding the output limit evict the oldest entries.

## TA behavior

Window lengths are positive simple integers, including inputs. The default
runtime maximum is 5,000, separate from retained-history `max_bars_back`.
Oversized windows and collections fail rather than being silently clamped.
The execution cap charges weighted work for evaluated expressions, scans, and
copies, including collection elements and string bytes. Preview work uses the
same per-frame budget as closed evaluation.

Stateful TA must execute once per bar. Constants and immutable-input switches
may choose a fixed calculation path. Calls in loops or bar-dependent branches,
short-circuited expressions, fallbacks, function defaults, and state initializers
must be hoisted into ordinary per-bar bindings first. This applies through user
functions and to guarded drawing/history expressions. For example:

```text
average = ta.sma(chart.close, 20)
state seed = average
plot(select(chart.close > 0, average, na))
```

Finite rolling windows consume one sample per bar; a missing sample makes the
window's result `na` until it leaves the window.

- `ta.pivothigh(source, left, right)` / `ta.pivotlow`: confirm the center
  `right` bars later. Both legs are positive; `left + right + 1` must fit the
  window limit. Ties qualify; missing samples return `na`.
- `ta.rsi(source, period)`: seeds with the mean of `period` valid adjacent price
  changes, then uses Wilder smoothing. Missing prices break adjacency but keep
  accumulated averages. Flat gain/loss gives 50, zero loss 100, zero gain 0.
- `ta.previous_when(condition, value)`: returns the value from the previous
  true event, then stores the current value if the condition is true.

For working formulas and presentation combinations, browse the
[bundled sources](../indicators/core/) and [catalog](../indicators/catalog/sources/).
