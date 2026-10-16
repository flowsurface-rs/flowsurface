# Contributing catalog indicators

Add a `.ind` file to `sources/` with a `catalog.<name>` ID and `<name>.ind`
filename. Open a pull request explaining its inputs, plots, and warm-up needs.

Run both checks from the repository root before submitting:

```sh
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- check indicators/sources --namespace catalog
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- test indicators/sources --namespace catalog
```

## Developer Certificate of Origin

Sources and documentation under `indicators/` use [Apache-2.0](LICENSE).
Sign off each commit with your real name and email for the Developer
Certificate of Origin (DCO):

```sh
git commit -s
```

This adds a line to the commit message:

```text
Signed-off-by: Your Name <you@example.com>
```

By signing off, you certify that you have the right to submit the work under
this license.
