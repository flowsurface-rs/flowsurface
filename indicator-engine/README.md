# Flowsurface indicator engine

The Rust library and `flowsurface-indicator` CLI compile and run `.ind` chart
indicators. See [LANGUAGE.md](LANGUAGE.md) for authoring and
[indicator catalog](https://github.com/flowsurface-rs/indicators#install-and-run) for installation.

## CLI

Build once with Rust, then use the native executable:

```sh
cargo build --locked -p flowsurface-indicator-engine --bin flowsurface-indicator
target/debug/flowsurface-indicator new /tmp/my-indicators --id community.example --title "Example"
target/debug/flowsurface-indicator check /tmp/my-indicators
target/debug/flowsurface-indicator test /tmp/my-indicators
target/debug/flowsurface-indicator catalog /tmp/my-indicators --output /tmp/catalog.json
```

`new` creates `<short-name>.ind` without overwriting an existing file.
`check` accepts a file or recursive collection directory and checks syntax, types, IDs,
filenames, and duplicate full IDs. Add `--namespace <prefix>` to enforce a
collection's namespace.

`test` also runs default inputs on synthetic bars and an open-bar preview using
the [application limits](LANGUAGE.md#resource-limits). Verify formulas, other
input settings, performance on real histories, and chart appearance separately.

### Catalog generation

`catalog <file.ind|directory> --output <catalog.json> [--namespace <prefix>]`
uses the same recursive discovery and compile-time validation as `check`. Invalid
sources, filenames, namespace mismatches, and duplicate IDs fail the whole export.
Run `test` separately for runtime smoke validation.

The JSON contains `schema_version` (currently `1`), `generator_version`,
`language_version` (currently `1`), and an `indicators` array sorted by full ID.
Each entry contains:

| Field                  | Meaning                                                                                           |
| ---------------------- | ------------------------------------------------------------------------------------------------- |
| `id`, `name`           | Full indicator ID and declared title                                                              |
| `description`          | Declared description, or `null`                                                                   |
| `author`               | Declared `{ "name": "...", "url": "..." }`, or `null`                                             |
| `source_url`           | Declared upstream source URL, or `null`                                                           |
| `path`                 | Source path relative to the input directory (or filename for a single file), using `/` separators |
| `sha256`, `size_bytes` | SHA-256 hex digest and byte count of the exact validated UTF-8 source                             |
| `bases`                | Supported bar bases: `time`, `tick`, or both                                                      |

The generator version records which engine validated the sources; it does not
claim a minimum compatible app version.

Output is deterministic for the same sources and generator version, with no local
absolute paths or build timestamps. The output's parent directory must exist.
The command replaces the output only after validation, serialization, and writing
a temporary file beside it succeed. An `.ind` output filename is rejected.

The engine is [Apache-2.0 licensed](LICENSE). The application and bundled
indicator sources are GPL-3.0-or-later; indicator source licenses are separate.
