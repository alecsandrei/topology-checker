"""Ensure artifact test jobs import the installed wheel throughout the session."""

import os

import pytest


def pytest_sessionstart(session: pytest.Session) -> None:
    """Verify wheel imports when CI requests installed-package testing."""
    if os.environ.get("TOPOLOGY_EXPECT_WHEEL") == "1":
        from scripts.check_install import check_install

        check_install()
