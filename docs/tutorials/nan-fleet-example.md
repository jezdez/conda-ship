# Use The `nan` Fleet Harness

`nan` is a feature-gated, low-level harness for the experimental
`conda-fleet` API. It is not a product CLI and it is not the user workflow
fleet is designed around.

Fleet is meant to be called by an orchestrator that already knows which
conda-ship runtime or locked tool runtime it wants to install. That
orchestrator should derive `RuntimeSpec` from its own catalog, downloaded
descriptor, or conda-ship-generated runtime metadata. `nan` reads a JSON
fixture only because it has no catalog or artifact-discovery layer.

Every command requires `--install-root PATH` so the example never writes to a
user-global location by accident.

## Build The Example

From the conda-ship repository:

```bash
cargo run --features fleet --bin nan -- --help
```

## Create A RuntimeSpec Fixture

`nan install` reads a JSON fixture that maps directly to
`conda_ship::fleet::RuntimeSpec`. This is useful for tests and API debugging,
but it should not be treated as a proposed end-user spec format.

```json
{
  "id": "demo",
  "version": "0.1.0",
  "delegate_executable": "conda",
  "lock_content": "---\nversion: 6\n...",
  "channels": ["conda-forge"],
  "requested_specs": ["conda"]
}
```

In a real test, `lock_content` should contain the full resolved rattler-lock
document for the runtime. Fleet does not solve environments. Production callers
should normally populate this field from the conda-ship runtime or tool
descriptor they already selected.

## Install

```bash
cargo run --features fleet --bin nan -- \
  --install-root /tmp/conda-fleet-demo \
  install --spec SPEC.fixture.json
```

For an offline or bundled install:

```bash
cargo run --features fleet --bin nan -- \
  --install-root /tmp/conda-fleet-demo \
  install --spec SPEC.fixture.json --bundle /path/to/bundle --offline
```

Use `--force` only to replace an existing managed runtime with the same id.
Fleet refuses unmanaged non-empty prefixes.

## List And Status

```bash
cargo run --features fleet --bin nan -- \
  --install-root /tmp/conda-fleet-demo \
  list
```

```bash
cargo run --features fleet --bin nan -- \
  --install-root /tmp/conda-fleet-demo \
  status demo
```

Both commands print JSON. `status` prints `null` when a runtime id is not
installed.

## Run A Command

`nan run` uses `InstalledRuntime::command()` and applies the returned
environment plus prefix PATH entries to a child process:

```bash
cargo run --features fleet --bin nan -- \
  --install-root /tmp/conda-fleet-demo \
  run demo conda -- --version
```

The `--` separates `nan` arguments from arguments passed to the runtime command.

## Inspect A Shim Plan

`nan shim-plan` prints a JSON `ShimPlan`. It does not write files:

```bash
cargo run --features fleet --bin nan -- \
  --install-root /tmp/conda-fleet-demo \
  shim-plan demo conda --shim-name conda
```

Callers should use the plan as input to their own shim writer. Recommended
behavior is to refuse overwrites by default, write caller-owned metadata into
shim files, and remove only files known to be caller-owned.

## Remove

```bash
cargo run --features fleet --bin nan -- \
  --install-root /tmp/conda-fleet-demo \
  remove demo --force
```

`remove` deletes managed fleet prefixes and empty directories only. It refuses
unmanaged non-empty prefixes, even with `--force`.
