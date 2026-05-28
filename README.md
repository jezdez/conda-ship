# pronto

Build ready-to-run conda bootstrap binaries.

`pronto` is being split out of `jezdez/conda-express` as the generic builder
and runtime foundation for `cx` / `cxz`-style conda distributions.

The intended artifact layouts are:

- `none`: `<name>` with embedded lock/metadata; packages are downloaded during bootstrap.
- `external`: `<name>` plus `<name>.bundle.tar.zst`.
- `embedded`: `<name>z`, the runtime plus compressed bundle embedded in one binary.

The current repository contents are the initial history-preserving extraction of
the generic builder/runtime pieces. The next migration work is to split generic
runtime behavior from the opinionated `conda-express` distribution and add
complete artifact metadata.

The GitHub Action uses `embed-bundle: true` for embedded `cxz` builds.

`pronto` is not an OS installer generator and does not target `.sh`, `.pkg`, or
`.msi` output. It produces bootstrap binaries that can be distributed directly
or wrapped by Homebrew, constructor, Docker, enterprise packaging systems, and
other release tooling.
