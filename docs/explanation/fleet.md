# Fleet Concepts

Fleet is an experimental Rust API inside the `conda-ship` crate. It is enabled
only with the non-default Cargo feature `fleet`.

Stamped runtime artifacts remain the primary conda-ship output. On first
invocation, a stamped artifact bootstraps one prefix and then passes the
original arguments to its configured delegate. The delegate can be `conda` or
any other installed entry point. The artifact does not reserve bootstrap,
status, repair, update, or uninstall commands.

Fleet is a separate, optional library surface for downstream orchestrators that
need several locked prefixes. It does not create a new conda-ship distribution
or CLI. Each runtime is installed under:

```text
install_root/<id>
```

The runtime id also selects its conda-ship metadata file:

```text
install_root/<id>/.<id>.json
```

## Shared Bootstrap Mechanics

Fleet and stamped artifacts use the same adjacent process lock, installing
marker, ready metadata commit, delegate validation, and full-lock reinstall
path. An interrupted owned install is retried by reinstalling every package in
the selected lock. This reruns link scripts while preserving named environments
and unrelated files under the prefix.

`Fleet::install` refuses unknown non-empty prefixes. `force = true` means an
in-place full-lock reinstall of a prefix already owned by the same Fleet runtime
id. It does not recursively replace the prefix. Recursive deletion happens only
through an explicit `Fleet::remove` call.

## Source Of Truth

Fleet has no separate registry database. `Fleet::list()` scans direct children
of the install root. `Fleet::get(id)` validates the exact regular metadata file,
its ready state, its runtime identity, and its recorded delegate.

Fleet metadata records the explicit delegate and the SHA256 digest of the lock
content. It also records lockfile channels as provenance. Those channels are
not turned into an implicit `.condarc`.

`RuntimeSpec` is the explicit programmatic input to `Fleet::install`, not a new
user-facing catalog format. A downstream orchestrator should derive it from its
own selected descriptor, catalog entry, or conda-ship stamped artifact data.

## Explicit Downstream Policy

The caller decides whether each runtime receives:

- exact downstream-owned `.condarc` content
- a CEP 22 frozen-base marker
- Constructor-compatible `.installer.info` provenance

All three are disabled by default. During a Fleet install or forced reinstall,
the three paths are exact managed outputs. Fleet safely removes the prior
regular files and rewrites only the enabled outputs. Symbolic links and other
nonregular entries are refused before package installation begins.

`.installer.info` is distribution provenance. It is not launcher ownership
evidence.

## Command And Launcher Boundary

Fleet returns executable paths and prefix-local PATH entries. It does not
fabricate `CONDA_PREFIX`, `CONDA_ROOT_PREFIX`, `CONDA_DEFAULT_ENV`,
`CONDA_SHLVL`, or completion variables. A delegate receives the same minimal
PATH treatment as a stamped runtime.

Shim plans are data only. Downstream orchestrators own shim contents, file
writes, overwrite policy, PATH setup, and removal. A launcher created by a
Fleet caller is externally managed and does not receive a conda-ship
direct-install launcher receipt. A receipt lookup for it therefore returns
`MissingReceipt`.

## Ownership Boundary

Fleet owns reusable mechanics close to conda-ship:

- installing a resolved lock into a known prefix
- shared package-cache, bundle, and offline behavior
- interrupted-install recovery and mutation locking
- constructor history and initial-state files
- ready prefix metadata and lock provenance
- explicit condarc, frozen-base, and installer-provenance outputs
- data-only command and shim plans

Downstream callers own product behavior:

- catalog lookup and runtime selection
- solving and lock production
- user-facing command and shim names
- global PATH and shell setup
- login, onboarding, enterprise policy, telemetry, and prompts
- update, uninstall, and migration workflows
- launcher installation and external package-manager guidance

## Experimental Status

The API has no stability promise. Consumers should pin a repository revision
and enable the feature explicitly:

```toml
[dependencies]
conda-ship = { git = "https://github.com/jezdez/conda-ship", rev = "<pinned-commit>", features = ["fleet"] }
```

Projects that use native TLS can keep that choice explicit:

```toml
[dependencies]
conda-ship = { git = "https://github.com/jezdez/conda-ship", rev = "<pinned-commit>", default-features = false, features = ["fleet", "native-tls"] }
```
