# Flowsurface indicator catalog

This directory is the standalone-shaped home of the open `catalog.*` indicator
collection. The `.ind` sources live under `sources/` so repository metadata can
stay at the collection root. It is Apache-2.0 licensed and contains source-only `.ind` files;
the Flowsurface application does not bundle these indicators automatically.

## Source rules

Catalog sources use the `catalog.<indicator>` namespace. The filename must be
the final ID segment, so `catalog.macd` belongs in `macd.ind`. The `sources/`
directory is intentionally flat, and each short name must be unique.

## Local validation

The CLI uses the same parser, type checker, and runtime as the application.
Building it requires the Rust toolchain; a built executable does not require
Rust to run.

From the current monorepo:

```text
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- check indicators/catalog/sources --namespace catalog
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- test indicators/catalog/sources --namespace catalog
```

For a new source, use:

```text
cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- new indicators/catalog/sources --id catalog.example --title "Example"
```

`check` validates syntax, types, filenames, IDs, and duplicate short names.
`test` also evaluates deterministic synthetic OHLCV, directional-volume, trade,
and open-interest frames, including an open-bar preview.

## Future repository shape

When extraction is useful, this directory will become the root of the
`flowsurface-indicators` repository:

```text
flowsurface-indicators/
├── sources/*.ind
├── README.md
├── CONTRIBUTING.md
└── LICENSE
```

The engine and CLI will live separately in `indicator-engine`; this repository
will not copy the Rust runtime or add a second test harness. CI will invoke the
CLI against the flat source collection.

## Contributions and license

Contributions must be submitted under the Apache-2.0 license and include a
DCO sign-off. See [`CONTRIBUTING.md`](CONTRIBUTING.md). Maintainer review and
merge approval are required for every contribution.
