# Configuration Reference

conda-ship reads project intent from a conda-compatible manifest and concrete
package records from the matching lockfile.

The preferred manifest is `conda.toml` with `conda.lock`. `pyproject.toml` with
`[tool.conda]` also uses `conda.lock`. `pixi.toml` with `pixi.lock` and
`pyproject.toml` with `[tool.pixi]` plus `pixi.lock` remain supported for
Pixi-compatible workflows.

Downstream distributions maintain these values in their own project manifest.
conda-ship treats the values as build input; it does not define a universal
conda distribution.

`cs inspect`, `cs build`, and `cs run` can read either manifest/lockfile pair.
Packaged builds find the installed runtime template automatically, so local
projects do not need a conda-ship source checkout.

## Manifest Discovery

conda-ship looks in the build root for:

1. `conda.toml`
2. `pixi.toml`
3. `pyproject.toml` when it contains `[tool.conda]` or `[tool.pixi]`

The selected manifest determines the lockfile:

| Manifest | Lockfile |
| --- | --- |
| `conda.toml` | `conda.lock` |
| `pixi.toml` | `pixi.lock` |
| `pyproject.toml` with `[tool.conda]` | `conda.lock` |
| `pyproject.toml` with `[tool.pixi]` | `pixi.lock` |

When `pyproject.toml` contains both `[tool.conda]` and `[tool.pixi]`,
conda-ship follows conda-workspaces and treats `[tool.conda]` as the selected
manifest.

`conda.lock` and `pixi.lock` are source lockfiles owned by their respective
workspace tools. conda-ship derives a runtime lock from that source lockfile
while inspecting, building, or smoke-testing a runtime.

## Source Environment

The selected source environment determines the conda packages available to the
generated runtime. In `conda.toml` or `pixi.toml`, use a dedicated `ship`
environment for the packages that should be included in the runtime:

```toml
[feature.ship.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"

[environments]
ship = { features = ["ship"], no-default-feature = true }
```

In `pyproject.toml`, conda-workspaces sections live below `[tool.conda]`, for
example `[tool.conda.feature.ship.dependencies]`. Pixi sections live below
`[tool.pixi]`, for example `[tool.pixi.feature.ship.dependencies]`.

The selected environment must include `conda`, `conda-rattler-solver`, and
`conda-spawn`. Generated runtimes install that environment as the managed base
prefix, write `solver: rattler` into the installed `.condarc`, and implement
`RUNTIME shell` through conda-spawn. Pass-through commands go to the configured
delegate executable inside that prefix.

## `[tool.conda-ship]`

`[tool.conda-ship]` records conda-ship-specific build policy:

```toml
[tool.conda-ship]
runtime = "demo"
runtime-version = "1.0.0"
delegate = "conda"
layout = "online"
source-environment = "ship"
exclude = ["conda-libmamba-solver"]
docs-url = "https://example.com/demo/"
install-scheme = "conda-home"
install-name = "demo"
install-method = "homebrew"
```

`runtime`
: Name for the generated runtime executable. `cs build` and `cs run` require
  this value, either here or through `--runtime`. It is not a conda environment
  name.

`runtime-version`
: Version shown by the generated runtime when users run `RUNTIME --version`.
  When omitted from `[tool.conda-ship]`, conda-ship uses static
  `[project].version` from the selected `pyproject.toml` if it exists. Release
  workflows can override this with `cs build --runtime-version VERSION` or the
  GitHub Action `runtime-version` input.

  Projects that declare `dynamic = ["version"]` can opt into standards-based
  metadata resolution:

  ```toml
  [tool.conda-ship]
  runtime-version = { from = "project-metadata" }
  ```

  The Python `conda ship` adapter resolves this source before invoking `cs`: it
  calls the project's PEP 517 `prepare_metadata_for_build_wheel` hook, reads
  `Version` from the generated `.dist-info/METADATA`, and passes the resolved
  value to `cs --runtime-version`. It does not fall back to building a wheel.
  The build backend must already be installed in the Python environment running
  `conda ship`.

`delegate`
: Executable inside the managed prefix that receives pass-through arguments
  after bootstrap. Use `conda` for conda-like runtimes such as `cx`. Other
  values, such as `python`, are supported when a runtime should expose a
  different command surface.

`layout`
: Artifact layout to build. Supported values are `online`, `external`, and
  `embedded`. When omitted, `cs build` defaults to `online`.

`source-environment`
: Name of the solved environment to turn into the runtime lock. This value is
  required; conda-ship does not fall back to a default environment because that
  can accidentally ship development or test dependencies.

`exclude`
: Package names removed from the derived runtime lock, including dependencies
  used only by excluded packages.

`docs-url`
: Documentation URL stamped into generated runtime help output. Must start
  with `https://` or `http://` and must not contain whitespace or control
  characters.

`install-scheme`
: Install scheme stamped into the generated runtime. Supported values are
  `conda-home`, which installs below `~/.conda/INSTALL_NAME`, and `user-data`,
  which installs below the platform user data directory. `conda-home` is the
  default when `install-scheme` is not configured.

`install-name`
: Name used inside the install scheme. When omitted, conda-ship uses the
  generated runtime name. For example, `runtime = "cx"` can use
  `install-name = "express"` so the `conda-home` install scheme resolves to
  `~/.conda/express`.
  Choose a product-specific install name. conda-ship does not reserve names
  under `~/.conda`; it relies on runtime metadata to avoid overwriting prefixes
  owned by other tools.

`install-method`
: Optional package-manager hint stamped into the generated runtime. `uninstall`
  uses it to tell users how to remove the runtime binary after the managed
  prefix has been removed. Known values such as `homebrew` get a concrete
  command; other values are printed as informational text. Release workflows
  can override this with `cs build --install-method METHOD` or the GitHub
  Action `install-method` input.

Generated runtimes write ownership metadata into every bootstrapped prefix.
That metadata records the schema version, display name, install name, and
metadata filename expected by the runtime. `status`, `bootstrap --force`,
`uninstall`, and pass-through commands refuse to operate on an existing conda
prefix when that ownership metadata is missing, invalid, or belongs to another
stamped runtime.

Generated runtimes also write constructor-compatible prefix metadata into
`conda-meta/history` and `conda-meta/initial-state.explicit.txt`. Conda uses
the history file to recognize the prefix as an environment and to preserve the
runtime's requested package specs for future conda operations.

Package and channel intent belongs in the selected source environment, not in
`[tool.conda-ship]`. conda-ship records the resolved package names and channel
URLs from the source lockfile environment into generated runtime metadata.

## Stamped Runtime Metadata

`cs build` stamps these values onto the runtime after resolving `runtime` and
`layout` from CLI flags or `[tool.conda-ship]`:

- runtime name: `RUNTIME` for `online` and `external`, `RUNTIME` plus `z` for
  `embedded`
- runtime version: the configured `runtime-version`, static
  `[project].version` from the selected `pyproject.toml`, or the concrete
  value resolved by `conda ship` from `{ from = "project-metadata" }`; builds
  fail when no downstream version can be resolved
- delegate executable: the configured `delegate`
- display name: `RUNTIME`
- install scheme: `conda-home`, or the configured `install-scheme`
- install name: `RUNTIME`, or the configured `install-name`
- install method: the configured `install-method`, when present
- metadata file: `.RUNTIME.json`
- bundle environment variable: uppercased `RUNTIME` plus `_BUNDLE`
- offline environment variable: uppercased `RUNTIME` plus `_OFFLINE`

At bootstrap time, the generated runtime writes a separate prefix metadata file
inside the managed prefix. That file is used for ownership checks before later
operations touch the prefix.

Non-alphanumeric characters in environment variable names become underscores.

## Downstream Defaults

conda-ship's repository default package set exists so the builder and
runtime behavior can be tested. A downstream distribution makes its own
package choices in its project manifest before committing the matching lockfile.

For example, conda-express owns the package set used when building `cx` and
`cxz`; those package choices are conda-express policy, not conda-ship policy.
