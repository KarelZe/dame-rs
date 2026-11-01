//! Board positions: piece placement plus side to move.

use std::fmt;
use std::str::FromStr;

use crate::bitboard::Bitboard;
use crate::moves::{Move, MoveList};
use crate::piece::{Color, Piece, PieceKind};
use crate::square::Square;
use crate::{movegen, zobrist};

/// A position: where the pieces stand and whose turn it is.
///
/// Positions are small (40 bytes) and `Copy`; the search uses copy-make
/// ([`Position::after`]) rather than undoing moves. The Zobrist hash is
/// maintained incrementally.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Position {
    white: Bitboard,
    black: Bitboard,
    kings: Bitboard,
    side: Color,
    hash: u64,
}

/// The FEN of the standard starting position.
pub const START_FEN: &str = "W:W31-50:B1-20";

/// The reason a set of pieces is not a valid position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionError {
    /// A square holds both a white and a black piece.
    Overlap(Square),
    /// A man stands on its own promotion row and should have been crowned.
    ManOnPromotionRow(Square),
    /// The FEN or Hub string is malformed.
    Syntax(String),
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overlap(sq) => write!(f, "square {sq} holds both a white and a black piece"),
            Self::ManOnPromotionRow(sq) => {
                write!(f, "the man on square {sq} stands on its promotion row")
            }
            Self::Syntax(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for PositionError {}

impl Position {
    /// The standard starting position: Black on 1..=20, White on 31..=50,
    /// White to move.
    pub fn initial() -> Self {
        let black = (1..=20).filter_map(Square::new).collect();
        let white = (31..=50).filter_map(Square::new).collect();
        Self::from_parts(white, black, Bitboard::EMPTY, Color::White)
            .expect("the starting position is valid")
    }

    /// Builds a position from piece sets.
    ///
    /// `kings` marks which of the white and black pieces are kings.
    pub fn from_parts(
        white: Bitboard,
        black: Bitboard,
        kings: Bitboard,
        side: Color,
    ) -> Result<Self, PositionError> {
        if let Some(sq) = (white & black).first() {
            return Err(PositionError::Overlap(sq));
        }
        let kings = kings & (white | black);
        let misplaced = (white & !kings & Bitboard::WHITE_PROMOTION)
            | (black & !kings & Bitboard::BLACK_PROMOTION);
        if let Some(sq) = misplaced.first() {
            return Err(PositionError::ManOnPromotionRow(sq));
        }
        let mut pos = Self {
            white,
            black,
            kings,
            side,
            hash: 0,
        };
        pos.hash = pos.compute_hash();
        Ok(pos)
    }

    /// Parses a position in PDN FEN notation, e.g. `W:W31-50:B1-20` or
    /// `B:WK5,28,33:B12,K40`.
    ///
    /// Ranges (`31-50`), king markers (`K`), either order of the two piece
    /// lists, surrounding quotes and a trailing `.` are accepted.
    pub fn from_fen(fen: &str) -> Result<Self, PositionError> {
        let syntax = |msg: &str| PositionError::Syntax(format!("invalid FEN `{fen}`: {msg}"));

        let body = fen.trim().trim_matches('"').trim_end_matches('.');
        let mut fields = body.split(':').map(str::trim);
        let side = match fields.next() {
            Some("W" | "w") => Color::White,
            Some("B" | "b") => Color::Black,
            _ => return Err(syntax("expected `W` or `B` as the side to move")),
        };

        let (mut white, mut black, mut kings) = (Bitboard::EMPTY, Bitboard::EMPTY, Bitboard::EMPTY);
        for field in fields {
            let (color, list) = match field.split_at_checked(1) {
                Some(("W" | "w", rest)) => (&mut white, rest),
                Some(("B" | "b", rest)) => (&mut black, rest),
                _ => return Err(syntax("expected a piece list starting with `W` or `B`")),
            };
            for item in list
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
            {
                let (is_king, item) = match item.strip_prefix('K') {
                    Some(rest) => (true, rest),
                    None => (false, item),
                };
                let square = |s: &str| {
                    s.trim()
                        .trim_start_matches('K')
                        .parse::<Square>()
                        .map_err(|e| syntax(&e.to_string()))
                };
                let squares = match item.split_once('-') {
                    Some((lo, hi)) => {
                        let (lo, hi) = (square(lo)?.number(), square(hi)?.number());
                        (lo..=hi).filter_map(Square::new).collect()
                    }
                    None => square(item)?.bitboard(),
                };
                *color |= squares;
                if is_king {
                    kings |= squares;
                }
            }
        }
        Self::from_parts(white, black, kings, side)
    }

    /// The position in PDN FEN notation, e.g. `W:W31,32,...:B1,2,...`.
    pub fn fen(&self) -> String {
        let list = |pieces: Bitboard| {
            pieces
                .iter()
                .map(|sq| {
                    if self.kings.contains(sq) {
                        format!("K{sq}")
                    } else {
                        sq.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{}:W{}:B{}",
            self.side.to_char(),
            list(self.white),
            list(self.black)
        )
    }

    /// Parses the 51-character position string of the Hub protocol: the side
    /// to move (`W`/`B`) followed by one of `w b W B e` for each square.
    pub fn from_hub(s: &str) -> Result<Self, PositionError> {
        let syntax =
            |msg: &str| PositionError::Syntax(format!("invalid Hub position `{s}`: {msg}"));
        let mut chars = s.chars();
        let side = match chars.next() {
            Some('W') => Color::White,
            Some('B') => Color::Black,
            _ => return Err(syntax("expected `W` or `B` as the side to move")),
        };
        let (mut white, mut black, mut kings) = (Bitboard::EMPTY, Bitboard::EMPTY, Bitboard::EMPTY);
        let mut count = 0;
        for (sq, c) in Square::all().zip(chars.by_ref()) {
            count += 1;
            if c == 'e' {
                continue;
            }
            let piece = Piece::from_char(c)
                .ok_or_else(|| syntax("expected one of `w b W B e` per square"))?;
            match piece.color {
                Color::White => white |= sq.bitboard(),
                Color::Black => black |= sq.bitboard(),
            }
            if piece.kind == PieceKind::King {
                kings |= sq.bitboard();
            }
        }
        if count != 50 || chars.next().is_some() {
            return Err(syntax("expected exactly 50 squares"));
        }
        Self::from_parts(white, black, kings, side)
    }

    /// The position as a 51-character Hub protocol string.
    pub fn hub(&self) -> String {
        std::iter::once(self.side.to_char())
            .chain(Square::all().map(|sq| self.piece_at(sq).map_or('e', Piece::to_char)))
            .collect()
    }

    /// The side to move.
    #[inline]
    pub const fn side_to_move(&self) -> Color {
        self.side
    }

    /// The Zobrist hash, including the side to move.
    #[inline]
    pub const fn hash(&self) -> u64 {
        self.hash
    }

    /// All pieces of `color`.
    #[inline]
    pub const fn pieces(&self, color: Color) -> Bitboard {
        match color {
            Color::White => self.white,
            Color::Black => self.black,
        }
    }

    /// The men of `color`.
    #[inline]
    pub fn men(&self, color: Color) -> Bitboard {
        self.pieces(color) & !self.kings
    }

    /// The kings of `color`.
    #[inline]
    pub fn kings(&self, color: Color) -> Bitboard {
        self.pieces(color) & self.kings
    }

    /// All occupied squares.
    #[inline]
    pub fn occupied(&self) -> Bitboard {
        self.white | self.black
    }

    /// All empty squares.
    #[inline]
    pub fn empty(&self) -> Bitboard {
        !self.occupied()
    }

    /// The piece on `square`, if any.
    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        let color = if self.white.contains(square) {
            Color::White
        } else if self.black.contains(square) {
            Color::Black
        } else {
            return None;
        };
        let kind = if self.kings.contains(square) {
            PieceKind::King
        } else {
            PieceKind::Man
        };
        Some(Piece { color, kind })
    }

    /// All legal moves. Captures are mandatory and only sequences taking the
    /// maximum number of pieces are legal.
    pub fn legal_moves(&self) -> MoveList {
        let mut list = MoveList::new();
        movegen::generate(self, &mut list);
        list
    }

    /// Whether the side to move must capture.
    pub fn has_captures(&self) -> bool {
        movegen::has_captures(self)
    }

    /// Whether the side to move has a non-capturing move, ignoring that a
    /// capture would make it illegal.
    pub fn has_quiet_moves(&self) -> bool {
        movegen::has_quiet_moves(self)
    }

    /// Plays `mv`, which must be legal in this position.
    ///
    /// Legality is not checked; playing an illegal move leaves the position
    /// in an unspecified (but memory-safe) state.
    pub fn play(&mut self, mv: Move) {
        let us = self.side;
        let them = !us;
        let (from, to, captured) = (mv.from(), mv.to(), mv.captured());
        debug_assert!(self.pieces(us).contains(from), "no {us} piece on {from}");

        for sq in captured {
            let kind = if self.kings.contains(sq) {
                PieceKind::King
            } else {
                PieceKind::Man
            };
            self.hash ^= zobrist::piece_key(Piece { color: them, kind }, sq);
        }
        *self.pieces_mut(them) &= !captured;
        self.kings &= !captured;

        let moved = from.bitboard() ^ to.bitboard();
        *self.pieces_mut(us) ^= moved;
        let kind = if self.kings.contains(from) {
            self.kings ^= moved;
            PieceKind::King
        } else {
            PieceKind::Man
        };
        self.hash ^= zobrist::piece_key(Piece { color: us, kind }, from);
        let promotion = match us {
            Color::White => Bitboard::WHITE_PROMOTION,
            Color::Black => Bitboard::BLACK_PROMOTION,
        };
        let kind = if kind == PieceKind::Man && promotion.contains(to) {
            self.kings |= to.bitboard();
            PieceKind::King
        } else {
            kind
        };
        self.hash ^= zobrist::piece_key(Piece { color: us, kind }, to);

        self.side = them;
        self.hash ^= zobrist::SIDE_KEY;
    }

    /// The position after `mv`, which must be legal in this position.
    #[inline]
    #[must_use]
    pub fn after(&self, mv: Move) -> Self {
        let mut next = *self;
        next.play(mv);
        next
    }

    /// The same position seen from the other side: the board is rotated by
    /// 180 degrees and the colors are swapped.
    #[must_use]
    pub fn flipped(&self) -> Self {
        // Rotating the board maps bit `b` to bit `53 - b`.
        let rotate = |bb: Bitboard| Bitboard(bb.0.reverse_bits() >> 10);
        Self::from_parts(
            rotate(self.black),
            rotate(self.white),
            rotate(self.kings),
            !self.side,
        )
        .expect("flipping preserves validity")
    }

    #[inline]
    fn pieces_mut(&mut self, color: Color) -> &mut Bitboard {
        match color {
            Color::White => &mut self.white,
            Color::Black => &mut self.black,
        }
    }

    fn compute_hash(&self) -> u64 {
        let pieces = Square::all()
            .filter_map(|sq| self.piece_at(sq).map(|piece| zobrist::piece_key(piece, sq)));
        let side = if self.side == Color::Black {
            zobrist::SIDE_KEY
        } else {
            0
        };
        pieces.fold(side, |hash, key| hash ^ key)
    }
}

impl Default for Position {
    fn default() -> Self {
        Self::initial()
    }
}

impl FromStr for Position {
    type Err = PositionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_fen(s)
    }
}

impl fmt::Debug for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Position({})", self.fen())
    }
}

impl fmt::Display for Position {
    /// A board diagram from White's point of view; `.` marks empty squares.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut squares = Square::all();
        for row in 0..10 {
            for file in 0..10 {
                let c = if (row + file) % 2 == 1 {
                    let sq = squares.next().expect("50 playable squares");
                    self.piece_at(sq).map_or('.', Piece::to_char)
                } else {
                    ' '
                };
                let sep = if file == 9 { "\n" } else { " " };
                write!(f, "{c}{sep}")?;
            }
        }
        write!(f, "{} to move", self.side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_position() {
        let pos = Position::initial();
        assert_eq!(pos.pieces(Color::White).count(), 20);
        assert_eq!(pos.pieces(Color::Black).count(), 20);
        assert_eq!(pos.side_to_move(), Color::White);
        assert_eq!(Position::from_fen(START_FEN).unwrap(), pos);
    }

    #[test]
    fn fen_round_trip() {
        let fen = "B:W18,24,27,28,K10,K15:B12,16,20,K22,K25,K29";
        let pos = Position::from_fen(fen).unwrap();
        assert_eq!(pos.side_to_move(), Color::Black);
        assert_eq!(pos.kings(Color::White).count(), 2);
        assert_eq!(pos.kings(Color::Black).count(), 3);
        assert_eq!(Position::from_fen(&pos.fen()).unwrap(), pos);
    }

    #[test]
    fn fen_variants() {
        let canonical = Position::from_fen("W:W31,32:B1,K2").unwrap();
        for fen in [
            "\"W:W31-32:B1,K2.\"",
            "W:B1,K2:W31,32",
            " W : W31-32 : B1,K2 ",
            "W:WK31-32:B1,2",
        ] {
            let pos = Position::from_fen(fen).unwrap();
            if fen.contains("WK31-32") {
                assert_eq!(pos.kings(Color::White).count(), 2);
            } else {
                assert_eq!(pos, canonical, "{fen}");
            }
        }
        for bad in ["", "X:W1", "W:W51", "W:W1:B1", "W:Z1", "W:W3"] {
            assert!(Position::from_fen(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hub_round_trip() {
        let pos = Position::initial();
        let hub = pos.hub();
        assert_eq!(hub.len(), 51);
        assert_eq!(&hub[..6], "Wbbbbb");
        assert_eq!(Position::from_hub(&hub).unwrap(), pos);
        assert!(Position::from_hub("W").is_err());
        assert!(Position::from_hub(&format!("{hub}e")).is_err());
    }

    #[test]
    fn incremental_hash_matches_full_hash() {
        let mut pos = Position::from_fen("W:W28,K40,45:B19,23,K32").unwrap();
        for _ in 0..6 {
            let Some(&mv) = pos.legal_moves().first() else {
                break;
            };
            pos.play(mv);
            assert_eq!(pos.hash(), pos.compute_hash(), "after {mv}");
        }
    }

    #[test]
    fn flipped_is_an_involution() {
        let pos = Position::from_fen("W:W28,K40,45:B19,23,K32").unwrap();
        let flipped = pos.flipped();
        assert_eq!(flipped.side_to_move(), Color::Black);
        assert_eq!(flipped.pieces(Color::Black).count(), 3);
        assert!(
            flipped
                .kings(Color::Black)
                .contains(Square::new(11).unwrap())
        );
        assert_eq!(flipped.flipped(), pos);
    }

    #[test]
    fn diagram() {
        let text = Position::initial().to_string();
        assert!(text.starts_with("  b   b   b   b   b\n"));
        assert!(text.ends_with("white to move"));
    }
}
