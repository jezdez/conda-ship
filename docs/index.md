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

`pronto` is not an OS installer generator. It produces bootstrap binaries that
can be distributed directly or wrapped by Homebrew, constructor, Docker,
enterprise packaging systems, and other release tooling.

```{toctree}
:hidden:
:caption: Project

roadmap
```
