# conda-fleet API Reference

`conda-fleet` is an experimental API behind the non-default Cargo feature
`fleet`.

Downstream Rust consumers enable the same feature:

```toml
[dependencies]
conda-ship = { git = "https://github.com/jezdez/conda-ship", features = ["fleet"] }
```

When embedding fleet in a native-TLS CLI, prefer:

```toml
[dependencies]
conda-ship = { git = "https://github.com/jezdez/conda-ship", default-features = false, features = ["fleet", "native-tls"] }
```

## Scope

Fleet installs and inspects locked conda prefixes. It does not solve, update,
repair, publish catalogs, write global shims, or edit shell startup files.

Every runtime is installed under `install_root/<id>`. Fleet reads conda-ship
prefix metadata from `install_root/<id>/.<id>.json` and has no separate registry
database.

## Core Types

`FleetConfig` selects the install root:

```rust
use conda_ship::fleet::{Fleet, FleetConfig};

let fleet = Fleet::new(FleetConfig::new("/tmp/fleet"));
```

`RuntimeSpec` is the complete install input. Downstream orchestrators normally
construct it from their own catalog, policy layer, downloaded descriptor, or
conda-ship-generated runtime metadata:

```rust
use conda_ship::fleet::RuntimeSpec;

let spec = RuntimeSpec {
    id: "conda".to_string(),
    version: "2026.6.0".to_string(),
    delegate_executable: "conda".to_string(),
    lock_content: std::fs::read_to_string("runtime.lock")?,
    requested_specs: vec!["conda".to_string()],
};
```

`RuntimeSpec::validate()` checks the runtime id, delegate executable, version,
and lock content before installation.

`RuntimeSpec::lock_sha256()` returns the digest fleet records in prefix
metadata. Callers can compare this value with `InstalledRuntime::lock_sha256`
from `Fleet::status(id)` to decide whether a locked runtime is already current.

Fleet derives `.condarc` channels from the default environment in the lockfile.
This lets callers that already have conda-ship runtime metadata or embedded
lockfiles avoid duplicating channel lists in their catalog.

## Install

Use `Fleet::install(spec, options)` with a resolved lockfile:

```rust
use conda_ship::fleet::InstallOptions;

let installed = fleet.install(spec, InstallOptions::default()).await?;
```

`InstallOptions` controls:

- `force`: replace an existing managed runtime with the same id
- `offline`: install without network access
- `bundle_dir`: pre-populate the shared rattler package cache from package
  archives
- `compile_python_bytecode`: compile bytecode when Python is present

Fleet refuses to install into unmanaged non-empty prefixes. When `force` is
true, it still validates that an existing non-empty prefix is managed by the
same runtime id before removing it.

## List, Status, And Remove

```rust
let runtimes = fleet.list()?;
let maybe_conda = fleet.status("conda")?;
fleet.remove("conda")?;
```

`Fleet::list()` scans direct children of the install root and ignores
directories without valid fleet metadata. `Fleet::status(id)` validates the
metadata for one runtime and returns `None` when the metadata file is absent.

`Fleet::remove(id)` removes only managed prefixes or empty directories. It
refuses unmanaged non-empty prefixes.

## Installed Runtime Inspection

`InstalledRuntime` contains the runtime id, version, prefix, delegate
executable, channels, lock SHA256, and requested specs. It also provides helper
methods for running or exposing binaries:

```rust
let runtime = fleet.status("conda")?.expect("installed");
let conda = runtime.command("conda")?;
let executable = runtime.executable_path("conda");
let env = runtime.activation_env("conda");
let path_entries = runtime.path_entries();
```

`activation_env()` returns conda environment variables but intentionally omits
`PATH`. Callers should prepend `path_entries()` to the existing `PATH` for child
processes or wrapper scripts.

## Binary And Shim Best Practices

Fleet provides data-only helpers so callers can implement exposure safely:

```rust
use conda_ship::fleet::ShimOptions;

let plan = runtime.shim_plan(
    "conda",
    ShimOptions {
        shim_name: "conda".to_string(),
        shim_dir: None,
    },
)?;
```

Recommended caller behavior:

- Prefer wrapper scripts for conda commands because wrappers can
  set `CONDA_PREFIX`, `CONDA_ROOT_PREFIX`, `CONDA_DEFAULT_ENV`,
  `CONDA_SHLVL`, `CONDA_COMPLETION_COMMAND_NAME`, and PATH entries.
- Do not overwrite existing files by default.
- Write self-identifying shim metadata so later removal can distinguish caller
  owned files from user files.
- Keep the user-facing shim name in the caller catalog or policy layer, not in
  fleet.
- Treat `ShimPlan` as a plan. Fleet never writes or removes shim files.

`ShimPlan` includes the shim name, target command, destination path, target
executable, environment variables, PATH entries, and wrapper script contents.

## Downstream Tool Manager Integration Shape

A managed-runtime layout maps directly to fleet when the downstream tool
manager keeps product policy outside this crate:

- `FleetConfig::new(tool_install_root)`
- each tool id maps to `tool_install_root/<id>`
- conda-ship runtime metadata, embedded lock content, or downloaded lock content
  maps to `RuntimeSpec::lock_content`
- global binary exposure remains caller-owned
- `Fleet::status(id)?.lock_sha256` can replace a separate lock-hash sidecar
  when checking whether a tool is current
- `InstalledRuntime::command(binary)` replaces direct `prefix/bin/<binary>`
  process construction
- `InstalledRuntime::shim_plan(binary, options)` returns the target,
  environment, PATH entries, and wrapper contents while leaving filesystem
  writes in the caller

Callers can keep their existing catalog entry for the user-facing shim name and
binary name, build a `RuntimeSpec` from their runtime descriptor or resolved
lock, install through fleet, then write the wrapper using their existing
overwrite and cleanup policy.
