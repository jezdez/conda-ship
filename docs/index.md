# conda-ship

Build ready-to-run conda runtimes.

`conda-ship` is a generic builder for single-binary conda runtimes. It
installs the `cs` CLI, but it does not ship a first-party distribution.
Downstream projects choose the runtime name, delegate executable, package set,
channels, documentation URL, and release channel.

`conda-express` is one downstream distribution maintained by Jannis Leidel: it
uses conda-ship to build the `cx` and `cxz` runtimes. conda-ship owns the
reusable builder; conda-express owns the product defaults and release channels
for `cx`.

## Start Here

If you are new to conda-ship, build the tutorial runtime first. It starts from
a small conda workspace and gives you a working mental model for locks,
artifacts, and the generated runtime:

```bash
conda create -n cs-demo -c conda-forge python pip conda-workspaces
conda activate cs-demo
python -m pip install conda-ship
cs --version
```

Then use the documentation by the kind of help you need.

## Common Workflows

::::{grid} 1 1 3 3
:gutter: 3

:::{grid-item-card} First Runtime
:link: tutorials/first-runtime
:link-type: doc

Build a small conda runtime and smoke-test the generated binary.
:::

:::{grid-item-card} GitHub Actions Runtime
:link: tutorials/github-action-runtime
:link-type: doc

Build release artifacts from CI with a platform matrix.
:::

:::{grid-item-card} Custom Delegate Runtime
:link: tutorials/custom-delegate-runtime
:link-type: doc

Expose a command other than `conda` through the generated runtime.
:::

::::

## Documentation By Need

::::{grid} 1 1 2 4
:gutter: 3

:::{grid-item-card} Learn
:link: tutorials/first-runtime
:link-type: doc

Follow a guided first build from lockfile to smoke test.
:::

:::{grid-item-card} Do
:link: how-to/customize-runtime
:link-type: doc

Build a downstream runtime with your own package set.
:::

:::{grid-item-card} Look Up
:link: reference/cli
:link-type: doc

Find exact commands, options, artifact names, and configuration keys.
:::

:::{grid-item-card} Understand
:link: explanation/concepts
:link-type: doc

Read the builder/runtime model and where conda-ship fits in the conda ecosystem.
:::

::::

## Example Distribution

::::{grid} 1 1 2 2
:gutter: 3

:::{grid-item-card} conda-ship
:link: explanation/project-boundaries
:link-type: doc

Read what the builder owns and what downstream distributions own.
:::

:::{grid-item-card} conda-express
:link: https://jezdez.github.io/conda-express/

See a concrete downstream distribution built with conda-ship.
:::

::::

```{toctree}
:hidden:
:caption: Tutorials
:maxdepth: 1

tutorials/first-runtime
tutorials/github-action-runtime
tutorials/custom-delegate-runtime
```

```{toctree}
:hidden:
:caption: How-To Guides
:maxdepth: 1

how-to/build-locally
how-to/choose-artifact-layout
how-to/customize-runtime
how-to/build-in-github-actions
how-to/build-offline-artifacts
how-to/package-a-runtime
how-to/verify-release-artifacts
how-to/troubleshoot-builds
```

```{toctree}
:hidden:
:caption: Reference
:maxdepth: 1

reference/cli
reference/conda-plugin
reference/runtime-cli
reference/github-action
reference/configuration
reference/artifacts
reference/environment-variables
reference/runtime-data-format
reference/release-assets
reference/errors
```

```{toctree}
:hidden:
:caption: Explanation
:maxdepth: 1

explanation/concepts
explanation/source-locks-and-runtime-locks
explanation/runtime-template
explanation/install-locations-and-ownership
explanation/artifact-layout-tradeoffs
explanation/trust-and-provenance
explanation/project-boundaries
explanation/manifests-and-conda-plugin
```

```{toctree}
:hidden:
:caption: Project
:maxdepth: 1

roadmap
changelog
```
