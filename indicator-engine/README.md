# Flowsurface indicator engine

The indicator engine is a source-only runtime for live chart indicators. Each
installable unit is one flat `.ind` file. The source declares its identity,
inputs, plots, panel placement, and runtime behavior. See
[`LANGUAGE.md`](LANGUAGE.md) for the declaration reference.

The host builds an `IndicatorFrame` from the primary chart and selected markets.
It evaluates loaded closed bars once for bootstrap, then evaluates the current
open bar as a preview from the last committed state. Missing aligned data is
`na`.

The runtime intentionally excludes remote requests, other timeframes,
strategies, alerts, correction replay, `varip`, mutable scene objects, and
cross-indicator dependencies. Arrays, records, persistent state, history
indexing, bounded loops, non-recursive functions, and the numeric intrinsics
used by the catalog remain available. Stateful TA windows default to a maximum
length of 5,000, and collection growth is bounded by runtime resource limits;
an indicator that exceeds a limit is quarantined rather than silently changed.

## Source workflow

The application loads the managed `indicators/core/` collection, the optional
installed collection, and any explicit `.ind` files supplied by
`--indicator-path` once during startup. For repository development,
`FLOWSURFACE_INDICATORS_PATH` can point at a source root containing `core/` and
`catalog/` instead. Source edits take effect after restart. Invalid installed
files are quarantined without blocking unrelated valid files; core source
errors are fatal.

```text
flowsurface-indicator new . --id community.example --title "Example"
flowsurface-indicator check ./example.ind
```

The CLI does not run runtime smoke fixtures, and indicators do not have
`main.ind`, `manifest.toml`, or `presentation.toml` sidecars. The source
filename must match the final segment of its DSL ID. Existing full-ID filenames
are diagnosed and skipped, and any duplicate short name across core, installed,
and explicit sources is fatal at startup.
