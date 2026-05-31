# conda-ship

Build ready-to-run conda runtimes.

`conda-ship` is a generic builder for single-binary conda runtimes. It
installs the `cs` CLI.

`conda-express` is a downstream distribution that uses conda-ship to publish the
official `cx` and `cxz` runtimes. conda-ship owns the generic builder; a
downstream distribution owns its package set, runtime names, release channels,
and installer wrappers.

Artifact layouts:

- `online`: runtime `<runtime>` with stamped lock/metadata; packages are downloaded during bootstrap.
- `external`: runtime `<runtime>` plus `<runtime>.bundle.tar.zst`.
- `embedded`: runtime `<runtime>z` with the compressed bundle embedded in one binary.

The CLI builds from a solved downstream project. Packaged builds find the
installed runtime template automatically. Use `--template` only for an explicit
template path, custom packaging, or cross-builds:

```toml
[tool.conda-ship]
runtime = "demo"
delegate = "conda"
layout = "online"
source-environment = "ship"
```

```bash
cs inspect
cs build --dry-run
cs build
cs build --layout embedded
cs run -- --path /tmp/demo-smoke bootstrap
```

Every `cs build` writes the runtime binary plus artifact metadata: the
runtime lock, a tab-separated package list, an info JSON document, and SHA256
checksums. The runtime is stamped with the runtime lock, distribution
metadata, and optional embedded bundle before checksums are written. The GitHub
Action downloads tagged `cs` and `cs-template` release assets, verifies
their GitHub attestations and `SHA256SUMS`, and then uses the same stamping path
against a committed downstream manifest and lockfile.

Most users run the builder as `cs`. The Python package can also make
`conda ship` available inside a conda environment; that command is a shortcut
for the same `cs` executable. Conda packages install the Rust binary and
the small Python adapter together.

Generic runtime behavior stays here; opinionated package sets and distribution
defaults belong in downstream distributions.

`conda.toml` plus `conda.lock` is the preferred manifest/lockfile pair for new
conda-ship project metadata. `pyproject.toml` with `[tool.conda]` also uses
`conda.lock`. `pixi.toml` plus `pixi.lock` and `pyproject.toml` with
`[tool.pixi]` plus `pixi.lock` remain supported for Pixi-compatible workflows.

`cs` is not an OS installer generator and does not target `.sh`, `.pkg`, or
`.msi` output. It produces runtimes that can be distributed directly
or wrapped by Homebrew, constructor, Docker, enterprise packaging systems, and
other release tooling.
