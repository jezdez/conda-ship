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
: Version shown by the generated runtime and written to prefix ownership
  metadata. This is independent from `runtime-name`; see {doc}`names`.

`embedded_artifact_name`
: Artifact executable name used when the artifact carries an embedded bundle.
  This is explicit build metadata, not a derived suffix.

`delegate_executable`
: Executable inside the managed prefix that receives pass-through arguments.

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
: Documentation URL shown in runtime help.

`installer`
: Optional package manager or installer hint used after `uninstall`.

`runtime_config`
: Resolved runtime channels and package names used for metadata and status
  output.

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
