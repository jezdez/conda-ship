# Changelog

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
