# dame-rs

`dame`[^name] is an [international draughts](https://en.wikipedia.org/wiki/International_draughts)
(10x10 checkers) engine written in Rust 🦀, with a pythonic API.

[^name]: *Dame* is German for draughts (checkers), and also for a king in the game: a
piece that reaches the far row is crowned a *Dame*.

## Installation

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
`info` callback. See the [API Reference](https://KarelZe.github.io/dame-rs/api/).

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

## Playing

Play against the engine in the terminal, or plug it into any GUI that speaks
the [Hub protocol](https://github.com/rhalbersma/scan/blob/master/protocol.txt):

```bash
uv tool install dame-rs               # the `dame` command; or: cargo install --path crates/dame-cli
dame play --time 2
dame hub
DAME_BOOK=games.pdn dame play         # open games from a PDN game collection
```

On macOS, `Dame.app` is a native board that plays against the engine. Download
the `.dmg` from the [latest release](https://github.com/KarelZe/dame-rs/releases/latest)
or build it with `scripts/bundle-macos.sh` (see [macOS app](https://KarelZe.github.io/dame-rs/macos/)).

<p align="center">
  <img src="https://raw.githubusercontent.com/KarelZe/dame-rs/main/docs/assets/macos-app.png" alt="Dame.app, a native macOS board, in the middle of a game, with the engine's expected line, depth and score in the status bar" width="480">
</p>

See [Command line](https://KarelZe.github.io/dame-rs/cli/) for details and
[Engine design](https://KarelZe.github.io/dame-rs/design/) for how the engine works.

## Development

```bash
uv sync                               # build the extension and install dev tools
cargo test --release                  # Rust tests
uv run pytest                         # Python tests
uv run prek run --all-files           # linters and formatters
```

See [AGENTS.md](AGENTS.md) for the full workflow.

> [!WARNING]
> 🧪 **Experimental.** `dame` is in early development and was built just for fun
> with [Claude Code](https://github.com/anthropics/claude-code). The API will change.

## License

`dame` is licensed under the [MIT license](LICENSE).

## Contact

To get in contact, please open an issue or contact me via `github@markusbilz.com`.
