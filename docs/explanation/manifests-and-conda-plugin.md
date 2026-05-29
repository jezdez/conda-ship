# Manifests And Plugin Entry Points

conda-pronto supports a conda-native manifest path and keeps Pixi compatibility for
existing builder workflows.

conda-pronto is not an environment manager. It consumes a solved conda environment
and turns it into bootstrap binaries.

## Manifest Priority

conda-pronto treats `conda.toml` as the preferred project manifest. `pixi.toml`
remains a compatibility input for existing Pixi-based downstream workflows.

Inside a build root, conda-pronto looks for manifests in this order:

1. `conda.toml`
2. `pixi.toml`

When `conda.toml` is selected, conda-pronto reads package records from `conda.lock`.
When `pixi.toml` is selected, it reads package records from `pixi.lock`.

The lockfile remains the source of concrete package records. If the selected
lockfile is missing, create it with the tool that owns the manifest, then run
`pronto lock` again.

## Runtime Environment Selection

The solved environment used for the runtime is selected by
`[tool.pronto].environment`:

```toml
[tool.pronto]
environment = "runtime"
exclude = ["conda-libmamba-solver"]
docs-url = "https://example.com/serpe/"
```

If `environment` is omitted, conda-pronto first looks for a solved environment named
`runtime`. If that is not present, it uses the lockfile's default environment.

conda-pronto writes a new generated lock at `target/pronto/runtime.lock`. That lock
contains only the selected runtime environment, renamed to `default` for the
generated bootstrap binary. It is build output, not another source project
lockfile.

## Conda Workspace Shape

A conda-native conda-pronto project puts conda intent in the workspace schema and
conda-pronto-specific build policy in `[tool.pronto]`:

```toml
[workspace]
name = "serpe"
channels = ["conda-forge"]
platforms = ["linux-64", "osx-arm64", "win-64"]

[feature.runtime.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"

[environments]
runtime = { features = ["runtime"], no-default-feature = true }

[tool.pronto]
environment = "runtime"
exclude = ["conda-libmamba-solver"]
```

`[tool.pronto]` is for conda-pronto build behavior: which solved environment to turn
into a runtime, which packages to prune after the solve, artifact naming
policy, bundle policy, and runtime documentation links.

Package and channel intent belongs in the conda workspace sections when that
manifest is available. The older `[tool.pronto].packages` and
`[tool.pronto].channels` fields remain compatibility metadata for the
Pixi-oriented workflow and for runtime status output.

## CLI And Plugin Entry Points

The `conda-pronto` Python package exposes the same builder through
`conda pronto`:

- `pronto ...` remains the primary CLI.
- `conda pronto ...` dispatches to the `pronto` executable.
- conda-pronto does not depend on being loaded as a conda plugin.
- The plugin package does not make conda-pronto part of conda core.

The plugin entry point is for conda CLI discovery. The builder identity remains
`pronto`, and downstream distributions still own the binaries they publish.

The plugin package expects the `pronto` executable to be available on `PATH`.
Conda recipes for `conda-pronto` package the Rust-built `pronto` binary and
the Python plugin in the same environment. For adapter tests or custom
packaging, `CONDA_PRONTO_EXECUTABLE` points at a specific executable.

## Runtime Template Boundary

The downstream project manifest lives in the downstream repository. The
conda-pronto builder and generic runtime template come from the conda-pronto
release or package installation.

`pronto build --template PATH` copies the prebuilt template, stamps the
copy with the distribution name, runtime lock, metadata, and optional embedded
bundle, then writes the staged artifacts to the downstream project's output
directory. Source checkouts can omit `--template` while changing
conda-pronto itself; that fallback builds `pronto-runtime` locally with Cargo.
