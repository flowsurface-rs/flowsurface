# Indicator sources

Indicator sources are flat, installable `.ind` files. The filename is the final
segment of the DSL ID, for example `rsi.ind` for `flowsurface.rsi`; the `id`
inside the source remains authoritative. Short names are case-sensitive and
must be unique across the complete startup catalog.
See [`indicator-engine/LANGUAGE.md`](../indicator-engine/LANGUAGE.md) for the
declaration reference and sparse authoring examples.

## Repository collections

`indicators/core` contains the six sources that Flowsurface manages and
guarantees at runtime:

- `volume.ind`
- `rsi.ind`
- `moving-averages.ind`
- `bar-analysis.ind`
- `open-interest.ind`
- `anchored-vwap.ind`

The application embeds these sources as delivery templates and synchronizes
them into its persistence `indicators/core` directory at startup.

`indicators/catalog` contains optional sources kept in the repository for
development and future distribution. It is not bundled automatically.

To run the application directly from the repository sources, set
`FLOWSURFACE_INDICATORS_PATH` to the repository collection root. This loads
`core/` and `catalog/` from that root and leaves the rest of the data directory
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
flowsurface --indicator-path ./indicators/catalog/example.ind
```

The managed core sources load first, followed by files in the installed
directory and then explicit development files. A duplicate short name anywhere
in those sources is a startup error. A malformed installed or explicit file is
quarantined while unrelated valid files continue loading.

## Development loop

1. Run `flowsurface-indicator new [directory] --id namespace.name --title "Title"`, or copy a nearby `.ind` file.
2. Edit the source.
3. Run `flowsurface-indicator check path/to/name.ind`.
4. Copy the file into the installed collection or pass its explicit path with `--indicator-path`.
5. Restart Flowsurface.

The CLI creates the destination directory when needed, permits an existing
collection directory, and refuses to overwrite an existing short-name file.
Existing full-ID files such as `community.example.ind` must be renamed to
`example.ind` before they can be loaded.
