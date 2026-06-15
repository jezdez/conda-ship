# Quickstart

Use this page when you want the shortest local path from an empty directory to a
staged runtime artifact. It uses conda-workspaces. The full first-runtime
tutorial also covers Pixi, bootstrap, status, uninstall, and embedded builds.

## Install The Builder

Create an environment with conda-workspaces, then install conda-ship:

```bash
conda create -n cs-demo -c conda-forge python pip conda-workspaces
conda activate cs-demo
python -m pip install conda-ship
```

Check that the builder is available:

```bash
cs --version
conda workspace --help
```

## Create A Runtime Project

Create a project and add the packages required by generated conda runtimes:

```bash
mkdir demo-runtime
cd demo-runtime
conda workspace init --format conda --name demo-runtime
conda workspace add --feature ship --no-lockfile-update \
  "python>=3.12" \
  "conda>=25.1" \
  conda-rattler-solver \
  "conda-spawn>=0.1.0"
```

Add conda-ship build policy:

```bash
cat >> conda.toml <<'TOML'

[tool.conda-ship]
runtime-name = "demo"
runtime-version = "0.1.0"
delegate = "conda"
layout = "online"
source-environment = "ship"
exclude = ["conda-libmamba-solver"]
TOML
```

## Lock And Build

Solve the source lockfile, inspect the derived runtime package set, preview the
build, and write the runtime artifact:

```bash
conda workspace lock
cs inspect
cs build --dry-run
cs build
```

```{figure} ../../demos/quickstart.gif
:alt: Terminal recording of the conda-ship quickstart inspect, dry-run, build, and version checks.

Quickstart: inspect, preview, build, and run a stamped runtime.
```

The online runtime is staged at `dist/demo` on Unix and `dist/demo.exe` on
Windows. Check the stamped runtime metadata:

```bash
./dist/demo --version
```

Next, follow the [first runtime tutorial](first-runtime.md) to bootstrap the
runtime into a temporary install path and clean it up again.
