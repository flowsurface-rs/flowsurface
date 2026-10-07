# Indicator language

A `.ind` file defines an indicator's inputs, calculations, and presentation.
See [installation](../indicators/README.md) and the [CLI](README.md#cli) for
file naming, collections, and validation.

Examples beginning with `language(1)` are complete files. Other fenced examples
and individual call signatures are partial snippets to use inside a file.

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

- `=` binds a local; `state` or `var` creates persistent state; `:=` updates
  either an ordinary local or persistent state.
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
  `max_bars_back`. Literal offsets grow retention to at least `offset + 1`,
  even when an explicitly declared bound is smaller, up to 50,000 retained bars.
- Inputs and ordinary output/presentation declarations must be top-level.
  `region` and `region_remove` may occur in bounded control-flow blocks outside
  functions. At most one `display` and one `format` are allowed, before outputs.

Records support dot access and `record.get`:

```text
zone = { upper: chart.high, lower: chart.low }
top = zone.upper
```

User-record fields have dynamic type `Any`; absent fields or non-record values
return `na` at runtime. Statically known wrong kinds are rejected at compilation.
Built-in chart fields are checked at compile time.

### Types and operators

Values include signed 64-bit integers, floats, booleans, strings, colors, time
inputs, tuples, arrays, records, and `na`. A **simple** value depends only on
constants or inputs; a **series** value may change each bar. Host price fields,
`bar_index`, and TA results are series. `instrument.min_tick` is simple metadata.
TA window lengths need simple integers, so integer inputs work but bar-dependent
lengths do not. Function parameters and returns are typed as numeric float
series, even when a caller passes an integer; parameters cannot supply simple
integer window lengths or history offsets.

Operators, from highest to lowest precedence: unary `+`, `-`, `!`; `*`, `/`,
`%`; `+`, `-`; `<`, `<=`, `>`, `>=`; `==`, `!=`; `&&`; `||`. Parentheses group
expressions. `&&` and `||` short-circuit; `select` provides conditional values.
Only boolean `true` takes an `if` branch or continues a `while`; `false` and `na`
do not. Statements end at a newline or semicolon; braces group bodies.
Calls and collection literals may span lines. `//` starts a line comment.

### Control flow and collections

This complete example uses per-bar locals, a numeric function with a default,
conditional expressions, and both range and collection loops:

```text
language(1)
indicator("Syntax example", id = "community.syntax-example")
format(precision = "base_ticker_min_tick")
method = input.string("Double", "Method", options = ["Double", "Identity"])

fn multiply(value, factor = 2) {
    return value * factor
}

chosen = switch method {
    "Double" => multiply(chart.close),
    "Identity" => chart.close,
    default => na
}
values = array.new()
for index = 0 to 2 {
    if index == 1 {
        continue
    }
    values := array.push(values, (index, chosen))
}
total = 0.0
for entry in values {
    (index, value) = entry
    if na(value) {
        break
    } else {
        total := total + value
    }
}
counter = 0
while counter < 2 {
    counter := counter + 1
}
zone = { upper: chart.high, lower: chart.low }
zone := record.set(zone, "middle", chart.hl2)
plot(total / counter, id = "average")
plot(zone.middle, id = "middle")
```

Range loops evaluate their bounds once, include both endpoints, and increment
by one. A start greater than the end produces no iterations; descending ranges
and custom steps are unavailable. Collection loops iterate an array or tuple's
evaluated contents. Compilation does not prove loop termination: iterations
and evaluated work are capped at runtime, including `while true`.
`switch` branches are comma-separated; `default => value` supplies the fallback.

Arrays use `[value, ...]`; tuples use `(first, second, ...)`. Both use zero-based
element indexing, and negative or out-of-range indices return `na`. In contrast,
indexing a numeric series means bars of history, not collection elements.
Destructuring uses `(first, second) = collection`; `:=` updates existing locals.
Records use `{ field: value }`, dot access, or `record.get(record, "field")`.
`array.push` and `record.set` return updated collections rather than mutating
the binding: retain the result with `values := array.push(values, value)`.

## Host data

Every indicator reads the base ticker's chart frame. Comparison tickers are
presentation-only.

| Fields | Meaning |
| --- | --- |
| `chart.time`, `bar_index` | Bar timestamp in Unix milliseconds and zero-based index |
| `chart.open`, `chart.high`, `chart.low`, `chart.close`, `chart.volume` | OHLCV |
| `chart.hl2`, `chart.hlc3`, `chart.ohlc4`, `chart.hlcc4` | Derived price averages |
| `chart.directional_volume_available`, `chart.buy_qty`, `chart.sell_qty` | Candle directional volume |
| `chart.trade.available`, `chart.trade.buy_qty`, `chart.trade.sell_qty` | Trade coverage and quantities |
| `chart.trade.buy_count`, `chart.trade.sell_count`, `chart.trade.count` | Trade counts |
| `open_interest` | Latest finite sample at or before the bar timestamp; `na` before the first sample |
| `instrument.min_tick` | Minimum price increment; simple metadata, `na` if unavailable |

| Evaluation flag | True when |
| --- | --- |
| `barstate.isfirst` | The frame's index is zero, including a preview at index zero |
| `barstate.isconfirmed` | Evaluating a closed frame |
| `barstate.islast` | Evaluating an open preview |
| `barstate.isrealtime` | Evaluating an open preview |

`islast` does not identify the last historical closed bar. Both `islast` and
`isrealtime` are false on closed evaluations, including live bar commits.

Candle directional volume does not substitute for missing trade aggregates.
Raw trade arrays are unavailable. Plot offsets change presentation only.

There is no arbitrary I/O, remote symbol/timeframe access, strategy or alert
API, `varip`, map type, or mutable drawing/table handle. Scripts cannot request
correction replay, and an engine instance cannot revise a committed bar. The
application may rebuild affected indicators and replay historical frames when
data changes, including closed-bar trade corrections.

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
Defaults and configured values must match the type; `int`, `float`, and `time`
values must satisfy inclusive `min`/`max` bounds, and strings must match declared
options. `step` is control metadata, not a runtime divisibility or alignment
rule. Color defaults to white; `int` and `time` settings preserve signed 64-bit
values. Time inputs use Unix milliseconds. Bounds metadata uses floating-point
numbers, including for integer and time comparisons.

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
| `max_bars_back` | `5000`; positive retained-history bound, automatically increased for literal offsets; at most `50000` |
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
to ID, `color_slot` to `main`, and `width` to `1`. The slot must be declared,
including when using the default `main`.

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

A complete drawing example:

```text
language(1)
indicator("Bar geometry", id = "community.bar-geometry")
display(view = "primary_overlay")
style("main", title = "Geometry", role = "primary", shade = "base")
plot(na, id = "anchor", display = "none", pane = "main", layer = true)
segment(chart.low, chart.high, bar_index, bar_index,
        id = "bar-span", color_slot = "main")
region(key = 0, upper = chart.high, lower = chart.low,
       left_index = bar_index, right_index = bar_index,
       id = "current-bar", border_color_slot = "main",
       fill_color_slot = "main", extend_right = true)
```

`primary_overlay` currently requires a numeric output layer. The hidden `na`
plot supplies that layer for a drawing-only source without drawing a price line.
This example emits a snapshot region; a persistent region instead uses
`persistent = true` and `region_remove` to manage retained keys.

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
- `ta.rsi(source, length)`: seeds with the mean of `length` valid adjacent price
  changes, then uses Wilder smoothing. Missing prices break adjacency but keep
  accumulated averages. Flat gain/loss gives 50, zero loss 100, zero gain 0.
- `ta.previous_when(condition, source)`: returns the value from the previous
  true event, then stores the current value if the condition is true.

For working formulas and presentation combinations, browse the
[bundled sources](../indicators/core/) and [catalog](../indicators/catalog/sources/).

## Built-in reference

The signatures below use the actual named-argument names. Unless stated
otherwise, arguments may be series. Numeric arguments accept integers or
floats; numeric TA/math results are floats or `na`. Missing values are accepted
at runtime, with behavior described below. All `ta.*` calls are stateful and
follow the scheduling rules above.

### TA

| Signature | Result and behavior |
| --- | --- |
| `ta.sma(source, length)` | Mean of a full window; `na` before it fills or while it contains missing samples |
| `ta.ema(source, length)` | First valid source seeds the result; then alpha is `2 / (length + 1)` |
| `ta.rma(source, length)` | First valid source seeds the result; then alpha is `1 / length` |
| `ta.rsi(source, length)` | Wilder RSI with the seeding and edge cases described above |
| `ta.previous_when(condition, source)` | Previous true-event value, initially `na`; preserves the numeric source kind; `condition` is boolean |
| `ta.vwma(source, volume, length)` | Full-window sum of price times volume divided by sum of volume; `na` for missing samples or zero total volume |
| `ta.highest(source, length)` / `ta.lowest(source, length)` | Full-window extreme; `na` for an incomplete or missing window |
| `ta.pivothigh(source, left, right)` / `ta.pivotlow(source, left, right)` | Confirmed centered pivot with positive legs, as described above |
| `ta.cum(source)` | Running sum starting at zero; first valid source is the first result |
| `ta.stdev(source, length)` | Full-window population standard deviation, dividing variance by `length` |
| `ta.dev(source, length)` | Full-window mean absolute deviation from the mean |
| `ta.highestbars(source, length)` / `ta.lowestbars(source, length)` | Nonnegative bars-back offset as a float; ties select the oldest extreme in the window |
| `ta.tr(high, low, close)` | True range using previous valid close; first valid bar returns `high - low` |
| `ta.avwap(source, volume, anchor)` | Anchored volume-weighted mean; `anchor` is a simple string: `Day`, `Week`, or `Month`, case-insensitive |

`length`, `left`, and `right` require simple positive integers. Finite-window
statistics, including `stdev`, `dev`, and extreme offsets, need the full window
and return `na` if any sample is missing. EMA, RMA, and cumulative sums return
`na` on a missing source and retain their accumulated state. True range returns
`na` if any current OHLC argument is missing and retains its previous close.
`previous_when` stores the current source, including `na`, on a true condition;
false or missing conditions leave the stored value unchanged.

AVWAP resets at UTC midnight for days, Monday midnight for weeks, or the first
day of the month. Unknown anchors return `na`. Missing source/volume samples
leave the totals unchanged and may still return the accumulated mean. It returns
`na` until accumulated volume exceeds floating-point epsilon. Non-finite sums
or recursive updates return `na` without replacing the last finite accumulated
state; other non-finite numeric results also become `na`.

### Math and missing values

| Signature | Result and behavior |
| --- | --- |
| `math.abs(value)` | Numeric absolute value; `na` for missing input |
| `math.min(left, right, ...)` / `math.max(left, right, ...)` | At least two numeric arguments; `na` if any argument is missing |
| `na(value)` | Boolean missingness test for any kind |
| `nz(source, replacement)` | Numeric source or replacement when missing; replacement is simple and required |
| `coalesce(source, fallback)` | Source or lazily evaluated fallback when source is missing; compatible result kinds |
| `select(condition, when_true, when_false)` | Lazily evaluates one result; false or missing condition chooses `when_false`; compatible result kinds |

`nz` evaluates both arguments. `select`, `coalesce`, and `switch` evaluate only
the selected result; `&&` and `||` may skip their right operand. Hoist stateful TA
out of paths whose selection depends on the bar, so skipped evaluation cannot
change TA state progression. Function defaults and state initializers also
require hoisting, even when their expression looks constant.

### Collections, strings, and colors

| Signature | Result and behavior |
| --- | --- |
| `array.new(...)` | Array containing zero or more positional initial values of any kind |
| `array.push(array, value)` | Updated array with one appended value |
| `array.get(array, index)` | Element of dynamic kind `Any`; integer index; `na` outside the array |
| `array.size(array)` | Integer element count |
| `record.new()` | Empty record |
| `record.set(record, key, value)` | Updated record; string key; adds or replaces a field |
| `record.get(record, key)` | Field of dynamic kind `Any`; string key; `na` for absent fields or runtime non-record values |
| `str.tostring(value)` | String for any kind; missing value becomes `"na"`; collections report their element/field count |
| `color.rgb(red, green, blue, alpha)` | Color; integer components in 0–255; omitted or missing alpha defaults to 255 |

Colors also have constants `color.red`, `color.green`, `color.blue`,
`color.white`, and `color.black`. `color.rgb(...)` with positional literal
components can supply input defaults; runtime color expressions can style
outputs. Invalid runtime components fail rather than being clamped. Collection
operations remain subject to heap, nesting, element-count, and weighted
copy-work limits.

## Resource limits

The Rust library's default limits differ from the application's limits. CLI
`test` uses the application's `RuntimeLimits::untrusted()` for both warmup
planning and execution. Closed frames and open previews have the same per-frame
budget. Exceeding a work or resource limit fails evaluation.

| Limit | Library default | Application / CLI test |
| --- | --- | --- |
| Weighted work per frame | 1,000,000 | 250,000 |
| Function call depth | 128 | 64 |
| Heap bytes | 67,108,864 (64 MiB) | 16,777,216 (16 MiB) |
| Collection elements | 100,000 | 25,000 |
| TA window length | 5,000 | 5,000 |
| Output items per frame | 1,000,000 | 4,096 |
| String bytes | 1,048,576 | 65,536 |

History analysis has separate fixed limits of 250,000 weighted work units and
64 nested traversal entries per pass. Visits, cache reads, and value copies
consume work. These limits apply to warmup and anchor resolution, independently
of per-frame execution limits. Instantiation rejects exhaustion with a planning
diagnostic; optional history queries return `None`. Hosts can call
`CompiledIndicator::validate_history_analysis` before accepting a configuration.

Collection nesting is limited to 256 levels. Retained-series history has the
separate 50,000-bar ceiling; increasing it does not increase TA window limits.
Successful CLI testing covers only default inputs on synthetic data, not every
configuration or long-history workload. See the [CLI workflow](README.md#cli).

## Extending the scripting boundary

Try existing primitives and composition first; keep indicator algorithms in
`.ind` sources. Do not extend the DSL merely to shorten a source. When a real
requirement cannot be expressed cleanly, identify the smallest missing reusable
capability and place it at the appropriate layer:

| Layer | Owns |
| --- | --- |
| Language/runtime | General computation, state, control flow, collections, history |
| TA library | Reusable calculations without new execution semantics |
| Host data | Stable market-domain series, coverage, and instrument metadata |
| Presentation | Reusable chart geometry and styling consuming outputs |

A capability should make sense without naming its originating indicator and
should plausibly support unrelated indicator families. Multiple existing callers
are not required. Avoid indicator-specific host calculations, leaked cache or
renderer internals, and speculative universal frameworks. Preserve bounded,
deterministic work and memory, failure isolation, historical replay, and
transactional open previews. Explain each extension's layer and reuse in its
handoff; recursion, arbitrary I/O, and unrestricted host access remain outside
the language's target.
