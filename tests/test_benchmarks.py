"""Benchmarks for CodSpeed; run with `pytest tests/test_benchmarks.py --codspeed`."""

from __future__ import annotations

import sys

import pytest

from dame import Board, Engine, perft

# Tag benchmarks with the running interpreter version so CodSpeed keeps the
# results from each matrix job (py3.13/3.14/3.15) distinct instead of merging
# identically-named benchmarks.
_PY_TAG = f"py{sys.version_info.major}.{sys.version_info.minor}"

pytestmark = [pytest.mark.benchmark, pytest.mark.parametrize("python", [_PY_TAG])]


def test_perft_initial_depth_5(python: str) -> None:
    assert perft(Board(), 5) == 27_117


def test_search_initial_depth_6(python: str) -> None:
    assert Engine(hash_mb=4).search(Board(), depth=6).best_move is not None


def test_legal_moves_and_push(python: str) -> None:
    board = Board()
    for _ in range(20):
        moves = board.legal_moves
        if not moves:
            break
        board.push(moves[0])
