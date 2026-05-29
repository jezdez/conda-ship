# Roadmap

`pronto` is focused on the generic build system and runtime for
single-binary conda distributions.

The builder CLI covers the core local workflow:

- `pronto lock`: derive the runtime lock from the selected conda or Pixi
  environment
- `pronto inspect`: summarize the package set for a target platform
- `pronto bundle`: download package archives into a compressed bundle
- `pronto build`: stage `none`, `external`, or `embedded` artifacts
- `pronto run`: build and execute a local artifact for smoke testing

Every staged build writes the binary plus artifact metadata: the runtime lock,
a package list, an info JSON file, and SHA256 checksums.

Generic runtime behavior lives in `pronto`; opinionated package sets and
distribution defaults belong in downstream projects.

The repository stays focused on producing bootstrap binaries. Distribution
wrappers such as Homebrew formulae, constructor-based installers, Docker images,
or enterprise package manager recipes live outside the core builder.

## Manifest And Plugin Work

Pronto supports the conda-native pieces needed for downstream distribution
builds:

- `conda.toml` is the primary manifest.
- `conda.lock` is the primary source lockfile.
- `pixi.toml` and `pixi.lock` remain compatibility inputs.
- `[tool.pronto].environment` chooses which solved environment becomes the
  runtime.
- `[tool.pronto].exclude` records post-solve pruning policy.
- Package and channel intent comes from conda workspace sections when
  `conda.toml` is available.
- `conda-pronto` provides a `conda pronto` adapter while preserving
  `pronto` as the primary CLI.

The remaining architectural work is to remove the source-checkout requirement
from `pronto build` and `conda pronto build`. That requires a packaged generic
runtime that an installed CLI can copy and stamp from a downstream distribution
repository.
