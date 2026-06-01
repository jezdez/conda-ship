"""CLI adapter used by the ``conda ship`` plugin command."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Sequence


EXIT_NOT_FOUND = 127
EXIT_NOT_EXECUTABLE = 126
EXIT_INTERRUPTED = 130


@dataclass(frozen=True)
class ResolvedExecutable:
    """Resolved ``cs`` executable path and how it was selected."""

    path: Path
    source: str


class AdapterError(Exception):
    """User-facing adapter error with the intended process exit code."""

    def __init__(self, message: str, exit_code: int) -> None:
        super().__init__(message)
        self.exit_code = exit_code


def configure_parser(parser: argparse.ArgumentParser) -> None:
    """Configure the parser for ``conda ship``."""
    parser.add_argument(
        "ship_args",
        nargs=argparse.REMAINDER,
        metavar="ARGS",
        help="Arguments passed through to the cs executable.",
    )


def execute(args: argparse.Namespace) -> int:
    """Run ``cs`` and return its status code."""
    return run_cs(args.ship_args)


def main(argv: Sequence[str] | None = None) -> None:
    """Standalone debugging entry point for the plugin adapter."""
    raise SystemExit(run_cs(sys.argv[1:] if argv is None else argv))


def run_cs(argv: Sequence[str], *, executable: str | None = None) -> int:
    """Delegate to the canonical ``cs`` executable."""
    try:
        cs = resolve_cs(executable=executable)
    except AdapterError as error:
        print(f"conda-ship: {error}", file=sys.stderr)
        return error.exit_code

    ship_args = list(argv)
    if ship_args[:1] == ["--"]:
        ship_args = ship_args[1:]
    if not ship_args:
        ship_args = ["--help"]

    try:
        status = subprocess.run([str(cs.path), *ship_args]).returncode
    except FileNotFoundError:
        print(
            f"conda-ship: {cs.path} was resolved from {cs.source}, but it no longer exists.",
            file=sys.stderr,
        )
        return EXIT_NOT_FOUND
    except PermissionError:
        print(
            f"conda-ship: {cs.path} was resolved from {cs.source}, but it is not executable.",
            file=sys.stderr,
        )
        return EXIT_NOT_EXECUTABLE
    except KeyboardInterrupt:
        return EXIT_INTERRUPTED
    except OSError as error:
        print(f"conda-ship: could not execute {cs.path}: {error}", file=sys.stderr)
        return EXIT_NOT_EXECUTABLE

    if status < 0:
        return 128 + abs(status)
    return status


def resolve_cs(*, executable: str | None = None) -> ResolvedExecutable:
    """Resolve the canonical ``cs`` executable for the adapter."""
    if executable is not None:
        return validate_executable(Path(executable), "explicit executable")

    env_value = os.environ.get("CONDA_SHIP_EXECUTABLE")
    if env_value is not None:
        if not env_value.strip():
            raise AdapterError(
                "CONDA_SHIP_EXECUTABLE is set but empty.",
                EXIT_NOT_FOUND,
            )
        return validate_executable(Path(env_value), "CONDA_SHIP_EXECUTABLE")

    installed = installed_cs()
    if installed is not None:
        return validate_executable(Path(installed), "current Python environment")

    expected = Path(sys.executable).with_name(cs_binary_name())
    raise AdapterError(
        f"could not find `cs` next to the current Python executable at {expected}. "
        "Install conda-ship in this environment or set CONDA_SHIP_EXECUTABLE "
        "for a source checkout or custom package.",
        EXIT_NOT_FOUND,
    )


def validate_executable(path: Path, source: str) -> ResolvedExecutable:
    """Validate an executable path before spawning it."""
    if not path.exists():
        raise AdapterError(
            f"{source} points to missing executable: {path}",
            EXIT_NOT_FOUND,
        )
    if path.is_dir():
        raise AdapterError(
            f"{source} points to a directory, not an executable: {path}",
            EXIT_NOT_EXECUTABLE,
        )
    if not path.is_file():
        raise AdapterError(
            f"{source} does not point to a regular executable file: {path}",
            EXIT_NOT_EXECUTABLE,
        )
    if os.name != "nt" and not os.access(path, os.X_OK):
        raise AdapterError(
            f"{source} points to a file that is not executable: {path}",
            EXIT_NOT_EXECUTABLE,
        )
    return ResolvedExecutable(path=path, source=source)


def cs_binary_name() -> str:
    """Return the platform-specific ``cs`` executable name."""
    return "cs.exe" if os.name == "nt" else "cs"


def installed_cs() -> str | None:
    """Find the ``cs`` executable installed with the current Python env."""
    env_binary = Path(sys.executable).with_name(cs_binary_name())
    if env_binary.is_file():
        return str(env_binary)
    return None


if __name__ == "__main__":
    main()
