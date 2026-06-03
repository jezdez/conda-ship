# GitHub Action Reference

The repository root provides a composite GitHub Action for downstream
distribution repositories.

The action downloads the tagged conda-ship release assets for the current
runner, verifies their GitHub artifact attestations and `SHA256SUMS`, and runs
the downloaded `cs` binary to preflight and build a runtime. The preflight
uses `cs build --dry-run`, then the action runs the real build. It does not
build conda-ship from source.
Self-hosted runners must provide the GitHub CLI because attestation
verification uses `gh attestation verify`.

The action builds only from committed project input. The selected root must
contain `conda.toml` plus `conda.lock`, `pyproject.toml` with `[tool.conda]`
plus `conda.lock`, `pixi.toml` plus `pixi.lock`, or `pyproject.toml` with
`[tool.pixi]` plus `pixi.lock`. When the manifest or matching lockfile is
missing, the action fails instead of generating or solving project configuration
in CI. This minimal example assumes the manifest contains
`[tool.conda-ship].runtime`, `[tool.conda-ship].delegate`, and a downstream
runtime version.

When the selected conda-ship config sets
`runtime-version = { from = "project-metadata" }`, the action first lets
`cs build --dry-run` report that project metadata resolution is required. It
then sets up Python with `actions/setup-python`, resolves the downstream
project version through `pypa/build`, and retries the build with an explicit
`--runtime-version`. Static runtime versions and explicit `runtime-version`
inputs do not set up Python.

```yaml
- uses: actions/checkout@v4

- uses: jezdez/conda-ship@0.2.1
  id: cs
```

## Inputs

`runtime`
: Runtime name override. Set this when the release job intentionally stamps a
  different runtime name than `[tool.conda-ship].runtime`.

`delegate`
: Delegate executable override. Set this when the release job intentionally
  changes which executable receives pass-through arguments.

`runtime-version`
: Runtime version override. Set this when the release job intentionally stamps
  a version different from `[tool.conda-ship].runtime-version` or
  `[project].version`, or when the manifest does not provide a downstream
  runtime version.

`python-version`
: Python version used only when the action must resolve
  `runtime-version = { from = "project-metadata" }`. Defaults to `3.12`.

`root`
: Project root containing `conda.toml`/`conda.lock`, `pixi.toml`/`pixi.lock`,
  or `pyproject.toml` with either `[tool.conda]`/`conda.lock` or
  `[tool.pixi]`/`pixi.lock`. Defaults to the workflow workspace.

`layout`
: Artifact layout to build. Supported values are `online`, `external`, and
  `embedded`. Overrides `[tool.conda-ship].layout` when set; otherwise the
  action leaves layout selection to the manifest and `cs` defaults to `online`.
  External artifacts stage the runtime and bundle as separate files. Embedded
  artifacts carry package archives inside the runtime and use the `z` suffix.

`docs-url`
: Documentation URL stamped into generated runtime help output. Must start
  with `https://` or `http://` and must not contain whitespace or control
  characters.

`install-scheme`
: Install scheme stamped into the generated runtime. Supported values are
  `conda-home` and `user-data`.

`install-name`
: Name used inside the install scheme. When omitted, `cs` uses
  `[tool.conda-ship].install-name` or the resolved runtime name.

`install-method`
: Package-manager or installer method stamped into the runtime for uninstall
  guidance.

The action does not duplicate `cs build` validation in shell. It passes
non-empty inputs to `cs build --dry-run` and then to `cs build`; invalid values
fail in the builder.

## Outputs

`dist-path`
: Absolute path to the directory containing all generated runtime artifacts.
  Use this for artifact uploads when the complete build output should be
  published together.

`binary-path`
: Absolute path to the generated runtime.

`asset-name`
: Platform-qualified asset filename.

`info-path`
: Absolute path to the artifact info JSON.

`lock-path`
: Absolute path to the staged runtime lock.

`package-list-path`
: Absolute path to the staged package list.

`checksums-path`
: Absolute path to the SHA256 checksum file.

`bundle-path`
: Absolute path to the external bundle when `layout: external`; empty for
  `online` and `embedded`.
