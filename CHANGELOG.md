# Changelog

All notable changes to `conda-ship` are documented here.

## 0.3.2 - 2026-06-15

### Fixed

- Set an explicit `conda-ship/<version>` user agent for package archive
  downloads. This fixes 403 responses from `repo.anaconda.com` during embedded
  bundle creation and online runtime bootstrap.

## 0.3.1 - 2026-06-15

### Added

- Added a `conda-ship-version` GitHub Action input so release workflows can
  pin the action source by full commit SHA while selecting the conda-ship
  release assets to download. Exact release-tag usage remains supported for
  backwards compatibility.

## 0.3.0 - 2026-06-10

### Added

- Added Windows ARM64 builder release assets and PyPI wheels for
  `aarch64-pc-windows-msvc`. Full Windows ARM64 runtime bootstrap coverage
  remains gated by the conda package ecosystem.
- Generated runtimes now write constructor-compatible
  `conda-meta/history` and `conda-meta/initial-state.explicit.txt` during
  bootstrap. Conda tooling can recognize the managed prefix as an environment,
  and runtimes that include `conda-self` can reset back to the shipped package
  set.
- Runtime delegate environments now set `CONDA_COMPLETION_COMMAND_NAME` to the
  stamped runtime executable name for shell completion integrations.

### Development

- CI linting now runs through `prek`, with test, documentation, package, and
  platform canary jobs split so common checks finish sooner.

### Fixed

- Fixed conda-workspaces `conda.lock` parsing for lockfiles that use the
  `version: 1` schema.

## 0.2.1 - 2026-06-03

### Fixed

- Fixed the GitHub Action path for
  `runtime-version = { from = "project-metadata" }`. The action now detects
  when the Rust `cs` binary needs project metadata resolution, sets up Python
  with the official `actions/setup-python` action, resolves the downstream
  version through `pypa/build`, and retries the build with an explicit
  `--runtime-version`.
- Fixed the Windows runtime template release build warning path so release
  binary builds can deny compiler warnings across all targets.

## 0.2.0 - 2026-06-03

### Added

- Added standards-compliant dynamic Python project version resolution for
  runtime stamping. Projects can set
  `runtime-version = { from = "project-metadata" }` in `[tool.conda-ship]`,
  and the Python `conda ship` adapter resolves the concrete version through the
  PEP 517 `prepare_metadata_for_build_wheel` hook before invoking `cs`.
- Added a quickstart tutorial and a compact README quickstart for the shortest
  local path from a solved conda-workspaces environment to a staged runtime.
- Added terminal demo recordings for quickstart, `cs inspect`,
  `cs build --dry-run`, staged artifact verification, and generated runtime CLI
  behavior. The recordings are embedded in the README and the relevant docs
  pages.

### Changed

- Generated runtime metadata now requires a downstream runtime version. Builds
  use `[tool.conda-ship].runtime-version`, static `[project].version`, an
  explicit `--runtime-version` or GitHub Action input, or the Python adapter's
  project metadata resolution. conda-ship no longer falls back to its own
  package version for generated runtimes.
- Documentation now describes the PyPI-first install flow, runtime version
  requirements, local preflight commands, artifact verification, and generated
  runtime CLI behavior in more detail.

### Fixed

- Fixed builder output handling for closed stdout pipes so commands such as
  `cs inspect | head` or filtered demo commands do not report Rust panics when
  the reader exits early.

## 0.1.0 - 2026-06-01

Initial release of `conda-ship`, a generic builder for producing ready-to-run
conda runtimes from solved conda environments.

### Added

- The `cs` builder CLI.
  - `cs inspect` checks the selected manifest, lockfile, source environment,
    exclusions, platforms, and package set without writing files.
  - `cs build` stages runtime artifacts.
  - `cs build --dry-run` validates planned artifact work before downloading,
    stamping, or writing files.
  - `cs run` builds a runtime and immediately runs it for local smoke tests.
- The generic `cs-template` runtime template used to produce downstream runtime
  binaries.
- Platform PyPI wheels that install `cs`, `cs-template`, and the Python adapter
  together.
- A Python adapter that exposes `conda ship` as a conda-style shortcut for
  `cs`, including structured builder diagnostics so common failures can be
  reported predictably through conda.
- Build input from committed source manifests and lockfiles:
  - `conda.toml` with `conda.lock`
  - `pyproject.toml` with `[tool.conda]` and `conda.lock`
  - `pixi.toml` with `pixi.lock`
  - `pyproject.toml` with `[tool.pixi]` and `pixi.lock`
- `[tool.conda-ship]` build policy for generated runtimes, including the
  runtime name, runtime version, delegate executable, source environment,
  artifact layout, package exclusions, install scheme, install name, install
  method, and documentation URL.
- Three artifact layouts:
  - `online`, for small runtime artifacts that download packages during
    bootstrap
  - `external`, for a runtime plus a separate compressed package bundle
  - `embedded`, for a larger single runtime that carries the compressed package
    bundle inside the binary
- Generated runtime commands for `bootstrap`, `status`, `shell`, and
  `uninstall`, plus pass-through support to the configured delegate executable.
- Generated runtime version output, so downstream binaries such as `cx` can
  report their own distribution version instead of the generic conda-ship
  builder version.
- Runtime install ownership metadata so generated runtimes can protect managed
  prefixes from accidental use or removal by the wrong runtime.
- Install schemes for `~/.conda/INSTALL_NAME` and platform user data
  directories, plus a runtime `--path` override for local testing and advanced
  install paths.
- Staged runtime metadata files:
  - `.runtime.lock`
  - `.packages.txt`
  - `.info.json`
  - `.sha256`
  - optional `.bundle.tar.zst` for `external` builds
- Package exclusion after source-lock resolution, so downstream distributions
  can prune packages from a solved environment before building a runtime.
- Validation that the selected runtime environment contains the packages
  required by generated runtimes: `conda`, `conda-rattler-solver`, and
  `conda-spawn`.
- A composite GitHub Action for downstream release jobs. The action uses
  committed manifest and lockfile input, verifies downloaded conda-ship release
  assets, runs `cs build --dry-run`, and exposes `dist-path` for publishing the
  complete generated artifact directory. Release jobs can override runtime
  metadata such as runtime name, runtime version, delegate, layout, install
  scheme, install name, install method, and documentation URL from workflow
  inputs or matrices.
- Tagged release assets for `cs`, `cs-template`, and `SHA256SUMS`.

### Security And Provenance

- Bundle builds require SHA256 package metadata.
- Downloaded, cached, external, embedded, and offline package archives are
  verified before they are staged or installed.
- Runtime templates refuse to run directly; `cs build` must stamp a template
  before it becomes a downstream runtime.
- Runtime names, runtime versions, delegates, install names, install methods,
  target labels, and documentation URLs are validated before they are stamped
  into runtime binaries or artifact names.
- The GitHub Action verifies artifact attestations for downloaded `cs`,
  `cs-template`, and `SHA256SUMS` assets before running them.
- The `conda ship` adapter only runs the `cs` executable installed in the same
  Python environment, unless `CONDA_SHIP_EXECUTABLE` explicitly selects another
  executable.
- Tagged GitHub releases publish immutable asset sets. If a release is wrong,
  publish a new version instead of replacing files under an existing tag.
- GitHub workflows and the composite action use pinned actions, minimal
  permissions, explicit artifact verification, and no shell `eval` for user
  input.
- Release workflows use unprefixed version tags such as `0.1.0`.
- Release checks include Rust advisory, license, dependency-ban, and source
  policy checks.

### Notes

- This is an alpha 0.1.0 release. The project is ready for early downstream
  distribution work, but configuration details and artifact metadata may still
  evolve before 1.0.
- `conda-ship` is not itself a conda distribution. Downstream projects choose
  package sets, channels, runtime names, delegates, install methods, release
  channels, signing policy, and user documentation.
- The GitHub Action should be used from a release tag. Branch refs do not have
  matching `cs` and `cs-template` release assets. Use tags such as `0.1.0`,
  without a leading `v`.
- Downstream release workflows should sign or attest the full `dist-path`
  output after `cs build`.
