# Runtime Template

Pronto keeps the runtime generic so downstream distributions own their public
identity.

The source binary target is `pronto-runtime`. It is gated behind the
non-default `runtime-template` Cargo feature so a normal Pronto build installs
only the `pronto` builder CLI. `pronto build` enables that feature, builds the
generic runtime, copies it under the requested artifact name, and stamps the
copy with distribution-specific runtime data.

## What Gets Stamped

During a named build, Pronto stamps distribution data onto a copy of the
generic runtime:

- the runtime lock
- optional compressed package bundle
- docs URL
- command and display name
- default prefix directory
- metadata filename
- bundle and offline environment variable names

This lets the same Rust runtime code produce many distribution-specific
binaries without hard-coding a distribution into Pronto itself.

## Runtime Behavior

The generated runtime has a small command surface:

- `bootstrap`: install conda into a prefix
- `status`: report runtime and prefix details
- `shell`: start a conda-spawn subshell
- `uninstall`: remove the managed prefix

All other commands are passed through to the installed conda executable after
bootstrap.

The base prefix is protected with a CEP 22 frozen marker. Users create named
environments for regular package work.

## Distribution Behavior

Some runtime-template behavior is visible to users:

- conda-spawn based activation through `NAME shell`
- disabled `activate`, `deactivate`, and `init` commands with guidance
- automatic bootstrap before pass-through conda commands
- uninstall that removes the managed prefix and prints a binary-removal hint

Those behaviors are part of the Pronto runtime template. The package set,
public name, documentation URL, and release channel remain downstream
distribution decisions.
