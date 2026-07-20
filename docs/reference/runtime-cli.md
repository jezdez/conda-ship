# Runtime CLI Reference

Every conda-ship artifact includes a generated runtime. In this page,
`RUNTIME` stands for the staged executable name resolved by `cs build` from
`[tool.conda-ship].runtime-name`, `--runtime-name`, or an explicit
`artifact-name`.
`DELEGATE` stands for the executable inside the managed prefix that receives
pass-through arguments.

For conda-express, `RUNTIME` is `cx`. A release artifact can be staged as `cxz`
by configuring `artifact-name`.

## Global Options

`-v, --verbose`
: Increase output detail.

`-q, --quiet`
: Suppress non-essential output.

`--path PATH`
: Use a custom install path instead of the distribution default. This applies to
  `bootstrap`, `status`, `shell`, `uninstall`, and pass-through delegate commands.
  Put it before pass-through commands so conda does not interpret it as one of
  its own options, for example `RUNTIME --path /tmp/demo install numpy`.

`-h, --help`
: Show runtime help.

`-V, --version`
: Show the runtime version.

## `RUNTIME bootstrap`

Install conda into the runtime's install path.

```bash
RUNTIME bootstrap [OPTIONS]
```

Options:

`--force`
: Remove an existing install path before bootstrapping again.

`--bundle DIR`
: Pre-populate the package cache from a directory containing `.conda` or
  `.tar.bz2` archives. The directory is treated as a flat package archive
  bundle, not as a conda channel mirror.

`--offline`
: Disable network access. Packages must be available from the local cache, an
  explicit `--bundle`, or an embedded bundle.

After package installation, bootstrap writes conda-ship ownership metadata,
`.condarc`, the CEP 22 frozen marker, and the prefix metadata that conda tools
expect in `conda-meta/history` and `conda-meta/initial-state.explicit.txt`.
The initial-state file records the package URLs and checksums from the stamped
runtime lock so installer reset tools such as `conda-self` can return the
managed base prefix to the shipped package set.

Examples:

```bash
# Standard network bootstrap from the stamped lockfile
RUNTIME bootstrap

# Re-bootstrap into the default install path
RUNTIME bootstrap --force

# Bootstrap into a custom install path
RUNTIME --path /opt/name bootstrap

# Bootstrap from an external bundle directory
RUNTIME bootstrap --bundle ./packages --offline
```

For an embedded runtime, conda-ship detects the built-in bundle
automatically:

```bash
RUNTIME bootstrap
```

An explicit `--bundle` still takes priority over the embedded bundle.
Embedded bundle extraction rejects anything except top-level `.conda` and
`.tar.bz2` package archive files.

## `RUNTIME status`

Show runtime and install details.

```bash
RUNTIME [--path PATH] status
```

The output includes the runtime name, runtime version, install path, configured
channels, configured package specs, installed package count, and delegate
executable path for the managed prefix.

## `RUNTIME shell`

Start a conda-spawn subshell for an environment.

```bash
RUNTIME shell [ENV]
```

Examples:

```bash
RUNTIME shell myenv
exit
```

This command delegates to `conda spawn`. It uses the runtime's default install
path. It is available when the downstream distribution includes `conda-spawn`
in its selected source environment.

## `RUNTIME uninstall`

Remove the install path and named environments.

```bash
RUNTIME uninstall [OPTIONS]
```

`-y, --yes`
: Skip the interactive confirmation prompt.

The command removes the managed install path, including named environments
below it, and prints a hint for removing the runtime through the package manager
or installer that provided the runtime binary.

## `RUNTIME help`

Show the runtime help text.

```bash
RUNTIME help
```

```{figure} ../../demos/runtime-cli.gif
:alt: Terminal recording of a generated runtime printing its version and runtime-specific activation guidance.

Generated runtimes expose their own command surface before pass-through to the delegate.
```

## Pass-Through Commands

Any command not listed above is passed through to the configured delegate
executable after bootstrap. For a runtime whose delegate is `conda`, this looks
like:

```bash
RUNTIME create -n myenv python=3.12 numpy
RUNTIME install -n myenv pandas
RUNTIME list -n myenv
RUNTIME env list
RUNTIME info

# Use a custom runtime install path for pass-through commands
RUNTIME --path /tmp/name install -n myenv pandas
```

If the install path does not exist, pass-through commands automatically
bootstrap first.

The delegate process receives a conda-like base environment: `CONDA_ROOT_PREFIX`,
`CONDA_PREFIX`, `CONDA_DEFAULT_ENV=base`, `CONDA_SHLVL=1`, and a `PATH` with the
managed prefix's executable directories first. On Unix this includes `bin` and
`condabin`; on Windows this includes the root prefix, `Library` binary
directories, `Scripts`, `bin`, and `condabin`.

## Disabled Shell Commands

Generated runtimes can use conda-spawn for activation when it is included in
the selected source environment. When the delegate is `conda`, these commands
are intercepted with runtime-specific guidance instead of being passed
through:

- `RUNTIME activate`
- `RUNTIME deactivate`
- `RUNTIME init`

Use `RUNTIME shell ENV` instead of `conda activate ENV`.
