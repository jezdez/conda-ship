# Runtime And Artifact Names

conda-ship uses several names because one build has several audiences: users
run a command, release jobs upload files, and the generated runtime manages an
install location. Keep these names the same unless a downstream distribution has
a reason to separate them.

## Quick Choice

For most projects, configure the normal build policy and let `runtime-name`
drive the other name defaults:

```toml
[tool.conda-ship]
runtime-name = "demo"
runtime-version = "1.0.0"
delegate-executable = "conda"
artifact-layout = "online"
source-environment = "ship"
```

This stages `dist/demo`, stamps `demo` into runtime metadata, writes install
metadata such as `.demo.json`, and uses runtime-specific environment variables
such as `DEMO_BUNDLE`.

Add `artifact-name` only when the release command or file stem should differ
from the base runtime identity:

```toml
[tool.conda-ship]
runtime-name = "cx"
artifact-name = "cxz"
```

This stages an artifact such as `dist/cxz`, while display, install metadata,
and environment variable names continue to use `cx`.

Add `install-name` only when the managed install location should use a different
name:

```toml
[tool.conda-ship]
runtime-name = "cx"
install-name = "express"
```

This keeps the user-facing runtime identity `cx`, but installs below the
install scheme path for `express`, such as `~/.conda/express` with the default
`conda-home` scheme.

## Name Fields

`runtime-name`
: Base runtime identity and default artifact name. This is the name users see
  in runtime metadata, install ownership metadata, and runtime-specific
  environment variables. It is required unless passed as `--runtime-name` or the
  GitHub Action `runtime-name` input.

`artifact-name`
: Optional staged executable and artifact stem. This is the name used for files
  written to `dist/`, including the staged runtime, `.info.json`,
  `.runtime.lock`, `.packages.txt`, `.sha256`, and the external bundle stem.
  When omitted, it defaults to `runtime-name`.

`install-name`
: Optional directory name for this runtime's managed base prefix under the
  selected install scheme. When omitted, it defaults to `runtime-name`.

## Related Fields

`delegate-executable`
: Executable inside the managed prefix that receives pass-through arguments.
  It is an executable name, not a path.

`artifact-layout`
: Artifact transport shape: `online`, `external`, or `embedded`.

`exclude-packages`
: Package names removed from the derived runtime lock.

`installer`
: Package manager or installer hint for uninstall guidance. It is not part of
  the install-location controls.

## Runtime Version

`runtime-version` is related to `runtime-name` only because both are stamped
runtime metadata. It is not derived from `runtime-name`, and changing one does
not change the other.

Use `runtime-name` to choose the runtime identity and default file stem. Use
`runtime-version` to choose what the generated runtime reports from
`RUNTIME --version` and records in prefix ownership metadata.

`runtime-version` can come from:

- `[tool.conda-ship].runtime-version`
- `cs build --runtime-version`
- the GitHub Action `runtime-version` input
- static `[project].version`
- `runtime-version = { from = "project-metadata" }` when the Python
  `conda ship` adapter or GitHub Action resolves project metadata

## Examples

Default naming:

```toml
[tool.conda-ship]
runtime-name = "demo"
runtime-version = "1.0.0"
```

Stages:

```text
dist/demo
dist/demo.info.json
dist/demo.runtime.lock
dist/demo.packages.txt
dist/demo.sha256
```

Separate artifact name:

```toml
[tool.conda-ship]
runtime-name = "cx"
artifact-name = "cxz"
runtime-version = "1.0.0"
```

Stages:

```text
dist/cxz
dist/cxz.info.json
dist/cxz.runtime.lock
dist/cxz.packages.txt
dist/cxz.sha256
```

The generated runtime still uses `cx` for display, install metadata, and
runtime-specific environment variables unless those are configured separately.
