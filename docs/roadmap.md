# Roadmap

`cs` is focused on the generic build system for single-binary conda
runtimes.

The builder CLI covers the core local workflow:

- `cs inspect`: preflight the selected manifest, lockfile, source
  environment, exclusions, and package set
- `cs build`: stage an `online`, `external`, or `embedded` runtime
- `cs run`: build and execute a local runtime for smoke testing

Every staged build writes the runtime plus artifact metadata: the runtime
lock, a package list, an info JSON file, and SHA256 checksums.
`cs build --dry-run` validates planned artifact work without writing files.

Generic runtime behavior lives in `cs`; opinionated package sets and
distribution defaults belong in downstream projects.

The repository stays focused on producing runtimes. Distribution
wrappers such as Homebrew formulae, constructor-based installers, Docker images,
or enterprise package manager recipes live outside the core builder.

## Manifest And Plugin Work

conda-ship supports conda-workspaces project input for downstream
distribution builds:

- `conda.toml` is the primary conda-workspaces manifest.
- `conda.lock` is the matching source lockfile.
- `pyproject.toml` with `[tool.conda]` is supported through conda-workspaces and
  uses `conda.lock`.
- `pixi.toml`/`pixi.lock` and `pyproject.toml` with `[tool.pixi]` are supported
  for downstream projects that use Pixi for the source solve.
- `[tool.conda-ship].source-environment` chooses which solved environment becomes the
  runtime.
- `[tool.conda-ship].runtime` names the generated runtime.
- `[tool.conda-ship].delegate` chooses which executable receives pass-through
  arguments after bootstrap.
- `[tool.conda-ship].exclude` records post-solve pruning policy.
- Package and channel intent comes from
  {external+conda-workspaces:doc}`conda workspace sections <reference/conda-toml-spec>`
  when `conda.toml` is available.
- `conda-ship` provides a `conda ship` adapter while preserving
  `cs` as the primary CLI.

The packaged builder path now uses release-published runtime templates, so
installed `cs build` and `conda ship build` can stamp downstream
runtimes without a conda-ship source checkout.

Current follow-up work is mostly distribution hardening:

- add richer provenance examples for package-manager specific release workflows
- keep the GitHub Action intentionally lockfile-first, with package and channel
  changes made in committed project manifests rather than action inputs
- keep Windows ARM64 as a canary build for `cs` and `cs-template`, and publish
  release assets only after the conda package ecosystem has enough stable
  `win-arm64` coverage for conda-ship runtimes
