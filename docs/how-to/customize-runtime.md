# Customize A Runtime

Use this guide when you want a conda-ship-built runtime with your own package
set, runtime name, delegate executable, install location, channels, or
documentation URL.

conda-ship is generic. It does not publish a first-party runtime, and it
does not reserve a default runtime name. `conda-express` is one downstream
distribution that uses conda-ship to publish `cx` and `cxz`; use a runtime name
owned by your distribution.

The manifest examples below describe the build input conda-ship consumes.
Packaged CLI builds find the runtime template installed next to `cs`
automatically. Source checkouts need either a packaged install that includes
`cs-template` or an explicit `--template` path.

## Choose A Runtime Name

The runtime name becomes part of the user interface:

- the executable users run
- the default install path, `~/.conda/INSTALL_NAME` with the `conda-home` install scheme
- the metadata file, `.RUNTIME.json`
- the bundle environment variable, `RUNTIME_BUNDLE`
- the offline environment variable, `RUNTIME_OFFLINE`

For environment variables, non-alphanumeric characters are converted to
underscores. A runtime named `demo` uses `DEMO_BUNDLE` and
`DEMO_OFFLINE`.

Use a product-specific name:

```toml
[tool.conda-ship]
runtime-name = "demo"
delegate-executable = "conda"
artifact-layout = "online"
```

Avoid publishing downstream builds as `cx` or `cxz`. In the conda ecosystem,
those names identify the conda-express artifacts maintained in the
`jezdez/conda-express` repository.

## Choose An Install Location

By default, a runtime uses the `conda-home` install scheme and installs below
`~/.conda/RUNTIME`, where `RUNTIME` is the runtime name. A downstream
distribution can choose a different install name without stamping an
operating-system-specific path:

```toml
[tool.conda-ship]
runtime-name = "cx"
delegate-executable = "conda"
artifact-layout = "online"
install-scheme = "conda-home"
install-name = "express"
```

```bash
cs build
```

That builds a runtime named `cx` whose default install path resolves to
`~/.conda/express` on the user's machine. Users can still override the resolved
path locally with the global runtime option, for example
`RUNTIME --path PATH bootstrap` or `RUNTIME --path PATH status`.

Choose a product-specific install name. conda-ship does not reserve names
under `~/.conda`; it writes runtime metadata into bootstrapped prefixes and
uses that metadata to avoid overwriting prefixes owned by other tools. The
metadata includes the runtime display name, install name, metadata filename,
and metadata schema version, so a runtime refuses to use or remove a prefix
that belongs to a different stamped runtime.

For a platformdirs-style location, use `install-scheme = "user-data"`. That stores the
runtime below the platform user data directory, such as
`${XDG_DATA_HOME:-~/.local/share}/conda/INSTALL_NAME` on Linux,
`~/Library/Application Support/conda/INSTALL_NAME` on macOS, and
`%LOCALAPPDATA%\\conda\\INSTALL_NAME` on Windows.

If a downstream package manager owns the runtime binary, set
`installer` in the manifest or pass it from the release job. The generated
runtime uses that value only after `uninstall`, when it tells users how to
remove the runtime binary itself:

```toml
[tool.conda-ship]
runtime-name = "demo"
delegate-executable = "conda"
installer = "homebrew"
```

For matrix builds that produce the same runtime for different distribution
channels, use `cs build --installer METHOD` or the GitHub Action
`installer` input.

## Choose Runtime Packages

A conda-ship runtime must include:

- `python`
- `conda`
- `conda-rattler-solver`
- `conda-spawn`

Additional plugins are a distribution decision. A downstream project records
its own plugin set in its manifest and committed lockfile; conda-ship does
not choose one for every runtime.

Add `conda-self` when the generated runtime should let users reset the managed
base prefix back to the packages shipped by the runtime:

```toml
[feature.ship.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"
conda-self = "*"
```

conda-ship writes `conda-meta/initial-state.explicit.txt` during bootstrap.
`conda-self` treats that file as the installer snapshot for
`conda self reset --snapshot installer-updated` and
`conda self reset --snapshot installer-exact`, so reset removes packages added
after bootstrap while preserving the runtime's initial package set.

## Configure Local Build Input

Keep package and channel intent in the manifest format owned by your workspace
tool. Keep conda-ship-specific build policy in `[tool.conda-ship]`.

::::{tab-set}

:::{tab-item} conda.toml

Use `conda.toml` when the project uses
{external+conda-workspaces:doc}`conda-workspaces <index>` as its primary
manifest:

```toml
[workspace]
name = "demo"
channels = ["conda-forge"]
platforms = ["linux-64", "osx-arm64", "win-64"]

[feature.ship.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"
numpy = "*"
pandas = "*"

[environments]
ship = { features = ["ship"], no-default-feature = true }

[tool.conda-ship]
runtime-name = "demo"
runtime-version = "0.1.0"
delegate-executable = "conda"
artifact-layout = "online"
source-environment = "ship"
exclude-packages = ["conda-libmamba-solver"]
docs-url = "https://example.com/demo/"
install-scheme = "conda-home"
install-name = "demo"
installer = "homebrew"
```

Refresh the source lockfile:

```bash
conda workspace lock
```

:::

:::{tab-item} pyproject.toml with `[tool.conda]`

Use this form when a Python project keeps conda-workspaces config in
`pyproject.toml`. conda-workspaces tables live under `[tool.conda]`, while
`[tool.conda-ship]` remains a sibling tool table:

```toml
[tool.conda.workspace]
name = "demo"
channels = ["conda-forge"]
platforms = ["linux-64", "osx-arm64", "win-64"]

[tool.conda.feature.ship.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"

[tool.conda.environments]
ship = { features = ["ship"], no-default-feature = true }

[tool.conda-ship]
runtime-name = "demo"
runtime-version = "0.1.0"
delegate-executable = "conda"
artifact-layout = "online"
source-environment = "ship"
exclude-packages = ["conda-libmamba-solver"]
```

Refresh the source lockfile:

```bash
conda workspace lock
```

This writes `conda.lock`.

:::

:::{tab-item} Pixi

For Pixi-compatible projects, keep the source environment package intent in
Pixi's own sections. If Pixi config lives in `pyproject.toml`, the package and
channel sections live under `[tool.pixi]`, while `[tool.conda-ship]` stays at
the Python project tool level:

```toml
[tool.pixi.workspace]
name = "demo"
channels = ["conda-forge"]
platforms = ["linux-64", "osx-arm64", "win-64"]

[tool.pixi.feature.ship.dependencies]
python = ">=3.12,<3.15"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"
numpy = "*"
pandas = "*"

[tool.pixi.environments]
ship = { features = ["ship"], no-default-feature = true }

[tool.conda-ship]
runtime-name = "demo"
runtime-version = "0.1.0"
delegate-executable = "conda"
artifact-layout = "online"
source-environment = "ship"
exclude-packages = ["conda-libmamba-solver"]
```

Refresh the source lockfile:

```bash
pixi lock
```

This writes `pixi.lock`.

:::

::::

conda-ship consumes the solved `ship` environment during `cs build`; it does
not replace the workspace solver.

Build the runtime:

```bash
cs build
```

The staged runtime and metadata files are written to `dist/`.

## Build In GitHub Actions

For CI builds, commit the manifest and lockfile, then point the composite action
at that project root:

```yaml
- uses: actions/checkout@v4

- uses: jezdez/conda-ship@FULL_RELEASE_COMMIT_SHA # X.Y.Z
  id: cs
  with:
    conda-ship-version: "X.Y.Z"
    root: .
```

The action does not run `conda workspace lock`, `pixi lock`, or any other solve
step. That keeps release artifacts tied to reviewed project files.

## Build An Embedded Variant

Use the `embedded` layout when you want a larger single binary that carries the
package archives inside itself:

```bash
cs build --artifact-layout embedded
```

The embedded runtime uses `runtime` by default, so the staged binary is
`dist/demo` on Unix and `dist/demo.exe` on Windows. Set
`artifact-name = "demo-offline"` or pass `--artifact-name demo-offline` when a
release artifact should have a distinct command name.

The embedded runtime detects its built-in bundle automatically during
`bootstrap`; users do not need to pass `--bundle` or `--offline`.
