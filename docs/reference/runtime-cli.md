# Generated Runtime Reference

Every conda-ship artifact is a stamped copy of the generic runtime template. In
this page, `RUNTIME` stands for the staged executable name and `DELEGATE`
stands for the configured executable inside the managed prefix.

The generated runtime does not expose a conda-ship CLI. It owns only the
bootstrap boundary needed to make the delegate available.

## First Invocation

When the managed prefix is absent, the first invocation automatically installs
the stamped package set and then executes the delegate with the original
arguments:

```bash
RUNTIME info
```

For a conda delegate, that invocation bootstraps the prefix and then runs
`conda info`. For a Python delegate, this invocation bootstraps the prefix and
then runs `python --version`:

```bash
RUNTIME --version
```

A bare invocation also bootstraps and then invokes the delegate without
arguments.

The same behavior applies to `online`, `external`, and `embedded` artifacts.
Online artifacts download packages from the stamped runtime lock. External
artifacts read archives from the configured bundle directory. Embedded
artifacts automatically extract their built-in bundle.

During bootstrap, the runtime writes conda-ship ownership metadata and the
prefix metadata expected by conda tools in `conda-meta/history` and
`conda-meta/initial-state.explicit.txt`. It writes `.condarc` only when the
build configured `condarc-file`, and writes the CEP 22 frozen marker only when
the build configured `freeze-base = true`.

Bootstrap is serialized with a process lock next to the managed prefix. An
internal `installing` marker identifies an incomplete prefix owned by this
runtime. The runtime metadata file is written after package installation,
post-link scripts, prefix metadata, configured policy, bytecode compilation,
and delegate validation finish. Its `ready` state marks bootstrap complete.

If bootstrap is interrupted, the next invocation automatically retries only
when that internal marker belongs to the same stamped runtime. Recovery forces
every locked package through Rattler's reinstall path so post-link scripts run
again. It does not delete the prefix, named environments, or unrelated paths.
An unknown non-empty prefix is still refused.

## Delegate Execution

After the prefix is available, every argument belongs to the delegate. The
runtime does not reserve or rewrite any of these names:

- `--help` and `--version`
- `status` and `uninstall`
- `shell` and `self`
- `activate`, `deactivate`, and `init`
- delegate verbosity and quiet options

Standard input, output, and error pass through unchanged, and the runtime
preserves the delegate's signal and exit behavior. It does not filter delegate
output or set `CONDA_PREFIX`, `CONDA_DEFAULT_ENV`, or `CONDA_SHLVL`.

For a conda delegate, normal commands therefore look like direct conda
commands:

```bash
RUNTIME create -n myenv python=3.12 numpy
RUNTIME install -n myenv pandas
RUNTIME list -n myenv
RUNTIME env list
RUNTIME info
RUNTIME --help
RUNTIME --version
```

`RUNTIME info` is the normal status command for a conda delegate. If the
distribution includes conda-spawn with the alias implemented by
[conda-spawn PR #59](https://github.com/conda/conda-spawn/pull/59),
`RUNTIME shell` uses the conda-spawn alias for `conda spawn`.

Use `conda doctor` and its supported fixes to diagnose and repair an installed
prefix. Use the commands supplied by conda-self for installer snapshots and
self-management when the distribution includes that plugin.

## Bootstrap Controls

Bootstrap controls are environment variables derived from the configured
runtime name. Non-alphanumeric characters become underscores and letters are
uppercased.

For a runtime named `demo`, the variables are:

`DEMO_PREFIX`
: Override the managed prefix path.

`DEMO_BUNDLE`
: Path to an external directory containing the package archives named in the
  stamped runtime lock.

`DEMO_OFFLINE`
: Disable network access during bootstrap. Empty, `0`, and `false` disable the
  flag. Other non-empty values enable it.

Example:

```bash
DEMO_PREFIX=/opt/demo \
DEMO_BUNDLE=/opt/demo-bundle \
DEMO_OFFLINE=1 \
demo info
```

The runtime reads these values only to locate and bootstrap the managed prefix.
They do not consume or replace delegate arguments.

Embedded artifacts need no bundle or offline override:

```bash
DEMO_PREFIX=/opt/demo demo info
```

For local smoke tests through the builder, prefer `cs run --install-path PATH`
instead of setting the prefix variable yourself:

```bash
cs run --install-path /tmp/demo-smoke -- info
```
