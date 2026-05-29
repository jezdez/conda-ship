# Changelog

## conda-pronto 0.1.0 (draft)

This is the first `conda-pronto` release after splitting the generic
builder/runtime out of `conda-express`. The Rust crate is published as
`conda-pronto` and installs the `pronto` CLI. The Python package registers the
optional `conda pronto` plugin entry point for conda installations that want the
subcommand form.

### Added

- Added publishable crates.io metadata for `conda-pronto`, including repository,
  homepage, documentation, keywords, package include rules, and the Pixi
  manifest needed by the packaged builder workflow.
- Added the `conda-pronto` Python package metadata for the conda plugin entry
  point.
- Added a generic, lockfile-first GitHub Action and reusable build workflow for
  named downstream runtime binaries.
- Added a release workflow that publishes tagged `pronto` and
  `pronto-runtime-template` assets plus `SHA256SUMS` for the action to consume.
- Documented the split between `conda-pronto` as the generic builder/runtime and
  downstream distributions such as `conda-express`.

### Changed

- Clarified that `conda-pronto` does not ship a default first-party runtime
  binary. Downstream projects choose the binary name, package set, channels,
  documentation URL, and release channel.
- Reworked the GitHub Action and reusable build workflow to use released
  `pronto` and runtime-template binaries instead of rebuilding the generic
  runtime from a Rust source checkout.
- Simplified the GitHub Action interface so release builds consume committed
  `conda.toml`/`conda.lock` or `pixi.toml`/`pixi.lock` input instead of
  generating or mutating manifests in CI.
- Updated the rattler dependency stack so current Pixi lockfiles can be read
  directly.
- Removed default runtime examples from the documentation landing page so the
  first screen describes the project boundary instead of implying a bundled
  distribution.
- Updated the Pixi test environment to install `conda-pronto` editable and run
  Python tests against the installed package.

### Security

- Require SHA256 metadata in runtime locks when creating bundles.
- Verify cached and newly downloaded bundle archives before staging them.
- Verify offline bundle archives before installing from a local bundle
  directory, and reject tampered packages.
- Hardened GitHub workflows and the composite action by pinning actions by SHA,
  using minimal permissions, disabling persisted checkout credentials, avoiding
  `eval` for action inputs, and gating Codecov token access.

### Verified

- `cargo test --locked --features runtime-template`
- `cargo clippy --locked --features runtime-template --bins --tests -- -D warnings`
- `cargo publish --locked --dry-run`
- `pixi run -e test pytest`
- `pixi run -e test ruff-check`
- `pixi run -e test ruff-format-check`
- `pixi run -e docs docs`
- `cargo audit`
- `zizmor --persona auditor`

## Split from conda-express (2026-05-28)

`conda-pronto` was split out of `jezdez/conda-express` to own the generic
builder and runtime foundation for ready-to-run conda bootstrap binaries. The
installed CLI remains `pronto`.

Moved project areas:

- runtime lock derivation from Pixi environments
- package exclusion after the solve
- bundle generation for offline bootstrap
- embedded-bundle runtime builds
- staged artifact metadata (`.runtime.lock`, `.packages.txt`, `.info.json`,
  and `.sha256`)
- generic GitHub Action and reusable local CLI workflow

Downstream distributions, such as `conda-express`, keep their own package sets,
binary names, documentation URLs, and release channels.

## Historical conda-express changes

These entries were originally recorded in the `conda-express` changelog before
the generic builder moved into this repository.

### 0.6.0 (2026-05-06)

- Replaced the large `conda-express` build script with a small script that
  copies pre-generated lock and bundle inputs into `$OUT_DIR`.
- Added the internal `cx-build` helper with `prepare`, `payload`, and
  `configure` subcommands. These concepts became conda-pronto's `lock`, `bundle`,
  `configure`, and `build` workflow.
- Added a Pixi runtime environment for the bootstrap package set so Pixi owns
  dependency solving before the runtime binary is built.
- Moved exclude filtering from runtime to build time.
- Updated GitHub Actions to run configure, lock, prepare, and build steps in
  CI.
- Added local and CI `sccache` support.

### 0.5.0 (2026-03-30)

- Added offline bootstrap from a local directory of pre-downloaded `.conda` and
  `.tar.bz2` archives.
- Added the compressed self-contained `cxz` binary variant. In conda-pronto terms,
  this is the `embedded` bundle layout with the `z` suffix.
- Added SHA256 verification for all downloaded package archives used by
  embedded builds.
- Added tests for offline mode, external bundle input validation, and embedded
  bundle bootstrap workflows.
- Added CI and release workflow support for embedded-bundle binaries.
