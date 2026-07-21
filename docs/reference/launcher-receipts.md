# Launcher Receipt Reference

A launcher receipt records which installer is responsible for replacing one
stamped launcher. It does not authorize changes to the managed prefix or the
packages inside it.

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
  Version 1 rejects non-UTF-8 paths, leading or trailing whitespace, line
  separators, and bidirectional formatting controls.

`launcher.sha256`
: Lowercase hexadecimal SHA-256 of that launcher.

`ownership.kind`
: Either `direct` or `external`.

`ownership.installer`
: Non-empty, single-line installer identity.

`ownership.release_source`
: Required for `direct`. It must be a canonical absolute HTTPS URL without
  credentials or a fragment. The writer normalizes input and the planner
  rejects non-canonical values. This URL is used only to discover releases.

`ownership.update_command`
: Optional printable ASCII for `external` that a downstream UI may display.
  conda-ship never passes it to a shell or process API.

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

`plan_launcher_update` returns an allowed plan only when the direct receipt
names the calling installer. A valid direct receipt for another installer
returns `InstallerMismatch`. The plan contains the canonical launcher and
receipt paths, expected launcher SHA-256, distribution identity, installer
identity, and HTTPS release source.

All other cases return `LauncherUpdateDecision::Refused`. For an external
receipt, the result includes the installer name and optional command to display.
A missing receipt returns a refused decision with reason `MissingReceipt`,
including for Fleet-managed launchers.

`revalidate_launcher_update` reads the receipt and hashes the exact regular
launcher again using the validated plan's installer identity. It returns
`UpdatePlanChanged` when a still-valid direct receipt no longer matches the
earlier plan. A downstream updater calls it immediately before replacement so a
changed plan is not treated like the earlier plan. The updater must require an
allowed plan that equals the earlier plan, not only a matching digest.

## Replacement Flow

A downstream updater can use an allowed plan to:

1. Pass the adapter or installer identity it implements to
   `plan_launcher_update` and require an allowed result.
2. Discover a candidate from the recorded HTTPS release source.
3. Download and verify the installer or release artifact using downstream
   checksums, signatures, attestations, or platform signing policy.
4. Revalidate the receipt, exact regular launcher path, and expected digest
   immediately before replacement.
5. Run the downstream installer's replacement code.
6. Write a new receipt for the replacement launcher.

`plan_launcher_update` validates the receipt and returns an allowed plan or a
refusal. It does not download artifacts, spawn commands, overwrite a running
executable, or change the managed prefix.

The planner does not enforce filesystem permissions or hold a cross-process
lock. The downstream installer must prevent changes between revalidation and
replacement, hold its own lock or otherwise control the swap, and compare the
digest again.

## Prefix Installer Metadata Is Separate

Constructor-compatible `<prefix>/.installer.info` reports how a managed prefix
was distributed. It does not contain the launcher path or digest and does not
authorize launcher replacement.

For conda-based distributions, a Python conda-self adapter implements the
documented JSON protocol and its downstream installer flow. It does not call
the Rust API directly. Runtimes with another delegate can expose the same flow
through their own interface. The receipt format itself has no conda-specific
fields.
