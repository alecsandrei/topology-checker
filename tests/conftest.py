"""Ensure artifact test jobs import the installed wheel throughout the session."""

import os

import pytest

from scripts.check_install import check_install


def pytest_sessionstart(session: pytest.Session) -> None:
    """Verify wheel imports when CI requests installed-package testing."""
    if os.environ.get("TOPOLOGY_EXPECT_WHEEL") == "1":
        check_install(without_shapely=os.environ.get("TOPOLOGY_WITHOUT_SHAPELY") == "1")
