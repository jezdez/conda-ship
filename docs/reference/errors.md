# Errors

This page lists common conda-ship errors and the usual fix.

`cs` prints human-readable diagnostics by default. The `conda ship` adapter uses
an internal structured diagnostic mode so it can show the same errors through
conda without depending on terminal formatting.

## Project Input

`could not find project root containing conda.toml, pixi.toml, or supported pyproject.toml`
: Run from the project root or pass `--root PATH`.

`could not find conda.toml, pixi.toml, or supported pyproject.toml`
: Add a supported manifest to the selected root.

`lockfile not found`
: Refresh and commit the matching source lockfile with `conda workspace lock`
  or `pixi lock`.

`source environment is required`
: Set `[tool.conda-ship].source-environment`.

`source environment "NAME" not found`
: Add the environment to the source manifest and refresh the source lockfile.

`runtime version is required`
: Pass `--runtime-version`, set `[tool.conda-ship].runtime-version`, define
  static `[project].version` in the selected `pyproject.toml`, or opt into
  `runtime-version = { from = "project-metadata" }` through the Python
  `conda ship` adapter for dynamic Python project versions.

## Runtime Package Set

`selected source environment ... is missing required package(s)`
: Add the missing runtime packages to the selected source environment:
  `conda`, `conda-rattler-solver`, and `conda-spawn`.

`cannot bundle packages without SHA256 hashes`
: Refresh the source lockfile with package hash metadata before building
  `external` or `embedded` layouts.

`no default environment in ... runtime.lock`
: The derived runtime lock is malformed. Rebuild from the source lockfile.

## Template Selection

`runtime template not found`
: Install a conda-ship package that includes `cs-template`, set
  `CONDA_SHIP_TEMPLATE`, or pass `--template PATH`.

`cross-builds require --template`
: Pass a prebuilt runtime template for the requested target.

`runtime template is already stamped`
: `--template` points at a generated runtime instead of a generic template.

## Naming

`runtime name must start with an ASCII letter or digit`
: Use a filename-safe runtime name such as `demo` or `demo-runtime`.

`delegate executable may only contain ASCII letters, digits, dots, dashes, and underscores`
: Use an executable name, not a path.

`target triple may only contain ASCII letters, digits, dots, dashes, and underscores`
: Use a target triple string, not a path to a custom target file.

`installer may only contain ASCII letters, digits, dots, dashes, and underscores`
: Use a short installer name such as `homebrew`, `conda-package`, or `standalone`.

## Runtime Bootstrap

`runtime template, not a runnable conda runtime`
: Run a binary produced by `cs build`, not the generic runtime template.

`runtime has no stamped lockfile`
: The binary is not a properly stamped runtime. Rebuild it with `cs build`.

`--offline requires a stamped runtime lock`
: Offline bootstrap requires a runtime built by `cs build`.

`--bundle path is not a directory`
: Pass a directory containing package archive files, not the compressed
  `.bundle.tar.zst` file itself.

## Prefix Ownership

`refusing to bootstrap into existing non-empty path`
: Choose another `--path` or remove the existing directory yourself.

`refusing to use unmanaged install path`
: The prefix does not contain ownership metadata for this runtime.

`refusing to remove symbolic-link install path`
: The runtime will not recursively remove a symlink path.

`refusing to remove dangerous path`
: The resolved path is too broad, such as a home directory or filesystem root.
