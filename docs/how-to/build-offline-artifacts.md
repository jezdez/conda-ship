# Build Offline Artifacts

Offline artifacts let the generated runtime install from package archives that
were downloaded during the build.

Use them when a downstream distribution needs air-gapped installs, native
installer integration, or a single self-contained runtime.

## Choose A Layout

::::{tab-set}

:::{tab-item} External
Use `external` when you want the runtime and compressed bundle as separate
files. This is useful when an installer or package manager already knows how to
place supporting files next to the binary.

```bash
cs build --artifact-layout external
```
:::

:::{tab-item} Embedded
Use `embedded` when you want one larger runtime that can bootstrap without a
separate bundle file.

```bash
cs build --artifact-layout embedded
```
:::

::::

## Bootstrap From An External Bundle

For an `external` build, distribute these files together:

- `demo`
- `demo.bundle.tar.zst`
- `demo.runtime.lock`
- `demo.info.json`
- `demo.packages.txt`
- `demo.sha256`

Point the runtime at an extracted bundle directory:

```bash
mkdir -p /opt/demo-bundle
tar -I zstd -xf demo.bundle.tar.zst -C /opt/demo-bundle
demo --path /opt/demo bootstrap --bundle /opt/demo-bundle --offline
```

Pass the directory that contains the package archive files themselves. A bundle
directory is not a conda channel mirror; conda-ship looks for top-level `.conda`
and `.tar.bz2` files named in the runtime lock.

conda-ship also stamps a runtime-specific bundle environment variable into the
runtime. For a runtime named `demo`, that variable is `DEMO_BUNDLE`.

```{note}
External bundles are transport artifacts, not package indexes. Do not add
`linux-64/`, `noarch/`, or `repodata.json`; pass the directory containing the
package archives directly.
```

## Bootstrap From An Embedded Bundle

An embedded runtime carries the bundle inside the binary:

```bash
demo --path /opt/demo bootstrap
```

The runtime extracts the compressed package archives to a temporary directory
during bootstrap and installs from that extracted bundle without network
access.

Embedded bundle extraction is deliberately narrow. The embedded tar archive may
only contain top-level package archive files. Nested paths, directory entries,
symbolic links, hard links, and non-package files are rejected before install.

```{important}
Keep embedded bundles as package archives only. conda-ship rejects paths and
links before extraction so a bundled runtime cannot write outside the temporary
bundle directory during bootstrap.
```

An explicit `--bundle` still takes priority over the embedded bundle. Use that
override to test a replacement package set without rebuilding the binary.
