# How Generated Runtimes Work

When you run `cs build`, conda-ship does not invent a new program from scratch.
It starts with a small generic runtime template, copies it to the resolved
runtime name, and writes your build data into that copy. The runtime name can
come from `[tool.conda-ship].runtime-name` or from `--runtime-name`. Builds can use
`[tool.conda-ship].artifact-name` or `--artifact-name` when the staged artifact
needs a distinct command name.

Users rarely need to think about the template. They run the
finished runtime, such as `demo`, `cx`, or a downstream-specific embedded name
like `cxz`.

## What `cs build` Writes

During a runtime build, conda-ship writes these details into the copied
binary:

- runtime name, artifact name, and delegate executable
- install scheme and install name
- installer, when configured
- runtime lock
- optional compressed package bundle
- documentation URL
- metadata filename
- bundle and offline environment variable names

That is what turns the same generic bootstrap code into a specific runtime
with its own runtime name, delegate, package set, help links, and install
location.

## Where The Template Comes From

For packaged builds, the template is downloaded from conda-ship's GitHub
Release assets. The asset name includes the platform it runs on, for example:

```text
cs-template-x86_64-unknown-linux-gnu
cs-template-aarch64-apple-darwin
cs-template-x86_64-pc-windows-msvc.exe
```

You usually only see those names when wiring a packaging job. The GitHub Action
downloads the matching template automatically. A packaged `cs` CLI looks for
an installed `cs-template` next to the `cs` executable; it does not
search arbitrary `PATH` entries for a template. `--template PATH` is an
override for custom packaging or cross-builds.

The template is not a runtime. Running it directly fails with a message that
points back to `cs build`; only the stamped copy has a runtime name,
lockfile, package metadata, and install policy.

When running from a source checkout, `cs build` still expects either an
installed template next to `cs`, a `CONDA_SHIP_TEMPLATE` environment variable,
or an explicit `--template PATH`.

## What Users See

The finished runtime has a small command surface:

- `bootstrap`: install conda into the runtime's install path
- `status`: report runtime and install details
- `shell`: start a conda-spawn subshell
- `uninstall`: remove the install path

All other commands are passed through to the configured delegate executable
after bootstrap.

The base prefix is protected with a CEP 22 frozen marker. Users create named
environments for regular package work.

## What Each Project Chooses

Some runtime behavior is visible to users:

- conda-spawn based activation through `RUNTIME shell`
- disabled `activate`, `deactivate`, and `init` commands with guidance when the delegate is `conda`
- automatic bootstrap before pass-through delegate commands
- uninstall that removes the install path and prints a runtime-removal hint

The package set, runtime name, delegate, documentation URL, and release channel belong to
the project using conda-ship.
