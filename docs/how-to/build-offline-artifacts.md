# Build Offline Artifacts

Offline artifacts let the generated runtime install from package archives that
were downloaded during the build.

Use them when a downstream distribution needs air-gapped installs, native
installer integration, or a single self-contained bootstrap binary.

## Choose A Layout

::::{tab-set}

:::{tab-item} External
Use `external` when you want the runtime and compressed bundle as separate
files. This is useful when an installer or package manager already knows how to
place supporting files next to the binary.

```bash
pronto build --layout external --name serpe
```
:::

:::{tab-item} Embedded
Use `embedded` when you want one larger binary that can bootstrap without a
separate bundle file.

```bash
pronto build --layout embedded --name serpe
```
:::

::::

## Bootstrap From An External Bundle

For an `external` build, distribute these files together:

- `serpe`
- `serpe.bundle.tar.zst`
- `serpe.runtime.lock`
- `serpe.info.json`
- `serpe.packages.txt`
- `serpe.sha256`

Point the runtime at an extracted bundle directory:

```bash
mkdir -p /opt/serpe-bundle
tar -I zstd -xf serpe.bundle.tar.zst -C /opt/serpe-bundle
serpe bootstrap --prefix /opt/serpe --bundle /opt/serpe-bundle --offline
```

Pronto also stamps a distribution-specific bundle environment variable into the
runtime. For a distribution named `serpe`, that variable is
`SERPE_BUNDLE`.

## Bootstrap From An Embedded Bundle

An embedded artifact carries the bundle inside the binary:

```bash
serpez bootstrap --prefix /opt/serpe
```

The runtime extracts the compressed package archives to a temporary directory
during bootstrap and installs from that extracted bundle without network
access.

An explicit `--bundle` still takes priority over the embedded bundle. Use that
override to test a replacement package set without rebuilding the binary.
