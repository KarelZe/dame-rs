# dame

`dame` is an [international draughts](https://en.wikipedia.org/wiki/International_draughts)
(10x10 checkers) engine: bitboard move generation, alpha-beta search and
evaluation. The crate is the engine core and has no dependencies.

## Usage

```bash
cargo add dame
```

```rust
use std::time::Duration;

use dame::{Game, Limits, Position, Searcher, perft};

let pos = Position::initial();
assert_eq!(pos.legal_moves().len(), 9);
assert_eq!(perft(&pos, 2), 81);

let game = Game::new();
let mut searcher = Searcher::default();
let result = searcher.search(&game, &Limits::move_time(Duration::from_millis(100)), |_| {});
println!("best move: {:?}", result.best_move);
```

See [docs.rs](https://docs.rs/dame) for the API and
[Engine design](https://KarelZe.github.io/dame-rs/design/) for how the engine works.
The crate requires Rust 1.88 or newer.

## Related

The same engine is also available as:

- [`dame-rs`](https://pypi.org/project/dame-rs/) on PyPI, a Python package with a pythonic API,
- the `dame` command, which speaks the [Hub protocol](https://github.com/rhalbersma/scan/blob/master/protocol.txt)
  and plays in the terminal (see [Command line](https://KarelZe.github.io/dame-rs/cli/)),
- `Dame.app`, a native macOS board (see the [latest release](https://github.com/KarelZe/dame-rs/releases/latest)).

> [!WARNING]
> 🧪 **Experimental.** `dame` is in early development. The API will change.

## License

`dame` is licensed under the [MIT license](LICENSE).
