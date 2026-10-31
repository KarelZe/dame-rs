# AGENTS.md

## Project Overview
`dame` is an international draughts (10x10 checkers) engine. The engine core
is a dependency-free Rust crate; a command-line front end speaks the Hub
protocol and lets you play in the terminal, a PyO3 extension exposes a
pythonic API, and a native macOS app drives the engine over Hub. Performance is the primary design constraint for the engine
core — prefer Rust-side optimizations over Python wrappers.

## Environment Setup
- Rust stable (see `rust-toolchain.toml`), MSRV in `Cargo.toml`
- Python >= 3.10, managed with `uv`
- Install: `uv sync`
- After editing Rust, rebuild the extension: `uv run maturin develop --release`

## Project Structure
- `crates/dame/` — engine core (no dependencies, no Python)
- `crates/dame-cli/` — `dame` command: Hub protocol and terminal play; a library
  too, so the Python package can run it as its `dame` script
- `crates/dame-python/` — PyO3 bindings, built as `dame._core`
- `crates/dame-macos/` — native macOS app (AppKit via objc2); a stub elsewhere
- `scripts/bundle-macos.sh` — builds `target/release/Dame.app`
- `scripts/dmg-macos.sh` — packs it into the `.dmg` attached to releases
- `python/dame/` — Python package: re-exports, type stubs (`_core.pyi`)
- `tests/` — pytest suite
- `docs/` — Zensical documentation source

## Testing
- Rust: `cargo test --release` (perft checks are slow in debug builds)
- Python: `uv run pytest`
- Types: `uv run ty check`
- Rust changes require a rebuild before pytest picks them up:
  `uv run maturin develop --release`

## Performance workflow

All performance changes follow this loop. One change per experiment,
measured on the same hardware, kept only if it wins.

1. Pin a baseline before any change:
   `cargo bench -p dame -- --save-baseline <name>`
2. State a single, testable hypothesis before editing (e.g. "precomputed king
   rays cut king-move generation time by 20%"). One change per experiment.
3. Re-run the same benchmark on identical hardware and a quiet system.
   `python scripts/ab_bench.py --a main --b <ref>` builds both refs and
   interleaves their runs; it reports median + IQR and flags changed node
   counts.
   Report median + IQR, not mean.
4. If the change is under 3% or within noise, REVERT.
5. Verify correctness with the perft suite before keeping any win.
   Search changes must not change node counts unless that is the point.
6. Check codegen for the hot loop with `cargo asm` when claiming wins from
   branch removal or vectorization.
7. Never edit the release profile in `Cargo.toml` (LTO, codegen-units)
   as part of an algorithmic experiment. Separate PR.

## Documentation
- Serve locally: `uv run zensical serve`, then open http://localhost:8000

## Code Style
- Python: `ruff format .` and `ruff check . --fix`
- Rust: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`
- Pre-commit hooks (via prek): `prek install` once, then `prek run --all-files`

## Before Committing
- All Rust and Python tests pass
- `prek run --all-files` passes
- Commit message follows Conventional Commits with a trailing emoji:
  - `feat: … ✨`  `fix: … 🐛`  `perf: … ⚡`
  - `docs: … 📝`  `test: … ✅`  `style: … 💄`  `build: … 🔧`

## Releasing
- The version lives in `[workspace.package]` of `Cargo.toml`; maturin passes it
  to the Python package.
- PRs are squash-merged, so the PR title becomes the commit on `main` and must be
  a conventional commit (checked by `.github/workflows/pr-title.yml`).
- Never merge a PR with a merge commit. release-please reads every commit
  reachable from `main`, and a merge commit brings in the branch's own commits
  plus a commit whose body repeats the PR title. The changelog then lists the
  change twice. Rebase merges skip the title check, so avoid them as well.
- release-please keeps a release PR open that bumps the version and
  `CHANGELOG.md`. Merging it tags the release, creates the GitHub Release and
  publishes the wheels to PyPI. The release also gets the wheels and the
  macOS app's `.dmg`.
- Never bump versions or push tags by hand.

## Third-party code
- Do not port code from GPL-licensed engines into this MIT-licensed crate.
  Ideas and published algorithms are fine; copied code is not.
- Record the source and license of any ported or vendored code in the file
  header and in `README.md`.

## PR Guidelines
- Keep diffs small and focused
- Add or update tests for changed code paths
- Update `docs/` and `README.md` when behavior or API changes

## Permissions

**Allowed without asking**
- Read files, run tests, run linters and formatters, run benchmarks
- Build the extension locally

**Ask first**
- Installing new packages (Python or Rust)
- Pushing to `main`
- Deleting files
- Modifying CI workflows or release configuration
