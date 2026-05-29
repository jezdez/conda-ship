# Configuration Reference

Pronto reads project intent from a conda-compatible manifest and concrete
package records from the matching lockfile.

The preferred manifest is `conda.toml` with `conda.lock`. `pixi.toml` with
`pixi.lock` remains supported for compatibility with the original Pronto build
workflow.

Downstream distributions can maintain these values in a Pronto checkout or pass
overrides to the GitHub Action. Pronto treats the values as build input; it
does not define a universal conda distribution.

`pronto lock` and `pronto inspect` can read either manifest/lockfile pair.
`pronto build` and `pronto run` still require a Pronto source checkout because
they build the generic runtime target from the selected root before stamping
the staged artifact.

## Manifest Discovery

Pronto looks in the build root for:

1. `conda.toml`
2. `pixi.toml`

The selected manifest determines the lockfile:

| Manifest | Lockfile |
| --- | --- |
| `conda.toml` | `conda.lock` |
| `pixi.toml` | `pixi.lock` |

`conda.lock` and `pixi.lock` are source lockfiles owned by their respective
workspace tools. `target/pronto/runtime.lock` is generated build output owned
by Pronto.

## Runtime Environment

The selected environment determines the conda packages available to the
generated bootstrap runtime. In `conda.toml` or `pixi.toml`, this is commonly a
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

## `[tool.pronto]`

`[tool.pronto]` records Pronto-specific build policy:

```toml
[tool.pronto]
environment = "runtime"
exclude = ["conda-libmamba-solver"]
docs-url = "https://example.com/serpe/"
```

`environment`
: Name of the solved environment to turn into the runtime lock. When omitted,
  Pronto first tries `runtime`, then falls back to the default environment.

`exclude`
: Package names removed from the derived runtime lock, including dependencies
  used only by excluded packages.

`docs-url`
: Documentation URL stamped into generated runtime help output. The GitHub
  Action also exposes this as the `docs-url` input.

The older compatibility metadata fields are still accepted:

```toml
[tool.pronto]
channels = ["conda-forge"]
packages = [
  "python >=3.12",
  "conda >=25.1",
  "conda-rattler-solver",
  "conda-spawn >=0.1.0",
]
```

`packages`
: Specs shown in runtime metadata and used for live solves with `--no-lock`.
  Prefer conda workspace dependency sections for `conda.toml` projects.

`channels`
: Channels used when deriving the runtime lock and written into runtime
  metadata. Prefer `[workspace].channels` for `conda.toml` projects.

## Stamped Runtime Metadata

`pronto build --name NAME` stamps these runtime values onto the staged binary:

- command name: `NAME` for `none` and `external`, `NAME` plus `z` for
  `embedded`
- display name: `NAME`
- default prefix: `~/.NAME`
- metadata file: `.NAME.json`
- bundle environment variable: uppercased `NAME` plus `_BUNDLE`
- offline environment variable: uppercased `NAME` plus `_OFFLINE`

Non-alphanumeric characters in environment variable names become underscores.

## Downstream Defaults

Pronto's repository default package set exists so the builder and runtime can be
tested. A downstream distribution makes its own package choices and passes them
through `pronto configure`, the GitHub Action inputs, or an equivalent release
workflow.

For example, conda-express passes its own package set when building `cx` and
`cxz`; those package choices are conda-express policy, not Pronto policy.
