# Configuration Reference

conda-pronto reads project intent from a conda-compatible manifest and concrete
package records from the matching lockfile.

The preferred manifest is `conda.toml` with `conda.lock`. `pixi.toml` with
`pixi.lock` and Pixi's `pyproject.toml` with `pixi.lock` remain supported for
Pixi-compatible workflows.

Downstream distributions maintain these values in their own project manifest.
conda-pronto treats the values as build input; it does not define a universal
conda distribution.

`pronto lock`, `pronto inspect`, `pronto build`, and `pronto run` can read either
manifest/lockfile pair. Installed builds pass `--template` so conda-pronto can
stamp a prebuilt generic runtime template without needing the conda-pronto
source checkout.

## Manifest Discovery

conda-pronto looks in the build root for:

1. `conda.toml`
2. `pixi.toml`
3. `pyproject.toml` when it contains `[tool.pixi]`

The selected manifest determines the lockfile:

| Manifest | Lockfile |
| --- | --- |
| `conda.toml` | `conda.lock` |
| `pixi.toml` | `pixi.lock` |
| `pyproject.toml` with Pixi config | `pixi.lock` |

`conda.lock` and `pixi.lock` are source lockfiles owned by their respective
workspace tools. `target/pronto/runtime.lock` is generated build output owned
by conda-pronto.

## Source Environment

The selected source environment determines the conda packages available to the
generated runtime. In `conda.toml` or `pixi.toml`, this is commonly a
dedicated `runtime` environment:

```toml
[feature.runtime.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"

[environments]
runtime = { features = ["runtime"], no-default-feature = true }
```

In Pixi's `pyproject.toml` layout, the same Pixi sections live below
`[tool.pixi]`, for example `[tool.pixi.feature.runtime.dependencies]`.

The selected environment must include `conda`, `conda-rattler-solver`, and
`conda-spawn`. Generated runtimes delegate commands to conda, write
`solver: rattler` into the installed `.condarc`, and implement `COMMAND shell`
through conda-spawn.

## `[tool.pronto]`

`[tool.pronto]` records conda-pronto-specific build policy:

```toml
[tool.pronto]
source-environment = "runtime"
exclude = ["conda-libmamba-solver"]
docs-url = "https://example.com/demo/"
scheme = "conda"
install-name = "demo"
```

`source-environment`
: Name of the solved environment to turn into the runtime lock. When omitted,
  conda-pronto first tries `runtime`, then falls back to the default environment.

`exclude`
: Package names removed from the derived runtime lock, including dependencies
  used only by excluded packages.

`docs-url`
: Documentation URL stamped into generated runtime help output. The GitHub
  Action also exposes this as the `docs-url` input.

`scheme`
: Install scheme stamped into the generated runtime. Supported values are
  `conda`, which installs below `~/.conda/INSTALL_NAME`, and `data`, which
  installs below the platform user data directory. `conda` is the default when
  `scheme` is not configured.

`install-name`
: Name used inside the install scheme. When omitted, conda-pronto uses the
  generated runtime command name. For example, `command = cx` can use
  `install-name = "express"` so the `conda` scheme resolves to
  `~/.conda/express`.
  Choose a product-specific install name. conda-pronto does not reserve names
  under `~/.conda`; it relies on runtime metadata to avoid overwriting prefixes
  owned by other tools.

Package and channel intent belongs in the selected source environment, not in
`[tool.pronto]`. conda-pronto records the resolved package names and channel
URLs from the source lockfile environment into generated runtime metadata.

## Stamped Runtime Metadata

`pronto build --command COMMAND` stamps these values onto the runtime:

- command name: `COMMAND` for `online` and `external`, `COMMAND` plus `z` for
  `embedded`
- display name: `COMMAND`
- install scheme: `conda`, or the configured `scheme`
- install name: `COMMAND`, or the configured `install-name`
- metadata file: `.COMMAND.json`
- bundle environment variable: uppercased `COMMAND` plus `_BUNDLE`
- offline environment variable: uppercased `COMMAND` plus `_OFFLINE`

Non-alphanumeric characters in environment variable names become underscores.

## Downstream Defaults

conda-pronto's repository default package set exists so the builder and
runtime behavior can be tested. A downstream distribution makes its own
package choices and passes them through `pronto configure` or direct manifest
edits before committing the matching lockfile.

For example, conda-express owns the package set used when building `cx` and
`cxz`; those package choices are conda-express policy, not conda-pronto policy.
