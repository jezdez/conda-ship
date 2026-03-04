# GitHub Action reference

cx provides a composite GitHub Action and a reusable workflow for building
custom cx binaries with your own package set.

## Composite action

The composite action builds a cx binary for the current runner's platform.
Use a matrix strategy for multi-platform builds.

```
uses: jezdez/conda-express@<ref>
```

### Inputs

`packages` {bdg-secondary}`optional`
: Comma-separated conda package specs to include in the bootstrapper.
  When empty, uses the default packages from conda-express.

  Example: `"python >=3.12, conda >=25.1, numpy, pandas"`

`channels` {bdg-secondary}`optional`
: Comma-separated conda channels. When empty, uses the default
  (`conda-forge`).

  Example: `"conda-forge, bioconda"`

`exclude` {bdg-secondary}`optional`
: Comma-separated packages to exclude from the bootstrapper, along with
  their exclusive dependencies. When empty, uses the default exclusions.

  Example: `"conda-libmamba-solver"`

`ref` {bdg-secondary}`optional` {bdg-info}`default: main`
: Git ref of conda-express to build from (tag, branch, or SHA).

### Outputs

`binary-path`
: Absolute path to the built cx binary on the runner.

`asset-name`
: Platform-qualified asset name (e.g. `cx-aarch64-apple-darwin`).

### What it does

1. Checks out conda-express into `.cx-build/`
2. Sets up pixi (Rust toolchain from conda-forge)
3. Sets up Rust build caching
4. Builds the cx binary with `CX_PACKAGES`, `CX_CHANNELS`, and `CX_EXCLUDE`
   environment variable overrides (see {ref}`build-time configuration <env-var-overrides>`)
5. Stages the binary with a platform-qualified name and SHA256 checksum

---

## Reusable workflow

The reusable workflow builds cx for all 5 supported platforms in a single
call, using the composite action internally.

```
uses: jezdez/conda-express/.github/workflows/build.yml@<ref>
```

### Inputs

All inputs from the composite action are supported, plus:

`retention-days` {bdg-secondary}`optional` {bdg-info}`default: 7`
: Number of days to retain build artifacts.

### Artifacts

The workflow uploads one artifact per platform, each containing the binary
and its `.sha256` checksum:

| Artifact | Platform |
|---|---|
| `cx-x86_64-unknown-linux-gnu` | Linux x86_64 |
| `cx-aarch64-unknown-linux-gnu` | Linux ARM64 |
| `cx-x86_64-apple-darwin` | macOS Intel |
| `cx-aarch64-apple-darwin` | macOS Apple Silicon |
| `cx-x86_64-pc-windows-msvc.exe` | Windows x86_64 |

---

## Environment variables

The action passes its inputs to the cx build system via environment
variables. These are the same variables you can set when
{ref}`building from source <env-var-overrides>`:

| Input | Environment variable | Effect |
|---|---|---|
| `packages` | `CX_PACKAGES` | Replaces `[tool.cx].packages` |
| `channels` | `CX_CHANNELS` | Replaces `[tool.cx].channels` |
| `exclude` | `CX_EXCLUDE` | Replaces `[tool.cx].exclude` |
