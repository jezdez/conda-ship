# Concepts

conda-ship separates three concerns:

- resolving and recording a conda runtime package set
- stamping a generic runtime template into a distribution-specific runtime
- staging release artifacts that downstream projects can distribute

The split from conda-express makes that separation explicit. conda-ship owns these
generic concerns; conda-express owns the `cx` and `cxz` distribution built with
them.

## Builder

The `cs` CLI is the builder. It reads `conda.toml`/`conda.lock`,
`pyproject.toml` with `[tool.conda]` plus `conda.lock`, the compatible
`pixi.toml`/`pixi.lock` pair, or `pyproject.toml` with `[tool.pixi]` plus
`pixi.lock`, applies `[tool.conda-ship]`, then derives a runtime lock, bundle files,
runtimes, and artifact metadata.

The selected source lockfile is the source of the concrete conda package
records. conda-ship is not a replacement for
{external+conda-workspaces:doc}`conda-workspaces <index>`, Pixi, or any other
workspace solver; it consumes a solved environment and turns it into
runtime artifacts.

## Runtime

A runtime is the executable that users run after conda-ship finishes.
Examples include `demo`, `cx`, and downstream-specific embedded names such as
`cxz`.

The runtime name is the base identity for that runtime. It can come from
`[tool.conda-ship].runtime-name` or `--runtime-name`, and it is not a conda
environment name. Builds use the same value as the staged executable and
artifact name unless `[tool.conda-ship].artifact-name` or `--artifact-name`
names a distinct artifact.

Use "runtime" for the executable conda-ship produces, "delegate" for the
executable inside the managed prefix that receives pass-through arguments, and
"artifact" for the release files written to `dist/`.

## Runtime Template

`cs-template` is an internal generic binary target. It is not a first-party
distribution. During `cs build`, the builder copies a generic runtime
template under the resolved runtime name and stamps the copy with the runtime
name, delegate, install scheme, install name, metadata filename, environment
variable names, runtime lock, and optional bundle. The stamped copy is the runtime.
Released builds and packaged local builds use prebuilt template assets.

## Runtime Lock

The runtime lock is derived from the configured source environment, then filtered
through `[tool.conda-ship].exclude`. conda-ship stamps the derived lock into every
runtime artifact and stages a copy next to the output binary. It is not a second
checked-in project lockfile.

`cs inspect` derives the same runtime lock without writing files, so it is
the local preflight command. `cs build` and `cs run` derive the lock as part
of their normal work.

The generated runtime can install from:

- the stamped lockfile and network package downloads
- the stamped lockfile and an external package bundle
- the stamped lockfile and an embedded package bundle

## Bundles

Bundles contain downloaded conda package archives. They are not conda channel
mirrors; conda-ship already has the channel and package records in the runtime
lock.

The `external` layout pairs a runtime with `RUNTIME.bundle.tar.zst`. The
`embedded` layout includes the compressed bundle inside the runtime and can use
an explicit artifact name when configured.

An embedded runtime automatically uses its bundled archives during
bootstrap. An explicit `--bundle` can still override that bundle.

The bundle format is intentionally narrow. conda-ship writes top-level `.conda`
and `.tar.bz2` files, then verifies them against the lockfile at install time.
Embedded bundles reject nested paths and links before extraction.
