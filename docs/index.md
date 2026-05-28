# pronto

Build ready-to-run conda bootstrap binaries.

`pronto` is the generic builder and runtime foundation for `cx` / `cxz`-style
conda distributions. It is being split out of `conda-express` so the reusable
build system can evolve independently from the opinionated distribution.

## Artifact layouts

| Layout | Output | Use |
|---|---|---|
| `none` | `<name>` | Embedded lock and metadata; packages download during bootstrap |
| `external` | `<name>` plus `<name>.bundle.tar.zst` | Runtime binary paired with a compressed bundle |
| `embedded` | `<name>z` | Runtime plus compressed bundle embedded in one binary |

## CLI workflow

Use the CLI locally the same way the GitHub Action builds release artifacts:

```bash
pronto lock
pronto inspect
pronto build --layout none --name cx
pronto build --layout embedded --name cx
pronto run -- bootstrap --prefix /tmp/cx-smoke
```

`pronto build` stages the binary and writes the artifact lock, package list,
info JSON, and SHA256 checksum file next to it.

`pronto` is not an OS installer generator. It produces bootstrap binaries that
can be distributed directly or wrapped by Homebrew, constructor, Docker,
enterprise packaging systems, and other release tooling.

```{toctree}
:hidden:
:caption: Project

roadmap
```
