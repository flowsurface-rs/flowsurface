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
          value_precision = "fixed_step:0.01")
period = input.int(14, "Period", min = 2, max = 200)
gain = select(chart.close > chart.close[1], chart.close - chart.close[1], 0)
average = ta.rma(gain, period)
plot(average, id = "value", title = "Average")
```

Inputs and output declarations must be top-level. `=` creates a local binding;
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

## Declaration reference

Named arguments are compile-time constants unless noted as series expressions.
The minimal forms below are the recommended starting point. For plot views,
`value_precision` is required because it controls the panel axis, domain
quantization, and crosshair values. Table views and primary overlays do not
use panel value precision. The remaining metadata can be added only when the
inferred result is not what the indicator needs.

### `indicator`

```text
indicator("Title", id = "namespace.name", view = "plot",
          optional_trades = false, max_bars_back = 5000,
          panel_title = "Title", layer_name = "Title",
          data_kind = "scalar", mark = "line", axis = "secondary",
          value_precision = "fixed_step:1",
          axis_label_mode = "compact", header_label_mode = "commas",
          preferred_scale = "fit_visible", max_decimals = 2)
```

`id` is the stable `namespace.name` identity. `view` defaults to `plot`,
`optional_trades` to `false`, and `max_bars_back` to `5000`. Use
`view = "primary_overlay"` for primary-chart placement and `view = "table"`
for table panels. Panel and layer titles default to the indicator title. Data
kind, mark, and axis default from the first numeric plot layer. Numeric plot
panels must declare `value_precision`; axis labels default to `compact`,
headers to `commas`, and scale to `fit_visible`.

Valid values are `plot`, `table`, `primary_overlay` for `view`; `scalar`,
`histogram`, `ohlc`, `range` for `data_kind`; `line`, `step`, `area`, `point`,
`cross`, `regular_bar`, `histogram`, `histogram_signed_overlay`,
`horizontal_level`, and `candle` for `mark`; `primary` or `secondary` for
`axis`; `compact`, `commas`, or `abbreviated` for label modes; and `absolute`,
`fit_visible`, `fit_visible_include_zero`, `logarithmic`, or `percent_from_base`
for `preferred_scale`.

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
to `main` for an indicator with `view = "primary_overlay"` and `indicator`
otherwise. Scale is `price`,
style is `line`, display is `value` (`none` for boolean outputs), width is `1`,
offset is `0`, overlay and layer are `true`, and data kind, mark, and axis are
inferred from the value type, style, pane, and scale. A numeric plot without a
color slot gets a deterministic slot named after its output ID.

Generic `plot` styles are `line`, `step`, `histogram`, and `area`. Panes are
`main`, `indicator`, or `named:<number>`;
scales are `price` or `independent`; display is `value` or `none`; and precision
is `instrument`, `decimals:<n>`, or `step:<number>`. Boolean plots
are output-only unless a presentation declaration references them.

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
indicator("Status", id = "catalog.status", view = "table")
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

## Common patterns

Ordinary plot:

```text
indicator("Momentum", id = "catalog.momentum",
          value_precision = "base_ticker_min_tick")
value = chart.close - chart.close[10]
plot(value, id = "value", title = "Momentum")
```

Overlay:

```text
indicator("Moving average", id = "catalog.ma", view = "primary_overlay")
average = ta.sma(chart.close, 20)
plot(average, id = "average", title = "Average")
```

Table:

```text
indicator("Status", id = "catalog.status", view = "table")
trend = chart.close > chart.close[1]
plot(trend, id = "trend", title = "Trend", display = "none", overlay = false, layer = false)
column(title = "Trend", source = "trend", format = "boolean")
```

Histogram:

```text
indicator("Flow", id = "catalog.flow",
          value_precision = "base_ticker_min_tick")
flow = chart.close - chart.open
plot(flow, id = "flow", title = "Flow", style = "histogram")
```

Fill and conditional colors:

```text
indicator("Band", id = "catalog.band",
          value_precision = "base_ticker_min_tick")
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
`chart.directional_volume_available`, `chart.buy_qty`, `chart.sell_qty`,
`chart.trade.*`, and `open_interest`. Missing aligned data is `na`.
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

The language rejects remote requests, multi-timeframe contexts, strategies,
alerts, `varip`, maps, mutable drawing objects, renderer-owned table handles,
raw trade arrays, liquidations, depth, remote symbols, and cross-indicator
dependencies.

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
