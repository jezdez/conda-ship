# Artifact Layout Tradeoffs

conda-ship has three artifact layouts because downstream distributions have
different transport constraints.

All layouts start from the same idea:

- the runtime lock is stamped into the runtime
- runtime metadata is stamped into the runtime
- package archives may be downloaded later or bundled during build

## Online

The online layout keeps package archives out of the runtime artifact.

This makes the binary small and release-friendly. Bootstrap needs network
access to the channels recorded in the runtime lock.

Online is a good default when:

- users are expected to have network access during first run
- the runtime is delivered by Homebrew or another package manager
- release artifacts should stay small
- package archives should come from the original channels at bootstrap time

## External

The external layout stages the runtime and package bundle as separate files.

This gives downstream packaging systems more control. An installer can place
the bundle next to the runtime, unpack it into a known directory, or store it in
an internal cache.

External is useful when:

- bootstrap must work offline
- a wrapper installer can carry multiple files
- the runtime binary should stay separate from package archive bytes
- an enterprise deployment system manages bundle files itself

The bundle is a flat archive of `.conda` and `.tar.bz2` files, not a conda
channel mirror.

## Embedded

The embedded layout appends the compressed bundle to the runtime binary.

This produces a larger single file. Bootstrap can work without network access
because the runtime extracts and verifies its built-in package archives.

Embedded is useful when:

- one file is much easier to distribute
- first bootstrap must work offline
- a wrapper installer is not available
- the runtime is intended for constrained or disconnected environments

Embedded runtimes use the configured runtime name by default. A downstream
distribution can set `artifact-name` when a release artifact should use a
different command name.

## Choosing At Release Time

Layout is release metadata, not package intent. A project can solve one source
environment and build multiple layouts from the same source lock:

```bash
cs build --layout online
cs build --layout external
cs build --layout embedded
```

This is why the GitHub Action exposes `layout` as an input. A release matrix can
choose layouts without changing package or channel input.
