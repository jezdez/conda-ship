# conda-ship

Build ready-to-run conda runtimes.

`conda-ship` is a generic builder for single-binary conda runtimes. It
installs the `cs` CLI, but it does not ship a first-party distribution.
Downstream projects choose the runtime name, delegate executable, package set,
channels, documentation URL, and release channel.

[conda-express](https://jezdez.github.io/conda-express/) is one downstream
distribution maintained by Jannis Leidel: it uses conda-ship to build the `cx`
and `cxz` runtimes. conda-ship owns the reusable builder; conda-express owns
the product defaults and release channels for `cx`.

## Start Here

If you are new to conda-ship, start with the quickstart. It creates a small
conda workspace, locks it, and stages a `demo` runtime:

```bash
conda install --name base -c conda-forge conda-pypi
conda create -n cs-demo -c conda-forge python pip conda-workspaces
conda activate cs-demo
conda pypi install conda-ship
mkdir demo-runtime
cd demo-runtime
```

Then follow the [quickstart](tutorials/quickstart.md).

## Choose A Path

- New to conda-ship: follow the
  [first runtime tutorial](tutorials/first-runtime.md).
- Building a downstream runtime: use
  [customize a runtime](how-to/customize-runtime.md), then check the exact
  fields in the [configuration reference](reference/configuration.md).
- Shipping from CI: start with
  [build in GitHub Actions](how-to/build-in-github-actions.md).
- Choosing names or release files: read
  [runtime and artifact names](reference/names.md) and
  [artifacts](reference/artifacts.md).
- Unsure what belongs here versus downstream: read
  [project boundaries](explanation/project-boundaries.md).
- Building an orchestrator for multiple locked runtimes: read
  [conda-fleet concepts](explanation/conda-fleet.md) and the
  [conda-fleet API reference](reference/conda-fleet.md).

## Scope

conda-ship builds runtimes from solved conda environments. It does not choose
package sets, reserve downstream runtime names, publish a first-party runtime,
or generate operating-system installers.

```{toctree}
:hidden:
:caption: Tutorials
:maxdepth: 1

tutorials/quickstart
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
reference/names
reference/conda-plugin
reference/runtime-cli
reference/conda-fleet
reference/github-action
reference/configuration
reference/artifacts
reference/environment-variables
reference/runtime-data-format
reference/errors
```

```{toctree}
:hidden:
:caption: Explanation
:maxdepth: 1

explanation/concepts
explanation/source-locks-and-runtime-locks
explanation/runtime-template
explanation/conda-fleet
explanation/install-locations-and-ownership
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
