# Overview

## Executive Summary

conda-ship turns a solved conda environment into a ready-to-run runtime.
It owns generic build and bootstrap mechanics. It is not a distribution, an
environment manager, or an installer generator.

At a glance:

- A downstream project uses conda-workspaces or Pixi to solve and commit its
  package records.
- The builder selects one solved environment, derives a runtime lock, stamps
  the generic runtime template, and stages online, external, or embedded
  artifacts.
- The generated runtime bootstraps a managed prefix and passes user commands to
  its configured delegate.
- The downstream project owns package sets, runtime names, user-facing policy,
  installers, documentation, and release channels.

## The Runtime Flow

```{mermaid}
flowchart TB
    subgraph downstream["Downstream project"]
        direction LR
        intent["Package intent"] --> solver["Solver (e.g. Pixi, conda)"] --> source_lock["Source lock"]
        choices["Runtime and release choices"]
    end

    subgraph ship["conda-ship"]
        direction LR
        builder["Builder"] --> runtime_lock["Runtime lock"]
        builder --> bundle["Optional package bundle"]
        builder --> artifacts["Staged artifacts"]
        runtime_lock --> artifacts
        bundle --> artifacts
    end

    subgraph machine["User machine"]
        direction LR
        runtime["Generated runtime"] --> prefix["Managed prefix"] --> delegate["Delegate"]
    end

    source_lock --> builder
    choices -. "configuration" .-> builder
    artifacts --> runtime
```

The rest of this section explains those boundaries and the data that moves
through them.

## Builder

The builder turns a project's selected locked environment into release
artifacts. It reads one of these standard manifest and lockfile pairs:

| Project type | Manifest | Lockfile |
| --- | --- | --- |
| Conda Workspaces | `conda.toml` or configured `pyproject.toml` | `conda.lock` |
| Pixi | `pixi.toml` or configured `pyproject.toml` | `pixi.lock` |

It applies the project's [build configuration](../reference/configuration.md),
then derives a runtime lock, bundle files, runtimes, and artifact metadata.

The selected source lockfile is the source of the concrete conda package
records. conda-ship is not a replacement for
{external+conda-workspaces:doc}`conda-workspaces <index>`, Pixi, or another
workspace solver. It consumes a solved environment and turns it into runtime
artifacts.

## Runtime

A runtime is the generated executable that users run after a build.

Its runtime name is its base identity, not a conda environment name. By
default, the build uses the same name for the staged executable and artifact.
Projects can choose a distinct artifact name when a release needs a different
filename. See the [configuration reference](../reference/configuration.md).

Runtime
: The executable conda-ship produces.

Delegate
: The executable inside the managed prefix that receives pass-through commands.

Artifact
: A release file staged by the build.

## Runtime Template

The generic runtime template is an internal binary target. It is not a
first-party distribution. During a build, the builder copies that template
under the runtime name and stamps it with the runtime name, delegate, install
scheme, install name, metadata filename, environment variable names, runtime
lock, and optional bundle. The stamped copy is the runtime.

Released builds and packaged local builds use prebuilt template assets.

## Runtime Lock

The runtime lock comes from the configured source environment after applying
the project's package exclusions. conda-ship stamps the derived lock into every
runtime artifact and stages a copy next to the output binary. It is build
output, not a second checked-in project lockfile.

The inspection command derives the same runtime lock without writing files,
which makes it the local preflight step. Build and run operations derive the
lock as part of their normal work.

The generated runtime can install from:

- the stamped lockfile and network package downloads
- the stamped lockfile and an external package bundle
- the stamped lockfile and an embedded package bundle

## Bundles

Bundles contain downloaded conda package archives. They are not conda channel
mirrors. The runtime lock already records the channels and package records.

An external artifact stages its bundle alongside the runtime. An embedded
artifact carries the compressed bundle inside the runtime. See
[artifact layouts](../how-to/choose-artifact-layout.md) for the exact files and
configuration choices.

An embedded runtime automatically uses its bundled archives during bootstrap.
Its bundle can be overridden when needed.

The bundle format is intentionally narrow. conda-ship writes top-level `.conda`
and `.tar.bz2` files, then verifies them against the lockfile at install time.
Embedded bundles reject nested paths and links before extraction.
