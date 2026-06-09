# Release Assets

This page lists the artifact names conda-ship itself publishes and the artifact
names `cs build` stages for downstream runtimes.

## conda-ship Release Assets

Tagged conda-ship releases publish builder assets:

`cs-<target>`
: Builder CLI for the target platform.

`cs-template-<target>`
: Generic runtime template for the target platform.

`SHA256SUMS`
: Checksums for release assets.

The PyPI release for the same tag publishes platform wheels that install the
`cs` and `cs-template` executables into the Python environment's scripts
directory, plus a source distribution for packaging systems.

Published release assets are immutable. A tag represents one complete asset set;
fixes use a new tag rather than replacing files under an existing tag.

```{important}
Do not script release repair by re-uploading files to an existing tag. Published
conda-ship releases are immutable; publish a newer tag for fixes.
```

Target examples:

```text
x86_64-unknown-linux-gnu
aarch64-unknown-linux-gnu
x86_64-apple-darwin
aarch64-apple-darwin
x86_64-pc-windows-msvc
aarch64-pc-windows-msvc
```

Windows assets use `.exe`.

## Downstream Runtime Artifacts

`cs build` writes downstream artifacts to `dist/` by default.

For an online runtime named `demo`:

```text
demo
demo.info.json
demo.packages.txt
demo.runtime.lock
demo.sha256
```

For an external runtime:

```text
demo
demo.bundle.tar.zst
demo.info.json
demo.packages.txt
demo.runtime.lock
demo.sha256
```

For an embedded runtime:

```text
demoz
demoz.info.json
demoz.packages.txt
demoz.runtime.lock
demoz.sha256
```

## Target Labels

Release workflows usually add a target label:

```bash
cs build \
  --target x86_64-unknown-linux-gnu \
  --target-label x86_64-unknown-linux-gnu
```

That produces names such as:

```text
demo-x86_64-unknown-linux-gnu
demo-x86_64-unknown-linux-gnu.info.json
demo-x86_64-unknown-linux-gnu.runtime.lock
```

For Windows targets, the runtime binary gets `.exe`:

```text
demo-x86_64-pc-windows-msvc.exe
```

The metadata files keep the same stem without `.exe`.

## GitHub Action Outputs

The composite action exposes:

`dist-path`
: Directory containing all generated files.

`binary-path`
: Runtime binary path.

`asset-name`
: Runtime binary filename.

`info-path`
: `.info.json` path.

`lock-path`
: `.runtime.lock` path.

`package-list-path`
: `.packages.txt` path.

`checksums-path`
: `.sha256` path.

`bundle-path`
: External bundle path when present.
