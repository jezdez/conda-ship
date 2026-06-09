# Build Your First Runtime

This tutorial builds a local conda runtime named `demo` from either a
conda-workspaces project or a Pixi project.

You will create a small project, lock it, build a runtime binary, bootstrap that
runtime into a temporary install path, and then remove it again.

## Before You Start

You need:

- `conda-ship`
- either {external+conda-workspaces:doc}`conda-workspaces <index>` or
  [Pixi](https://pixi.sh/latest/)
- network access for solving and for the first bootstrap

Install the tools in an environment where you want to run the builder:

::::{tab-set}

:::{tab-item} conda-workspaces

```bash
conda create -n cs-demo -c conda-forge python pip conda-workspaces
conda activate cs-demo
python -m pip install conda-ship
```

Check that both commands are available:

```bash
conda ship --help
conda workspace --help
```

:::

:::{tab-item} Pixi

```bash
conda create -n cs-demo -c conda-forge python pip pixi
conda activate cs-demo
python -m pip install conda-ship
```

Check that both commands are available:

```bash
cs --version
pixi --version
```

:::

::::

## Create A Project

Create an empty project directory:

```bash
mkdir demo-runtime
cd demo-runtime
```

Then choose the manifest tool you want to use. The two paths produce the same
runtime intent: a `ship` environment containing conda itself, the rattler solver
plugin, and conda-spawn for `demo shell`.

::::{tab-set}

:::{tab-item} conda-workspaces

Create a `conda.toml`:

```bash
conda workspace init --format conda --name demo-runtime
```

Add the runtime packages to a `ship` feature. `conda workspace add` creates the
matching `ship` environment in the manifest:

```bash
conda workspace add --feature ship --no-lockfile-update \
  "python>=3.12" \
  "conda>=25.1" \
  conda-rattler-solver \
  "conda-spawn>=0.1.0"
```

Add conda-ship's build policy:

```bash
cat >> conda.toml <<'TOML'

[tool.conda-ship]
runtime = "demo"
runtime-version = "0.1.0"
delegate = "conda"
layout = "online"
source-environment = "ship"
exclude = ["conda-libmamba-solver"]
TOML
```

:::

:::{tab-item} Pixi

Create a `pixi.toml`:

```bash
pixi init --channel conda-forge
```

Add the `ship` feature, `ship` environment, and conda-ship build policy:

```bash
cat >> pixi.toml <<'TOML'

[feature.ship.dependencies]

[environments]
ship = { features = ["ship"], no-default-feature = true }

[tool.conda-ship]
runtime = "demo"
runtime-version = "0.1.0"
delegate = "conda"
layout = "online"
source-environment = "ship"
exclude = ["conda-libmamba-solver"]
TOML
```

Use Pixi's native add command to put packages in the `ship` feature:

```bash
pixi add --feature ship --no-install \
  "python>=3.12" \
  "conda>=25.1" \
  conda-rattler-solver \
  "conda-spawn>=0.1.0"
```

:::

::::

## Lock The Project

Solve the source lockfile with the tool that owns the manifest:

::::{tab-set}

:::{tab-item} conda-workspaces

```bash
conda workspace lock
```

This writes `conda.lock`.

:::

:::{tab-item} Pixi

```bash
pixi lock
```

This writes `pixi.lock`. The earlier `pixi add --no-install` command may have
already refreshed it; running `pixi lock` here makes the tutorial state
explicit.

:::

::::

conda-ship consumes the matching lockfile; it does not solve directly from
loose package names during normal builds. The builder will derive its own
runtime lock from this source lockfile.

## Inspect The Package Set

Run a preflight check before building. This derives the runtime package set,
applies exclusions, and prints the selected packages without writing files:

::::{tab-set}

:::{tab-item} conda-workspaces

```bash
conda ship inspect
```

:::

:::{tab-item} Pixi

```bash
cs inspect
```

:::

::::

The output lists the manifest and lockfile conda-ship selected, each locked
platform, and the package set for your current platform.

## Build The Runtime

Build an online runtime named `demo`:

::::{tab-set}

:::{tab-item} conda-workspaces

```bash
conda ship build
```

:::

:::{tab-item} Pixi

```bash
cs build
```

:::

::::

The generated runtime is written to `dist/demo` on Unix and `dist/demo.exe` on
Windows.

An online runtime contains the lockfile and runtime metadata. It downloads conda
package archives when it bootstraps.

## Smoke-Test The Runtime

For this tutorial, bootstrap the generated runtime into a temporary local path
to prove that the artifact works:

```bash
mkdir -p .tmp
./dist/demo --path "$PWD/.tmp/demo" bootstrap
```

This creates a conda installation managed by the `demo` runtime. This local
bootstrap is only a smoke test; a real downstream distribution should document
how its users install and update the runtime it publishes.

The runtime also writes conda prefix metadata during bootstrap:

```bash
ls "$PWD/.tmp/demo/conda-meta/history"
ls "$PWD/.tmp/demo/conda-meta/initial-state.explicit.txt"
```

`history` lets conda recognize the install path as an environment.
`initial-state.explicit.txt` records the exact package URLs from the stamped
runtime lock. If your runtime package set includes `conda-self`, that file is
the installer snapshot used by `conda self reset --snapshot installer-updated`
and `conda self reset --snapshot installer-exact`.

```{note}
The explicit `--path` keeps this tutorial install inside the project directory.
Published runtimes should document their normal install location and reserve
`--path` for local testing or advanced overrides.
```

Check it:

```bash
./dist/demo --path "$PWD/.tmp/demo" status
```

The status output shows the install path, configured channels, package metadata,
installed package count, and delegate executable path.

Clean up the temporary install:

```bash
./dist/demo --path "$PWD/.tmp/demo" uninstall --yes
```

## Optional: Build An Embedded Runtime

The embedded layout puts compressed package archives inside the generated binary.
This makes the build slower and the binary larger, but bootstrap no longer needs
to download package archives.

::::{tab-set}

:::{tab-item} conda-workspaces

```bash
conda ship build --layout embedded
```

:::

:::{tab-item} Pixi

```bash
cs build --layout embedded
```

:::

::::

Embedded runtimes use the `z` suffix, so this stages `dist/demoz` on Unix and
`dist/demoz.exe` on Windows.

Smoke-test it:

```bash
./dist/demoz --path "$PWD/.tmp/demoz" bootstrap
./dist/demoz --path "$PWD/.tmp/demoz" status
./dist/demoz --path "$PWD/.tmp/demoz" uninstall --yes
```

## What You Learned

You created a small workspace project, solved it, built an online runtime, and
used that binary to install and manage its own conda prefix in a temporary smoke
test.

For a real downstream distribution, choose a runtime name owned by that
distribution, keep its package choices in the source manifest, and publish the
staged files from `dist/`.
