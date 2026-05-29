# Build Locally

Use local builds while iterating on runtime package sets, channel choices, or
conda-pronto runtime code.

Installed local builds use a prebuilt runtime template:

```bash
pronto build \
  --layout none \
  --name serpe \
  --template ./pronto-runtime-template
```

When developing conda-pronto itself from a source checkout, you can omit
`--template`. In that mode `pronto build` builds the generic
`pronto-runtime` target from the checkout before stamping it.

If you are changing a downstream distribution such as conda-express, keep the
package-set decision in that downstream project, then reproduce the build with
the `pronto` CLI or the GitHub Action.

## Refresh The Artifact Lock

Run this after changing `conda.lock`, `pixi.lock`, or `[tool.pronto]`:

```bash
pronto lock
```

If you changed the `runtime` environment in `conda.toml`, refresh the source
lockfile before deriving conda-pronto's runtime lock:

```bash
conda workspace lock
pronto lock
```

For Pixi-compatible builds, use Pixi to refresh the source lockfile:

```bash
pixi lock
pronto lock
```

CI checks the generated runtime lock with:

```bash
pronto lock --check
```

## Build A Named Distribution Binary

`--name` is required. conda-pronto does not provide a default distribution name.

```bash
pronto build \
  --layout none \
  --name serpe \
  --template ./pronto-runtime-template
```

Use `--out-dir` to stage somewhere other than `dist/`:

```bash
pronto build \
  --layout none \
  --name serpe \
  --template ./pronto-runtime-template \
  --out-dir /tmp/pronto-artifacts
```

Pass `--template` when using an installed `pronto` binary outside a
conda-pronto source checkout.

## Run A Smoke Test

Use `pronto run` to build and immediately execute the staged runtime:

```bash
pronto run \
  --name serpe \
  --template ./pronto-runtime-template \
  -- bootstrap --prefix /tmp/serpe-smoke
```

Everything after `--` is passed to the generated runtime.

## Cross-Compile With A Rust Target

Pass both the Rust target triple and an artifact label:

```bash
pronto build \
  --name serpe \
  --target x86_64-unknown-linux-gnu \
  --target-label x86_64-unknown-linux-gnu \
  --template ./pronto-runtime-template-x86_64-unknown-linux-gnu
```

The target label is appended to staged artifact names and metadata files.

## Keep Names Distribution-Specific

Use the public name of the distribution you are building. For example,
conda-express uses `cx` for its network-bootstrap artifact and `cxz` for its
embedded-bundle artifact. A different distribution uses a different
`--name`.
