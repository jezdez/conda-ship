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
runtime = "cx"
install-name = "express"
```

With the `conda-home` scheme, that runtime installs below `~/.conda/express`.

## Runtime `--path`

Users can override the resolved install path at runtime:

```bash
RUNTIME --path /tmp/demo bootstrap
RUNTIME --path /tmp/demo status
RUNTIME --path /tmp/demo uninstall --yes
```

This is intentionally a runtime option, not a build-time path. Build artifacts
should remain cross-platform. A path that makes sense on one build machine may
not make sense for users on another operating system.

## Ownership Metadata

After bootstrap, the runtime writes a metadata file inside the managed prefix.
It records:

- schema version
- display name
- install name
- metadata filename
- runtime version
- channels
- package names

Later operations check that metadata before using or removing a prefix.

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
This protects existing conda installations from accidental mutation or deletion.

`bootstrap --force`, pass-through commands, `status`, and `uninstall` all use
ownership checks before touching an existing prefix.

## Uninstall

`RUNTIME uninstall` removes the managed install path. It does not remove the
runtime binary itself because that binary may be owned by Homebrew, a conda
package, a constructor installer, Docker, or another channel.

If `install-method` was stamped into the runtime, uninstall prints it as a hint
for removing the runtime binary after the managed prefix is gone.
