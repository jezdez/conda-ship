# pronto

Build ready-to-run conda bootstrap binaries.

`pronto` is a generic builder and runtime for single-binary conda
distributions.

`conda-express` is a downstream distribution that uses Pronto to publish the
official `cx` and `cxz` binaries. Pronto owns the generic builder/runtime; a
downstream distribution owns its package set, binary names, release channels,
and installer wrappers.

Artifact layouts:

- `none`: `<name>` with stamped lock/metadata; packages are downloaded during bootstrap.
- `external`: `<name>` plus `<name>.bundle.tar.zst`.
- `embedded`: `<name>z`, the runtime plus compressed bundle embedded in one binary.

The local CLI builds from a Pronto source checkout:

```bash
pronto lock
pronto inspect
pronto build --layout none --name serpe
pronto build --layout embedded --name serpe
pronto run --name serpe -- bootstrap --prefix /tmp/serpe-smoke
```

Every `pronto build` writes the staged binary plus artifact metadata: the
runtime lock, a tab-separated package list, an info JSON document, and SHA256
checksums. The staged binary is stamped with the runtime lock, distribution
metadata, and optional embedded bundle before checksums are written. The GitHub
Action uses the same build path and `embed-bundle: true` for embedded builds.

The `conda-pronto` Python package in `python/conda_pronto` registers
`conda pronto` as a conda plugin entry point. It delegates to the primary
`pronto` executable, so conda packages install the Rust binary and the
Python plugin together.

Generic runtime behavior stays here; opinionated package sets and distribution
defaults belong in downstream distributions.

`conda.toml` plus `conda.lock` is the preferred manifest/lockfile pair for new
Pronto project metadata. `pixi.toml` plus `pixi.lock` remains supported for the
Pixi-compatible workflow.

Historical builder release notes from `conda-express` live in
[CHANGELOG.md](CHANGELOG.md).

`pronto` is not an OS installer generator and does not target `.sh`, `.pkg`, or
`.msi` output. It produces bootstrap binaries that can be distributed directly
or wrapped by Homebrew, constructor, Docker, enterprise packaging systems, and
other release tooling.
