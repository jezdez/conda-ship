# Builder CLI Reference

The `cs` CLI builds and stages conda runtimes.

This page covers the builder CLI. For the command surface exposed by generated
runtimes, see {doc}`runtime-cli`.

The `conda-ship` package can also make `conda ship` available as a
conda-style shortcut for this CLI. See {doc}`conda-plugin`.

Packaged `cs` builds find the installed runtime template next to the `cs`
executable. Pass `--template` only when you need to override that template, use
an explicit release asset, or cross-build for another target.

## `cs inspect`

Inspect the project input and derived runtime package set without writing files.
Use this as a preflight check before a local build or release job.

```bash
cs inspect [--platform PLATFORM] [--json] [--root PATH]
```

Options:

- `--platform PLATFORM`: inspect a conda platform such as `linux-64`.
- `--json`: emit machine-readable JSON with project input, validation,
  exclusions, platform summaries, and packages for the selected platform.
- `--root PATH`: use a build root instead of auto-detecting one.

## `cs build`

Build and stage a runtime artifact.

```bash
cs build [--runtime RUNTIME] [--delegate EXECUTABLE] [--layout LAYOUT] [--target-label LABEL] \
  [--platform PLATFORM] [--target TRIPLE] [--template PATH] \
  [--runtime-version VERSION] [--docs-url URL] [--install-scheme SCHEME] \
  [--install-name NAME] [--install-method METHOD] \
  [--out-dir PATH] [--dry-run] [--root PATH]
```

Identifier-like values such as `RUNTIME`, `EXECUTABLE`, `LABEL`, `TRIPLE`, and
`METHOD` must start with an ASCII letter or digit and may only contain ASCII
letters, digits, `.`, `_`, and `-`. `RUNTIME` names the generated runtime; it
is not a conda environment name. Runtime metadata can come from CLI flags or
`[tool.conda-ship]`.

Options:

- `--runtime RUNTIME`: override `[tool.conda-ship].runtime`.
- `--delegate EXECUTABLE`: override `[tool.conda-ship].delegate`.
- `--runtime-version VERSION`: version shown by the generated runtime. Overrides
  `[tool.conda-ship].runtime-version`, `[project].version`, and project
  metadata resolution.
- `--layout online`: stage a runtime that downloads packages during bootstrap.
- `--layout external`: stage a runtime plus compressed bundle.
- `--layout embedded`: stage a runtime with the compressed bundle embedded.
  When omitted, `cs` uses `[tool.conda-ship].layout` or `online`.
- `--target-label LABEL`: append a platform or target label to artifact names.
- `--platform PLATFORM`: choose the conda platform for metadata and bundles.
- `--target TRIPLE`: target triple used for artifact naming and template
  selection. It also selects the staged `.exe` suffix for Windows artifacts.
  Path-like custom target specifications are not supported here.
- `--template PATH`: prebuilt generic runtime template binary to copy and
  stamp. When omitted, packaged builds use the template installed next to `cs`.
- `--docs-url URL`: documentation URL stamped into runtime help output. Must
  start with `https://` or `http://` and must not contain whitespace or control
  characters.
- `--install-scheme SCHEME`: install scheme stamped into the runtime. Currently
  supported: `conda-home`, which installs below `~/.conda/INSTALL_NAME`, and
  `user-data`, which installs below the platform user data directory.
- `--install-name NAME`: name used inside the install scheme. Defaults to
  `RUNTIME`.
- `--install-method METHOD`: package-manager or installer method stamped into
  the runtime for uninstall guidance. Overrides
  `[tool.conda-ship].install-method`.
- `--out-dir PATH`: write staged artifacts somewhere other than `dist/`.
- `--dry-run`: validate the build input and print the planned artifacts without
  downloading, stamping, or writing files.
- `--root PATH`: use a project root instead of auto-detecting one.

## `cs run`

Build a runtime artifact and execute it immediately.

```bash
cs run [--runtime RUNTIME] [--delegate EXECUTABLE] [--layout LAYOUT] [--platform PLATFORM] \
  [--template PATH] [--runtime-version VERSION] [--docs-url URL] \
  [--install-scheme SCHEME] [--install-name NAME] [--install-method METHOD] \
  [--out-dir PATH] [--root PATH] \
  -- RUNTIME_ARGS...
```

Everything after `--` is passed to the staged runtime.

Options:

- `--runtime RUNTIME`: override `[tool.conda-ship].runtime`.
- `--delegate EXECUTABLE`: override `[tool.conda-ship].delegate`.
- `--runtime-version VERSION`: version shown by the generated runtime. Overrides
  `[tool.conda-ship].runtime-version`, `[project].version`, and project
  metadata resolution.
- `--layout online`: stage a runtime that downloads packages during bootstrap.
- `--layout external`: stage a runtime plus compressed bundle.
- `--layout embedded`: stage a runtime with the compressed bundle embedded.
  When omitted, `cs` uses `[tool.conda-ship].layout` or `online`.
- `--platform PLATFORM`: choose the conda platform for metadata and bundles.
- `--template PATH`: prebuilt generic runtime template binary to copy and
  stamp. When omitted, packaged builds use the template installed next to `cs`.
- `--docs-url URL`: documentation URL stamped into runtime help output. Must
  start with `https://` or `http://` and must not contain whitespace or control
  characters.
- `--install-scheme SCHEME`: install scheme stamped into the runtime. Currently
  supported: `conda-home` and `user-data`.
- `--install-name NAME`: name used inside the install scheme. Defaults to
  `RUNTIME`.
- `--install-method METHOD`: package-manager or installer method stamped into
  the runtime for uninstall guidance.
- `--out-dir PATH`: write staged artifacts somewhere other than `dist/`.
- `--root PATH`: use a project root instead of auto-detecting one.
- `RUNTIME_ARGS`: arguments passed to the staged runtime after it is built.
