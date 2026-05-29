# Project Boundaries

conda-pronto builds ready-to-run conda bootstrap binaries. It is not itself a conda
distribution.

The split from conda-express puts the generic pieces here and leaves
distribution policy in downstream projects.

## Ownership At A Glance

::::{grid} 1 1 3 3
:gutter: 3

:::{grid-item-card} conda-pronto

Generic builder, runtime behavior, artifact layouts, bundle handling, and
metadata files.
:::

:::{grid-item-card} Downstream Distributions

Package sets, binary names, release channels, installer wrappers, and user
documentation.
:::

:::{grid-item-card} conda-wasm

Browser, WebAssembly, Emscripten, JupyterLite, and browser-specific package
handling.
:::

::::

## What conda-pronto Owns

conda-pronto owns the reusable build and runtime machinery:

- deriving a runtime lock from a conda or Pixi source lockfile
- pruning excluded packages and exclusive dependencies after the solve
- downloading package archives into compressed bundles
- copying or building the generic runtime template and stamping distribution data
- staging `none`, `external`, and `embedded` artifact layouts
- writing artifact metadata: `.runtime.lock`, `.packages.txt`, `.info.json`,
  and `.sha256`
- exposing the composite GitHub Action and local builder CLI

The generated runtime also lives here: `bootstrap`, `status`, `shell`,
`uninstall`, pass-through to conda, offline bundle handling, embedded bundle
handling, and conda-spawn based activation.

## What Downstream Distributions Own

Downstream projects decide what their users get:

- binary names
- package sets
- channels
- package exclusions
- default release channels
- documentation URLs
- Homebrew formulae
- PyPI and crates.io wrapper packages
- Docker images
- GitHub Release policy
- constructor-based installers or enterprise package manager recipes

conda-pronto produces the binaries and metadata those channels can distribute. It
does not decide whether every runtime includes the same conda plugins or uses
the same name.

## conda-express

{external+conda-express:doc}`conda-express <index>` is the downstream
distribution that publishes `cx` and `cxz`.

It owns the opinionated native conda package set, the `cx`/`cxz` names,
Homebrew and shell-script installation, Docker images, PyPI and crates.io
distribution wrappers, and release policy for those artifacts.

When conda-express needs binaries, its workflows call conda-pronto from the
conda-express project root and pass the `cx`/`cxz` artifact names. The package
set remains conda-express project input; conda-pronto does not hard-code those
choices. Its own scope page is
{external+conda-express:doc}`Project scope <scope>`.

## conda-wasm

Browser and WebAssembly work belongs in
{external+conda-wasm:doc}`conda-wasm <index>`, not conda-pronto:

- WebAssembly crates
- Emscripten conda patches
- JupyterLite integration
- browser package extraction and solving behavior
- emscripten-forge packaging

conda-pronto is focused on native bootstrap binaries.

## Relationship To Other Tools

conda-pronto complements other conda ecosystem tools:

| Tool | Role |
| --- | --- |
| conda-workspaces | Defines conda-native workspace manifests and lockfiles that conda-pronto can consume |
| Pixi | Solves and records compatible runtime environments that conda-pronto can consume |
| rattler-build | Builds conda packages |
| constructor | Builds OS installers |
| conda-pronto | Builds bootstrap binaries that can be distributed directly or wrapped by other channels |
| {external+conda-express:doc}`conda-express <index>` | A conda-pronto-based downstream distribution for `cx` and `cxz` |

conda-pronto does not produce installer-generator output such as `.sh`, `.pkg`, or
`.msi`. Those formats can wrap conda-pronto-built binaries when a downstream
distribution needs them.

## What Moved From conda-express

These areas used to be documented or implemented in conda-express and now
belong here:

- custom bootstrap binary builds
- package-set customization
- runtime lock derivation
- offline and embedded bundle layouts
- staged artifact metadata
- generic GitHub Action usage
- generated runtime command behavior

The {external+conda-express:doc}`conda-express docs <index>` describe `cx`
and `cxz` as products. conda-pronto docs describe how to build and reason about
products like them.
