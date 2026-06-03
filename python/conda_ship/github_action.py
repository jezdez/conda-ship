"""Resolve runtime-version for the GitHub Action wrapper."""

from __future__ import annotations

import argparse
import sys
import tempfile
from pathlib import Path
from typing import TYPE_CHECKING

from build import ProjectBuilder
from build.env import DefaultIsolatedEnv
from pyproject_hooks import quiet_subprocess_runner

from .project_metadata import (
    CondaShipProject,
    ProjectMetadataError,
    metadata_version,
)

if TYPE_CHECKING:
    from collections.abc import Sequence


def main(argv: Sequence[str] | None = None) -> None:
    """Print a resolved runtime version when the selected project requests one."""
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    args = parser.parse_args(argv)

    try:
        version = resolve_runtime_version(args.root)
    except ProjectMetadataError as error:
        print(f"conda-ship: {error}", file=sys.stderr)
        raise SystemExit(1) from error

    if version is not None:
        print(version)


def resolve_runtime_version(root: Path) -> str | None:
    """Resolve the project metadata runtime version for the action, if requested."""
    project = CondaShipProject.from_root(root.resolve())
    if project is None or not project.uses_project_metadata_version:
        return None
    return resolve_with_build(project.root)


def resolve_with_build(root: Path) -> str:
    """Resolve project metadata with pypa/build in an isolated environment."""
    pyproject = root / "pyproject.toml"
    if not pyproject.exists():
        raise ProjectMetadataError(
            "runtime-version requested project metadata, but pyproject.toml was not found"
        )

    try:
        with DefaultIsolatedEnv() as env:
            builder = ProjectBuilder.from_isolated_env(env, root, runner=quiet_subprocess_runner)
            env.install(builder.build_system_requires)
            env.install(builder.get_requires_for_build("wheel"))

            with tempfile.TemporaryDirectory() as metadata_dir:
                metadata_root = Path(builder.metadata_path(metadata_dir))
                metadata = (metadata_root / "METADATA").read_text(encoding="utf-8")
    except Exception as error:
        raise ProjectMetadataError(
            f"failed to resolve project metadata version with pypa/build: {error}"
        ) from error

    version = metadata_version(metadata)
    if version is None:
        raise ProjectMetadataError("project metadata does not contain a Version field")
    return version


if __name__ == "__main__":
    main()
