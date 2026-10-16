# Contributing catalog indicators

The catalog is a curated collection. Open a pull request with the `.ind` source
and explain its intended inputs, plots, and any warm-up requirements.

Run both checks from the repository root before submitting:

```text
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- check indicators/catalog/sources --namespace catalog
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- test indicators/catalog/sources --namespace catalog
```

Use the final ID segment as the filename. For example,
`catalog.example` must be stored as `example.ind`.

## Developer Certificate of Origin

Contributions are accepted under the Developer Certificate of Origin (DCO).
Sign off each commit with your real name and email:

```text
git commit -s
```

The commit must contain a line in this form:

```text
Signed-off-by: Your Name <you@example.com>
```

By signing off, you certify that you have the right to submit the work under
the project license. The source and documentation in this catalog are licensed
under Apache-2.0; see [`LICENSE`](LICENSE).
