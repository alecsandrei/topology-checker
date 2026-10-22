from __future__ import annotations

from enum import Enum

PERFORMANCE_TIP = (
    'For maximum performance, install topology-checker into your python'
    ' environment, available on pypi'
)


class EngineMode(str, Enum):
    """Execution backends supported by the plugin."""

    CLI = 'CLI'
    NATIVE = 'NATIVE'

    def __str__(self) -> str:
        return self.value

    @classmethod
    def _missing_(cls, value: object) -> EngineMode | None:
        if isinstance(value, str):
            for member in cls:
                if member.value.lower() == value.lower():
                    return member
        return None


Engine = EngineMode


def native_available() -> bool:
    """Checks if topology_checker can be imported and has Topology."""
    try:
        import topology_checker

        return hasattr(topology_checker, 'Topology')
    except Exception:
        return False


def active_engine() -> str:
    """Returns the current engine type/name."""
    if native_available():
        return EngineMode.NATIVE.value
    return EngineMode.CLI.value
