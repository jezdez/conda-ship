# Build In GitHub Actions

Use the composite action when a downstream distribution repository wants conda-pronto
to build release artifacts in CI.

The action is the public CI interface for conda-pronto-built binaries. Downstream
repositories, including conda-express, keep their package set in a committed
manifest and lockfile. The action reads that project input and stamps a named
runtime artifact instead of carrying a copy of the generic builder.

Pin the action to a conda-pronto release tag. The action downloads the matching
`pronto` and `pronto-runtime-template` release assets, verifies them against the
release `SHA256SUMS`, and stamps the generated runtime artifact.

## Single-Platform Example

The checked-out repository must contain either `conda.toml` plus `conda.lock`
or `pixi.toml` plus `pixi.lock`.

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: jezdez/conda-pronto@v0.1.0
        id: pronto
        with:
          name: serpe

      - uses: actions/upload-artifact@v4
        with:
          name: ${{ steps.pronto.outputs.asset-name }}
          path: |
            ${{ steps.pronto.outputs.binary-path }}
            ${{ steps.pronto.outputs.info-path }}
            ${{ steps.pronto.outputs.lock-path }}
            ${{ steps.pronto.outputs.package-list-path }}
            ${{ steps.pronto.outputs.checksums-path }}
```

Use a tag for release builds. Branch refs do not have matching release assets.

## Project Root Example

When the downstream manifest lives below the repository root, point the action
at that directory:

```yaml
steps:
  - uses: actions/checkout@v4

  - uses: jezdez/conda-pronto@v0.1.0
    id: pronto
    with:
      name: serpe
      root: dist/serpe
```

The action does not run a solve, generate a manifest, or refresh a lockfile.
Update and commit the lockfile before running release builds.

## Embedded Bundle Example

Set `layout` to `embedded` when the runtime must bootstrap without network
access:

```yaml
- uses: jezdez/conda-pronto@v0.1.0
  id: pronto
  with:
    name: serpe
    layout: embedded
```

The output binary uses the `z` suffix, for example `serpez` on Unix or
`serpez.exe` on Windows.

## Matrix Builds

Run the action across operating systems to produce platform-specific binaries:

```yaml
strategy:
  fail-fast: false
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]

runs-on: ${{ matrix.os }}

steps:
  - uses: actions/checkout@v4

  - uses: jezdez/conda-pronto@v0.1.0
    id: pronto
    with:
      name: serpe
```

Each job emits an asset name qualified with the runner target triple.

## Downstream Release Preparation

Use the action output paths as the source of truth for release uploads and
package-manager wrappers. A downstream repository can upload the binary,
`.info.json`, `.runtime.lock`, `.packages.txt`, and `.sha256` files together so
users and packagers can audit exactly what was built.
