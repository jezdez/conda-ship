# pronto

Build ready-to-run conda bootstrap binaries.

`pronto` is being split out of `jezdez/conda-express` as the generic builder
and runtime foundation for `cx` / `cxz`-style conda distributions.

The intended artifact layouts are:

- `none`: `<name>` with embedded lock/metadata; packages are downloaded during bootstrap.
- `external`: `<name>` plus `<name>.bundle.tar.zst`.
- `embedded`: `<name>z`, the runtime plus compressed bundle embedded in one binary.

The local CLI mirrors the GitHub Actions workflow:

```bash
pronto lock
pronto inspect
pronto build --layout none --name cx
pronto build --layout embedded --name cx
pronto run -- bootstrap --prefix /tmp/cx-smoke
```

Every `pronto build` writes the staged binary plus artifact metadata: the
artifact lock, a tab-separated package list, an info JSON document, and SHA256
checksums. The GitHub Action uses the same build path and `embed-bundle: true`
for embedded `cxz` builds.

The remaining migration work is to split generic runtime behavior from the
opinionated `conda-express` distribution.

`pronto` is not an OS installer generator and does not target `.sh`, `.pkg`, or
`.msi` output. It produces bootstrap binaries that can be distributed directly
or wrapped by Homebrew, constructor, Docker, enterprise packaging systems, and
other release tooling.
