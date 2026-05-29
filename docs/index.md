# pronto

Build ready-to-run conda bootstrap binaries.

`pronto` is a generic builder and runtime for single-binary conda
distributions. It does not ship a first-party distribution runtime. Downstream
projects choose the binary name, package set, channels, documentation URL, and
release channel.

`conda-express` is one downstream distribution: it uses Pronto to build the
official `cx` and `cxz` binaries. Pronto owns the reusable builder/runtime;
conda-express owns the product defaults and release channels for `cx`.

:::{important}
Pronto does not publish a default runtime binary named `pronto`. It builds
named downstream binaries such as `serpe`, `serpez`, `cx`, or `cxz`.
:::

## Choose A Path

::::{grid} 1 1 2 3
:gutter: 3

:::{grid-item-card} First Runtime
:link: tutorials/first-runtime
:link-type: doc

Build a local runtime named `serpe`, then smoke-test it in a temporary prefix.
:::

:::{grid-item-card} Custom Runtime
:link: how-to/customize-runtime
:link-type: doc

Choose a binary name, package set, channels, and documentation URL.
:::

:::{grid-item-card} GitHub Actions
:link: how-to/build-in-github-actions
:link-type: doc

Use the composite action from a downstream distribution repository.
:::

:::{grid-item-card} Artifact Layouts
:link: reference/artifacts
:link-type: doc

Compare `none`, `external`, and `embedded` outputs and their metadata files.
:::

:::{grid-item-card} Builder CLI
:link: reference/cli
:link-type: doc

Look up exact `pronto lock`, `inspect`, `bundle`, `build`, and `run` options.
:::

:::{grid-item-card} Project Boundaries
:link: explanation/project-boundaries
:link-type: doc

See what belongs in Pronto, conda-express, conda-wasm, and installer tooling.
:::

::::

```{toctree}
:caption: Tutorials
:maxdepth: 1

tutorials/first-runtime
```

```{toctree}
:caption: How-To Guides
:maxdepth: 1

how-to/build-locally
how-to/customize-runtime
how-to/build-in-github-actions
how-to/build-offline-artifacts
```

```{toctree}
:caption: Reference
:maxdepth: 1

reference/cli
reference/conda-plugin
reference/runtime-cli
reference/github-action
reference/configuration
reference/artifacts
```

```{toctree}
:caption: Explanation
:maxdepth: 1

explanation/concepts
explanation/runtime-template
explanation/project-boundaries
explanation/manifests-and-conda-plugin
```

```{toctree}
:caption: Project
:maxdepth: 1

roadmap
changelog
```
