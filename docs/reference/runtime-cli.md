# Generated Runtime CLI Reference

Every Pronto artifact is a named runtime binary. In this page, `NAME` stands for
the distribution name passed to `pronto build --name NAME`.

For conda-express, `NAME` is `cx`. For an embedded conda-express artifact, the
staged binary is `cxz`.

## Global Options

`-v, --verbose`
: Increase output detail.

`-q, --quiet`
: Suppress non-essential output.

`-h, --help`
: Show runtime help.

`-V, --version`
: Show the runtime version.

## `NAME bootstrap`

Install conda into the runtime's managed prefix.

```bash
NAME bootstrap [OPTIONS]
```

Options:

`--force`
: Remove an existing managed prefix before bootstrapping again.

`--prefix DIR`
: Install into a custom prefix instead of the distribution default, `~/.NAME`.

`-c, --channel CH`
: Add a channel for a live solve. Can be passed multiple times.

`-p, --package SPEC`
: Add a package spec for a live solve. Can be passed multiple times. Use with
  `--no-lock`; locked bootstraps install the package set recorded in the
  runtime lock.

`--no-lock`
: Ignore the stamped runtime lock and perform a live solve. Requires network
  access.

`--lockfile PATH`
: Use an external rattler-lock file instead of the stamped runtime lock.

`--bundle DIR`
: Pre-populate the package cache from a directory containing `.conda` or
  `.tar.bz2` archives.

`--offline`
: Disable network access. Packages must be available from the local cache, an
  explicit `--bundle`, or an embedded bundle.

Examples:

```bash
# Standard network bootstrap from the stamped lockfile
NAME bootstrap

# Re-bootstrap into the default prefix
NAME bootstrap --force

# Bootstrap into a custom prefix
NAME bootstrap --prefix /opt/name

# Live solve with extra packages
NAME bootstrap --no-lock --package conda-build --package rattler-build

# Bootstrap from an external bundle directory
NAME bootstrap --bundle ./packages --offline
```

For an embedded artifact, Pronto detects the built-in bundle automatically:

```bash
NAMEz bootstrap
```

An explicit `--bundle` still takes priority over the embedded bundle.

## `NAME status`

Show runtime and prefix details.

```bash
NAME status [--prefix DIR]
```

The output includes the binary name, runtime version, prefix, configured
channels, configured package specs, installed package count, and conda
executable path.

## `NAME shell`

Start a conda-spawn subshell for an environment.

```bash
NAME shell [ENV]
```

Examples:

```bash
NAME shell myenv
exit
```

This command delegates to `conda spawn`. It uses the runtime's default managed
prefix.

## `NAME uninstall`

Remove the managed prefix and named environments.

```bash
NAME uninstall [OPTIONS]
```

Options:

`--prefix DIR`
: Remove a custom managed prefix instead of the distribution default,
  `~/.NAME`.

`-y, --yes`
: Skip the interactive confirmation prompt.

The command removes the prefix, attempts to remove named environments cleanly,
cleans PATH entries from common shell profiles, and prints a hint for removing
the runtime binary through the package manager or install method that provided
it.

## `NAME help`

Show the runtime help text.

```bash
NAME help
```

## Pass-Through Commands

Any command not listed above is passed through to the installed conda
executable after bootstrap:

```bash
NAME create -n myenv python=3.12 numpy
NAME install -n myenv pandas
NAME list -n myenv
NAME env list
NAME info
```

If the managed prefix does not exist, pass-through commands automatically
bootstrap first.

## Disabled Shell Commands

Generated runtimes use conda-spawn for activation. These commands are
intercepted with runtime-specific guidance instead of being passed through:

- `NAME activate`
- `NAME deactivate`
- `NAME init`

Use `NAME shell ENV` instead of `conda activate ENV`.
