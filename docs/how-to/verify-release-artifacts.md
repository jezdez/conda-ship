# Verify Release Artifacts

Use this guide when you need to check conda-ship-built artifacts before
publishing or wrapping them.

Verification has two layers:

- verify the conda-ship tools used by the build
- verify the runtime artifacts produced by the build

## Verify conda-ship Release Tools In GitHub Actions

The composite action downloads `cs`, `cs-template`, and `SHA256SUMS`
from a tagged conda-ship release. It verifies GitHub artifact attestations and
then checks SHA256 sums before running `cs`.

Use a tag:

```yaml
- uses: jezdez/conda-ship@0.3.0
```

Do not use a branch ref for release builds. Branch refs do not have matching
release assets.

Self-hosted runners must provide the GitHub CLI because the action calls
`gh attestation verify`.

conda-ship releases are immutable after publication. If a released asset set is
wrong, use a newer release tag instead of expecting the existing tag or files to
change.

## Verify Staged Checksums

Every `cs build` writes a `.sha256` file next to the runtime and metadata:

```bash
shasum -a 256 --check dist/demo.sha256
```

On Linux, `sha256sum` works too:

```bash
sha256sum --check dist/demo.sha256
```

The checksum file covers the staged runtime, runtime lock, package list, info
JSON, and external bundle when present.

## Inspect Artifact Metadata

Open the `.info.json` file:

```bash
python -m json.tool dist/demo.info.json
```

```{figure} ../../demos/verify.gif
:alt: Terminal recording of building release files, checking SHA256 sums, and inspecting artifact metadata.

Verify staged files and release metadata before publishing.
```

Check:

- `name`
- `layout`
- `platform`
- `binary`
- `bundle`
- `package_count`
- `checksums`

This file is intended for release tooling and package-manager wrappers. It
describes what conda-ship wrote, not what an external installer later did.

## Inspect The Runtime Lock

The staged `.runtime.lock` is the lock the runtime will use during bootstrap.
It should be reproducible from the committed source lockfile and
`[tool.conda-ship]`.

Use it to answer release questions such as:

- Which concrete conda packages are shipped?
- Which channels are recorded?
- Which platforms are present?

Do not edit it by hand. Change the source manifest or source lockfile instead,
then rebuild.

## Verify Bundle Contents

For external bundles, extract into a temporary directory and check that it
contains only top-level package archives:

```bash
mkdir -p /tmp/demo-bundle
tar --zstd -xf dist/demo.bundle.tar.zst -C /tmp/demo-bundle
find /tmp/demo-bundle -maxdepth 2 -type f
```

The runtime verifies package archive hashes against the runtime lock before
installing. Embedded bundles are verified by the runtime before extraction.

## Add Downstream Signing And Release Controls

conda-ship does not sign downstream runtime artifacts. Sign after `cs build`,
when the final files are staged and checksums are written.

In GitHub Actions, attest the complete `dist-path` output:

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

steps:
  - uses: jezdez/conda-ship@0.3.0
    id: cs

  - uses: actions/attest@59d89421af93a897026c735860bf21b6eb4f7b26 # v4.1.0
    with:
      subject-path: ${{ steps.cs.outputs.dist-path }}/*
```

That attests the runtime binary, `.runtime.lock`, `.packages.txt`,
`.info.json`, `.sha256`, and the optional external bundle as the output of the
downstream workflow. Verify a published file against that workflow identity:

```bash
gh attestation verify dist/demo \
  --repo OWNER/REPO \
  --signer-workflow OWNER/REPO/.github/workflows/release.yml
```

Good downstream controls include:

- GitHub Release artifact attestations
- GitHub release immutability
- Sigstore signing for uploaded artifacts
- in-toto provenance around the packaging workflow
- platform-specific signing for installer wrappers

Keep signing outside the generic runtime. A runtime built by conda-ship may be
wrapped by several downstream channels, and each channel owns its own trust
policy.
