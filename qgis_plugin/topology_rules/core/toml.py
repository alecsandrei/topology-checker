from __future__ import annotations

import importlib
import types
import typing as t


class TomlUnavailable(Exception):
    """The Python of QGIS has no TOML reader: it is older than 3.11."""


def parser() -> types.ModuleType:
    """The TOML reader of Python 3.11 or newer, or tomli, which is the same
    reader for older ones if it is installed."""
    for name in ('tomllib', 'tomli'):
        try:
            return importlib.import_module(name)
        except ImportError:
            continue
    raise TomlUnavailable(
        'Reading topology.toml needs Python 3.11 or newer. '
        'The Python of this QGIS is older.'
    )


def loads(text: str) -> dict[str, t.Any]:
    """Parses a TOML text, raising a ValueError for a text that is not TOML
    and TomlUnavailable when this Python cannot read any."""
    return t.cast('dict[str, t.Any]', parser().loads(text))
