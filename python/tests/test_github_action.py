from __future__ import annotations

import textwrap

from conda_ship import github_action


def test_resolve_runtime_version_ignores_static_version(tmp_path) -> None:
    write_project(tmp_path, runtime_version='"1.2.3"')

    assert github_action.resolve_runtime_version(tmp_path) is None


def test_resolve_runtime_version_reads_project_metadata(tmp_path) -> None:
    write_project(tmp_path)
    write_backend(tmp_path, version="2.3.4")

    assert github_action.resolve_runtime_version(tmp_path) == "2.3.4"


def write_project(tmp_path, *, runtime_version: str = '{ from = "project-metadata" }') -> None:
    (tmp_path / "conda.toml").write_text(
        textwrap.dedent(
            f"""
            [tool.conda-ship]
            runtime-version = {runtime_version}
            """
        ),
        encoding="utf-8",
    )
    (tmp_path / "pyproject.toml").write_text(
        textwrap.dedent(
            """
            [project]
            name = "demo"
            dynamic = ["version"]

            [build-system]
            requires = []
            build-backend = "demo_backend"
            backend-path = ["backend"]
            """
        ),
        encoding="utf-8",
    )


def write_backend(tmp_path, *, version: str) -> None:
    backend_dir = tmp_path / "backend"
    backend_dir.mkdir()
    (backend_dir / "demo_backend.py").write_text(
        textwrap.dedent(
            f"""
            import os


            def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
                dist_info = "demo-{version}.dist-info"
                path = os.path.join(metadata_directory, dist_info)
                os.makedirs(path)
                with open(os.path.join(path, "METADATA"), "w", encoding="utf-8") as metadata:
                    metadata.write("Metadata-Version: 2.4\\nName: demo\\nVersion: {version}\\n")
                return dist_info
            """
        ),
        encoding="utf-8",
    )
