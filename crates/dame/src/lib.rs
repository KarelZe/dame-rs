//! An international draughts (10x10 checkers) engine.
//!
//! The crate is the engine core: board representation, move generation,
//! search and evaluation. It has no dependencies and no Python coupling; the
//! `dame-cli`, `dame-python` and `dame-macos` crates build on top of it.
//!
//! ```
//! use dame::{Position, perft};
//!
//! let pos = Position::initial();
//! let moves = pos.legal_moves();
//! assert_eq!(moves.len(), 9);
//!
//! assert_eq!(perft(&pos, 2), 81);
//!
//! let next = pos.after(moves[0]);
//! assert_eq!(next.legal_moves().len(), 9);
//! ```

#![warn(missing_docs)]

pub mod bitboard;
pub mod book;
pub mod egdb;
pub mod eval;
pub mod game;
pub mod hub;
mod movegen;
pub mod moves;
pub mod notation;
pub mod pdn;
mod perft;
pub mod piece;
pub mod position;
pub mod search;
pub mod square;
mod tt;
mod zobrist;

pub use bitboard::Bitboard;
pub use book::OpeningBook;
pub use egdb::{EndgameDb, Wdl};
pub use eval::{Classical, Evaluate};
pub use game::{Game, IllegalMoveError, Outcome, Termination};
pub use moves::{Move, MoveList};
pub use notation::ParseMoveError;
pub use perft::{divide, perft};
pub use piece::{Color, Piece, PieceKind};
pub use position::{Position, PositionError, START_FEN};
pub use search::{Info, Limits, SearchResult, Searcher};
pub use square::Square;

/// The crate version, as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Compiles the examples in the crate's README, which is its crates.io page.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
