//! Legal move generation for international draughts.
//!
//! The rules that shape the generator:
//!
//! * Capturing is mandatory, and among all capture sequences only those that
//!   take the maximum number of pieces are legal.
//! * Men move one square diagonally forward but capture in all four
//!   directions. Kings fly: they move and capture along whole diagonals and
//!   may land on any empty square behind the captured piece.
//! * Captured pieces are removed only after the sequence ends ("Turkish
//!   strike"): they still block, and no piece may be jumped twice.
//! * A man is crowned only if its move ends on the last row; passing through
//!   it during a capture does not count.
//! * Sequences with the same origin, destination and captured set are one
//!   move, so the generator removes such duplicates.

use crate::bitboard::Bitboard;
use crate::moves::{Move, MoveList};
use crate::piece::Color;
use crate::position::Position;
use crate::square::Square;

/// The four diagonal steps: up-left, up-right, down-left, down-right.
const DIRECTIONS: [i32; 4] = [-6, -5, 5, 6];

/// Fills `list` with all legal moves of `pos`.
pub(crate) fn generate(pos: &Position, list: &mut MoveList) {
    list.clear();
    match pos.side_to_move() {
        Color::White => generate_for::<true>(pos, list),
        Color::Black => generate_for::<false>(pos, list),
    }
}

/// Whether the side to move has a capture.
pub(crate) fn has_captures(pos: &Position) -> bool {
    let us = pos.side_to_move();
    let them = pos.pieces(!us);
    let empty = pos.empty();
    let men = pos.men(us);
    for dir in DIRECTIONS {
        // A man on `s` captures towards `dir` if `s + dir` holds an enemy
        // piece and `s + 2 * dir` is empty.
        if !(men & them.shift(-dir) & empty.shift(-2 * dir)).is_empty() {
            return true;
        }
    }
    pos.kings(us).iter().any(|king| {
        DIRECTIONS.into_iter().any(|dir| {
            let blocker = slide(king.bitboard(), dir, empty);
            !(blocker & them).is_empty() && !(blocker.shift(dir) & empty).is_empty()
        })
    })
}

/// Whether the side to move has a non-capturing move.
pub(crate) fn has_quiet_moves(pos: &Position) -> bool {
    let us = pos.side_to_move();
    let empty = pos.empty();
    let forward: [i32; 2] = match us {
        Color::White => [-6, -5],
        Color::Black => [5, 6],
    };
    let (men, kings) = (pos.men(us), pos.kings(us));
    forward
        .into_iter()
        .any(|dir| !(men.shift(dir) & empty).is_empty())
        || DIRECTIONS
            .into_iter()
            .any(|dir| !(kings.shift(dir) & empty).is_empty())
}

/// Returns the first square beyond `from` in direction `dir` that is not in
/// `empty`; it may be off the board, i.e. a ghost or out-of-range bit.
#[inline(always)]
fn slide(from: Bitboard, dir: i32, empty: Bitboard) -> Bitboard {
    let mut sq = from.shift(dir);
    while !(sq & empty).is_empty() {
        sq = sq.shift(dir);
    }
    sq
}

#[inline(always)]
fn generate_for<const WHITE: bool>(pos: &Position, list: &mut MoveList) {
    let (us, them) = if WHITE {
        (Color::White, Color::Black)
    } else {
        (Color::Black, Color::White)
    };
    let men = pos.men(us);
    let kings = pos.kings(us);
    let empty = pos.empty();

    let mut captures = Captures {
        list,
        them: pos.pieces(them),
        empty,
        from: Bitboard::EMPTY,
        best: 0,
    };
    for man in men {
        captures.from = man.bitboard();
        captures.empty = empty | captures.from;
        captures.man(captures.from, Bitboard::EMPTY);
    }
    for king in kings {
        captures.from = king.bitboard();
        captures.empty = empty | captures.from;
        captures.king(captures.from, Bitboard::EMPTY, 0);
    }
    if !list.is_empty() {
        return;
    }

    // Quiet man moves: forward is up the board for White, down for Black.
    let forward: [i32; 2] = if WHITE { [-6, -5] } else { [5, 6] };
    for dir in forward {
        for to in men.shift(dir) & empty {
            let from = to.bitboard().shift(-dir);
            list.push(Move::new(square_of(from), to, Bitboard::EMPTY));
        }
    }

    // Quiet king moves: every empty square along each diagonal.
    for king in kings {
        for dir in DIRECTIONS {
            let mut to = king.bitboard().shift(dir) & empty;
            while let Some(sq) = to.first() {
                list.push(Move::new(king, sq, Bitboard::EMPTY));
                to = to.shift(dir) & empty;
            }
        }
    }
}

#[inline(always)]
fn square_of(single: Bitboard) -> Square {
    single.first().expect("bitboard holds one square")
}

/// Depth-first enumeration of capture sequences for one moving piece.
struct Captures<'a> {
    list: &'a mut MoveList,
    /// Enemy pieces, including those already captured in this sequence.
    them: Bitboard,
    /// Empty squares, including the moving piece's origin.
    empty: Bitboard,
    /// The origin of the moving piece.
    from: Bitboard,
    /// The number of pieces taken by the moves in `list`.
    best: u32,
}

impl Captures<'_> {
    /// Extends a man's sequence from `at`, having taken `captured` so far.
    fn man(&mut self, at: Bitboard, captured: Bitboard) {
        let mut extended = false;
        for dir in DIRECTIONS {
            let over = at.shift(dir) & self.them & !captured;
            if over.is_empty() {
                continue;
            }
            let land = over.shift(dir) & self.empty;
            if !land.is_empty() {
                extended = true;
                self.man(land, captured | over);
            }
        }
        if !extended {
            self.record(at, captured);
        }
    }

    /// Extends a king's sequence from `at`, having taken `captured` so far.
    ///
    /// `came_from` is the direction of the previous jump and 0 at the start.
    fn king(&mut self, at: Bitboard, captured: Bitboard, came_from: i32) {
        let mut extended = false;
        for dir in DIRECTIONS {
            // Going back would jump the piece just taken, which is not allowed.
            if dir == -came_from {
                continue;
            }
            let over = slide(at, dir, self.empty) & self.them & !captured;
            if over.is_empty() {
                continue;
            }
            let mut land = over.shift(dir) & self.empty;
            let first_landing = land;
            while !land.is_empty() {
                extended = true;
                self.king_landing(land, captured | over, dir, land == first_landing);
                land = land.shift(dir) & self.empty;
            }
        }
        if !extended {
            self.record(at, captured);
        }
    }

    /// Continues a king sequence after landing on `at` while moving in `dir`.
    ///
    /// Continuing straight on from a later landing square repeats exactly
    /// what continuing from the first landing square finds, so only the
    /// first landing square explores the straight direction. A sequence cut
    /// short this way takes fewer pieces than its straight continuation, so
    /// the maximum-capture rule discards it.
    fn king_landing(&mut self, at: Bitboard, captured: Bitboard, dir: i32, first_landing: bool) {
        if first_landing {
            self.king(at, captured, dir);
            return;
        }
        let mut extended = false;
        for turn in DIRECTIONS {
            if turn == dir || turn == -dir {
                continue;
            }
            let over = slide(at, turn, self.empty) & self.them & !captured;
            if over.is_empty() {
                continue;
            }
            let mut land = over.shift(turn) & self.empty;
            let first = land;
            while !land.is_empty() {
                extended = true;
                self.king_landing(land, captured | over, turn, land == first);
                land = land.shift(turn) & self.empty;
            }
        }
        if !extended {
            self.record(at, captured);
        }
    }

    /// Adds a finished sequence ending on `to`, applying the maximum-capture
    /// rule and removing duplicates.
    fn record(&mut self, to: Bitboard, captured: Bitboard) {
        let count = captured.count();
        if count == 0 || count < self.best {
            return;
        }
        if count > self.best {
            self.best = count;
            self.list.clear();
        }
        let mv = Move::new(square_of(self.from), square_of(to), captured);
        // Capture lists are short; a linear scan is cheaper than hashing.
        if !self.list.contains(&mv) {
            self.list.push(mv);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moves(fen: &str) -> Vec<String> {
        let pos = Position::from_fen(fen).unwrap();
        let mut moves: Vec<String> = pos.legal_moves().iter().map(|mv| mv.to_hub()).collect();
        moves.sort();
        moves
    }

    #[test]
    fn initial_moves() {
        assert_eq!(
            moves("W:W31-50:B1-20"),
            [
                "31-26", "31-27", "32-27", "32-28", "33-28", "33-29", "34-29", "34-30", "35-30"
            ]
        );
    }

    #[test]
    fn men_capture_backwards() {
        assert_eq!(moves("W:W23:B18,28"), ["23x12x18", "23x32x28"]);
    }

    #[test]
    fn maximum_capture_is_mandatory() {
        // 28x19 takes one piece, 32x12 takes two and is the only legal move.
        assert_eq!(moves("W:W28,32:B17,23,27"), ["32x12x17x27"]);
    }

    #[test]
    fn man_is_not_crowned_when_passing_the_last_row() {
        // 13x2 takes 8 and lands on the last row, then 2x11 takes 7.
        let pos = Position::from_fen("W:W13:B7,8").unwrap();
        let legal = pos.legal_moves();
        assert_eq!(legal.len(), 1);
        assert_eq!(legal[0].to_hub(), "13x11x7x8");
        assert!(pos.after(legal[0]).kings(Color::White).is_empty());
    }

    #[test]
    fn man_is_crowned_on_the_last_row() {
        let pos = Position::from_fen("W:W7:B20").unwrap();
        let mv = pos
            .legal_moves()
            .iter()
            .copied()
            .find(|mv| mv.to_string() == "7-2")
            .unwrap();
        assert_eq!(pos.after(mv).kings(Color::White).count(), 1);
    }

    #[test]
    fn flying_king_moves_and_captures() {
        // A lone king in the corner slides along the whole long diagonal.
        assert_eq!(moves("W:WK46:B1").len(), 9);
        // It captures from a distance and may land on any square behind.
        assert_eq!(
            moves("W:WK46:B28"),
            ["46x10x28", "46x14x28", "46x19x28", "46x23x28", "46x5x28"]
        );
    }

    #[test]
    fn king_circles_back_to_its_origin() {
        // The king on 32 can take the diamond 28, 18, 17, 27 in either
        // direction. Both circles end on 32 with the same captured set, so
        // they are one move; the other endings are the squares behind 27
        // (38, 43, 49) and behind 28 (37, 41, 46).
        let pos = Position::from_fen("W:WK32:B17,18,27,28").unwrap();
        let legal = pos.legal_moves();
        let mut ends: Vec<u8> = legal.iter().map(|mv| mv.to().number()).collect();
        ends.sort_unstable();
        assert_eq!(ends, [32, 37, 38, 41, 43, 46, 49]);
        assert!(legal.iter().all(|mv| mv.captured().count() == 4));
        let circle = legal.iter().find(|mv| mv.from() == mv.to()).unwrap();
        let after = pos.after(*circle);
        assert_eq!(after.kings(Color::White), circle.from().bitboard());
        assert!(after.pieces(Color::Black).is_empty());
    }

    #[test]
    fn no_moves_when_blocked() {
        assert_eq!(moves("W:W46:B41,37"), Vec::<String>::new());
    }

    #[test]
    fn has_captures_agrees_with_generation() {
        for fen in [
            "W:W31-50:B1-20",
            "W:W23:B18,28",
            "W:WK46:B28",
            "W:WK46:B28,23",
            "B:W28:B23",
        ] {
            let pos = Position::from_fen(fen).unwrap();
            let generated = pos.legal_moves().first().is_some_and(|mv| mv.is_capture());
            assert_eq!(pos.has_captures(), generated, "{fen}");
        }
    }

    #[test]
    fn has_quiet_moves_agrees_with_generation() {
        // Random games played to the end cover blocked and empty sides too.
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for _ in 0..200 {
            let mut pos = Position::initial();
            loop {
                let moves = pos.legal_moves();
                if !pos.has_captures() {
                    assert_eq!(pos.has_quiet_moves(), !moves.is_empty(), "{}", pos.fen());
                }
                if moves.is_empty() {
                    break;
                }
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                pos = pos.after(moves[(seed % moves.len() as u64) as usize]);
            }
        }
    }
}
