# Build Locally

Use local builds while iterating on runtime package sets, channel choices, or
conda-ship runtime behavior.

Packaged local builds find the runtime template installed next to `cs`
automatically. When your manifest contains `[tool.conda-ship].runtime`, a
normal build is:

```bash
cs build
```

Source checkouts do not build a runtime template implicitly. Install
conda-ship from a package that includes `cs-template`, or pass
`--template` with an explicit prebuilt template.

If you are changing a downstream distribution such as conda-express, keep the
package-set decision in that downstream project, then reproduce the build with
the `cs` CLI or the GitHub Action.

## Check The Runtime Input

When you want to check the selected source environment before building, run:

```bash
cs inspect
```

If you changed the configured source environment in `conda.toml` or `pyproject.toml`
with `[tool.conda]`, use
{external+conda-workspaces:doc}`conda workspace lock <reference/cli>` to refresh
the source lockfile before building:

```bash
conda workspace lock
cs inspect
```

For Pixi-compatible builds, including `pyproject.toml` with `[tool.pixi]`, use
Pixi to refresh the source lockfile:

```bash
pixi lock
cs inspect
```

CI can use JSON output for machine-readable preflight checks:

```bash
cs inspect --json
```

Use `build --dry-run` when you want to validate artifact names, template
selection, install settings, and bundle suitability without writing files:

```bash
cs build --dry-run
```

## Build A Runtime

`[tool.conda-ship].runtime`, `[tool.conda-ship].delegate`, and
`[tool.conda-ship].source-environment` are required unless you pass the runtime
and delegate through CLI flags. conda-ship does not provide default values for
them.

```bash
cs build
```

Use `--out-dir` to stage somewhere other than `dist/`:

```bash
cs build \
  --out-dir /tmp/cs-artifacts
```

Pass `--template` when you need an explicit release template asset, custom
packaging path, or cross-build template. conda-ship does not search `PATH` for
templates.

## Run A Smoke Test

Use `cs run` to build and immediately execute the staged runtime:

```bash
cs run \
  -- --path /tmp/demo-smoke bootstrap
```

Everything after `--` is passed to the generated runtime.

## Build For Another Target

Pass a target triple, an artifact label, and a matching prebuilt template:

```bash
cs build \
  --runtime demo \
  --target x86_64-unknown-linux-gnu \
  --target-label x86_64-unknown-linux-gnu \
  --template ./cs-template-x86_64-unknown-linux-gnu
```

The target label is appended to staged artifact names and metadata files.

## Keep Names Distribution-Specific

Use a runtime name owned by the distribution you are building. For example,
conda-express uses `cx` as its runtime name. The online layout stages `cx`; the
embedded layout stages `cxz`. A different distribution uses a different
`[tool.conda-ship].runtime` value or the `--runtime` override.

## Run Release Checks

Before publishing a conda-ship release, run the same local checks used for
the release pass:

```bash
pixi run test
pixi run lint
pixi run -e test pytest
pixi run -e test ruff-check
pixi run -e test ruff-format-check
pixi run docs
cargo audit --deny warnings
cargo deny check
zizmor --persona auditor .
```

`cargo deny check` enforces the repository's Rust advisory, license, dependency
ban, and source policies. Duplicate dependency versions are warnings for now
because the rattler dependency graph still contains expected overlap.
