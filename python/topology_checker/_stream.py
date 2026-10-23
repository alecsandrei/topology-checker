"""Closeable streaming validation over immutable input and rule snapshots."""

from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass
from types import MappingProxyType, TracebackType
from typing import TYPE_CHECKING

from . import _native
from ._enums import ErrorType
from ._models import FeatureRef, Layer, RuleSpec, Violation

if TYPE_CHECKING:
    from pyproj import CRS


@dataclass(frozen=True)
class RuleSummary:
    """Rule counts without retaining violation geometry."""

    spec: RuleSpec
    counts: MappingProxyType[ErrorType, int]
    error_count: int

    @property
    def is_valid(self) -> bool:
        """Whether this rule passed."""
        return self.error_count == 0


@dataclass(frozen=True)
class ReportSummary:
    """Final metadata available after successful stream exhaustion."""

    tolerance: float
    warnings: tuple[str, ...]
    rules: tuple[RuleSummary, ...]
    crs: CRS | None

    @property
    def error_count(self) -> int:
        """Return the total number of aggregated violations."""
        return sum(rule.error_count for rule in self.rules)

    @property
    def is_valid(self) -> bool:
        """Whether every rule passed."""
        return self.error_count == 0


def decode(
    error: _native.NativeViolation,
    layers: tuple[Layer, ...],
    specs: tuple[RuleSpec, ...],
) -> Violation:
    """Wrap native WKB with provenance scoped to the captured topology."""

    def reference(value: tuple[int, int, int | None] | None) -> FeatureRef | None:
        return None if value is None else layers[value[0]].reference(value[1], value[2])

    spec = specs[error.rule_index]
    return Violation(
        spec.rule,
        ErrorType(error.error_type),
        error.wkb,
        reference(error.origin),
        reference(error.destination),
        spec.resolved_rule,
    )


def warning_messages(
    warnings: list[_native.NativeWarning], layers: tuple[Layer, ...]
) -> tuple[str, ...]:
    """Map normalized native part positions back to the original Python inputs."""
    result: list[str] = []
    for warning in warnings:
        message = warning.message
        if (
            warning.layer is not None
            and warning.fid is not None
            and warning.part is not None
        ):
            reference = layers[warning.layer].reference(warning.fid, warning.part)
            message = message.replace(
                f", part {warning.part}:", f", part {reference.part}:", 1
            )
        result.append(message)
    return tuple(result)


class ValidationStream(Iterator[Violation]):
    """Yield aggregated violations in deterministic rule and feature order.

    Use a context manager or close() when stopping early. The worker owns immutable
    snapshots; later registrations cannot change this stream. Final summary is
    available only after successful exhaustion, and is None after early close.
    """

    def __init__(
        self,
        native: _native.NativeValidationStream,
        layers: tuple[Layer, ...],
        specs: tuple[RuleSpec, ...],
        warnings: tuple[str, ...],
        crs: CRS | None,
    ) -> None:
        self._native = native
        self._layers = layers
        self._specs = specs
        self._warnings = warnings
        self._crs = crs
        self._batch: Iterator[_native.NativeViolation] = iter(())
        self._closed = False
        self.summary: ReportSummary | None = None

    def __iter__(self) -> ValidationStream:
        return self

    def __next__(self) -> Violation:
        if self._closed:
            raise StopIteration
        try:
            item = next(self._batch, None)
            if item is None:
                self._batch = iter(self._native.next_batch())
                item = next(self._batch, None)
                if item is None:
                    summary = self._native.summary
                    if summary is not None:
                        self.summary = ReportSummary(
                            summary.tolerance,
                            self._warnings
                            + warning_messages(summary.warnings, self._layers),
                            tuple(
                                RuleSummary(
                                    spec,
                                    MappingProxyType(
                                        {
                                            ErrorType(k): n
                                            for k, n in rule.counts.items()
                                        }
                                    ),
                                    rule.total,
                                )
                                for spec, rule in zip(
                                    self._specs, summary.rules, strict=True
                                )
                            ),
                            self._crs,
                        )
                    self.close()
                    raise StopIteration
            return decode(item, self._layers, self._specs)
        except StopIteration:
            raise
        except BaseException:
            self.close()
            raise

    def close(self) -> None:
        """Cancel work and release queued violations; repeated close is harmless."""
        if not self._closed:
            self._closed = True
            self._batch = iter(())
            self._native.close()

    def __enter__(self) -> ValidationStream:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()
