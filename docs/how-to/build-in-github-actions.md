# Build In GitHub Actions

Use the composite action when a downstream distribution repository wants conda-pronto
to build release artifacts in CI.

The action is the public CI interface for conda-pronto-built binaries. Downstream
repositories, including conda-express, pass their own package set and artifact
name to this action instead of carrying a copy of the generic builder.

The action checks out conda-pronto, applies the input overrides to that checkout, and
builds and stamps the generic runtime from the checked-out source.

## Single-Platform Example

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: jezdez/conda-pronto@main
        id: pronto
        with:
          name: serpe
          packages: "python >=3.12, conda >=25.1"
          channels: "conda-forge"

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

Pin `jezdez/conda-pronto` to a tag or commit SHA for release builds.

## Embedded Bundle Example

Set `embed-bundle` when the runtime must bootstrap without network access:

```yaml
- uses: jezdez/conda-pronto@main
  id: pronto
  with:
    name: serpe
    embed-bundle: "true"
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
  - uses: jezdez/conda-pronto@main
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
