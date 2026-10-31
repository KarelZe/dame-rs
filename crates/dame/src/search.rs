//! Alpha-beta search.
//!
//! The searcher is a principal variation search (PVS) with iterative
//! deepening, aspiration windows, a transposition table, killer and history
//! move ordering and late move reductions. The quiescence search resolves
//! pending captures: as capturing is mandatory there is no stand-pat while
//! the side to move has a capture.
//!
//! Draws by repetition (two-fold inside the tree) and by the 25-move rule are
//! scored as zero. Endgame databases and opening books plug in through
//! [`EndgameDb`] and [`OpeningBook`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::book::OpeningBook;
use crate::egdb::{EndgameDb, Wdl};
use crate::eval::{Classical, Evaluate};
use crate::game::Game;
use crate::moves::{MAX_MOVES, Move, MoveList};
use crate::position::Position;
use crate::tt::{self, Bound, TranspositionTable};

/// The maximum search depth in plies, including extensions.
pub const MAX_PLY: usize = 128;

/// The score of a win at the root; a win in `n` plies scores `MATE - n`.
pub const MATE: i32 = 30_000;

/// The score of a database win at the root, below mate scores.
pub const DB_WIN: i32 = 20_000;

/// Evaluations must stay within `±MAX_EVAL`.
pub const MAX_EVAL: i32 = 10_000;

const INFINITY: i32 = MATE + 1;

/// When to stop searching. Unset limits do not constrain the search; with
/// no limit at all it runs until [`Limits::stop`] is raised.
#[derive(Clone, Debug, Default)]
pub struct Limits {
    /// Maximum depth in plies.
    pub depth: Option<u32>,
    /// Maximum number of nodes.
    pub nodes: Option<u64>,
    /// Fixed time for this move.
    pub move_time: Option<Duration>,
    /// Remaining time on the clock of the side to move.
    pub time: Option<Duration>,
    /// Increment per move.
    pub increment: Duration,
    /// Moves until the next time control, if any.
    pub moves_to_go: Option<u32>,
    /// Raised by another thread to stop the search early.
    pub stop: Option<Arc<AtomicBool>>,
}

impl Limits {
    /// Search to a fixed depth.
    pub fn depth(depth: u32) -> Self {
        Self {
            depth: Some(depth),
            ..Self::default()
        }
    }

    /// Search for a fixed time.
    pub fn move_time(time: Duration) -> Self {
        Self {
            move_time: Some(time),
            ..Self::default()
        }
    }

    /// Search a fixed number of nodes.
    pub fn nodes(nodes: u64) -> Self {
        Self {
            nodes: Some(nodes),
            ..Self::default()
        }
    }

    /// Soft and hard time budgets for this move: no new iteration starts
    /// once half the soft budget is used, and the search stops at the hard
    /// budget.
    pub fn time_budgets(&self) -> (Option<Duration>, Option<Duration>) {
        if let Some(time) = self.move_time {
            return (Some(time), Some(time));
        }
        let Some(time) = self.time else {
            return (None, None);
        };
        // Keep a safety margin for process and protocol overhead.
        let available = time.saturating_sub(Duration::from_millis(50));
        let moves = self.moves_to_go.unwrap_or(30).clamp(1, 30);
        let soft = (available / moves + self.increment * 3 / 4).min(available / 2);
        let hard = (soft * 4).min(available / 2).max(soft);
        (Some(soft), Some(hard))
    }
}

/// Progress after each completed iteration.
#[derive(Clone, Debug)]
pub struct Info {
    /// The completed depth in plies.
    pub depth: u32,
    /// The deepest ply reached, including quiescence.
    pub seldepth: u32,
    /// The score in centi-men from the side to move's point of view.
    pub score: i32,
    /// Nodes searched so far.
    pub nodes: u64,
    /// Time spent so far.
    pub elapsed: Duration,
    /// The principal variation.
    pub pv: Vec<Move>,
}

/// The outcome of a search.
#[derive(Clone, Debug)]
pub struct SearchResult {
    /// The best move, or `None` if the game is already over.
    pub best_move: Option<Move>,
    /// The expected reply, useful for pondering.
    pub ponder: Option<Move>,
    /// The score in centi-men from the side to move's point of view.
    pub score: i32,
    /// The last completed depth; zero for book moves.
    pub depth: u32,
    /// The total number of nodes searched.
    pub nodes: u64,
    /// The time spent.
    pub elapsed: Duration,
    /// The principal variation, starting with the best move.
    pub pv: Vec<Move>,
    /// Whether the move came from the opening book.
    pub from_book: bool,
}

/// Whether `score` is a forced win or loss (by search or database).
pub const fn is_decisive(score: i32) -> bool {
    score.abs() > MAX_EVAL
}

/// Moves to mate for a mate score: positive if the side to move wins.
pub const fn mate_in(score: i32) -> Option<i32> {
    if score > MATE - MAX_PLY as i32 {
        Some((MATE - score + 1) / 2)
    } else if score < -MATE + MAX_PLY as i32 {
        Some(-(MATE + score) / 2)
    } else {
        None
    }
}

/// Converts a score relative to the root into one relative to the node at
/// `ply`, so that stored win distances stay correct when retrieved elsewhere.
fn to_tt(score: i32, ply: usize) -> i32 {
    match score {
        s if s > MAX_EVAL => s + ply as i32,
        s if s < -MAX_EVAL => s - ply as i32,
        s => s,
    }
}

fn from_tt(score: i32, ply: usize) -> i32 {
    match score {
        s if s > MAX_EVAL => s - ply as i32,
        s if s < -MAX_EVAL => s + ply as i32,
        s => s,
    }
}

/// Positions on the current path, for repetition and 25-move detection.
#[derive(Default)]
struct PathEntry {
    key: u64,
    /// Plies since the last man move or capture.
    reversible: u32,
}

/// A reusable search engine. Keeping it between moves preserves the
/// transposition table and move-ordering statistics.
pub struct Searcher<E: Evaluate = Classical> {
    eval: E,
    tt: TranspositionTable,
    egdb: Option<Arc<dyn EndgameDb>>,
    book: Option<Arc<OpeningBook>>,
    history: Vec<i32>,
    killers: [[Option<Move>; 2]; MAX_PLY],
    pv: Vec<Vec<Move>>,
    path: Vec<PathEntry>,
    // Per-search state.
    nodes: u64,
    seldepth: usize,
    start: Instant,
    hard_deadline: Option<Instant>,
    node_limit: u64,
    stop: Option<Arc<AtomicBool>>,
    stopped: bool,
}

impl Default for Searcher<Classical> {
    fn default() -> Self {
        Self::new(Classical::default(), 64)
    }
}

impl<E: Evaluate> std::fmt::Debug for Searcher<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Searcher")
            .field("egdb", &self.egdb.as_ref().map(|db| db.max_pieces()))
            .field("book", &self.book.as_ref().map(|book| book.len()))
            .finish_non_exhaustive()
    }
}

impl<E: Evaluate> Searcher<E> {
    /// A searcher with evaluation `eval` and a `hash_mb` MiB transposition
    /// table.
    pub fn new(eval: E, hash_mb: usize) -> Self {
        Self {
            eval,
            tt: TranspositionTable::new(hash_mb),
            egdb: None,
            book: None,
            history: vec![0; 64 * 64],
            killers: [[None; 2]; MAX_PLY],
            pv: (0..=MAX_PLY).map(|_| Vec::with_capacity(MAX_PLY)).collect(),
            path: Vec::with_capacity(1024),
            nodes: 0,
            seldepth: 0,
            start: Instant::now(),
            hard_deadline: None,
            node_limit: u64::MAX,
            stop: None,
            stopped: false,
        }
    }

    /// Uses `egdb` for positions it covers.
    pub fn set_endgame_db(&mut self, egdb: Option<Arc<dyn EndgameDb>>) {
        self.egdb = egdb;
    }

    /// Plays book moves while the book has them.
    pub fn set_book(&mut self, book: Option<Arc<OpeningBook>>) {
        self.book = book;
    }

    /// Resizes the transposition table, clearing it.
    pub fn set_hash_size(&mut self, hash_mb: usize) {
        self.tt = TranspositionTable::new(hash_mb);
    }

    /// Forgets everything learned in previous searches, e.g. for a new game.
    pub fn clear(&mut self) {
        self.tt.clear();
        self.history.fill(0);
        self.killers = [[None; 2]; MAX_PLY];
    }

    /// Searches the current position of `game`, calling `on_info` after each
    /// completed iteration.
    ///
    /// The game's history is used to detect repetitions. If the game is
    /// over, the result has no best move.
    pub fn search(
        &mut self,
        game: &Game,
        limits: &Limits,
        mut on_info: impl FnMut(&Info),
    ) -> SearchResult {
        self.start = Instant::now();
        let root = *game.position();
        let mut result = SearchResult {
            best_move: None,
            ponder: None,
            score: 0,
            depth: 0,
            nodes: 0,
            elapsed: Duration::ZERO,
            pv: Vec::new(),
            from_book: false,
        };
        let root_moves = game.legal_moves();
        if root_moves.is_empty() {
            return result;
        }
        result.best_move = Some(root_moves[0]);
        result.pv = vec![root_moves[0]];

        if let Some(mv) = self
            .book
            .as_ref()
            .and_then(|book| book.pick(&root, root.hash() ^ time_seed()))
        {
            result.best_move = Some(mv);
            result.pv = vec![mv];
            result.from_book = true;
            return result;
        }

        self.prepare(game, limits);
        let (soft, _) = limits.time_budgets();
        let max_depth = limits.depth.map_or(MAX_PLY as i32 - 1, |d| {
            d.clamp(1, MAX_PLY as u32 - 1) as i32
        });

        let mut score = 0;
        for depth in 1..=max_depth {
            let iteration = self.aspiration(&root, depth, score);
            if self.stopped && depth > 1 {
                // A partial iteration only counts if it found a new best move
                // that was searched completely.
                if let Some(&mv) = self.pv[0].first()
                    && Some(mv) != result.best_move
                {
                    result.best_move = Some(mv);
                    result.pv.clone_from(&self.pv[0]);
                }
                break;
            }
            score = iteration;
            result.score = score;
            result.depth = depth as u32;
            if !self.pv[0].is_empty() {
                result.pv.clone_from(&self.pv[0]);
                result.best_move = result.pv.first().copied();
            }
            on_info(&Info {
                depth: depth as u32,
                seldepth: self.seldepth as u32,
                score,
                nodes: self.nodes,
                elapsed: self.start.elapsed(),
                pv: result.pv.clone(),
            });
            if self.stopped
                || mate_in(score).is_some_and(|n| n.unsigned_abs() as i32 * 2 <= depth)
                || soft.is_some_and(|soft| self.start.elapsed() > soft / 2)
                || (root_moves.len() == 1 && limits.time.is_some())
            {
                break;
            }
        }

        result.ponder = result.pv.get(1).copied();
        result.nodes = self.nodes;
        result.elapsed = self.start.elapsed();
        result
    }

    fn prepare(&mut self, game: &Game, limits: &Limits) {
        self.tt.new_search();
        for h in &mut self.history {
            *h /= 8;
        }
        self.nodes = 0;
        self.seldepth = 0;
        self.stopped = false;
        self.stop.clone_from(&limits.stop);
        self.node_limit = limits.nodes.unwrap_or(u64::MAX);
        self.hard_deadline = limits.time_budgets().1.map(|hard| self.start + hard);

        // Rebuild the repetition path from the game since the last
        // irreversible move.
        self.path.clear();
        let positions = game.positions();
        let window = game.reversible_plies() as usize;
        let first = positions.len() - 1 - window;
        for (i, pos) in positions[first..].iter().enumerate() {
            self.path.push(PathEntry {
                key: pos.hash(),
                reversible: i as u32,
            });
        }
    }

    /// Searches the root with a window around the previous score, widening
    /// it on failure.
    fn aspiration(&mut self, root: &Position, depth: i32, previous: i32) -> i32 {
        if depth < 4 || is_decisive(previous) {
            return self.negamax(root, depth, -INFINITY, INFINITY, 0, true);
        }
        let mut delta = 25;
        let (mut alpha, mut beta) = (previous - delta, previous + delta);
        loop {
            let score = self.negamax(root, depth, alpha, beta, 0, true);
            if self.stopped {
                return score;
            }
            if score <= alpha {
                alpha = (score - delta).max(-INFINITY);
            } else if score >= beta {
                beta = (score + delta).min(INFINITY);
            } else {
                return score;
            }
            delta *= 2;
            if delta > 400 {
                alpha = -INFINITY;
                beta = INFINITY;
            }
        }
    }

    /// Polls the stop conditions every 1024 nodes.
    #[inline]
    fn check_stop(&mut self) -> bool {
        if self.nodes.trailing_zeros() >= 10 && !self.stopped {
            self.stopped = self.nodes >= self.node_limit
                || self
                    .hard_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                || self
                    .stop
                    .as_ref()
                    .is_some_and(|stop| stop.load(Ordering::Relaxed));
        }
        self.stopped
    }

    /// Whether the position at the top of the path is drawn by repetition
    /// (two-fold) or by the 25-move rule.
    fn is_draw(&self) -> bool {
        let n = self.path.len();
        let current = &self.path[n - 1];
        if current.reversible >= 50 {
            return true;
        }
        let window = current.reversible as usize;
        (4..=window)
            .step_by(2)
            .any(|back| self.path[n - 1 - back].key == current.key)
    }

    fn push_path(&mut self, pos: &Position, mv: Move, child: &Position) {
        let reversible = if mv.is_capture() || pos.men(pos.side_to_move()).contains(mv.from()) {
            0
        } else {
            self.path.last().map_or(0, |entry| entry.reversible + 1)
        };
        self.path.push(PathEntry {
            key: child.hash(),
            reversible,
        });
    }

    #[allow(clippy::too_many_lines)]
    fn negamax(
        &mut self,
        pos: &Position,
        mut depth: i32,
        mut alpha: i32,
        mut beta: i32,
        ply: usize,
        pv_node: bool,
    ) -> i32 {
        self.pv[ply].clear();
        self.nodes += 1;
        if self.check_stop() {
            return 0;
        }
        let root = ply == 0;
        if !root {
            if self.is_draw() {
                return 0;
            }
            // Mate distance pruning: no line from here beats a shorter mate.
            alpha = alpha.max(-MATE + ply as i32);
            beta = beta.min(MATE - ply as i32 - 1);
            if alpha >= beta {
                return alpha;
            }
        }
        if ply >= MAX_PLY - 1 {
            return self.eval.evaluate(pos);
        }

        let mut moves = pos.legal_moves();
        if moves.is_empty() {
            return -MATE + ply as i32;
        }
        // A forced move costs no depth.
        if moves.len() == 1 && !root {
            depth += 1;
        }
        if depth <= 0 {
            return self.quiesce(pos, &moves, alpha, beta, ply);
        }

        if !root && let Some(score) = self.probe_egdb(pos, &moves, ply) {
            return score;
        }

        let key = pos.hash();
        let mut tt_move = 0;
        if let Some(hit) = self.tt.probe(key) {
            tt_move = hit.mv;
            let score = from_tt(hit.score, ply);
            if !pv_node && hit.depth >= depth {
                let cutoff = match hit.bound {
                    Bound::Exact => true,
                    Bound::Lower => score >= beta,
                    Bound::Upper => score <= alpha,
                };
                if cutoff {
                    return score;
                }
            }
        }

        let mut scores = [0; MAX_MOVES];
        self.score_moves(&moves, &mut scores, tt_move, ply);

        let original_alpha = alpha;
        let mut best_score = -INFINITY;
        let mut best_move = None;
        for i in 0..moves.len() {
            let mv = pick_next(moves.as_mut_slice(), &mut scores, i);
            let child = pos.after(mv);
            self.push_path(pos, mv, &child);

            let score = if i == 0 {
                -self.negamax(&child, depth - 1, -beta, -alpha, ply + 1, pv_node)
            } else {
                // Late quiet moves are searched one ply shallower first.
                let reduction = i32::from(depth >= 3 && i >= 3 && !mv.is_capture() && !pv_node);
                let mut score = -self.negamax(
                    &child,
                    depth - 1 - reduction,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    false,
                );
                if score > alpha && reduction > 0 {
                    score = -self.negamax(&child, depth - 1, -alpha - 1, -alpha, ply + 1, false);
                }
                if score > alpha && score < beta {
                    score = -self.negamax(&child, depth - 1, -beta, -alpha, ply + 1, true);
                }
                score
            };
            self.path.pop();
            if self.stopped {
                return 0;
            }

            if score > best_score {
                best_score = score;
                best_move = Some(mv);
                if score > alpha {
                    alpha = score;
                    let (head, tail) = self.pv.split_at_mut(ply + 1);
                    head[ply].clear();
                    head[ply].push(mv);
                    head[ply].extend_from_slice(&tail[0]);
                    if score >= beta {
                        if !mv.is_capture() {
                            self.reward(mv, depth, ply);
                        }
                        break;
                    }
                }
            }
        }

        let bound = if best_score >= beta {
            Bound::Lower
        } else if best_score > original_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        let mv = best_move.map_or(0, tt::compact);
        self.tt.store(key, mv, to_tt(best_score, ply), depth, bound);
        best_score
    }

    /// Resolves captures. While the side to move must capture, every capture
    /// is searched; otherwise the static evaluation stands.
    fn quiesce(
        &mut self,
        pos: &Position,
        moves: &MoveList,
        mut alpha: i32,
        beta: i32,
        ply: usize,
    ) -> i32 {
        self.seldepth = self.seldepth.max(ply);
        if ply >= MAX_PLY - 1 {
            return self.eval.evaluate(pos);
        }
        if !moves[0].is_capture() {
            return self.eval.evaluate(pos);
        }
        let mut best = -INFINITY;
        for &mv in moves {
            let child = pos.after(mv);
            self.nodes += 1;
            if self.check_stop() {
                return 0;
            }
            let replies = child.legal_moves();
            let score = if replies.is_empty() {
                MATE - ply as i32 - 1
            } else {
                -self.quiesce(&child, &replies, -beta, -alpha, ply + 1)
            };
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    if score >= beta {
                        break;
                    }
                }
            }
        }
        best
    }

    fn probe_egdb(&self, pos: &Position, moves: &MoveList, ply: usize) -> Option<i32> {
        let egdb = self.egdb.as_ref()?;
        if pos.occupied().count() > egdb.max_pieces() || moves[0].is_capture() {
            return None;
        }
        let win = DB_WIN - ply as i32;
        egdb.probe(pos).map(|wdl| match wdl {
            Wdl::Win => win,
            Wdl::Draw => 0,
            Wdl::Loss => -win,
        })
    }

    /// Orders moves: the table move, then captures by size, then killers,
    /// then by history.
    fn score_moves(
        &self,
        moves: &MoveList,
        scores: &mut [i32; MAX_MOVES],
        tt_move: u16,
        ply: usize,
    ) {
        for (mv, score) in moves.iter().zip(scores.iter_mut()) {
            *score = if tt_move != 0 && tt::compact(*mv) == tt_move {
                1 << 30
            } else if mv.is_capture() {
                (1 << 28) + mv.captured().count() as i32
            } else if self.killers[ply][0] == Some(*mv) {
                (1 << 27) + 1
            } else if self.killers[ply][1] == Some(*mv) {
                1 << 27
            } else {
                self.history[history_index(*mv)]
            };
        }
    }

    fn reward(&mut self, mv: Move, depth: i32, ply: usize) {
        let entry = &mut self.history[history_index(mv)];
        *entry = (*entry + depth * depth).min(1 << 26);
        if self.killers[ply][0] != Some(mv) {
            self.killers[ply][1] = self.killers[ply][0];
            self.killers[ply][0] = Some(mv);
        }
    }
}

/// A seed that varies between searches, for book move selection.
fn time_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

#[inline]
fn history_index(mv: Move) -> usize {
    usize::from(mv.from().bit()) * 64 + usize::from(mv.to().bit())
}

/// Moves the highest-scored remaining move to position `i` and returns it.
#[inline]
fn pick_next(moves: &mut [Move], scores: &mut [i32; MAX_MOVES], i: usize) -> Move {
    let best = (i..moves.len()).max_by_key(|&j| scores[j]).unwrap_or(i);
    moves.swap(i, best);
    scores.swap(i, best);
    moves[i]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(fen: &str, depth: u32) -> SearchResult {
        let game = Game::from_fen(fen).unwrap();
        Searcher::default().search(&game, &Limits::depth(depth), |_| {})
    }

    #[test]
    fn finished_game_has_no_move() {
        let result = best("W:W46:B41,37", 4);
        assert!(result.best_move.is_none());
    }

    #[test]
    fn forced_capture_is_played() {
        let result = best("W:W28,32:B17,23,27", 4);
        assert_eq!(result.best_move.unwrap().to_hub(), "32x12x17x27");
    }

    #[test]
    fn captures_the_last_piece() {
        // The king takes the last black man from a distance: mate in one.
        let result = best("W:WK46:B10", 6);
        assert_eq!(result.best_move.unwrap().to_hub(), "46x5x10");
        assert_eq!(mate_in(result.score), Some(1));
        assert_eq!(result.pv.first(), result.best_move.as_ref());
    }

    #[test]
    fn avoids_losing_material() {
        // White is a man up. 27-22 would let Black take two men with
        // 17x39 (over 22 and 33), so the search must keep the advantage.
        let result = best("W:W27,33,40:B12,17", 6);
        assert_ne!(result.best_move.unwrap().to_string(), "27-22");
        assert!(result.score > 0, "{result:?}");
    }

    #[test]
    fn mate_scores() {
        assert_eq!(mate_in(MATE - 1), Some(1));
        assert_eq!(mate_in(MATE - 3), Some(2));
        assert_eq!(mate_in(-MATE + 2), Some(-1));
        assert_eq!(mate_in(150), None);
        assert_eq!(from_tt(to_tt(MATE - 7, 3), 3), MATE - 7);
    }

    #[test]
    fn respects_node_limit() {
        let game = Game::new();
        let result = Searcher::default().search(&game, &Limits::nodes(10_000), |_| {});
        assert!(result.best_move.is_some());
        assert!(result.nodes < 20_000, "{}", result.nodes);
    }

    #[test]
    fn stops_when_asked() {
        let stop = Arc::new(AtomicBool::new(true));
        let limits = Limits {
            stop: Some(stop),
            ..Limits::default()
        };
        let result = Searcher::default().search(&Game::new(), &limits, |_| {});
        assert!(result.best_move.is_some());
    }

    #[test]
    fn reports_each_iteration() {
        let mut depths = Vec::new();
        Searcher::default().search(&Game::new(), &Limits::depth(5), |info| {
            depths.push(info.depth);
        });
        assert_eq!(depths, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn book_moves_are_played_first() {
        let book = OpeningBook::from_pdn("1. 32-28 *", OpeningBook::DEFAULT_PLIES).unwrap();
        let mut searcher = Searcher::default();
        searcher.set_book(Some(Arc::new(book)));
        let result = searcher.search(&Game::new(), &Limits::depth(3), |_| {});
        assert!(result.from_book);
        assert_eq!(result.best_move.unwrap().to_string(), "32-28");
    }
}
