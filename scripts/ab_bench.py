#!/usr/bin/env python3
"""
A/B benchmark of two builds of the `dame` engine on the same machine.

Builds the `dame` command for two git refs, then times a fixed suite of
searches and perft runs. Every round runs each case once per build, in
alternating order, which cancels most of the drift of a busy machine. The
report gives the median time over rounds, its IQR and the B/A ratio of
medians, as AGENTS.md asks.

Node counts double as a correctness check: perft counts must match exactly,
and a changed search node count means the search tree changed, which a pure
speed-up must not do.

Usage:
    python scripts/ab_bench.py                       # origin/main vs working tree
    python scripts/ab_bench.py --a main --b my-branch --rounds 9
    python scripts/ab_bench.py --b pull/42/head      # a GitHub PR
    python scripts/ab_bench.py --summary out.md      # also write the report

`.` stands for the working tree, uncommitted changes included. Other refs are
checked out into temporary worktrees; refs of the form `pull/N/head` are
fetched from `origin` first.
"""

from __future__ import annotations

import argparse
import os
import platform
import re
import statistics
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TARGET_ROOT = REPO_ROOT / "target" / "ab"

# Ratios inside this band are noise; AGENTS.md reverts changes under 3%.
NOISE = 0.03

WORKTREE = "."

_SEARCH_LINE = re.compile(r"^depth\s+\d+.*\bnodes\s+(\d+)\s+time\s+([\d.]+)s", re.MULTILINE)
_PERFT_LINE = re.compile(r"perft\(\d+\) = (\d+) in ([\d.]+)s")


@dataclass(frozen=True)
class Case:
    """One benchmark: a search or perft of `fen` to `depth`."""

    name: str
    kind: str
    fen: str
    depth: int

    def args(self) -> list[str]:
        """
        Return the `dame` arguments that run this case.

        Returns:
            list[str]: the command line after the program name.

        """
        if self.kind == "perft":
            return ["perft", str(self.depth), "--fen", self.fen]
        # A generous time limit, so the depth alone ends the search.
        return ["search", "--fen", self.fen, "--depth", str(self.depth), "--time", "600"]


INITIAL = "W:W31-50:B1-20"
WOLDOUBY = "W:B12,13,14,16,18,19,21,23,24,26:W25,27,28,30,32,33,34,35,37,38"
KINGS = "B:BK17,K24:W6,9,10,11,20,21,22,23,30,K31,33,37,41,42,43,44,46"

# Each case takes a few hundred milliseconds, long enough for the millisecond
# timer of the `dame` output and short enough for several rounds.
CASES = [
    Case("search/initial", "search", INITIAL, 12),
    Case("search/woldouby", "search", WOLDOUBY, 14),
    Case("search/kings", "search", KINGS, 13),
    Case("perft/initial", "perft", INITIAL, 8),
    Case("perft/kings", "perft", KINGS, 7),
]


def _git(*args: str, cwd: Path = REPO_ROOT) -> str:
    """
    Run git and return its standard output.

    Returns:
        str: the stripped output.

    """
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def _checkout(ref: str, tmp: Path) -> tuple[Path, str]:
    """
    Check `ref` out into a worktree below `tmp`, or use the working tree for `.`.

    Returns:
        tuple[Path, str]: the source directory and a label with the commit.

    """
    if ref == WORKTREE:
        dirty = " + changes" if _git("status", "--porcelain", "--untracked-files=no") else ""
        return REPO_ROOT, f"working tree ({_git('rev-parse', '--short', 'HEAD')}{dirty})"
    if ref.startswith("pull/"):
        local = "ab-bench/" + ref.replace("/", "-")
        _git("fetch", "--quiet", "--force", "origin", f"{ref}:{local}")
        ref = local
    commit = _git("rev-parse", "--verify", f"{ref}^{{commit}}")
    path = tmp / commit[:12]
    if not path.exists():
        _git("worktree", "add", "--quiet", "--detach", str(path), commit)
    return path, f"{ref} ({commit[:7]})"


def _build(source: Path, slot: str) -> Path:
    """
    Build the release `dame` command from `source`.

    Each slot keeps its own target directory, so rebuilds are incremental.

    Returns:
        Path: the binary.

    """
    target = TARGET_ROOT / slot
    subprocess.run(
        ["cargo", "build", "--quiet", "--release", "--locked", "-p", "dame-cli", "--target-dir", str(target)],
        cwd=source,
        check=True,
    )
    return target / "release" / ("dame.exe" if os.name == "nt" else "dame")


def _run(binary: Path, case: Case) -> tuple[int, float]:
    """
    Run one case.

    Returns:
        tuple[int, float]: the node count and the time in seconds the engine reports.

    Raises:
        RuntimeError: if the output holds no result.

    """
    env = {**os.environ, "NO_COLOR": "1"}
    out = subprocess.run([binary, *case.args()], check=True, capture_output=True, text=True, env=env).stdout
    matches = (_PERFT_LINE if case.kind == "perft" else _SEARCH_LINE).findall(out)
    if not matches:
        msg = f"no result in the output of {case.name}:\n{out}"
        raise RuntimeError(msg)
    nodes, seconds = matches[-1]
    return int(nodes), float(seconds)


def _quartiles(xs: list[float]) -> tuple[float, float, float]:
    """
    Return the quartiles of `xs`, all equal to the value for a single sample.

    Returns:
        tuple[float, float, float]: first quartile, median, third quartile.

    """
    if len(xs) < 2:
        return xs[0], xs[0], xs[0]
    q1, q2, q3 = statistics.quantiles(xs, n=4, method="inclusive")
    return q1, q2, q3


def _verdict(ratio: float) -> str:
    """
    Classify a B/A ratio of medians.

    Returns:
        str: "faster", "slower" or "noise".

    """
    if ratio < 1 - NOISE:
        return "faster"
    if ratio > 1 + NOISE:
        return "slower"
    return "noise"


def main() -> int:
    """
    Run the A/B comparison and print a markdown report.

    Returns:
        int: 0 on success, 1 if node counts disagree.

    """
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--a", default="origin/main", help="baseline ref (default: origin/main)")
    parser.add_argument("--b", default=WORKTREE, help="candidate ref; '.' is the working tree (default)")
    parser.add_argument("--rounds", type=int, default=7, help="runs of each case per build (default: 7)")
    parser.add_argument("--case", action="append", help="run only cases whose name contains this; repeatable")
    parser.add_argument("--summary", type=Path, help="also write the report to this markdown file")
    args = parser.parse_args()

    cases = [c for c in CASES if not args.case or any(s in c.name for s in args.case)]
    if not cases:
        parser.error("no case matches --case")

    # The binaries live in `target/ab`, so the worktrees are only needed to build.
    builds = []
    try:
        with tempfile.TemporaryDirectory(prefix="dame-ab-") as tmp:
            for slot, ref in (("a", args.a), ("b", args.b)):
                source, label = _checkout(ref, Path(tmp))
                print(f"building {slot}: {label}", file=sys.stderr)
                builds.append((label, _build(source, slot)))
    finally:
        _git("worktree", "prune")

    times: dict[str, tuple[list[float], list[float]]] = {c.name: ([], []) for c in cases}
    nodes: dict[str, tuple[set[int], set[int]]] = {c.name: (set(), set()) for c in cases}
    for round_ in range(args.rounds):
        print(f"round {round_ + 1}/{args.rounds}", file=sys.stderr)
        order = (0, 1) if round_ % 2 == 0 else (1, 0)
        for case in cases:
            for i in order:
                count, seconds = _run(builds[i][1], case)
                times[case.name][i].append(seconds)
                nodes[case.name][i].add(count)

    lines = [
        "## dame A/B benchmark",
        "",
        f"- A: {builds[0][0]}",
        f"- B: {builds[1][0]}",
        f"- {args.rounds} rounds, alternating order; {platform.platform()}, {platform.machine()}",
        f"- times are medians in ms with the IQR; ratios within ±{NOISE:.0%} are noise",
        "",
        "| case | nodes A | nodes B | A ms [IQR] | B ms [IQR] | B/A | verdict |",
        "|---|--:|--:|--:|--:|--:|---|",
    ]
    failed = False
    for case in cases:
        (qa1, qa2, qa3), (qb1, qb2, qb3) = (_quartiles(t) for t in times[case.name])
        node_a, node_b = (", ".join(map(str, sorted(n))) for n in nodes[case.name])
        ratio = qb2 / qa2 if qa2 else float("nan")
        verdict = _verdict(ratio)
        if node_a != node_b:
            failed |= case.kind == "perft"
            verdict = "**perft mismatch**" if case.kind == "perft" else f"{verdict}, **tree changed**"
        lines.append(
            f"| {case.name} | {node_a} | {node_b} "
            f"| {qa2 * 1e3:.1f} [{qa1 * 1e3:.1f}, {qa3 * 1e3:.1f}] "
            f"| {qb2 * 1e3:.1f} [{qb1 * 1e3:.1f}, {qb3 * 1e3:.1f}] "
            f"| {ratio:.3f} | {verdict} |"
        )
    report = "\n".join(lines) + "\n"
    print(report)
    if args.summary:
        args.summary.write_text(report, encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
