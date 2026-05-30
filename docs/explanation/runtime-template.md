# How Generated Runtimes Work

When you run `pronto build --name demo`, conda-pronto does not invent a new
program from scratch. It starts with a small generic bootstrap binary, copies it
to the requested name, and writes your build data into that copy.

That generic bootstrap binary is called the runtime template. The name is mostly
build-time terminology: users run the finished binary, such as `demo`, `demoz`,
`cx`, or `cxz`.

## What `pronto build` Writes

During a named build, conda-pronto writes these details into the copied binary:

- command and display name
- default prefix directory
- runtime lock
- optional compressed package bundle
- documentation URL
- metadata filename
- bundle and offline environment variable names

That is what turns the same generic bootstrap code into a specific runtime
binary with its own name, package set, help links, and install location.

## Where The Template Comes From

For packaged builds, the template is downloaded from conda-pronto's GitHub
Release assets. The asset name includes the platform it runs on, for example:

```text
pronto-runtime-template-x86_64-unknown-linux-gnu
pronto-runtime-template-aarch64-apple-darwin
pronto-runtime-template-x86_64-pc-windows-msvc.exe
```

You usually only see those names when wiring a local build or packaging job. The
GitHub Action downloads the matching template automatically. An installed
`pronto` CLI can use one explicitly with `--template PATH`.

When developing conda-pronto itself from a source checkout, `--template` is
optional. In that mode, `pronto build` compiles the local generic runtime before
writing the named artifact.

## What Users See

The finished runtime has a small command surface:

- `bootstrap`: install conda into a prefix
- `status`: report runtime and prefix details
- `shell`: start a conda-spawn subshell
- `uninstall`: remove the managed prefix

All other commands are passed through to the installed conda executable after
bootstrap.

The base prefix is protected with a CEP 22 frozen marker. Users create named
environments for regular package work.

## What Each Project Chooses

Some generated-runtime behavior is visible to users:

- conda-spawn based activation through `NAME shell`
- disabled `activate`, `deactivate`, and `init` commands with guidance
- automatic bootstrap before pass-through conda commands
- uninstall that removes the managed prefix and prints a binary-removal hint

The package set, public name, documentation URL, and release channel belong to
the project using conda-pronto.
