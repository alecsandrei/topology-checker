"""Closeable streaming validation over immutable input and rule snapshots."""

from __future__ import annotations

import threading
from collections.abc import Iterator
from dataclasses import dataclass
from types import MappingProxyType, TracebackType
from typing import TYPE_CHECKING

from . import _native
from ._enums import ErrorType
from ._models import FeatureRef, Layer, RuleReport, RuleSpec, Violation

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


def counts(rule: _native.NativeRuleSummary) -> MappingProxyType[ErrorType, int]:
    """Map native category names to error types."""
    return MappingProxyType({ErrorType(k): n for k, n in rule.counts.items()})


def rule_report(
    spec: RuleSpec,
    rule: _native.NativeRuleSummary,
    violations: tuple[Violation, ...],
    layers: tuple[Layer, ...],
) -> RuleReport:
    """Build one rule's report from its native summary and decoded violations."""
    return RuleReport(
        spec.rule,
        spec.layer,
        spec.other,
        layers[rule.layer].name,
        None if rule.other is None else layers[rule.other].name,
        counts(rule),
        violations,
        spec,
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


def all_warnings(
    layers: tuple[Layer, ...],
    native: list[_native.NativeWarning],
    crs: tuple[str, ...],
) -> tuple[str, ...]:
    """Order input, native and CRS warnings as eager reports do."""
    return (
        tuple(w for layer in layers for w in layer.warnings)
        + warning_messages(native, layers)
        + crs
    )


def summarize(
    summary: _native.NativeSummary,
    layers: tuple[Layer, ...],
    specs: tuple[RuleSpec, ...],
    crs: tuple[CRS | None, tuple[str, ...]],
) -> ReportSummary:
    """Wrap final native metadata for the captured topology."""
    return ReportSummary(
        summary.tolerance,
        all_warnings(layers, summary.warnings, crs[1]),
        tuple(
            RuleSummary(spec, counts(rule), rule.total)
            for spec, rule in zip(specs, summary.rules, strict=True)
        ),
        crs[0],
    )


class ValidationStream(Iterator[Violation]):
    """Yield aggregated violations in deterministic rule and feature order.

    Use a context manager or close() when stopping early. The worker owns immutable
    snapshots; later registrations cannot change this stream. Final summary is
    available only after successful exhaustion, and is None after early close.
    Threads may share one stream; each violation goes to one of them.
    """

    def __init__(
        self,
        native: _native.NativeValidationStream,
        layers: tuple[Layer, ...],
        specs: tuple[RuleSpec, ...],
        crs: tuple[CRS | None, tuple[str, ...]],
    ) -> None:
        self._native = native
        self._layers = layers
        self._specs = specs
        self._crs = crs
        self._batch: Iterator[_native.NativeViolation] = iter(())
        # Guards the batch. close() takes it only after cancelling, which wakes
        # a holder waiting for the next batch.
        self._lock = threading.RLock()
        self._closed = False
        self.summary: ReportSummary | None = None

    def __iter__(self) -> ValidationStream:
        return self

    def __next__(self) -> Violation:
        with self._lock:
            item = self._take()
        return decode(item, self._layers, self._specs)

    def _take(self) -> _native.NativeViolation:
        if self._closed:
            raise StopIteration
        try:
            item = next(self._batch, None)
            if item is None:
                batch = self._native.next_batch()
                if self._closed:
                    raise StopIteration
                self._batch = iter(batch)
                item = next(self._batch, None)
                if item is None:
                    summary = self._native.summary
                    if summary is not None:
                        self.summary = summarize(
                            summary, self._layers, self._specs, self._crs
                        )
                    self.close()
                    raise StopIteration
            return item
        except StopIteration:
            raise
        except BaseException:
            self.close()
            raise

    def close(self) -> None:
        """Cancel work and release queued violations; repeated close is harmless.

        Returns once the worker has stopped, also when another thread closes too.
        """
        self._closed = True
        self._native.close()
        with self._lock:
            self._batch = iter(())

    def __enter__(self) -> ValidationStream:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()


class ReportStream(Iterator[RuleReport]):
    """Yield each rule's report as soon as that rule finishes, in rule order.

    Reports equal `validate().rules`. Use a context manager or close() when
    stopping early; closing cancels the remaining rules. The worker owns
    immutable snapshots. Final summary is available only after successful
    exhaustion, and is None after early close.
    """

    def __init__(
        self,
        native: _native.NativeReportStream,
        layers: tuple[Layer, ...],
        specs: tuple[RuleSpec, ...],
        crs: tuple[CRS | None, tuple[str, ...]],
    ) -> None:
        self._native = native
        self._layers = layers
        self._specs = specs
        self._crs = crs
        self._closed = False
        self.summary: ReportSummary | None = None

    def __iter__(self) -> ReportStream:
        return self

    def __next__(self) -> RuleReport:
        if self._closed:
            raise StopIteration
        try:
            item = self._native.next_report()
            if item is None:
                summary = self._native.summary
                if summary is not None:
                    self.summary = summarize(
                        summary, self._layers, self._specs, self._crs
                    )
                self.close()
                raise StopIteration
            index, rule, errors = item
            violations = tuple(decode(e, self._layers, self._specs) for e in errors)
            return rule_report(self._specs[index], rule, violations, self._layers)
        except StopIteration:
            raise
        except BaseException:
            self.close()
            raise

    def close(self) -> None:
        """Cancel the remaining rules; repeated close is harmless.

        Returns once the worker has stopped, also when another thread closes too.
        """
        self._closed = True
        self._native.close()

    def __enter__(self) -> ReportStream:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()
