"""Conda plugin adapter for conda-pronto."""

from __future__ import annotations

try:
    from importlib.metadata import version
except ImportError:  # pragma: no cover
    from importlib_metadata import version


try:
    __version__ = version("conda-pronto")
except Exception:  # pragma: no cover
    __version__ = "0+unknown"
