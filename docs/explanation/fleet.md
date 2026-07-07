# Fleet Concepts

Fleet is an experimental Rust API layer inside the `conda-ship` crate. It is
enabled with the non-default Cargo feature `fleet`.

The API is for orchestrators that need to manage several locked conda
prefixes, including prefixes that back conda-ship-built runtime binaries. Those
orchestrators can install a Miniconda-like runtime and separate tool runtimes
while keeping catalog lookup, user identity, onboarding, policy, and PATH setup
outside conda-ship.

## How Fleet Differs From Runtime Binaries

conda-ship runtime binaries are single bootstrappable artifacts. A generated
runtime owns one install prefix, one delegate executable, and one package set.
Users interact with that runtime through commands such as `bootstrap`, `status`,
`shell`, and `uninstall`.

Fleet is a library API. It does not produce a new end-user distribution and it
does not reserve a new CLI surface. It installs each runtime into:

```text
install_root/<id>
```

The runtime id is also used for the conda-ship metadata file:

```text
install_root/<id>/.<id>.json
```

## Source Of Truth

Fleet uses conda-ship prefix metadata as the source of truth. There is no
separate registry database in the first experimental API.

`Fleet::list()` scans direct children of the install root and returns only
prefixes with valid fleet metadata. Directories without valid metadata are
ignored. `Fleet::status(id)` validates the exact metadata file for that runtime
id.

Fleet-installed metadata also records the SHA256 digest of the lock content, so
an orchestrator can compare its candidate lock with the installed runtime
without maintaining a separate registry or hash sidecar.

`RuntimeSpec` is the explicit API input to `Fleet::install`, not a user-facing
specification format. Production callers should derive it from their own
catalog, downloaded descriptor, or conda-ship-generated runtime metadata.

## What Fleet Owns

Fleet owns reusable mechanics that belong close to conda-ship:

- installing a resolved lockfile into a known prefix
- using rattler's package cache and optional bundle or offline install modes
- writing `.condarc`
- deriving `.condarc` channels from the default lockfile environment
- writing CEP 22 `conda-meta/frozen`
- writing constructor-compatible `conda-meta/history`
- writing `conda-meta/initial-state.explicit.txt`
- writing conda-ship prefix metadata
- refusing unmanaged non-empty prefixes
- returning data-only command and shim plans

## What Callers Own

Fleet intentionally leaves product and policy decisions to callers:

- catalog lookup and runtime selection
- user-facing command names
- global PATH setup
- shim file creation, overwrite policy, and removal
- login, onboarding, and enterprise policy
- update, repair, and migration workflows
- telemetry and user prompts

Downstream orchestrators should keep their own runtime or tool catalog. That
catalog can point at conda-ship artifacts, downloaded descriptors, or embedded
locks, but fleet should only receive the resolved `RuntimeSpec` after the
caller has already decided what to install.

This keeps the first API small while still letting downstream tools share the
same install mechanics as conda-ship runtime binaries.

## Experimental Status

The API has no stability promise yet. Downstream consumers should depend on the
repository by git revision and enable the `fleet` feature explicitly:

```toml
[dependencies]
conda-ship = { git = "https://github.com/jezdez/conda-ship", features = ["fleet"] }
```

Projects that already standardize on native TLS can keep that choice explicit:

```toml
[dependencies]
conda-ship = { git = "https://github.com/jezdez/conda-ship", default-features = false, features = ["fleet", "native-tls"] }
```
