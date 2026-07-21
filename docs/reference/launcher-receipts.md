# Launcher Receipt Reference

Launcher receipts let an installer prove that it owns replacement of one exact
stamped launcher. They do not grant permission to modify the managed prefix or
the packages inside it.

conda-ship does not write a receipt during `cs build` or automatic bootstrap.
The final installer writes it only after placing a launcher at its permanent
regular-file path. Fleet-managed launchers remain externally managed and do not
receive these receipts.

## Sidecar Location

The receipt is adjacent to the canonical launcher. Its filename is the complete
launcher filename followed by `.conda-ship-receipt.json`:

| Launcher | Receipt |
| --- | --- |
| `/opt/demo/bin/demo` | `/opt/demo/bin/demo.conda-ship-receipt.json` |
| `C:\\Tools\\demo.exe` | `C:\\Tools\\demo.exe.conda-ship-receipt.json` |

The launcher path passed to the receipt API must name the exact regular file.
A symbolic link is refused. This prevents a package-manager entry-point symlink
from creating direct ownership metadata beside its versioned target.

## Version 1 Schema

A direct installation uses this shape:

```json
{
  "schema_version": 1,
  "distribution": {
    "name": "Demo Distribution",
    "version": "1.2.3"
  },
  "launcher": {
    "path": "/opt/demo/bin/demo",
    "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  },
  "ownership": {
    "kind": "direct",
    "installer": "demo-installer",
    "release_source": "https://example.com/demo/releases.json"
  }
}
```

An external package manager uses this ownership object instead:

```json
{
  "kind": "external",
  "installer": "homebrew",
  "update_command": "brew update && brew upgrade demo"
}
```

The fields are:

`schema_version`
: Integer `1`. Unknown versions are refused.

`distribution.name`, `distribution.version`
: Non-empty, single-line downstream distribution identity.

`launcher.path`
: UTF-8 canonical absolute path of the exact regular launcher file.
  Version 1 cannot encode a non-UTF-8 native path. The writer and planner
  refuse such a path instead of weakening the binding. Leading or trailing
  whitespace, line separators, and bidi formatting controls also fail closed.

`launcher.sha256`
: Lowercase hexadecimal SHA-256 of that launcher.

`ownership.kind`
: Either `direct` or `external`.

`ownership.installer`
: Non-empty, single-line installer identity.

`ownership.release_source`
: Required for `direct`. It must be the URL parser's canonical serialization of
  an absolute HTTPS URL without credentials or a fragment. The Rust writer
  normalizes its input. The planner refuses a non-canonical externally authored
  value so downstream implementations do not parse a different origin. This is
  a release discovery source, not proof that a future artifact was verified.

`ownership.update_command`
: Optional for `external`. It is exact printable ASCII that a downstream UI may
  display. conda-ship never passes it to a shell or process API.

Unknown fields, malformed data, non-regular sidecars, path mismatches, and hash
mismatches are refused. A rendered receipt may be at most 64 KiB. The writer
refuses larger input, and the planner refuses a larger sidecar before parsing.

## Rust API

The public `conda_ship::launcher_receipt` module exposes:

```rust
pub fn receipt_path_for_launcher(
    launcher: &Path,
) -> Result<PathBuf, LauncherReceiptError>;

pub fn write_launcher_receipt(
    launcher: &Path,
    distribution: DistributionIdentity,
    ownership: LauncherOwnership,
) -> Result<PathBuf, LauncherReceiptError>;

pub fn plan_launcher_update(
    launcher: &Path,
    expected_installer: &str,
) -> LauncherUpdateDecision;

pub fn revalidate_launcher_update(
    plan: &LauncherUpdatePlan,
) -> LauncherUpdateDecision;
```

`write_launcher_receipt` canonicalizes and hashes the exact regular launcher,
then atomically replaces the adjacent sidecar. It does not change the launcher.
On Windows the sidecar replacement is atomic, while directory durability after
a sudden power loss remains best effort.

`plan_launcher_update` fails closed. The caller passes the exact installer
identity it implements. Only a matching `direct` receipt owned by that installer
returns `LauncherUpdateDecision::Allowed(LauncherUpdatePlan)`. A valid direct
receipt for another installer returns `InstallerMismatch`. The plan contains
the canonical launcher and receipt paths, expected launcher SHA-256,
distribution identity, installer identity, and HTTPS release source.

Every other case returns `LauncherUpdateDecision::Refused` with a structured
reason. A validated `external` receipt also returns its installer identity and
optional display-only command. Missing receipts have no inferred owner, which
is the expected result for Fleet-managed launchers.

`revalidate_launcher_update` reads the receipt and hashes the exact regular
launcher again using the validated plan's installer identity. It returns
`UpdatePlanChanged` when a still-valid direct receipt no longer matches the
earlier plan. A downstream updater calls it immediately before replacement so a
changed plan is not treated like the earlier plan. The updater must require an
allowed plan that equals the complete earlier plan, not only a matching digest.

## Replacement Flow

A downstream updater can use an allowed plan to:

1. Pass the adapter or installer identity it implements to
   `plan_launcher_update` and require an allowed result.
2. Discover a candidate from the recorded HTTPS release source.
3. Download and verify the installer or release artifact using downstream
   checksums, signatures, attestations, or platform signing policy.
4. Revalidate the receipt, exact regular launcher path, and expected digest
   immediately before replacement.
5. Run the installer-owned cross-platform replacement flow.
6. Write a new receipt for the replacement launcher.

The conda-ship primitive performs none of those mutations. It does not download
artifacts, spawn commands, overwrite a running executable, or change the
managed prefix.

The planner is fail-closed installer guidance, not a privilege boundary or a
cross-process lock. The downstream installer owns locking and the final atomic
replacement operation. Filesystem permissions must prevent an untrusted actor
from rewriting the launcher and receipt between final revalidation and that
operation. The replacement operation must compare the expected digest again
while it holds its own lock or otherwise controls the swap.

## Prefix Installer Metadata Is Separate

Constructor-compatible `<prefix>/.installer.info` reports how a managed prefix
was distributed. It does not contain the launcher path or launcher digest and
is never accepted as launcher replacement authority.

For conda-based distributions, a Python conda-self adapter implements the
documented JSON protocol and its downstream installer flow. It does not call
the Rust API directly. Runtimes with another delegate can expose the same flow
through their own interface. The receipt format itself has no conda-specific
fields.
