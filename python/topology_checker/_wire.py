"""Private types for native report serialization."""

from typing import TypedDict


class NativeReference(TypedDict):
    layer: int
    fid: int
    part: int | None


class NativeViolation(TypedDict):
    rule: str
    error_type: str
    wkb: list[int]
    origin: NativeReference | None
    destination: NativeReference | None


class NativeRuleReport(TypedDict):
    rule: str
    layer: int
    other: int | None
    counts: dict[str, int]
    errors: list[NativeViolation]


class NativeReport(TypedDict):
    tolerance: float
    warnings: list[str]
    rules: list[NativeRuleReport]
