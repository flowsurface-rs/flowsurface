# Indicator sources

Indicator sources are flat, installable `.ind` files. The filename is the final
segment of the DSL ID, for example `rsi.ind` for `catalog.rsi`; the `id`
inside the source remains authoritative. Short names are case-sensitive and
must be unique across the complete startup catalog.
See [`indicator-engine/LANGUAGE.md`](../indicator-engine/LANGUAGE.md) for the
declaration reference and sparse authoring examples.

The three namespaces are intentionally separate:

- `core.*` is the GPL-3.0-or-later bundled first-party collection maintained by
  Flowsurface.
- `catalog.*` is the Apache-2.0 open catalog curated by Flowsurface and its
  contributors.

## Repository collections

`indicators/core` is the source of truth for the app-managed first-party
collection. Every immediate `.ind` file in this directory is embedded as a
delivery template and synchronized into the persistence `indicators/core`
directory at startup.

`indicators/catalog/sources` contains the open catalog sources. It is not bundled
automatically. Its standalone-repository preparation is documented in
[`catalog/README.md`](catalog/README.md).

To run the application directly from the repository sources, set
`FLOWSURFACE_INDICATORS_PATH` to the repository collection root. This loads
`core/` and `catalog/sources/` from that root and leaves the rest of the data directory
unchanged:

```text
FLOWSURFACE_INDICATORS_PATH="$PWD/indicators" cargo run
```

There are no per-indicator directories, `main.ind` files, JSON fixtures, or
presentation sidecars in the repository.

## Installing a source

Copy a `.ind` file directly into the persistence installed collection:

```text
$FLOWSURFACE_DATA_PATH/indicators/installed/example.ind
```

When `FLOWSURFACE_DATA_PATH` is not set, use the platform data directory for
Flowsurface. Development files can be supplied with one or more explicit
options:

```text
flowsurface --indicator-path ./indicators/catalog/sources/example.ind
```

The managed core sources load first, followed by files in the installed
directory and then explicit development files. A duplicate short name anywhere
in those sources is a startup error. A malformed installed or explicit file is
quarantined while unrelated valid files continue loading.

## Development loop

1. Run `cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- new [directory] --id catalog.name --title "Title"`, or copy a nearby `.ind` file.
2. Edit the source.
3. Run `cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- check path/to/name.ind --namespace catalog`.
4. Run `cargo run -p flowsurface-indicator-engine --bin flowsurface-indicator -- test path/to/name.ind --namespace catalog`.
5. Run the same `check` or `test` command on the containing directory with `--namespace catalog` to validate the collection.
6. Copy the file into the installed collection or pass its explicit path with `--indicator-path`.
7. Restart Flowsurface.

The CLI creates the destination directory when needed, permits an existing
collection directory, and refuses to overwrite an existing short-name file.
Existing full-ID files such as `catalog.example.ind` must be renamed to
`example.ind` before they can be loaded. The CLI smoke test uses deterministic
synthetic market data and does not require a running exchange connection.
