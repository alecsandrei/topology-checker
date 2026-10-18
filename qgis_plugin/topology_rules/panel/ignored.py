from __future__ import annotations

import collections.abc as c
import dataclasses
import json

from .rows import Row


def fingerprint(row: Row, layers: c.Mapping[str, str]) -> str:
    """What an error is told by: its rule and layers, its kind, the features
    it is about and its geometry. When a feature moves, its errors are new."""
    return json.dumps(
        [
            row.rule_name,
            layers.get(row.layer_alias, row.layer_alias),
            layers.get(row.other_alias, row.other_alias),
            row.error,
            row.feature,
            row.other_feature,
            row.shape,
        ]
    )


def stamp(rows: c.Iterable[Row], layers: c.Mapping[str, str]) -> list[Row]:
    """The errors with their fingerprints, to look up as often as needed."""
    return [
        dataclasses.replace(row, mark=fingerprint(row, layers)) for row in rows
    ]


class Ignored:
    """The errors that the user accepts, which stay out of the results when
    the rules are run again."""

    def __init__(self) -> None:
        self.keys: set[str] = set()

    def has(self, row: Row) -> bool:
        return row.mark in self.keys

    def add(self, rows: c.Iterable[Row]) -> None:
        self.keys.update(row.mark for row in rows)

    def remove(self, rows: c.Iterable[Row]) -> None:
        self.keys.difference_update(row.mark for row in rows)

    def clear(self) -> None:
        self.keys.clear()

    def dump(self) -> str:
        return json.dumps(sorted(self.keys))

    def load(self, text: str) -> None:
        try:
            found = json.loads(text) if text else []
        except ValueError:
            found = []
        self.keys = {key for key in found if isinstance(key, str)}
