# dame

## Introduction

`dame`[^name] is an [international draughts](https://en.wikipedia.org/wiki/International_draughts)
(10x10 checkers) engine written in Rust 🦀, with a pythonic API.

[^name]: *Dame* is German for draughts (checkers), and also for a king in the game: a
piece that reaches the far row is crowned a *Dame*.

## Installation

Install the latest version from [pypi](https://pypi.org/project/dame-rs/):

```bash
uv add dame-rs
```

or if you prefer slow:

```bash
pip install dame-rs
```

## Usage

```python
import dame

board = dame.Board()  # or dame.Board("W:W31-50:B1-20")
board.legal_moves  # [Move('31-26'), Move('32-27'), ...]
board.push("32-28")
board.push("19-23")
board.legal_moves  # [Move('28x19x23')]: capturing is mandatory

engine = dame.Engine(hash_mb=64)
result = engine.search(board, time=1.0)
result.best_move, result.score, result.depth  # (Move('28x19x23'), 2, 13)
board.push(result.best_move)
print(board)
```

The search runs without the GIL and reports progress through an optional
`info` callback. See the [API Reference](api.md).

### Rust

The engine core is also published as the [`dame`](https://crates.io/crates/dame) crate:

```bash
cargo add dame
```

```rust
use dame::{Position, perft};

let pos = Position::initial();
assert_eq!(pos.legal_moves().len(), 9);
assert_eq!(perft(&pos, 2), 81);
```

The crate requires Rust 1.88 or newer. See [docs.rs](https://docs.rs/dame) for details.

To play against the engine, use the [command line](cli.md) or the
[macOS app](macos.md).

!!! example "Experimental"
    `dame` is in early development and was built just for fun with
    [Claude Code](https://github.com/anthropics/claude-code). The API will change.
