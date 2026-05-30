# Customize A Runtime

Use this guide when you want a conda-pronto-built runtime with your own package
set, command name, install location, channels, or documentation URL.

conda-pronto is generic. It does not publish a first-party runtime, and it
does not reserve a default command name. `conda-express` is one downstream
distribution that uses conda-pronto to publish `cx` and `cxz`; use a command
owned by your distribution.

The manifest examples below describe the build input conda-pronto consumes.
Installed CLI builds pass a released runtime template with `--template`.
Source checkouts can omit that option while changing conda-pronto itself.

## Choose A Command Name

The runtime command name becomes part of the user interface:

- the command users run
- the default install path, `~/.conda/INSTALL_NAME` with the `conda` scheme
- the metadata file, `.COMMAND.json`
- the bundle environment variable, `COMMAND_BUNDLE`
- the offline environment variable, `COMMAND_OFFLINE`

For environment variables, non-alphanumeric characters are converted to
underscores. A runtime named `demo` uses `DEMO_BUNDLE` and
`DEMO_OFFLINE`.

Use a product-specific name:

```bash
pronto build --layout online --command demo
```

Avoid publishing downstream builds as `cx` or `cxz`. In the conda ecosystem,
those names identify the official conda-express artifacts.

## Choose An Install Location

By default, a runtime uses the `conda` scheme and installs below
`~/.conda/COMMAND`, where `COMMAND` is the runtime command name. A downstream
distribution can choose a different install name without stamping an
operating-system-specific path:

```toml
[tool.pronto]
scheme = "conda"
install-name = "express"
```

```bash
pronto build --layout online --command cx --scheme conda --install-name express
```

That builds a runtime command named `cx` whose default install path resolves to
`~/.conda/express` on the user's machine. Users can still override the resolved
path locally with the global runtime option, for example
`COMMAND --path PATH bootstrap` or `COMMAND --path PATH status`.

Choose a product-specific install name. conda-pronto does not reserve names
under `~/.conda`; it writes runtime metadata into bootstrapped prefixes and
uses that metadata to avoid overwriting prefixes owned by other tools.

For a platformdirs-style location, use `scheme = "data"`. That stores the
runtime below the platform user data directory, such as
`${XDG_DATA_HOME:-~/.local/share}/conda/INSTALL_NAME` on Linux,
`~/Library/Application Support/conda/INSTALL_NAME` on macOS, and
`%LOCALAPPDATA%\\conda\\INSTALL_NAME` on Windows.

## Choose Runtime Packages

A conda-pronto runtime must include:

- `python`
- `conda`
- `conda-rattler-solver`
- `conda-spawn`

Additional plugins are a distribution decision. A downstream project records
its own plugin set in its manifest and committed lockfile; conda-pronto does
not choose one for every runtime.

## Configure Local Build Input

When a project carries `conda.toml` and `conda.lock`, keep package and channel
intent in the
{external+conda-workspaces:doc}`conda workspace sections <reference/conda-toml-spec>`
and put conda-pronto-specific build policy in `[tool.pronto]`:

```toml
[workspace]
name = "demo"
channels = ["conda-forge"]
platforms = ["linux-64", "osx-arm64", "win-64"]

[feature.runtime.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"
numpy = "*"
pandas = "*"

[environments]
runtime = { features = ["runtime"], no-default-feature = true }

[tool.pronto]
source-environment = "runtime"
exclude = ["conda-libmamba-solver"]
docs-url = "https://example.com/demo/"
scheme = "conda"
install-name = "demo"
```

Then refresh the source lockfile with
{external+conda-workspaces:doc}`conda workspace lock <reference/cli>` and derive
conda-pronto's runtime lock from the same project:

```bash
conda workspace lock
pronto lock
```

For Pixi-compatible projects, `pronto configure` rewrites the runtime package
intent in the selected manifest. With Pixi config in `pyproject.toml`, it writes
Pixi sections under `[tool.pixi]` and keeps conda-pronto policy under
`[tool.pronto]`:

```bash
pronto configure \
  --package "python >=3.12,<3.15" \
  --package "conda >=25.1" \
  --package "conda-rattler-solver" \
  --package "conda-spawn" \
  --package "numpy" \
  --package "pandas" \
  --channel "conda-forge" \
  --exclude "conda-libmamba-solver"
```

Then refresh the source lockfile and derive conda-pronto's runtime lock. conda-pronto
consumes the solved `runtime` environment; it does not replace the workspace
solver.

```bash
pixi lock
pronto lock
```

Build the runtime:

```bash
pronto build --layout online --command demo --template ./pronto-runtime-template
```

The staged runtime and metadata files are written to `dist/`.

## Build In GitHub Actions

For CI builds, commit the manifest and lockfile, then point the composite action
at that project root:

```yaml
- uses: actions/checkout@v4

- uses: jezdez/conda-pronto@v0.1.0
  id: pronto
  with:
    command: demo
    root: .
    docs-url: "https://example.com/demo/"
```

The action does not run `pronto configure`, `pixi lock`, or any other solve
step. That keeps release artifacts tied to reviewed project files.

## Build An Embedded Variant

Use the `embedded` layout when you want a larger single binary that carries the
package archives inside itself:

```bash
pronto build \
  --layout embedded \
  --command demo \
  --template ./pronto-runtime-template
```

The embedded runtime uses the `z` suffix, so the staged binary is
`dist/demoz` on Unix and `dist/demoz.exe` on Windows.

The embedded runtime detects its built-in bundle automatically during
`bootstrap`; users do not need to pass `--bundle` or `--offline`.
