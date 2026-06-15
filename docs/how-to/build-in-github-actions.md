# Build In GitHub Actions

Use the composite action when a downstream distribution repository wants
conda-ship to build release artifacts in CI.

The action is the public CI interface for conda-ship-built runtimes.
Downstream repositories, including conda-express, keep their package set in a
committed manifest and lockfile. The action reads that project input and stamps
a runtime instead of carrying a copy of the generic builder.

Pin the action source to a full conda-ship release commit SHA and pass the
matching conda-ship release through `conda-ship-version`. The action downloads the
configured `cs` and `cs-template` release assets, verifies their GitHub artifact
attestations and release `SHA256SUMS`, and stamps the generated runtime. It
runs `cs build --dry-run` before the real build so manifest, lockfile, naming,
template, install location, and bundle metadata issues fail before artifact
files are written.

GitHub-hosted runners already include the GitHub CLI used for attestation
verification. Self-hosted runners must provide `gh`.

## Single-Platform Example

The checked-out repository must contain `conda.toml` plus `conda.lock`,
`pyproject.toml` with `[tool.conda]` plus `conda.lock`, `pixi.toml` plus
`pixi.lock`, or `pyproject.toml` with `[tool.pixi]` plus `pixi.lock`. These
examples assume the manifest contains `[tool.conda-ship].runtime-name`,
`[tool.conda-ship].delegate-executable`, and a downstream runtime version, unless those
values are supplied as action inputs.

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: jezdez/conda-ship@FULL_RELEASE_COMMIT_SHA # X.Y.Z
        id: cs
        with:
          conda-ship-version: "X.Y.Z"

      - uses: actions/upload-artifact@v4
        with:
          name: ${{ steps.cs.outputs.asset-name }}
          path: ${{ steps.cs.outputs.dist-path }}
```

When the action is invoked by an exact release tag, `conda-ship-version` can be
omitted for backwards compatibility. Release workflows should prefer full
commit SHA pins.

## Project Root Example

When the downstream manifest lives below the repository root, point the action
at that directory:

```yaml
steps:
  - uses: actions/checkout@v4

  - uses: jezdez/conda-ship@FULL_RELEASE_COMMIT_SHA # X.Y.Z
    id: cs
    with:
      conda-ship-version: "X.Y.Z"
      root: dist/demo
```

The action does not run a solve, generate a manifest, or refresh a lockfile.
Update and commit the lockfile before running release builds.
Release-job metadata such as `runtime-name`, `runtime-version`,
`delegate-executable`, `docs-url`, `install-scheme`, `install-name`, and
`installer` can come from the manifest or from action inputs. The action passes
those inputs to `cs build --dry-run`, so validation still happens in
conda-ship.

## External Bundle Example

Set `artifact-layout` to `external` when you want to distribute the runtime and
package bundle as separate files:

```yaml
- uses: jezdez/conda-ship@FULL_RELEASE_COMMIT_SHA # X.Y.Z
  id: cs
  with:
    conda-ship-version: "X.Y.Z"
    artifact-layout: external

- uses: actions/upload-artifact@v4
  with:
    name: ${{ steps.cs.outputs.asset-name }}
    path: ${{ steps.cs.outputs.dist-path }}
```

## Embedded Bundle Example

Set `artifact-layout` to `embedded` when the runtime must bootstrap without network
access:

```yaml
- uses: jezdez/conda-ship@FULL_RELEASE_COMMIT_SHA # X.Y.Z
  id: cs
  with:
    conda-ship-version: "X.Y.Z"
    artifact-layout: embedded
```

The output runtime uses the configured runtime name by default. Set the
`artifact-name` input when the staged artifact should have a distinct command
name.

## Matrix Builds

Run the action across operating systems to produce platform-specific
runtimes:

```yaml
strategy:
  fail-fast: false
  matrix:
    include:
      - os: ubuntu-latest
        layout: online
        runtime_name: demo
        installer: standalone
      - os: macos-15-intel
        layout: embedded
        runtime_name: demo
        installer: homebrew
      - os: macos-15
        layout: embedded
        runtime_name: demo
        installer: homebrew
      - os: windows-latest
        layout: online
        runtime_name: demo
        installer: standalone

runs-on: ${{ matrix.os }}

steps:
  - uses: actions/checkout@v4

  - uses: jezdez/conda-ship@FULL_RELEASE_COMMIT_SHA # X.Y.Z
    id: cs
    with:
      conda-ship-version: "X.Y.Z"
      artifact-layout: ${{ matrix.layout }}
      runtime-name: ${{ matrix.runtime_name }}
      runtime-version: ${{ github.ref_name }}
      delegate-executable: conda
      docs-url: https://example.com/demo/
      install-scheme: conda-home
      install-name: demo
      installer: ${{ matrix.installer }}
```

Each job emits an asset name qualified with the runner target triple.

## Downstream Release Preparation

Use `dist-path` as the source of truth for artifact uploads. It contains the
runtime, optional external bundle, `.info.json`, `.runtime.lock`,
`.packages.txt`, and `.sha256` files for that build. The individual path
outputs are still available when release tooling or package-manager wrappers
need to address one file directly.

## Attest Runtime Outputs

For release workflows, attest the complete `dist-path` before publishing or
wrapping the files. This records the GitHub workflow identity that produced the
runtime output set.

```{warning}
Use the latest reviewed `actions/attest` release in your workflow and pin it by
commit SHA. The SHA below is an example, not a recommendation to keep using that
exact revision indefinitely.
```

```yaml
permissions:
  contents: read
  id-token: write
  attestations: write
  artifact-metadata: write

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: jezdez/conda-ship@FULL_RELEASE_COMMIT_SHA # X.Y.Z
        id: cs
        with:
          conda-ship-version: "X.Y.Z"

      - uses: actions/attest@59d89421af93a897026c735860bf21b6eb4f7b26 # v4.1.0
        with:
          subject-path: ${{ steps.cs.outputs.dist-path }}/*

      - uses: actions/upload-artifact@v4
        with:
          name: ${{ steps.cs.outputs.asset-name }}
          path: ${{ steps.cs.outputs.dist-path }}
```

The attestation covers the runtime binary, `.runtime.lock`, `.packages.txt`,
`.info.json`, `.sha256`, and the optional external bundle for that job. Keep
package-manager signing and platform installer signing as separate downstream
steps when those distribution channels require them.
