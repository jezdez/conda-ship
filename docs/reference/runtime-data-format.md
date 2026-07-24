# Runtime Data Format

conda-ship stamps runtime data onto a copy of the generic runtime template.
This page documents the compatibility surface at a high level. It is not a
general-purpose file format for other tools to write.

```{important}
Treat the stamped runtime data as private executable metadata. Release tooling
should read `.info.json`, `.runtime.lock`, `.packages.txt`, and `.sha256`
instead of parsing or writing bytes inside the runtime binary.
```

## Location

Runtime data is appended to the staged runtime binary.

For embedded builds, the compressed bundle bytes are also appended before the
footer. The runtime reads the footer, validates checksums, and then reads the
stamped header and optional bundle.

The reader accepts up to 16 MiB of trailing bytes after the footer so common
platform signing steps can append data without hiding the stamp. A finalized
file with more trailing data is not accepted by `cs package-update` or the
runtime update verifier.

## Header Fields

The stamped header records:

`schema_version`
: Runtime data schema version.

`artifact_name`
: Staged executable and artifact name.

`runtime_name`
: Base runtime identity. This is the value from `runtime-name`, independent of
  an optional `artifact-name`.

`runtime_version`
: Version written to runtime and prefix ownership metadata. This is independent
  from `runtime-name`. See {doc}`names`.

`artifact_layout`
: Staged artifact layout. Executable updates support `online` and `embedded`.

`platform`
: Native conda platform for the staged executable and any runtime update
  package.

`embedded_artifact_name`
: Artifact executable name used when the artifact carries an embedded bundle.
  This is explicit build metadata, not a derived suffix.

`delegate_executable`
: Executable inside the managed prefix that receives every runtime argument.

`install_scheme`
: Stamped install scheme, such as `conda-home` or `user-data`.

`install_name`
: Name used inside the install scheme.

`metadata_file`
: Ownership metadata filename written inside the managed prefix.

`bundle_env_var`
: Runtime-specific environment variable for an external bundle path.

`offline_env_var`
: Runtime-specific environment variable for offline bootstrap mode.

`docs_url`
: Documentation URL retained in stamped runtime metadata.

`installer`
: Optional package manager or installer metadata.

`update`
: Optional executable update policy. It contains:

  - `channel`: absolute `https://` or `file://` conda channel URL
  - `package`: conda package used for update records
  - `build-number`: current executable build number
  - `ownership`: initial direct capability or compatibility external default
  - `instruction`: optional instruction paired with the stamped external default

  Installed ownership is not fixed by this stamped object. It is recorded in
  `.RUNTIME_NAME.json`, together with the stable executable path and optional
  installation kind. This lets package managers distribute the canonical
  direct-capable executable without changing its bytes.

`runtime_config`
: Resolved runtime channels and package names used for bootstrap metadata, plus
  optional stamped condarc text and the frozen-base policy.

`runtime_lock`
: Runtime lock used for bootstrap.

## Footer

The footer contains enough information for the runtime to find and verify the
appended data:

- header offset information
- bundle length
- header SHA256
- bundle SHA256
- format version
- conda-ship magic bytes

If the footer or checksums are invalid, the runtime refuses to start.

## Compatibility Notes

Generated runtimes are expected to read the format written by the same
conda-ship release family. Downstream tools should treat the staged runtime as
an opaque executable plus documented artifact metadata files.

Use `.info.json`, `.runtime.lock`, `.packages.txt`, and `.sha256` for release
automation instead of parsing the appended runtime data directly.

The version-one update coordinator contract does not make the appended runtime
format public. A coordinator invokes the stamped executable as a child process
and exchanges JSON through the environment-driven helper documented in
{doc}`runtime-cli`.

After bootstrap, executable update and recovery state is stored in the existing
`.RUNTIME_NAME.json` prefix metadata file. The update engine does not add a
second persistent receipt or state record.
