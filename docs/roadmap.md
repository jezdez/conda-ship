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

conda-pronto supports the conda-native pieces needed for downstream distribution
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

The packaged builder path now uses release-published runtime templates, so
installed `pronto build` and `conda pronto build` can stamp downstream
artifacts without a conda-pronto source checkout.

Current follow-up work is mostly distribution hardening:

- document downstream signing and provenance workflows around the staged
  `.sha256`, `.info.json`, `.runtime.lock`, and binary artifacts
- keep the GitHub Action intentionally lockfile-first, with package and channel
  changes made in committed project manifests rather than action inputs
- add Windows ARM64 release assets only after the conda package ecosystem has
  enough stable `win-arm64` coverage for conda-pronto runtimes
