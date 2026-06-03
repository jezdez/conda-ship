# Environment Variables

This page lists environment variables used by conda-ship and generated
runtimes.

## Builder Variables

`CONDA_SHIP_TEMPLATE`
: Path to a prebuilt generic runtime template. `cs build` uses this when
  `--template` is not supplied and no installed template is found next to `cs`.

`CONDA_SHIP_EXECUTABLE`
: Path used by the Python `conda ship` adapter to find the `cs` executable.
  This is mainly useful for source checkouts, tests, and custom packaging. If
  set, it must point to an executable file. The adapter does not fall back to
  another `cs` when this value is invalid.

`CONDA_SHIP_ERROR_FORMAT`
: Internal diagnostic format used by the Python `conda ship` adapter. When set
  to `json`, `cs` writes a single structured JSON diagnostic line for builder
  failures. The adapter parses that line and renders a normal conda-facing
  error. Users normally do not need to set this themselves.

## Runtime Variables

`RUNTIME_BUNDLE`
: Runtime-specific path to an external package bundle directory. The actual
  variable name is based on the runtime name. Non-alphanumeric characters become
  underscores and letters are uppercased. For `demo`, the variable is
  `DEMO_BUNDLE`.

`RUNTIME_OFFLINE`
: Runtime-specific flag for offline bootstrap mode. For `demo`, the variable is
  `DEMO_OFFLINE`. Empty, `0`, and `false` disable the flag; other non-empty
  values enable it.

## Runtime Delegate Environment

When a runtime runs its delegate, it sets a conda-like base environment:

`CONDA_ROOT_PREFIX`
: Managed prefix path.

`CONDA_PREFIX`
: Managed prefix path.

`CONDA_DEFAULT_ENV`
: `base`.

`CONDA_SHLVL`
: `1`.

`CONDA_COMPLETION_COMMAND_NAME`
: Runtime executable name. Shell completion integrations can use this to
  register the wrapper command instead of the delegate executable. For a
  runtime named `cx`, this is `cx`.

`PATH`
: Managed prefix executable directories first, followed by the existing `PATH`.

On Unix, the runtime prepends:

- `bin`
- `condabin`

On Windows, the runtime prepends:

- the root prefix
- `Library/mingw-w64/bin`
- `Library/usr/bin`
- `Library/bin`
- `Scripts`
- `bin`
- `condabin`

## Test And Development Variable

`CONDA_SHIP_ALLOW_UNSTAMPED_TEMPLATE`
: Allows the generic runtime template binary to run without stamped runtime
  data. This is used by tests. Downstream runtimes should not set it.

```{warning}
Do not set `CONDA_SHIP_ALLOW_UNSTAMPED_TEMPLATE` in distribution builds or user
environments. It exists only so tests can exercise the generic template binary.
```
