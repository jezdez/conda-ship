# Configuration Reference

conda-ship reads project intent from a conda-compatible manifest and concrete
package records from the matching lockfile.

The preferred manifest is `conda.toml` with `conda.lock`. `pyproject.toml` with
`[tool.conda]` also uses `conda.lock`. `pixi.toml` with `pixi.lock` and
`pyproject.toml` with `[tool.pixi]` plus `pixi.lock` remain supported for
Pixi-compatible workflows.

Downstream distributions maintain these values in their own project manifest.
conda-ship treats the values as build input; it does not define a universal
conda distribution.

`cs inspect`, `cs build`, and `cs run` can read either manifest/lockfile pair.
Packaged builds find the installed runtime template automatically, so local
projects do not need a conda-ship source checkout.

## Manifest Discovery

conda-ship looks in the build root for:

1. `conda.toml`
2. `pixi.toml`
3. `pyproject.toml` when it contains `[tool.conda]` or `[tool.pixi]`

The selected manifest determines the lockfile:

| Manifest | Lockfile |
| --- | --- |
| `conda.toml` | `conda.lock` |
| `pixi.toml` | `pixi.lock` |
| `pyproject.toml` with `[tool.conda]` | `conda.lock` |
| `pyproject.toml` with `[tool.pixi]` | `pixi.lock` |

When `pyproject.toml` contains both `[tool.conda]` and `[tool.pixi]`,
conda-ship follows conda-workspaces and treats `[tool.conda]` as the selected
manifest.

`conda.lock` and `pixi.lock` are source lockfiles owned by their respective
workspace tools. conda-ship derives a runtime lock from that source lockfile
while inspecting, building, or smoke-testing a runtime.

## Source Environment

The selected source environment determines the conda packages available to the
generated runtime. In `conda.toml` or `pixi.toml`, use a dedicated `ship`
environment for the packages that should be included in the runtime:

```toml
[feature.ship.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"

[environments]
ship = { features = ["ship"], no-default-feature = true }
```

In `pyproject.toml`, conda-workspaces sections live below `[tool.conda]`, for
example `[tool.conda.feature.ship.dependencies]`. Pixi sections live below
`[tool.pixi]`, for example `[tool.pixi.feature.ship.dependencies]`.

conda-ship does not require specific packages in the selected environment. The
environment must provide the configured delegate executable. Conda-like
distributions include `conda` and the plugins they use. `RUNTIME shell` is
available when the selected conda-spawn version provides the alias from
[conda-spawn PR #59](https://github.com/conda/conda-spawn/pull/59).
Generated runtimes automatically install the selected environment
as the managed base prefix, then pass every argument to the configured delegate
executable inside that prefix.

`conda-self` is optional. Include it in the selected source environment when
the runtime should expose `conda self reset` for the managed base prefix.
Generated runtimes always write the reset snapshot that `conda-self` expects.

## `[tool.conda-ship]`

`[tool.conda-ship]` records conda-ship-specific build policy:

```toml
[tool.conda-ship]
runtime-name = "demo"
artifact-name = "demo-cli"
runtime-version = "1.0.0"
delegate-executable = "conda"
artifact-layout = "online"
source-environment = "ship"
exclude-packages = ["conda-libmamba-solver"]
docs-url = "https://example.com/demo/"
install-scheme = "conda-home"
install-name = "demo"
installer = "homebrew"
condarc-file = "runtime.condarc"
freeze-base = true
```

For the naming model behind `runtime-name`, `artifact-name`, `install-name`, and
`runtime-version`, see {doc}`names`.

`runtime-name`
: Base runtime identity and default artifact name. `cs build` and `cs run`
  require this value, either here or through `--runtime-name`. It is not a
  conda environment name.

`artifact-name`
: Optional staged executable and artifact stem for any layout. When omitted,
  builds use `runtime-name` exactly. Set this when a release artifact should
  have a distinct command name, such as `cxz` while keeping
  `runtime-name = "cx"` for install metadata and environment variable names.

`runtime-version`
: Version stamped into runtime and prefix ownership metadata.
  When omitted from `[tool.conda-ship]`, conda-ship uses static
  `[project].version` from the selected `pyproject.toml` if it exists. Release
  workflows can override this with `cs build --runtime-version VERSION` or the
  GitHub Action `runtime-version` input.

  Projects that declare `dynamic = ["version"]` can opt into standards-based
  metadata resolution:

  ```toml
  [tool.conda-ship]
  runtime-version = { from = "project-metadata" }
  ```

  The Python `conda ship` adapter resolves this source before invoking `cs`: it
  calls the project's PEP 517 `prepare_metadata_for_build_wheel` hook, reads
  `Version` from the generated `.dist-info/METADATA`, and passes the resolved
  value to `cs --runtime-version`. It does not fall back to building a wheel.
  The build backend must already be installed in the Python environment running
  `conda ship`.

`delegate-executable`
: Executable inside the managed prefix that receives every argument after
  automatic bootstrap. Use `conda` for conda-like runtimes such as `cx`. Other
  values, such as `python`, are supported when a runtime should expose a
  different commands.

`artifact-layout`
: Artifact layout to build. Supported values are `online`, `external`, and
  `embedded`. When omitted, `cs build` defaults to `online`.

`source-environment`
: Name of the solved environment to turn into the runtime lock. This value is
  required; conda-ship does not fall back to a default environment because that
  can accidentally ship development or test dependencies.

`exclude-packages`
: Package names removed from the derived runtime lock, including dependencies
  used only by excluded packages.

`docs-url`
: Documentation URL stamped into generated runtime metadata. Must start
  with `https://` or `http://` and must not contain whitespace or control
  characters.

`install-scheme`
: Install scheme stamped into the generated runtime. Supported values are
  `conda-home`, which installs below `~/.conda/INSTALL_NAME`, and `user-data`,
  which installs below the platform user data directory. `conda-home` is the
  default when `install-scheme` is not configured.

`install-name`
: Directory name for this runtime's managed base prefix under the install
  scheme. When omitted, conda-ship uses the runtime name. For example,
  `runtime-name = "cx"` can use `install-name = "express"` so the `conda-home`
  install scheme resolves to `~/.conda/express`.
  Choose a product-specific install name. conda-ship does not reserve names
  under `~/.conda`; it relies on runtime metadata to avoid overwriting prefixes
  owned by other tools.

`installer`
: Optional package manager or installer hint stamped into the generated runtime.
  Release workflows can override this with `cs build --installer INSTALLER` or
  the GitHub Action `installer` input.

  When configured, automatic bootstrap writes Constructor-compatible
  `<prefix>/.installer.info` JSON with the exact fields `name`, `version`,
  `platform`, and `type`. The configured `installer` value becomes `type`.
  This metadata reports how the prefix was distributed. It is not proof that
  the runtime binary may update or uninstall itself.

`condarc-file`
: Optional path to a YAML condarc file. Relative paths are resolved from
  the selected project manifest. The builder requires a YAML mapping and stamps
  the file's exact text content into the runtime. During bootstrap, the runtime
  writes that content to `<prefix>/.condarc`.

  When omitted, conda-ship does not create, replace, or remove `.condarc`.
  The runtime lock still records the channels used to build the prefix.
  conda-ship does not merge them into persistent conda configuration.

`freeze-base`
: Whether bootstrap writes the existing CEP 22 marker to
  `<prefix>/conda-meta/frozen`. Defaults to `false`. When false, conda-ship
  leaves any marker created by an installed package untouched.

Generated runtimes write ownership metadata into every bootstrapped prefix.
That metadata records the schema version, display name derived from
`runtime-name`, install name, and metadata filename expected by the runtime.
Automatic bootstrap refuses to use an existing non-empty conda prefix when
that ownership metadata is missing, invalid, or belongs to another stamped
runtime.

Generated runtimes also write constructor-compatible prefix metadata into
`conda-meta/history` and `conda-meta/initial-state.explicit.txt`. Conda uses
the history file to recognize the prefix as an environment and to preserve the
runtime's requested package specs for future conda operations. The explicit
initial-state file records the exact package URLs and checksums from the
stamped runtime lock. When `conda-self` is installed in the runtime, it uses
that file as the installer snapshot for the `installer-updated` and
`installer-exact` reset modes.

Keep package selection and lockfile channels in the selected source environment.
conda-ship records the resolved package names and channel URLs in runtime
metadata. It writes persistent conda configuration only when `condarc-file` is
set.

## Stamped Runtime Metadata

`cs build` stamps these values onto the runtime after resolving `runtime-name`,
`artifact-name`, and `artifact-layout` from CLI flags or `[tool.conda-ship]`:

- artifact name: `ARTIFACT_NAME`, or `RUNTIME_NAME` when
  `artifact-name` is not configured
- runtime version: the configured `runtime-version`, static
  `[project].version` from the selected `pyproject.toml`, or the concrete
  value resolved by `conda ship` from `{ from = "project-metadata" }`. Builds
  fail when no downstream version can be resolved
- runtime name: `RUNTIME_NAME`
- delegate executable: the configured `delegate-executable`
- install scheme: `conda-home`, or the configured `install-scheme`
- install name: `RUNTIME_NAME`, or the configured `install-name`
- installer: the configured `installer`, when present
- condarc contents: the exact text from `condarc-file`, when configured
- frozen base policy: the configured `freeze-base` value, defaulting to `false`
- metadata file: `.RUNTIME_NAME.json`
- bundle environment variable: uppercased `RUNTIME_NAME` plus `_BUNDLE`
- offline environment variable: uppercased `RUNTIME_NAME` plus `_OFFLINE`

The runtime also derives its prefix environment variable from the stamped
runtime name as uppercased `RUNTIME_NAME` plus `_PREFIX`.

At bootstrap time, the generated runtime writes a separate prefix metadata file
inside the managed prefix. That file is used for ownership checks before later
operations touch the prefix. It is written last to mark bootstrap complete.
The internal installing marker is then removed.

The bootstrap also writes standard conda prefix metadata:

- `conda-meta/history`
- `conda-meta/initial-state.explicit.txt`

These files are not stamped into the runtime binary. They are rendered from the
runtime lock when the prefix is bootstrapped.

The runtime writes `.condarc` and the CEP 22 frozen marker only when their
corresponding options are set.

Non-alphanumeric characters in environment variable names become underscores.

## Downstream Defaults

conda-ship's repository default package set exists so the builder and
runtime behavior can be tested. A downstream distribution makes its own
package choices in its project manifest before committing the matching lockfile.

For example, conda-express owns the package set and runtime names used when
building `cx` and `cxz`. Those choices are conda-express policy, not
conda-ship policy.
