from __future__ import annotations


class ExecutableError(Exception):
    """The topology-checker executable is missing or unusable."""


class RunError(Exception):
    """The executable failed to run or reported an error."""
