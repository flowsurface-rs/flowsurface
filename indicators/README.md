# Indicator sources

Each indicator is a `.ind` file defining its calculations and presentation.
Its filename must match the final ID segment: `alice.rsi` goes in `rsi.ind`.
Full IDs must be unique; matching filenames can coexist in separate folders.
Folder names do not assign IDs.

| Collection | IDs | License | Availability |
| --- | --- | --- | --- |
| [Bundled](../data/indicators/README.md) | `core.*` | [GPL-3.0-or-later](../data/indicators/LICENSE) | Included in Flowsurface |
| [Catalog](sources/) | `catalog.*` | [Apache-2.0](LICENSE) | Optional; install manually |

## Install and run

Copy `.ind` files or folders into `<data-directory>/indicators/`,
then restart Flowsurface. The data directory is `$FLOWSURFACE_DATA_PATH` when
set, otherwise the platform data directory's `flowsurface/` folder.
Flowsurface creates this directory at startup and searches subfolders,
skipping symlinks.

For development, pass an explicit file (repeat the option for more files):

```sh
flowsurface --indicator-path /path/to/example.ind
```

Or use the repository catalog as the installed-source directory:

```sh
FLOWSURFACE_INDICATORS_PATH="$PWD/indicators/sources" cargo run
```

Bundled indicators remain available when using either option.

Sources load once at startup. Invalid installed sources are skipped with
diagnostics. Bundled IDs take precedence over installed duplicates; otherwise,
the first installed source with an ID wins.

For authoring, see the [language reference](../indicator-engine/LANGUAGE.md)
and [CLI workflow](../indicator-engine/README.md#cli). Run `check` and `test`
before installing a source.

## Catalog contributions

Add sources to `sources/` using `catalog.<name>` IDs and `<name>.ind` filenames.
See [CONTRIBUTING.md](CONTRIBUTING.md) for validation, Apache-2.0 licensing,
and DCO sign-off requirements.
