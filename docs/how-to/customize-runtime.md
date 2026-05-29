# Customize A Runtime

Use this guide when you want a conda-pronto-built runtime with your own package set,
binary name, channels, or documentation URL.

conda-pronto is generic. It does not publish a first-party runtime binary, and it
does not reserve a default name. `conda-express` is one downstream distribution
that uses conda-pronto to publish `cx` and `cxz`; use a name owned by your
distribution.

Local builds still run from a conda-pronto source checkout. The manifest examples
below describe the build input conda-pronto consumes. Installed-CLI builds from
downstream repositories require the generic-runtime packaging work tracked in
the roadmap.

## Choose A Binary Name

The runtime name becomes part of the user interface:

- the command users run
- the default prefix, `~/.NAME`
- the metadata file, `.NAME.json`
- the bundle environment variable, `NAME_BUNDLE`
- the offline environment variable, `NAME_OFFLINE`

For environment variables, non-alphanumeric characters are converted to
underscores. A runtime named `serpe` uses `SERPE_BUNDLE` and
`SERPE_OFFLINE`.

Use a product-specific name:

```bash
pronto build --layout none --name serpe
```

Avoid publishing downstream builds as `cx` or `cxz`. In the conda ecosystem,
those names identify the official conda-express artifacts.

## Choose Runtime Packages

A conda bootstrap runtime typically needs at least:

- `python`
- `conda`
- a solver plugin, such as `conda-rattler-solver`

If you want the generated runtime to follow the conda-express activation model,
also include `conda-spawn`.

Additional plugins are a distribution decision. A downstream project records
its own plugin set in its manifest or release workflow; conda-pronto does not choose
one for every runtime.

## Configure Local Build Input

When a conda-pronto source checkout carries `conda.toml` and `conda.lock`, keep
package and channel intent in the workspace sections and put conda-pronto-specific
build policy in `[tool.pronto]`:

```toml
[workspace]
name = "serpe"
channels = ["conda-forge"]
platforms = ["linux-64", "osx-arm64", "win-64"]

[feature.runtime.dependencies]
python = ">=3.12"
conda = ">=25.1"
conda-rattler-solver = "*"
conda-spawn = ">=0.1.0"
numpy = "*"
pandas = "*"

[environments]
runtime = { features = ["runtime"], no-default-feature = true }

[tool.pronto]
environment = "runtime"
exclude = ["conda-libmamba-solver"]
docs-url = "https://example.com/serpe/"
```

Then refresh the source lockfile and derive conda-pronto's runtime lock from the
same checkout:

```bash
conda workspace lock
pronto lock
```

The repository's checked-in build input is still `pixi.toml` and `pixi.lock`.
For that path, `pronto configure` rewrites the runtime package intent in the
selected manifest:

```bash
pronto configure \
  --packages "python >=3.12, conda >=25.1, conda-rattler-solver, conda-spawn, numpy, pandas" \
  --channels "conda-forge" \
  --exclude "conda-libmamba-solver"
```

Then refresh the source lockfile and derive conda-pronto's runtime lock. conda-pronto
consumes the solved `runtime` environment; it does not replace the workspace
solver.

```bash
pixi lock
pronto lock
```

Build the named runtime:

```bash
pronto build --layout none --name serpe
```

The staged binary and metadata files are written to `dist/`.

## Configure In GitHub Actions

For CI builds, pass the same choices to the composite action:

```yaml
- uses: jezdez/conda-pronto@main
  id: pronto
  with:
    name: serpe
    packages: "python >=3.12, conda >=25.1, conda-rattler-solver, conda-spawn, numpy, pandas"
    channels: "conda-forge"
    exclude: "conda-libmamba-solver"
    docs-url: "https://example.com/serpe/"
```

Pin `jezdez/conda-pronto` to a tag or commit SHA for release builds.

## Build An Embedded Variant

Use the `embedded` layout when you want a larger single binary that carries the
package archives inside itself:

```bash
pronto build --layout embedded --name serpe
```

The embedded artifact uses the `z` suffix, so the staged binary is
`dist/serpez` on Unix and `dist/serpez.exe` on Windows.

The embedded runtime detects its built-in bundle automatically during
`bootstrap`; users do not need to pass `--bundle` or `--offline`.
