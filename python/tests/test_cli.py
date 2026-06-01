from __future__ import annotations

import argparse
from types import SimpleNamespace
from typing import TYPE_CHECKING

import pytest
from conda_ship import cli
from conda_ship.cli import configure_parser, execute, run_cs

if TYPE_CHECKING:
    from collections.abc import Sequence


def test_configure_parser_collects_ship_args() -> None:
    parser = argparse.ArgumentParser(prog="conda ship")
    configure_parser(parser)

    args = parser.parse_args(["build", "--layout", "online", "--runtime", "demo"])

    assert args.ship_args == ["build", "--layout", "online", "--runtime", "demo"]


@pytest.mark.parametrize(
    ("argv", "expected"),
    [
        pytest.param(
            ["build", "--runtime", "demo"],
            ["build", "--runtime", "demo"],
            id="args",
        ),
        pytest.param(["--"], ["--help"], id="separator-defaults-to-help"),
        pytest.param([], ["--help"], id="empty-defaults-to-help"),
    ],
)
def test_run_cs_delegates_to_executable(
    argv: Sequence[str],
    expected: list[str],
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cs = tmp_path / ("cs.exe" if cli.os.name == "nt" else "cs")
    cs.write_text("")
    cs.chmod(0o755)
    expected_command = [str(cs), *expected]
    calls: list[list[str]] = []

    def fake_run(args: list[str]) -> SimpleNamespace:
        calls.append(args)
        return SimpleNamespace(returncode=17)

    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    status = run_cs(argv, executable=str(cs))

    assert status == 17
    assert calls == [expected_command]


def test_run_cs_reports_missing_executable(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    python = bin_dir / "python"
    python.write_text("")
    monkeypatch.setattr(cli.sys, "executable", str(python))

    status = run_cs([])

    assert status == 127
    assert "could not find" in capsys.readouterr().err


def test_run_cs_prefers_current_environment_binary(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    python = bin_dir / "python"
    cs = bin_dir / ("cs.exe" if cli.os.name == "nt" else "cs")
    python.write_text("")
    cs.write_text("")
    cs.chmod(0o755)
    calls: list[list[str]] = []

    def fake_run(args: list[str]) -> SimpleNamespace:
        calls.append(args)
        return SimpleNamespace(returncode=0)

    monkeypatch.setattr(cli.sys, "executable", str(python))
    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    assert run_cs(["inspect"]) == 0
    assert calls == [[str(cs), "inspect"]]


def test_run_cs_rejects_invalid_env_executable(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    python = bin_dir / "python"
    fallback_cs = bin_dir / ("cs.exe" if cli.os.name == "nt" else "cs")
    python.write_text("")
    fallback_cs.write_text("")
    fallback_cs.chmod(0o755)

    monkeypatch.setattr(cli.sys, "executable", str(python))
    monkeypatch.setenv("CONDA_SHIP_EXECUTABLE", str(tmp_path / "missing-cs"))

    status = run_cs([])

    assert status == 127
    assert "CONDA_SHIP_EXECUTABLE" in capsys.readouterr().err


def test_run_cs_rejects_directory_env_executable(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setenv("CONDA_SHIP_EXECUTABLE", str(tmp_path))

    status = run_cs([])

    assert status == 126
    assert "directory" in capsys.readouterr().err


def test_run_cs_rejects_empty_env_executable(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setenv("CONDA_SHIP_EXECUTABLE", "")

    status = run_cs([])

    assert status == 127
    assert "set but empty" in capsys.readouterr().err


def test_run_cs_normalizes_spawn_file_not_found(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    cs = tmp_path / ("cs.exe" if cli.os.name == "nt" else "cs")
    cs.write_text("")
    cs.chmod(0o755)

    def fake_run(_args: list[str]) -> SimpleNamespace:
        raise FileNotFoundError

    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    assert run_cs([], executable=str(cs)) == 127
    assert "no longer exists" in capsys.readouterr().err


def test_run_cs_normalizes_spawn_permission_error(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    cs = tmp_path / ("cs.exe" if cli.os.name == "nt" else "cs")
    cs.write_text("")
    cs.chmod(0o755)

    def fake_run(_args: list[str]) -> SimpleNamespace:
        raise PermissionError

    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    assert run_cs([], executable=str(cs)) == 126
    assert "not executable" in capsys.readouterr().err


def test_run_cs_normalizes_spawn_os_error(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    cs = tmp_path / ("cs.exe" if cli.os.name == "nt" else "cs")
    cs.write_text("")
    cs.chmod(0o755)

    def fake_run(_args: list[str]) -> SimpleNamespace:
        raise OSError("spawn failed")

    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    assert run_cs([], executable=str(cs)) == 126
    assert "spawn failed" in capsys.readouterr().err


def test_run_cs_normalizes_signal_exit(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cs = tmp_path / ("cs.exe" if cli.os.name == "nt" else "cs")
    cs.write_text("")
    cs.chmod(0o755)

    def fake_run(_args: list[str]) -> SimpleNamespace:
        return SimpleNamespace(returncode=-15)

    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    assert run_cs([], executable=str(cs)) == 143


def test_execute_returns_cs_status(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[list[str]] = []

    def fake_run_cs(args: Sequence[str]) -> int:
        calls.append(list(args))
        return 3

    monkeypatch.setattr(cli, "run_cs", fake_run_cs)
    args = argparse.Namespace(ship_args=["inspect"])

    assert execute(args) == 3
    assert calls == [["inspect"]]
