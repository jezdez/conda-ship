# Conda Plugin Reference

The `conda-pronto` package provides a conda plugin entry point for Pronto.

It registers a `conda pronto` subcommand that delegates to the primary
`pronto` executable:

```bash
conda pronto lock
conda pronto inspect
conda pronto build --layout none --name serpe
```

The plugin does not reimplement the builder in Python and it does not make
Pronto part of conda core.

The plugin has the same build limitations as the standalone CLI. In particular,
`pronto build` still needs a Pronto source checkout for the generic runtime.

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
conda pronto build --layout embedded --name serpe
```

When you need to pass an argument that conda's own parser would consume, insert
`--` before the Pronto arguments:

```bash
conda pronto -- --help
```

Running `conda pronto` without arguments shows `pronto --help`.
