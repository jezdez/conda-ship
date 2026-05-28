# Roadmap

`pronto` is currently the history-preserving extraction of the generic builder
and runtime code from `conda-express`.

The next implementation pass will split the generic runtime behavior from the
opinionated `conda-express` distribution behavior:

- `none`: base binary with lock and metadata embedded
- `external`: base binary plus `<name>.bundle.tar.zst`
- `embedded`: `<name>z`, with the compressed bundle embedded

The repository should stay focused on producing bootstrap binaries. Distribution
wrappers such as Homebrew formulae, constructor-based installers, Docker images,
or enterprise package manager recipes should live outside the core builder.
