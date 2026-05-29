# GitHub Action Reference

The repository root provides a composite GitHub Action for downstream
distribution repositories.

The action checks out Pronto, builds the generic runtime from that checkout,
and stamps the staged artifact with its inputs.

The action does not read a downstream repository's own `conda.toml`; pass build
policy through the action inputs.

```yaml
- uses: jezdez/pronto@main
  id: pronto
  with:
    name: serpe
```

## Inputs

`name`
: Required distribution binary name. For example, conda-express passes `cx`.

`packages`
: Optional comma-separated conda package specs. When omitted, Pronto uses the
  package specs in its runtime configuration.

`channels`
: Optional comma-separated conda channels. When omitted, Pronto uses the
  configured channels.

`exclude`
: Optional comma-separated package names to remove from the generated runtime
  lock, including exclusive dependencies.

`ref`
: Git ref of Pronto to build from. Defaults to `main`.

`embed-bundle`
: Set to `"true"` to embed package archives into the runtime binary. The output
  binary uses the `z` suffix.

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
