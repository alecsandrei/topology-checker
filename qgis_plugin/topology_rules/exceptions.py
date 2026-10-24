from __future__ import annotations


class ExecutableError(Exception):
    """The topology-checker executable is missing or unusable."""


class MissingError(ExecutableError):
    """No topology-checker executable is set, and none is on the PATH or in
    Cargo's bin folder."""


class PackageError(ExecutableError):
    """The topology-checker Python package is missing or unusable, with why
    in a few words for where the message has no room."""

    def __init__(self, message: str, brief: str = '') -> None:
        super().__init__(message)
        self.brief = brief


class RunError(Exception):
    """The executable failed to run or reported an error."""
