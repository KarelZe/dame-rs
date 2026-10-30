"""Pytest configuration for the `dame` test suite."""

from __future__ import annotations

import sys
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import pytest

# Tag benchmarks with the running interpreter version so CodSpeed keeps the
# results from each matrix job (py3.13/3.14/3.15) distinct instead of merging
# identically-named benchmarks.
_PY_TAG = f"py{sys.version_info.major}.{sys.version_info.minor}"


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    """Append the Python version to each benchmark's node id."""
    for item in items:
        if item.get_closest_marker("benchmark") is not None:
            if hasattr(item, "_nodeid"):
                item._nodeid = f"{item.nodeid}[{_PY_TAG}]"
            else:
                # pytest >= 8 compatibility: nodeid is a property backed by _nodeid
                # in some versions, or might need to be set via an internal attribute.
                # If _nodeid isn't present, try setting it anyway.
                object.__setattr__(item, "_nodeid", f"{item.nodeid}[{_PY_TAG}]")
