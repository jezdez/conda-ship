# GitHub Action Reference

The repository root provides a composite GitHub Action for downstream
distribution repositories.

The action downloads the tagged conda-pronto release assets for the current
runner, verifies them against `SHA256SUMS`, and runs the downloaded `pronto`
binary. It does not build conda-pronto from source.

The action builds only from committed project input. The selected root must
contain `conda.toml` plus `conda.lock` or `pixi.toml` plus `pixi.lock`. When the
manifest or matching lockfile is missing, the action fails instead of generating
or solving project configuration in CI.

```yaml
- uses: actions/checkout@v4

- uses: jezdez/conda-pronto@v0.1.0
  id: pronto
  with:
    name: serpe
```

## Inputs

`name`
: Required distribution binary name. For example, conda-express passes `cx`.

`root`
: Project root containing `conda.toml`/`conda.lock` or `pixi.toml`/`pixi.lock`.
  Defaults to the workflow workspace.

`layout`
: Artifact layout to build. Supported values are `none` and `embedded`.
  Defaults to `none`. Embedded artifacts carry package archives inside the
  runtime binary and use the `z` suffix.

`docs-url`
: Documentation URL stamped into the generated runtime help output.

## Outputs

`binary-path`
: Absolute path to the generated runtime binary.

`asset-name`
: Platform-qualified asset filename.

`info-path`
: Absolute path to the artifact info JSON.

`lock-path`
: Absolute path to the staged runtime lock.

`package-list-path`
: Absolute path to the staged package list.

`checksums-path`
: Absolute path to the SHA256 checksum file.
