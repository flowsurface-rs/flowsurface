# Indicator language

An indicator is one installable `.ind` source file. The file name must be the
final segment of its DSL ID, for example `rsi.ind` for
`catalog.rsi`; the `id` declaration remains authoritative. Names are
case-sensitive, and the application rejects duplicate final segments across
the complete startup catalog.

## Namespaces

Indicator IDs use one of three ownership namespaces:

- `core.<indicator>` is reserved for the GPL-licensed sources bundled and
  maintained by Flowsurface.
- `catalog.<indicator>` identifies the Apache-2.0 public catalog curated by
  Flowsurface contributors.

The engine accepts all valid IDs. Collection policy is checked by the host and,
when desired, by the CLI:

```text
cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- check indicators/core --namespace core
cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- test indicators/catalog/sources --namespace catalog
```

The future `indicator-engine` repository will contain the library and
`flowsurface-indicator` CLI. The future `flowsurface-indicators` repository will
contain only the curated `catalog.*` sources and their documentation. Source
licensing is independent of the Apache-2.0 engine license.

```text
language(1)
indicator("Relative strength", id = "catalog.rsi",
          sourceable = true, max_bars_back = 5000)
format(precision = "fixed_step:0.01")
period = input.int(14, "Period", min = 2, max = 200)
gain = select(chart.close > chart.close[1], chart.close - chart.close[1], 0)
average = ta.rma(gain, period)
plot(average, id = "value", title = "Average")
```

Inputs and ordinary output declarations must be top-level. `region` is the
exception: it may be emitted from top-level bounded control-flow blocks. `=`
creates a local binding;
`state` or `var` creates persistent state and `:=` updates it. A binding keeps
its inferred concrete type across updates; integer/float updates widen to
float, and `na` is compatible with any type. Incompatible concrete updates
are compile errors. Conditionals,
bounded `for`/`while` loops, non-recursive functions, tuples, arrays, records,
destructuring, switch expressions, history indexing, and named arguments are
supported.

## Inputs

Inputs are top-level bindings. The first two arguments are the default value
and its label; metadata is optional:

```text
length = input.int(14, "Length", min = 1, max = 200, step = 1,
                   group = "period", inline = "length", tooltip = "Bars")
multiplier = input.float(2.0, "Multiplier", min = 0.1, max = 10.0, step = 0.1)
enabled = input.bool(true, "Enabled")
method = input.string("Sma", "Method", options = ["Sma", "Ema"])
anchor = input.time(0, "Anchor")
color = input.color()
```

Supported input types are `int`, `float`, `bool`, `string`, `time`, and
`color` (a color input uses the engine's default color when no literal default
is supplied).
`options` accepts only a literal array of strings. Defaults, ranges, steps,
labels, and option values, when supplied, must be compile-time literals.
The host accepts a configured value only when it matches the declared type,
numeric bounds, and string options. Scripted `int` and `time` inputs retain
signed 64-bit values through the host settings bridge.

User functions are numeric in language version 1: parameters and return
expressions must be numeric or `na`. Integer arguments can be used where a
float is expected, and parameters may have numeric or `na` defaults.

`na` is a missing value, not an ordinary value. `==` and `!=` return `na` when
either operand is `na`; a `switch` branch whose condition is `na` does not
match. A `state` initializer is retried until it produces a non-`na` value,
while an explicit `:= na` update remains authoritative. Logical operators
require boolean operands, ordering operators accept numeric or time operands,
and arithmetic remains numeric-only. `select`, `coalesce`, and `switch` result
branches use the same compatibility rules. Non-finite numeric results are
exposed as `na`.

History indexing such as `source[1]` requires an immutable retained series.
Direct chart series and immutable series locals are retained automatically;
`source[0]` does not need history storage. Mutable locals, function parameters,
and function-call results cannot be indexed historically and are compile-time
errors. Use an immutable top-level series binding when a value needs history.

## Declaration reference

Named arguments are compile-time constants unless noted as series expressions.
The minimal forms below are the recommended starting point. Presentation
metadata is split into `display` and `format`, and both declarations must be
top-level and appear before the first output declaration. For numeric plot
views, `format(precision = ...)` is required because it controls the panel
axis, domain quantization, and crosshair values. Table views and primary
overlays do not use panel value precision.

### `indicator`

```text
indicator("Title", id = "namespace.name",
          sourceable = true, optional_trades = false, max_bars_back = 5000)
```

`id` is the stable `namespace.name` identity. `sourceable` and
`optional_trades` default to `true` and `false`, respectively, and
`max_bars_back` defaults to `5000`. `sourceable` is an indicator-level policy:
when it is `false`, the indicator's visible numeric plots still render
normally, but none of them can be used as another indicator's dependency
input. `optional_trades` and `max_bars_back` remain indicator-level policies.

### `display`

```text
display(view = "plot", attachment = "panel",
        panel_title = "Title", layer_name = "Title")
```

There can be at most one top-level `display` declaration. It must appear before
the first output declaration. `view` defaults to `plot`; use `primary_overlay`
for primary-chart placement and `table` for table panels. An `attachment` of
`overlay` or `panel` is required when an indicator has an indicator-enabled
source input. `overlay` lets attached output layers share the source panel's
scale; `panel` keeps the indicator's normal standalone placement and scale.
Panel and layer titles default to the indicator title. Valid views are `plot`,
`table`, and `primary_overlay`; valid attachments are `overlay` and `panel`.

### `format`

```text
format(precision = "fixed_step:1",
       axis_labels = "compact", header_labels = "commas",
       scale = "fit_visible", max_decimals = 2)
```

There can be at most one top-level `format` declaration. It must appear before
the first output declaration. `precision` is required for numeric plot
panels. Axis labels default to `compact`, headers to `commas`, and scale to
`fit_visible`. Valid precision values are `base_ticker_min_tick`,
`base_ticker_min_qty`, and `fixed_step:<positive number>`. Label modes are
`compact`, `commas`, and `abbreviated`; scale values are `absolute`,
`fit_visible`, `fit_visible_include_zero`, `logarithmic`, and
`percent_from_base`. `max_decimals` is an integer from 0 to 18.

### `plot`

```text
plot(value, id = "value", title = "Value", pane = "indicator",
     scale = "price", style = "line", display = "value",
     precision = "decimals:2", width = 1.5, offset = 0,
     color_slot = "value", style_by = "rising",
     true_color_slot = "bull", false_color_slot = "bear",
     layer_title = "Value", overlay = true, data_kind = "scalar",
     mark = "line", axis = "secondary", layer = true)
```

Only the value expression is required. The output ID defaults to the source
identifier (or a generated ID), and the title defaults to `Plot`. Pane defaults
to `main` for an indicator with `display(view = "primary_overlay")` and
`indicator` otherwise. Scale is `price`,
style is `line`, display is `value` (`none` for boolean outputs), width is `1`,
offset is `0`, overlay and layer are `true`, and data kind, mark, and axis are
inferred from the value type, style, pane, and scale. A numeric plot without a
color slot gets a deterministic slot named after its output ID.

Generic `plot` styles are `line`, `step`, `histogram`, and `area`. Panes are
`main`, `indicator`, or `named:<number>`;
scales are `price` or `independent`; display is `value` or `none`; and precision
is `instrument`, `decimals:<n>`, or `step:<number>`. Boolean plots
are output-only unless a presentation declaration references them.

`layer = false` keeps a plot as a private output channel for marks, fills,
readouts, or table columns. It is not a visible numeric plot and cannot be
selected as an indicator dependency source.

Structured outputs use dedicated constructors instead of `plot` styles:

```text
plotcandle(open, high, low, close, id = "candles")
plotohlc(open, high, low, close, id = "ohlc")
plotrange(upper, lower, id = "range")
broken_cloud(upper, lower, id = "broken")
```

`plotcandle` and `plotohlc` produce OHLC output; `plotrange` and
`broken_cloud` produce range output. Using `candles`, `bars`, `cloud`, or
`broken_cloud` as a generic `plot` style is invalid.

Rolling pivot intrinsics return the confirmed center value, or `na`:

```text
high_pivot = ta.pivothigh(chart.high, left, right)
low_pivot = ta.pivotlow(chart.low, left, right)
```

The center bar is confirmed `right` bars later. The inclusive window contains
`left + right + 1` samples; equal highs and lows are accepted, and missing
samples produce `na`. Both legs must be positive integers and the complete
window must fit the runtime window limit. Legs may be simple runtime inputs,
so the selected window is checked on every evaluation. A preview evaluates
from the last committed intrinsic state and never commits its pivot history.

### `style`

```text
style("bull", title = "Bullish", role = "success", shade = "base")
```

The ID is required. Title defaults to the ID, role to `neutral`, and shade to
`base`. Roles are `neutral`, `secondary`, `primary`, `success`, `danger`, and
`warning`. Shades are `faint`, `soft`, `base`, `strong`, and `vivid`.

### `mark`

````text
mark(title = "Histogram", source = "value", kind = "histogram",
     color_slot = "base", true_color_slot = "bull",
     false_color_slot = "bear", requires = "directional_volume",
     fallback = "histogram")

Bar marks can optionally draw a non-destructive outline when a boolean output
is true:

```text
mark(title = "Overlay bars", source = "value", kind = "histogram",
     color_slot = "base", highlight_by = "warning_signal",
     highlight_color_slot = "warning")
````

`highlight_by` must reference a boolean output and is supported by regular and
histogram bar marks. The outline is drawn without replacing the bar's normal
fill or conditional colors.

````

`title`, `source`, `kind`, and `color_slot` are required. A mark is a selectable
alternate rendering of an output. Conditional slots are optional, but must be
provided as a true/false pair. `requires` is optional and declares an input that
must be available for this rendering. Use `klines`, `directional_volume`,
`trades`, or `open_interest`. The requirement applies to the selected mark,
not to the indicator as a whole. `fallback` is optional and names another mark
kind declared by the same indicator. When the required input has no coverage,
the runtime uses that fallback mark. The fallback target must not require its
own input. Use the mark values listed for `indicator`.

### `fill`

```text
fill(upper = "upper", lower = "lower", color_slot = "band",
     geometry = "series_band", opacity = 0.2, style_by = "rising",
     true_color_slot = "bull", false_color_slot = "bear",
     upper_offset = 0, lower_offset = 0)
````

`upper`, `lower`, and `color_slot` are required. Geometry defaults to
`panel_band` and can be `panel_band` or `series_band`; opacity defaults to `0.2`.
Offsets default to zero. Conditional color slots must be a pair.

### `readout`

```text
readout(title = "Signal", source = "signal", color_slot = "signal",
        mark = "line")
```

Title, source, and color slot are required. `mark` is optional and limits the
readout to a selected rendering.

### `column`

```text
indicator("Status", id = "catalog.status")
display(view = "table")
column(title = "Trend", source = "trend", format = "boolean",
       color_slot = "main")
column(title = "Delta", source = "delta", color_mode = "signed",
       positive_color_slot = "positive", negative_color_slot = "negative")
```

Title and source are required. Format defaults to `quantity`; use `quantity`,
`integer`, `percent`, or `boolean`. `color_mode` defaults to `static`. Static
columns use `color_slot`, which defaults to `main`. Signed columns use both
`positive_color_slot` and `negative_color_slot`; they cannot also provide
`color_slot`, and they require a numeric output. Zero uses the positive slot.
Unavailable values remain blank. Color slots must refer to declared styles.

### `segment`

```text
segment(start_value, end_value, start_index, end_index,
        id = "projection", title = "Projection", color_slot = "projection",
        width = 1.5, visible = show_projection, text = "Projection")
```

The four positional expressions are required. ID defaults from title or a
generated value, title defaults to the ID, color slot to `main`, and width to
`1`. Visibility, color, and text are runtime expressions or labels; the scene
metadata remains static.

### `region`

```text
region(key = zone_key, upper = top, lower = bottom,
       left_index = left, right_index = bar_index,
       border_color = border, fill_color = fill,
       extend_right = true, visible = active, persistent = false,
       id = "order_blocks", width = 1.0)
```

Static style slots can be used instead of runtime colors:

```text
style("bullish", title = "Bullish", role = "success")
region(..., border_color_slot = "bullish", fill_color_slot = "bullish")
```

Color slots refer to declared semantic styles and use the indicator accent
controls. Runtime colors remain available when a region needs per-bar color
values. A region cannot provide both a runtime color and a style slot for the
same edge.

`region` emits non-interactive price-zone scene data. `id` and `width` are
compile-time constants; `key` is a stable runtime integer used with `id` to
update a zone. The geometry and visibility expressions may be emitted from
bounded top-level `if` and `for` blocks, but not from user functions. Regions
are price overlays only and do not create plot channels, drawing objects,
handles, labels, or hit-testing.

By default, each closed-bar evaluation emits a complete active-set snapshot. A
repeated `id` and `key` updates that region and the last emission wins; keys
omitted by the next snapshot disappear. `extend_right` reaches the primary
plot clip. With the literal `persistent = true`, the call emits an incremental
upsert instead; omission on later bars does not remove the region. Persistent
regions remain until a `region_remove` call or indicator rebuild.

```text
region_remove(id = "flat_open_zones", key = zone_key)
```

`region_remove` emits an incremental deletion for the `(id, key)` pair. Both
persistent updates and removals use last-write-wins ordering within a bar.
Open-bar previews apply persistent updates transactionally and never mutate
the committed region store. Persistent regions are retained oldest-first up to
the runtime output limit; older entries are evicted deterministically.

## Common patterns

Ordinary plot:

```text
indicator("Momentum", id = "catalog.momentum", max_bars_back = 500)
format(precision = "base_ticker_min_tick")
value = chart.close - chart.close[10]
plot(value, id = "value", title = "Momentum")
```

Overlay:

```text
indicator("Moving average", id = "catalog.ma")
display(view = "primary_overlay")
average = ta.sma(chart.close, 20)
plot(average, id = "average", title = "Average")
```

Table:

```text
indicator("Status", id = "catalog.status")
display(view = "table")
trend = chart.close > chart.close[1]
plot(trend, id = "trend", title = "Trend", display = "none", overlay = false, layer = false)
column(title = "Trend", source = "trend", format = "boolean")
```

Histogram:

```text
indicator("Flow", id = "catalog.flow")
format(precision = "base_ticker_min_tick")
flow = chart.close - chart.open
plot(flow, id = "flow", title = "Flow", style = "histogram")
```

Fill and conditional colors:

```text
indicator("Band", id = "catalog.band")
format(precision = "base_ticker_min_tick")
upper = ta.sma(chart.close, 20) + 2 * ta.stdev(chart.close, 20)
lower = ta.sma(chart.close, 20) - 2 * ta.stdev(chart.close, 20)
rising = upper > upper[1]
style("band", role = "primary", shade = "faint")
style("bull", role = "success")
style("bear", role = "danger")
plot(upper, id = "upper", title = "Upper", color_slot = "band",
     style_by = "rising", true_color_slot = "bull", false_color_slot = "bear")
plot(lower, id = "lower", title = "Lower", color_slot = "band")
fill(upper = "upper", lower = "lower", color_slot = "band")
```

## Runtime contract

The host supplies one frame for each base-ticker chart bar. Every indicator
reads that one chart frame; comparison tickers remain a chart presentation
feature and are not visible to the DSL. Kline history and live subscriptions
may be requested for comparison tickers, but trade history and live trade
subscriptions are requested only for the base ticker. A frame includes
`chart.time`,
`chart.open`, `chart.high`, `chart.low`, `chart.close`, `chart.volume`,
`chart.hl2`, `chart.hlc3`, `chart.ohlc4`, `chart.hlcc4`,
`chart.directional_volume_available`, `chart.buy_qty`, `chart.sell_qty`,
`chart.trade.*`, and `open_interest`. Missing aligned data is `na`.
The stable base-instrument minimum price increment is available as
`instrument.min_tick`. It is simple instrument metadata, not a chart series,
and is `na` when unavailable.
Open interest is aligned as-of: each bar receives the latest finite sample at
or before its timestamp, and bars before the first sample receive `na`. Plot
offsets are presentation metadata and do not change input alignment.
Closed evaluation commits state; open evaluation is a preview from the previous
closed state. There is no correction, rewind, checkpoint, intrabar-persistent
state, or repainting mode.

Execution also runs under host resource limits. Stateful TA windows are capped
at a default length of 5,000, and array/record growth is bounded by the host's
collection and heap budgets. Values are checked as collection-producing
expressions run, and oversized windows or collections fail the indicator and
are quarantined; they are never silently clamped. These limits are separate
from `max_bars_back`, which bounds retained series history.

Finite rolling calculations consume one source sample per chart bar. If any
sample in a rolling window is `na`, the result is `na`; missing bars do not
disappear from the window.

Trade-derived values have an explicit namespace:

```text
chart.trade.available
chart.trade.buy_qty
chart.trade.sell_qty
chart.trade.buy_count
chart.trade.sell_count
chart.trade.count
```

`chart.trade.*` is the base-ticker chart series and is the input for indicators
such as Bar Analysis. `chart.buy_qty` and `chart.sell_qty` are base-ticker
directional volume; `chart.directional_volume_available` reports its
availability. Candle directional volume is not a fallback for `chart.trade.*`.
Trade aggregates may currently come from per-bar trades or footprint trade
groups; raw trade-event arrays are not exposed to the language.

The `source` argument in `mark`, `readout`, and `column` declarations names an
indicator output channel. It is presentation wiring and does not select a
ticker. The DSL has no `sources`, `source("SOURCE_ID")`, or `flow.*` inputs.

`input.source` declares a numeric series input and its chart-series default:

```text
source = input.source(chart.close, "Source")
base_only = input.source(chart.close, "Base only", allow_indicator = false)
```

`allow_indicator` defaults to `true`. An indicator-enabled source input is the
consumer side of composition; it requires `view = "plot"`, an `attachment` of
`overlay` or `panel`, and at least one visible numeric plot layer. A base-only
input with `allow_indicator = false` accepts the declared default and host
base-series selections, but rejects indicator bindings and may be used by
table or primary-overlay indicators. The engine and host support multiple
source inputs; the minimal one-click attachment UI offers only indicators with
exactly one indicator-enabled input.

The standard price sources are also available as chart expressions for
declared defaults: `chart.hl2`, `chart.hlc3`, `chart.ohlc4`, and
`chart.hlcc4`. For example, CCI can preserve its typical-price default with:

```text
source = input.source(chart.hlc3, "Source")
```

The chart expression is the host-neutral fallback. A valid host binding
replaces it with the same-index visible numeric plot output of another active,
sourceable indicator; an unavailable or invalid binding evaluates as `na` and
never falls back to the declared chart source.
When no indicator binding exists, the host may select a standard base-chart
source (`Open`, `High`, `Low`, `Close`, `HL2`, `HLC3`, `OHLC4`, or `HLCC4`)
for the slot; the declared chart expression remains the default. Source inputs
support history indexing and all numeric TA functions. Numeric
plot layers in one indicator must share one resolved pane target. `pane` only
controls placement; it is not a dependency selector. Cross-indicator source
bindings are restricted to the same Kline pane and base series/timeframe. The
indicator's `attachment` declaration controls only source-owned placement; it
does not change dependency evaluation or the source binding itself.

The language rejects remote requests, multi-timeframe contexts, strategies,
alerts, `varip`, maps, mutable drawing objects, renderer-owned table handles,
raw trade arrays, liquidations, depth, and remote symbols.

## Validation and startup

The application loads `indicators/core/` first, then flat files in the installed
directory, then explicit files supplied with repeated `--indicator-path`. For
repository development, set `FLOWSURFACE_INDICATORS_PATH` to a root containing
`core/` and `catalog/sources/` to load those collections directly instead of the data
directory. It does this once at startup; changing a source requires an
application restart.
Invalid installed files are diagnosed and skipped, while core errors are fatal.
After valid files are compiled, a duplicate short name in core, installed, or
explicit development sources is a startup-fatal error. The application does
not choose one source by precedence when their short names collide.

Use the standalone CLI workflow while developing:

    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- new indicators/catalog/sources --id catalog.example --title "Example"
    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- check indicators/catalog/sources/example.ind --namespace catalog
    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- test indicators/catalog/sources/example.ind --namespace catalog

`check` also accepts a flat source directory. `test` runs the source with
deterministic synthetic market frames after checking it, which catches runtime
and resource-limit failures without requiring exchange data. There are no JSON
fixtures or presentation sidecars. Copy the resulting `.ind` file into the
installed collection root or pass its path with `--indicator-path`, then
restart the application. Existing full-ID filenames such as
`catalog.example.ind` must be renamed manually; they are diagnosed and
skipped.
