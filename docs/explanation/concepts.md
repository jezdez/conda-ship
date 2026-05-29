# Concepts

conda-pronto separates three concerns:

- resolving and recording a conda runtime package set
- building a generic bootstrap runtime and stamping it with distribution data
- staging release artifacts that downstream projects can distribute

The split from conda-express makes that separation explicit. conda-pronto owns these
generic concerns; conda-express owns the `cx` and `cxz` distribution built with
them.

## Builder

The `pronto` CLI is the builder. It reads `conda.toml`/`conda.lock` or the
compatible `pixi.toml`/`pixi.lock` pair, applies `[tool.pronto]`, then derives
a runtime lock, bundle files, runtime binaries, and artifact metadata.

The selected source lockfile is the source of the concrete conda package
records. conda-pronto is not a replacement for conda-workspaces, Pixi, or any other
workspace solver; it consumes a solved environment and turns it into bootstrap
artifacts.

## Runtime Template

`pronto-runtime` is an internal generic binary target. It is not a first-party
distribution. During `pronto build`, the builder builds the generic runtime,
copies it under the requested artifact name, and stamps the copy with the
downstream distribution name, prefix, metadata filename, environment variable
names, runtime lock, and optional bundle.

## Runtime Lock

The runtime lock is derived from the configured environment, then filtered
through `[tool.pronto].exclude`. conda-pronto writes it to
`target/pronto/runtime.lock` as generated build output, stamps it into every
runtime artifact, and stages a copy next to the output binary. It is not a
second checked-in project lockfile.

The generated runtime can install from:

- the stamped lockfile and network package downloads
- an external lockfile passed with `--lockfile`
- a live solve when `--no-lock` is used

## Bundles

Bundles contain downloaded conda package archives.

The `external` layout pairs a runtime binary with `NAME.bundle.tar.zst`. The
`embedded` layout appends `z` to the binary name and includes the compressed
bundle inside the executable.

An embedded runtime automatically uses its bundled archives during bootstrap.
An explicit `--bundle` can still override that bundle.
