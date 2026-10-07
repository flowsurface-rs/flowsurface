# Indicator sources

Each indicator is one `.ind` file containing its calculations and presentation.
The filename must match the final segment of its ID: `community.example` goes
in `example.ind`. Short names are case-sensitive and must be unique across all
loaded collections, even when namespaces differ.

| Collection | Namespace | License | Delivery |
| --- | --- | --- | --- |
| `core/` | `core.*` | GPL-3.0-or-later | Bundled; synchronized into the data directory at startup |
| [catalog/sources/](catalog/README.md) | `catalog.*` | Apache-2.0 | Optional; install manually |

Flowsurface reserves `core.*` for managed sources; installed sources may use
other valid namespaces. Managed core files are replaced at startup when they
differ from the bundled collection. Edit repository sources for development.

## Install and run

Copy a source into `<data-directory>/indicators/installed/`, then restart
Flowsurface. The data directory is `$FLOWSURFACE_DATA_PATH` when set, otherwise
the platform data directory's `flowsurface/` folder.

For development, pass an explicit file (repeat the option for more files):

```sh
flowsurface --indicator-path /path/to/example.ind
```

Or run directly from both repository collections, from the repository root:

```sh
FLOWSURFACE_INDICATORS_PATH="$PWD/indicators" cargo run
```

This replaces the managed and installed source directories with `core/` and
`catalog/sources/` under that root. Sources load once at startup: core, installed,
then explicit files. Invalid installed files are skipped with diagnostics;
core errors and duplicate short names fail startup.

For authoring, see the [language reference](../indicator-engine/LANGUAGE.md)
and [CLI workflow](../indicator-engine/README.md#cli). Check and smoke-test a
source before installing it.
