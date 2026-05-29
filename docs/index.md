# conda-pronto

Build ready-to-run conda bootstrap binaries.

`conda-pronto` is a generic builder and runtime for single-binary conda
distributions. It installs the `pronto` CLI, but it does not ship a first-party
distribution runtime. Downstream projects choose the binary name, package set,
channels, documentation URL, and release channel.

`conda-express` is one downstream distribution: it uses conda-pronto to build the
official `cx` and `cxz` binaries. conda-pronto owns the reusable builder/runtime;
conda-express owns the product defaults and release channels for `cx`.

## Start Here

If you are new to conda-pronto, build the tutorial runtime first. It gives you a
working mental model for locks, artifacts, and the generated runtime command:

```bash
pronto lock
pronto build --layout none --name serpe
pronto run --name serpe -- bootstrap --prefix /tmp/serpe
```

Then use the documentation by the kind of help you need.

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

Build a named downstream runtime with your own package set.
:::

:::{grid-item-card} Look Up
:link: reference/cli
:link-type: doc

Find exact commands, options, artifact names, and configuration keys.
:::

:::{grid-item-card} Understand
:link: explanation/concepts
:link-type: doc

Read the builder/runtime model and where conda-pronto fits in the conda ecosystem.
:::

::::

## Project Boundaries

::::{grid} 1 1 3 3
:gutter: 3

:::{grid-item-card} conda-pronto
:link: explanation/project-boundaries
:link-type: doc

Generic builder/runtime machinery for native bootstrap binaries.
:::

:::{grid-item-card} conda-express
:link: https://jezdez.github.io/conda-express/

Downstream distribution that publishes the official `cx` and `cxz` binaries.
:::

:::{grid-item-card} conda-wasm
:link: https://jezdez.github.io/conda-wasm/

Browser, WebAssembly, Emscripten, and JupyterLite conda tooling.
:::

::::

```{toctree}
:hidden:
:caption: Tutorials
:maxdepth: 1

tutorials/first-runtime
```

```{toctree}
:hidden:
:caption: How-To Guides
:maxdepth: 1

how-to/build-locally
how-to/customize-runtime
how-to/build-in-github-actions
how-to/build-offline-artifacts
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
```

```{toctree}
:hidden:
:caption: Explanation
:maxdepth: 1

explanation/concepts
explanation/runtime-template
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
