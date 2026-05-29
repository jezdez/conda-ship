# Builder CLI Reference

The `pronto` CLI builds and stages named conda bootstrap runtimes.

This page covers the builder CLI. For the command surface exposed by generated
runtime binaries, see {doc}`runtime-cli`.

The `conda-pronto` package also registers `conda pronto` as an adapter that
delegates to this CLI. See {doc}`conda-plugin`.

In this release, `pronto build` and `pronto run` require a Pronto source
checkout because they build the generic `pronto-runtime` target from the
selected root before stamping the staged artifact.

## `pronto lock`

Derive the runtime lock from the selected project lockfile environment and
write it to `target/pronto/runtime.lock`.

```bash
pronto lock [--check] [--root PATH]
```

Options:

- `--check`: verify that the runtime lock can be derived; do not write it.
- `--root PATH`: use a build root instead of auto-detecting one.

## `pronto inspect`

Summarize the derived runtime lock.

```bash
pronto inspect [--platform PLATFORM] [--json] [--root PATH]
```

Options:

- `--platform PLATFORM`: inspect a conda platform such as `linux-64`.
- `--json`: emit machine-readable JSON.
- `--root PATH`: use a build root instead of auto-detecting one.

## `pronto bundle`

Download package archives from the derived runtime lock into
`target/pronto/bundle/` and compress them as `target/pronto/bundle.tar.zst`.

```bash
pronto bundle [--platform PLATFORM] [--root PATH]
```

Options:

- `--platform PLATFORM`: choose the conda platform to download.
- `--root PATH`: use a build root instead of auto-detecting one.

## `pronto build`

Build and stage a named runtime artifact.

```bash
pronto build --name NAME [--layout LAYOUT] [--target-label LABEL] \
  [--platform PLATFORM] [--target TRIPLE] [--out-dir PATH] [--root PATH]
```

Options:

- `--name NAME`: required distribution binary name.
- `--layout none`: stage a network bootstrap binary.
- `--layout external`: stage a runtime plus compressed bundle.
- `--layout embedded`: stage a runtime with the compressed bundle embedded.
- `--target-label LABEL`: append a platform or target label to artifact names.
- `--platform PLATFORM`: choose the conda platform for metadata and bundles.
- `--target TRIPLE`: pass a Rust target triple to `cargo build`.
- `--out-dir PATH`: write staged artifacts somewhere other than `dist/`.
- `--root PATH`: use a build root instead of auto-detecting one. For
  `pronto build`, this root must be a Pronto source checkout.

## `pronto run`

Build a named runtime and execute it immediately.

```bash
pronto run --name NAME [--layout LAYOUT] [--platform PLATFORM] \
  [--out-dir PATH] [--root PATH] -- RUNTIME_ARGS...
```

Everything after `--` is passed to the staged runtime.

Options:

- `--name NAME`: required distribution binary name.
- `--layout none`: stage a network bootstrap binary.
- `--layout external`: stage a runtime plus compressed bundle.
- `--layout embedded`: stage a runtime with the compressed bundle embedded.
- `--platform PLATFORM`: choose the conda platform for metadata and bundles.
- `--out-dir PATH`: write staged artifacts somewhere other than `dist/`.
- `--root PATH`: use a Pronto source checkout instead of auto-detecting one.
- `RUNTIME_ARGS`: arguments passed to the staged runtime after it is built.

## `pronto configure`

Patch runtime packages, channels, or excludes in the selected project manifest.

```bash
pronto configure [--packages SPECS] [--channels CHANNELS] [--exclude NAMES] \
  [--root PATH]
```

Values are comma-separated. After configuration changes, refresh the source
lockfile with the tool that owns the manifest, then run `pronto lock`.

Options:

- `--packages SPECS`: comma-separated conda package specs.
- `--channels CHANNELS`: comma-separated conda channel names or URLs.
- `--exclude NAMES`: comma-separated package names to prune after solving.
- `--root PATH`: use a build root instead of auto-detecting one.
