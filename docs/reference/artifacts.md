# Artifact Reference

Every `pronto build` writes a runtime binary plus metadata files. The runtime
binary is the final stamped artifact. Downstream signing and attestation
workflows run after conda-pronto writes these files.

## Layouts

| Layout | Binary name | Bundle file | Network during bootstrap |
| --- | --- | --- | --- |
| `none` | `NAME` | none | yes |
| `external` | `NAME` | `NAME.bundle.tar.zst` | optional |
| `embedded` | `NAMEz` | embedded in binary | no |

On Windows, binary filenames also include `.exe`.

## Metadata Files

For a `none` build named `serpe`, conda-pronto stages:

- `serpe` or `serpe.exe`
- `serpe.runtime.lock`
- `serpe.packages.txt`
- `serpe.info.json`
- `serpe.sha256`

When `--target-label` is used, the label is inserted into the stem, for example
`serpe-linux-64.info.json`.

For an `embedded` build, the stem uses the `z` suffix, for example
`serpez.info.json` or `serpez-linux-64.info.json`.

For an `external` build, conda-pronto also stages `serpe.bundle.tar.zst` or a
target-qualified equivalent.

## Stamped Runtime Data

conda-pronto appends a runtime data block to every staged binary. The block contains
the runtime lock, runtime metadata, command names, default prefix, docs URL,
bundle environment variable names, and the embedded bundle bytes for
`embedded` builds.

The data block ends with:

- format version
- header length
- bundle length
- header SHA256
- bundle SHA256, or the SHA256 of empty bytes when no embedded bundle is present
- conda-pronto runtime-data magic bytes

The generated runtime validates the stamped header at startup. For embedded
artifacts, it also verifies the bundle checksum before extracting package
archives during `bootstrap`.

The binary checksum in `.sha256` covers the final stamped artifact. The
conda-pronto release workflow also publishes GitHub Artifact Attestations for
the `pronto` CLI, runtime templates, and `SHA256SUMS` manifest.

Verify a downloaded release asset with:

```bash
gh attestation verify ./pronto-x86_64-unknown-linux-gnu \
  -R jezdez/conda-pronto \
  --signer-workflow jezdez/conda-pronto/.github/workflows/release.yml
```

Downstream distributions can add their own attestations or platform signing
after conda-pronto finishes staging their named artifacts.

## Info JSON

The info JSON contains:

- schema version
- artifact name
- layout
- conda platform
- binary filename
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
