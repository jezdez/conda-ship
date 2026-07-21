# Install Locations And Ownership

Generated runtimes install into managed prefixes. They also record ownership
metadata so later operations can tell whether a prefix belongs to that runtime.

## Install Schemes

The install scheme is stamped at build time.

`conda-home`
: Installs below `~/.conda/INSTALL_NAME`.

`user-data`
: Installs below the platform user data directory:

  - Linux: `${XDG_DATA_HOME:-~/.local/share}/conda/INSTALL_NAME`
  - macOS: `~/Library/Application Support/conda/INSTALL_NAME`
  - Windows: `%LOCALAPPDATA%\\conda\\INSTALL_NAME`

The default is `conda-home`.

## Install Name

The install name is the final directory name inside the scheme.

If omitted, conda-ship uses the runtime name. A downstream distribution can use
a short executable name and a clearer install name:

```toml
[tool.conda-ship]
runtime-name = "cx"
install-name = "express"
```

With the `conda-home` scheme, that runtime installs below `~/.conda/express`.

## Runtime Prefix Override

Users can override the resolved install path with the prefix environment
variable derived from the runtime name:

```bash
DEMO_PREFIX=/tmp/demo demo info
```

For a runtime named `demo`, the variable is `DEMO_PREFIX`. Non-alphanumeric
characters in the runtime name become underscores and letters are uppercased.
The override remains a runtime choice so build artifacts stay cross-platform.

For a local `cs run` smoke test, use the builder-side option instead:

```bash
cs run --install-path /tmp/demo -- info
```

## Ownership Metadata

After automatic bootstrap, the runtime writes a metadata file inside the
managed prefix.
It records:

- schema version
- display name
- install name
- metadata filename
- runtime version
- channels
- package names

Later runtime invocations check that metadata before reusing a prefix.

This ownership file is conda-ship-specific. The runtime also writes standard
conda prefix metadata:

- `conda-meta/history`
- `conda-meta/initial-state.explicit.txt`

Those files serve a different purpose. `history` lets conda recognize the
managed prefix as a conda environment. `initial-state.explicit.txt` records the
exact packages that were installed from the runtime lock at bootstrap time.
Tools that understand constructor-style installer snapshots, including
`conda-self`, can use that explicit file to reset the managed base prefix back
to the package set originally shipped by the runtime.

## Why Runtimes Refuse Unmanaged Prefixes

A runtime can find an existing directory at its install path. That directory may
be:

- a prefix created by the same runtime
- a prefix created by another runtime
- a normal conda installation
- an unrelated directory

conda-ship-generated runtimes refuse to operate on non-empty unmanaged prefixes.
This protects existing conda installations from accidental mutation.

Automatic bootstrap and later delegate invocations use ownership checks before
reusing an existing prefix.

## Lifecycle Commands

The generated runtime does not own `status`, `repair`, or `uninstall` commands.
Those names are passed to the configured delegate like every other argument.

For conda delegates, use `conda info` for status. Use `conda doctor` and its
supported fixes to diagnose and repair an installed prefix. Installer snapshot
and self-management commands can come from conda-self when a distribution
includes it. Removal of the runtime binary remains the responsibility of the
package manager or installer that placed it.
