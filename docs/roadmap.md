# Roadmap

`pronto` is currently the history-preserving extraction of the generic builder
and runtime code from `conda-express`.

The builder CLI now covers the core local workflow:

- `pronto lock`: derive `artifact.lock` from the `cx-env` Pixi environment
- `pronto inspect`: summarize the package set for a target platform
- `pronto bundle`: download package archives into a compressed bundle
- `pronto build`: stage `none`, `external`, or `embedded` artifacts
- `pronto run`: build and execute a local artifact for smoke testing

Every staged build writes the binary plus artifact metadata: the artifact lock,
a package list, an info JSON file, and SHA256 checksums.

The next implementation pass will split the generic runtime behavior from the
opinionated `conda-express` distribution behavior.

The repository should stay focused on producing bootstrap binaries. Distribution
wrappers such as Homebrew formulae, constructor-based installers, Docker images,
or enterprise package manager recipes should live outside the core builder.
