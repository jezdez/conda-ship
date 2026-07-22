# Trust And Provenance

conda-ship has a narrow trust model. It verifies the inputs it consumes and the
package archives it installs, but downstream distributions still own signing and
release policy for their final artifacts.

## Build Tool Trust

The GitHub Action downloads conda-ship release assets:

- `cs-<target>`
- `cs-template-<target>`
- `SHA256SUMS`

It verifies GitHub artifact attestations for those files and checks the
published SHA256 sums before running `cs`.

This protects the builder path from accidentally executing an unverified
downloaded binary in CI.

Published conda-ship GitHub releases are immutable. The release workflow creates
a draft release, uploads the complete asset set, and publishes the release once.
After publication, the tag and assets are not replaced. If a release is wrong,
the project should publish a new version instead of modifying the existing one.

## Source Lock Trust

The source lockfile is committed project input. conda-ship assumes the
downstream project reviewed and committed that lockfile intentionally.

conda-ship does not solve loose matchspecs in the GitHub Action. That avoids a
release build changing package records because a channel changed between
workflow runs.

## Package Archive Trust

The runtime lock contains concrete package records. For bundle builds,
conda-ship requires SHA256 metadata so downloaded package archives can be
verified.

During bootstrap:

- online installs use the stamped runtime lock
- external bundle installs match local package archives to the lock
- embedded bundle installs verify the embedded bundle before extraction

The runtime rejects package archive mismatches instead of silently installing
unexpected files.

## Executable Update Trust

An update-enabled runtime resolves native `.conda` packages from its stamped
channel. Before staging an executable it verifies:

- the package size and SHA256 recorded in repodata
- package name, version, build number, platform, dependencies, and payload count
- the payload size and SHA256 recorded by the package
- the executable stamp, runtime and artifact identity, platform, and version
- update ownership, channel, package, build number, and instruction continuity

The runtime accepts only a newer version or build number. It refuses a package
that rotates the stamped update source or changes ownership.

These checks do not verify GitHub attestations, a provider-specific signature,
or an external package manager's signature. Downstream publication and signing
policy remains separate from the native update package format.

## Runtime Artifact Trust

Every staged build writes checksums and metadata:

- `.sha256`
- `.info.json`
- `.runtime.lock`
- `.packages.txt`

These files describe and verify what conda-ship produced. They are not a
replacement for signing.

## Downstream Signing And Attestation

Sign or attest after conda-ship has staged the files. For executable updates,
sign the runtime before running `cs package-update --binary`. That ensures the
package contains and reports the finalized bytes. The GitHub Action exposes
`dist-path` so downstream workflows can attest the complete output set:
the runtime binary, `.runtime.lock`, `.packages.txt`, `.info.json`, `.sha256`,
and optional external bundle.

Good places for downstream release controls include:

- GitHub Release artifact attestations
- Sigstore signatures
- in-toto provenance for release workflows
- platform-specific installer signing
- package-manager-specific signatures or checksums

Signing belongs downstream because one runtime can be distributed through
several channels, and each channel has different trust requirements.
GitHub release immutability is useful downstream too, but it is not a
replacement for signing. It keeps a published asset set stable; attestations and
signatures explain who produced that asset set and from which workflow.

## Authentication And Offline Updates

Stamped update channel URLs cannot contain credentials, a query, or a fragment.
HTTPS requests can read credentials from the explicit JSON file selected with
`RATTLER_AUTH_FILE`. This build does not enable keyring, netrc, or default
auth-file discovery. The runtime does not implement an interactive provider
login or call a provider API.

Offline mode can use a previously cached HTTPS channel only when both its
repodata and the selected package are already cached. A `file://` channel reads
repodata and packages directly and does not need a network cache. Cached data is
still checked against the same package and payload hashes.

## What conda-ship Does Not Promise

conda-ship does not:

- decide which channels are trusted for a downstream distribution
- sign downstream runtime artifacts
- make a wrapper installer trustworthy by itself
- replace review of committed source lockfiles
- hide the need for package-manager or platform signing

It provides reproducible build output, narrow runtime verification, and metadata
that downstream release systems can sign.
