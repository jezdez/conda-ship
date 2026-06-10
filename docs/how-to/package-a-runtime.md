# Package A Runtime

Use this guide after `cs build` has produced runtime artifacts and you want to
hand those files to another distribution channel.

conda-ship does not generate `.sh`, `.pkg`, `.msi`, Homebrew formulae, Docker
images, or constructor installers. It produces runtimes and metadata that those
systems can wrap.

::::{grid} 1 1 2 3
:gutter: 3

:::{grid-item-card} Direct Assets

Upload the complete `dist/` contents to a release channel.
:::

:::{grid-item-card} Package Managers

Install the runtime binary through Homebrew, a conda package, or another
package manager.
:::

:::{grid-item-card} Installers And Images

Wrap online, external, or embedded runtimes in installers, Docker images, or
internal deployment systems.
:::

::::

## Start From The Output Directory

Every build writes a directory like `dist/`:

```bash
cs build --out-dir dist
```

For release automation, use the GitHub Action `dist-path` output. It contains
all files produced by the build.

```yaml
- uses: jezdez/conda-ship@0.3.0
  id: cs

- uses: actions/upload-artifact@v4
  with:
    name: ${{ steps.cs.outputs.asset-name }}
    path: ${{ steps.cs.outputs.dist-path }}
```

## Publish Direct Release Assets

For direct GitHub Releases, upload the full `dist/` contents:

- runtime binary
- optional `.bundle.tar.zst`
- `.runtime.lock`
- `.packages.txt`
- `.info.json`
- `.sha256`

The metadata files help users and downstream packagers inspect what was built.
Do not publish only the runtime binary unless your release channel has another
place for checksums and package metadata.

```{important}
Treat `dist/` as the release unit. If a channel uploads only the runtime binary,
it needs an equivalent place for checksums, package metadata, and provenance.
```

## Wrap With Homebrew

For an online runtime, a Homebrew formula usually installs the runtime binary
and lets the runtime download packages at first bootstrap.

Set an install method so uninstall guidance is useful:

```bash
cs build --install-method homebrew
```

The formula should install the runtime onto `PATH`. It should not modify the
managed prefix directly; the runtime owns bootstrap, status, shell, pass-through,
and uninstall behavior.

## Wrap With A Conda Package

A conda package can install the runtime binary into the package environment.
This is useful for distributing a downstream runtime in a conda channel.

Keep two boundaries clear:

- the conda package installs the runtime binary
- the generated runtime bootstraps and owns its managed prefix

Use `install-method` to tell users where the runtime binary came from:

```bash
cs build --install-method conda-package
```

## Wrap With constructor Or Another Installer

Installer generators can include either:

- an online runtime and no package bundle
- an external runtime plus extracted or adjacent bundle
- an embedded runtime

For `external`, place the bundle where the installer or first-run script can
pass it to bootstrap:

```bash
RUNTIME bootstrap --bundle /path/to/bundle --offline
```

For `embedded`, no extra bundle path is needed:

```bash
RUNTIMEz bootstrap
```

The installer should not unpack the managed conda prefix by itself. Let the
runtime bootstrap so ownership metadata, `.condarc`, the frozen marker,
constructor-compatible prefix metadata, and package verification are applied
consistently.

```{warning}
Do not preinstall the managed prefix behind the runtime's back. Runtime
bootstrap writes ownership metadata, `conda-meta/history`,
`conda-meta/initial-state.explicit.txt`, and verification state that later
`status`, pass-through, reset, and `uninstall` commands rely on.
```

## Package For Docker Or Internal Images

For images, decide whether bootstrap happens at image build time or container
run time.

Build-time bootstrap gives faster startup:

```dockerfile
COPY demo /usr/local/bin/demo
RUN demo --path /opt/demo bootstrap
```

Run-time bootstrap gives a smaller image layer before first use:

```dockerfile
COPY demo /usr/local/bin/demo
ENTRYPOINT ["demo"]
```

Use an explicit `--path` in images. Avoid relying on a user home directory when
the image will run as different users.

## Verify Before Publishing

Before handing files to another system:

```bash
shasum -a 256 --check dist/*.sha256
```

For GitHub Action builds, also keep the release attestation checks enabled in
the action. They verify the conda-ship tools used to stamp the downstream
runtime.

For release workflows, also attest the full output directory before publishing
it. See {doc}`verify-release-artifacts` for a GitHub Actions example.
