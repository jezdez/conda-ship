# Artifact Reference

Every `cs build` writes a runtime plus metadata files. The runtime
is the final stamped binary artifact. Downstream signing and attestation
workflows run after conda-ship writes these files.

## conda-ship Release Assets

Tagged conda-ship releases publish the builder assets that the GitHub Action
downloads:

`cs-<target>`
: Builder CLI for the target platform.

`cs-template-<target>`
: Generic runtime template for the target platform.

`SHA256SUMS`
: Checksums for release assets.

The PyPI release for the same tag publishes platform wheels that install `cs`
and `cs-template` into the Python environment's scripts directory, plus a
source distribution for packaging systems. Published release assets are
immutable. Fixes use a new tag rather than replacing files under an existing
tag.

## Layouts

| Layout | Runtime | Bundle file | Network during bootstrap |
| --- | --- | --- | --- |
| `online` | `RUNTIME` | none | yes |
| `external` | `RUNTIME` | `RUNTIME.bundle.tar.zst` | optional |
| `embedded` | `RUNTIME` | embedded in binary | no |

When `[tool.conda-ship].artifact-name` or `--artifact-name` is set, all layouts use
that value for the staged runtime and metadata stem. On Windows, binary
filenames also include `.exe`.

For the difference between `runtime-name` and `artifact-name`, see
{doc}`names`.

## Runtime Update Packages

`cs package-update` writes a dependency-free native `.conda` package from a
finalized `online` or `embedded` runtime with direct update ownership. This is
separate from `cs build` and does not add a file to the normal `dist/` set
unless that directory is selected explicitly.

The package contains one executable payload plus normal conda package metadata:

- `bin/ARTIFACT_NAME` on Unix
- `ARTIFACT_NAME.exe` on Windows

The package name, version, build number, and platform come from the executable
stamp and its artifact info JSON. The package has no runtime dependencies. It
is transport for the executable update engine and is not installed into the
managed prefix.

The command refuses to overwrite an existing output. Its `--json` result
reports SHA256 and size values for both the package and finalized executable
payload. Channel indexing and upload remain downstream release operations.

## Bundle Contents

Bundles are a transport for the conda package archives already named in the
runtime lock. They are not channel mirrors and do not use a `linux-64/` or
`noarch/` directory layout.

`external` and `embedded` bundles contain top-level `.conda` and `.tar.bz2`
package archive files. The runtime matches those filenames against the stamped
lockfile and verifies package SHA256 values before installing from them.

External bundle directories may contain unrelated files, but conda-ship only
indexes top-level conda package archives and skips symbolic links. Embedded
bundles are stricter because they are extracted from a tar archive: every entry
must be a top-level regular `.conda` or `.tar.bz2` file. Directory entries,
nested paths, symbolic links, hard links, and other file types are rejected.

## Metadata Files

For an `online` build with runtime `demo`, conda-ship stages:

- `demo` or `demo.exe`
- `demo.runtime.lock`
- `demo.packages.txt`
- `demo.info.json`
- `demo.sha256`

When `--target-label` is used, the label is inserted into the stem, for example
`demo-linux-64.info.json`.

If `artifact-name = "demo-cli"` or `--artifact-name demo-cli` is set, the artifact
stem uses that explicit name for any layout, for example `demo-cli.info.json`
or `demo-cli-linux-64.info.json`.

For an `external` build, conda-ship also stages `demo.bundle.tar.zst` or a
target-qualified equivalent.

These files describe the staged release output. During automatic first-run
bootstrap, the generated runtime also writes managed-prefix metadata such as
`conda-meta/history` and `conda-meta/initial-state.explicit.txt` inside the
install path.

## Stamped Runtime Data

conda-ship appends a runtime data block to every staged runtime. The block
contains the runtime lock, runtime and artifact identity, version, platform,
delegate executable, install scheme, install name, docs URL, installer,
optional executable update policy, bundle and offline environment variable
names, and the embedded bundle bytes for `embedded` builds. The universal
`CONDA_SHIP_PREFIX` override is runtime behavior rather than a stamped variable
name.

The data block ends with:

- format version
- header length
- bundle length
- header SHA256
- bundle SHA256, or the SHA256 of empty bytes when no embedded bundle is present
- conda-ship runtime-data magic bytes

The generated runtime validates the stamped header at startup. For
embedded artifacts, it also verifies the bundle checksum before extracting package
archives during automatic bootstrap.

The binary checksum in `.sha256` covers the final stamped artifact. The
conda-ship release workflow also publishes GitHub Artifact Attestations for
the `cs` CLI, runtime templates, and `SHA256SUMS` manifest.

If signing or another downstream step changes the staged executable, the
original `.sha256` continues to describe the `cs build` output. Pass the
finalized file to `cs package-update --binary`. The command snapshots those
bytes and reports the finalized payload digest in its JSON output.

Verify a downloaded release asset with:

```bash
gh attestation verify ./cs-x86_64-unknown-linux-gnu \
  -R jezdez/conda-ship \
  --signer-workflow jezdez/conda-ship/.github/workflows/release.yml
```

Downstream distributions can add their own attestations or platform signing
after conda-ship finishes staging their runtime artifacts.

## Info JSON

The info JSON contains:

- schema version
- artifact stem, artifact name, and runtime name
- runtime version
- layout
- conda platform
- optional executable update policy
- runtime filename
- optional external bundle filename
- lock filename
- package list filename
- package count
- SHA256 checksums

## Package List

The package list is tab-separated and contains:

- package name
- version
- build string
- package URL
- SHA256, when available from the lockfile
