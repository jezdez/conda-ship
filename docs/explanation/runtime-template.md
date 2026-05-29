# Runtime Template

conda-pronto keeps the runtime generic so downstream distributions own their public
identity.

The source binary target is `pronto-runtime`. It is gated behind the
non-default `runtime-template` Cargo feature so a normal conda-pronto CLI build
installs only the `pronto` builder.

Release builds publish `pronto-runtime-template-<target>` assets. Installed
`pronto` builds copy one of those template binaries under the requested artifact
name and stamp the copy with distribution-specific runtime data. Source
checkouts can still omit `--template`; that local-development fallback
builds the generic target with Cargo before stamping it.

## What Gets Stamped

During a named build, conda-pronto stamps distribution data onto a copy of the
generic runtime:

- the runtime lock
- optional compressed package bundle
- docs URL
- command and display name
- default prefix directory
- metadata filename
- bundle and offline environment variable names

This lets the same Rust runtime code produce many distribution-specific
binaries without hard-coding a distribution into conda-pronto itself.

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

Those behaviors are part of the conda-pronto runtime template. The package set,
public name, documentation URL, and release channel remain downstream
distribution decisions.
