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
            ["/tmp/cs", "build", "--runtime", "demo"],
            id="args",
        ),
        pytest.param(["--"], ["/tmp/cs", "--help"], id="separator-defaults-to-help"),
        pytest.param([], ["/tmp/cs", "--help"], id="empty-defaults-to-help"),
    ],
)
def test_run_cs_delegates_to_executable(
    argv: Sequence[str],
    expected: list[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[list[str]] = []

    def fake_run(args: list[str]) -> SimpleNamespace:
        calls.append(args)
        return SimpleNamespace(returncode=17)

    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    status = run_cs(argv, executable="/tmp/cs")

    assert status == 17
    assert calls == [expected]


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
    monkeypatch.setattr(cli.shutil, "which", lambda _name: None)

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
    calls: list[list[str]] = []

    def fake_run(args: list[str]) -> SimpleNamespace:
        calls.append(args)
        return SimpleNamespace(returncode=0)

    monkeypatch.setattr(cli.sys, "executable", str(python))
    monkeypatch.setattr(cli.shutil, "which", lambda _name: "/tmp/path-cs")
    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    assert run_cs(["inspect"]) == 0
    assert calls == [[str(cs), "inspect"]]


def test_execute_returns_cs_status(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[list[str]] = []

    def fake_run_cs(args: Sequence[str]) -> int:
        calls.append(list(args))
        return 3

    monkeypatch.setattr(cli, "run_cs", fake_run_cs)
    args = argparse.Namespace(ship_args=["inspect"])

    assert execute(args) == 3
    assert calls == [["inspect"]]
