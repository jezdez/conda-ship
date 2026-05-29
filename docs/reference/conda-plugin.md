# Conda Plugin Reference

The `conda-pronto` package provides a conda plugin entry point for conda-pronto.

It registers a `conda pronto` subcommand that delegates to the primary
`pronto` executable:

```bash
conda pronto lock
conda pronto inspect
conda pronto build --layout none --name serpe --template ./pronto-runtime-template
```

The plugin does not reimplement the builder in Python and it does not make
conda-pronto part of conda core.

The plugin has the same build behavior as the standalone CLI. Installed builds
pass `--template` to stamp a prebuilt generic runtime template; source
checkouts can omit that option while developing conda-pronto itself.

## Packaging Contract

`conda-pronto` expects a `pronto` executable on `PATH`.

A conda package must install both pieces into the same environment:

- the Rust-built `pronto` executable
- the Python `conda_pronto` plugin package

For custom packaging or tests, set `CONDA_PRONTO_EXECUTABLE` to an explicit
executable path.

## Argument Forwarding

Arguments after `conda pronto` are passed to `pronto`:

```bash
conda pronto build \
  --layout embedded \
  --name serpe \
  --template ./pronto-runtime-template
```

When you need to pass an argument that conda's own parser would consume, insert
`--` before the conda-pronto arguments:

```bash
conda pronto -- --help
```

Running `conda pronto` without arguments shows `pronto --help`.
