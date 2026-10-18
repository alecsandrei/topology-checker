from __future__ import annotations

import typing as t

# What Processing hands over: the stubs differ in the type of the keys
Parameters: t.TypeAlias = dict[t.Any, t.Any]  # noqa: UP040


class RuleJson(t.TypedDict):
    name: str
    description: str
    arcgis: str | None
    qgis: str | None
    layer: str
    other: str | None


class RuleReportJson(t.TypedDict):
    counts: dict[str, int]
    layer: str
    other: str | None


class ReportJson(t.TypedDict):
    tolerance: float
    warnings: list[str]
    rules: list[RuleReportJson]
