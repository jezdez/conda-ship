# Build A Runtime With A Custom Delegate

This tutorial builds a runtime whose pass-through command is `python` instead of
`conda`.

The managed prefix only needs Python and its dependencies. conda-ship does not
require conda, conda-rattler-solver, or conda-spawn when the distribution does
not use them.

## Before You Start

Install conda-ship and either conda-workspaces or Pixi:

::::{tab-set}

:::{tab-item} conda-workspaces

```bash
conda install --name base -c conda-forge conda-pypi
conda create -n cs-python-demo -c conda-forge python pip conda-workspaces
conda activate cs-python-demo
conda pypi install conda-ship
```

:::

:::{tab-item} Pixi

```bash
conda install --name base -c conda-forge conda-pypi
conda create -n cs-python-demo -c conda-forge python pip pixi
conda activate cs-python-demo
conda pypi install conda-ship
```

:::

::::

If you prefer not to install `conda-pypi` into `base`, use
`python -m pip install conda-ship` in the activated environment instead.

## Create The Project

Create a project directory:

```bash
mkdir python-runtime
cd python-runtime
```

::::{tab-set}

:::{tab-item} conda-workspaces

```bash
conda workspace init --format conda --name python-runtime
conda workspace add --feature ship --no-lockfile-update \
  "python>=3.12"
```

Add conda-ship policy:

```bash
cat >> conda.toml <<'TOML'

[tool.conda-ship]
runtime-name = "pydemo"
runtime-version = "0.1.0"
delegate-executable = "python"
artifact-layout = "online"
source-environment = "ship"
TOML
```

Lock it:

```bash
conda workspace lock
```

:::

:::{tab-item} Pixi

```bash
pixi init --channel conda-forge
cat >> pixi.toml <<'TOML'

[feature.ship.dependencies]

[environments]
ship = { features = ["ship"], no-default-feature = true }

[tool.conda-ship]
runtime-name = "pydemo"
runtime-version = "0.1.0"
delegate-executable = "python"
artifact-layout = "online"
source-environment = "ship"
TOML
pixi add --feature ship --no-install \
  "python>=3.12"
pixi lock
```

:::

::::

## Build It

Run a preflight, then build:

```bash
cs inspect
cs build
```

The runtime is staged as `dist/pydemo` on Unix and `dist/pydemo.exe` on Windows.

## Bootstrap It

Use a temporary install path for the tutorial:

```bash
mkdir -p .tmp
./dist/pydemo --path "$PWD/.tmp/pydemo" bootstrap
```

The runtime installs the selected Python environment into the managed prefix.

## Run Python Through The Runtime

Create a small script:

```bash
cat > hello.py <<'PY'
import sys

print("hello from", sys.executable)
PY
```

Run it through the runtime:

```bash
./dist/pydemo --path "$PWD/.tmp/pydemo" hello.py
```

`pydemo` bootstraps or reuses its managed prefix, prepares the delegate
environment, and passes `hello.py` to the `python` executable inside that
prefix.

## Clean Up

Remove the tutorial install path:

```bash
./dist/pydemo --path "$PWD/.tmp/pydemo" uninstall --yes
```

## What You Learned

The `delegate-executable` is the executable that receives pass-through
arguments after the runtime is bootstrapped. Use `delegate-executable = "conda"`
for conda-like distributions, and another executable when the runtime should
present a smaller or different command surface. The selected source environment
only needs to include that delegate and its runtime dependencies.
