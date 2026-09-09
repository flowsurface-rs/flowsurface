# Flowsurface indicator engine

This directory is Apache-2.0 licensed. The Flowsurface application and its
bundled `core.*` sources remain GPL-3.0-or-later; indicator source licensing is
separate from the engine license.

The indicator engine is a source-only runtime for live chart indicators. Each
installable unit is one flat `.ind` file. The source declares its identity,
inputs, plots, panel placement, and runtime behavior. See
[`LANGUAGE.md`](LANGUAGE.md) for the declaration reference.

The host builds an `IndicatorFrame` from the primary chart. Selected markets
provide comparison Kline series for presentation; the DSL frame, open interest,
and trade data are base-ticker inputs. It evaluates loaded closed bars once for
bootstrap, then evaluates the current open bar as a preview from the last
committed state. Missing aligned data is `na`.

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
`catalog/sources/` instead. Source edits take effect after restart. Invalid installed
files are quarantined without blocking unrelated valid files; core source
errors are fatal.

The CLI is built from this package and uses the same parser, type checker, and
runtime as the application:

Building it requires Rust; the resulting native executable does not.

    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- new . --id catalog.example --title "Example"
    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- check ./example.ind
    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- test ./example.ind

For the repository catalog, run:

    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- check indicators/catalog/sources --namespace catalog
    cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- test indicators/catalog/sources --namespace catalog

Use `--namespace core` for the bundled collection. Namespace checking is an
optional CLI policy; the engine itself accepts any valid namespace.

`check` accepts one source or a flat directory of sources. It validates source
syntax, types, filenames, IDs, and short-name uniqueness. `test` also
instantiates each source with its default inputs, evaluates deterministic
synthetic closed bars, and evaluates an open-bar preview. It does not replace
Flowsurface host and chart integration tests.

Indicators do not have `main.ind`, `manifest.toml`, or `presentation.toml`
sidecars. The source filename must match the final segment of its DSL ID.
Existing full-ID filenames are diagnosed and skipped, and any duplicate short
name across core, installed, and explicit sources is fatal at startup.

The intended repository boundary is `indicator-engine/` for the reusable Rust
library and CLI, `flowsurface/` for the application and `core.*` sources, and
`flowsurface-indicators/` for the Apache-2.0 `catalog.*` source collection.
The current monorepo keeps those directories together until extraction is
useful. Apache-2.0 is compatible with GPLv3 for the application-to-engine
relationship; proprietary indicator/interface questions remain deferred for
legal review. See the [Apache compatibility guidance](https://www.apache.org/licenses/GPL-compatibility)
and [GNU GPL FAQ](https://www.gnu.org/licenses/gpl-faq.en.html).
