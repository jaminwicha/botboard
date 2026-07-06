//! Rung 0 of the search ladder (§8.2): perfect-information alpha-beta with
//! a transposition table over the full ground-truth Zobrist key (§7.5),
//! iterative deepening, TT-move + capture ordering, and a capture-only
//! quiescence. Repetition is judged on full state-key equality.

use crate::eval::Eval;
use crate::game::{GameDef, StalematePolicy};
use crate::movegen::{is_legal, pseudo_moves};
use crate::moves::{Move, MoveKind};
use crate::position::Position;
use crate::zobrist::Zobrist;

pub const MATE: i32 = 1_000_000;
const TT_BITS: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, Copy)]
struct TtEntry {
    key: u64,
    depth: i32,
    score: i32,
    bound: Bound,
    best: Option<Move>,
}

pub struct Searcher {
    pub zob: Zobrist,
    pub eval: Eval,
    tt: Vec<Option<TtEntry>>,
    /// Hashes of positions already seen in the actual game (repetition).
    pub history: Vec<u64>,
    path: Vec<u64>,
    pub nodes: u64,
    node_budget: u64,
}

pub struct SearchResult {
    pub best: Option<Move>,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
}

impl Searcher {
    pub fn new(g: &GameDef, eval: Eval) -> Self {
        Searcher {
            zob: Zobrist::new(g),
            eval,
            tt: vec![None; 1 << TT_BITS],
            history: Vec::new(),
            path: Vec::new(),
            nodes: 0,
            node_budget: u64::MAX,
        }
    }

    pub fn clear_tt(&mut self) {
        self.tt.iter_mut().for_each(|e| *e = None);
    }

    /// Iterative deepening to `max_depth` or until the node budget runs out.
    pub fn search(
        &mut self,
        g: &GameDef,
        pos: &mut Position,
        max_depth: i32,
        node_budget: u64,
    ) -> SearchResult {
        self.nodes = 0;
        self.node_budget = node_budget;
        self.path.clear();
        let mut result = SearchResult { best: None, score: 0, depth: 0, nodes: 0 };
        for d in 1..=max_depth {
            let score = self.negamax(g, pos, d, -MATE, MATE, 0);
            if self.nodes >= self.node_budget && d > 1 {
                break; // partial iteration: keep the previous depth's move
            }
            let h = self.zob.hash(g, pos);
            let best = self.tt[(h & ((1 << TT_BITS) - 1) as u64) as usize]
                .filter(|e| e.key == h)
                .and_then(|e| e.best);
            result = SearchResult { best, score, depth: d, nodes: self.nodes };
            if score.abs() >= MATE - 1000 {
                break;
            }
        }
        result
    }

    fn negamax(
        &mut self,
        g: &GameDef,
        pos: &mut Position,
        depth: i32,
        mut alpha: i32,
        beta: i32,
        ply: i32,
    ) -> i32 {
        self.nodes += 1;
        let key = self.zob.hash(g, pos);

        // Repetition on full ground-truth state (§7.5): a repeat anywhere in
        // the search path, or a position already twice in game history.
        if ply > 0 {
            let path_rep = self.path.iter().any(|&h| h == key);
            let hist_rep = self.history.iter().filter(|&&h| h == key).count() >= 2;
            if path_rep || hist_rep {
                return 0;
            }
        }

        let idx = (key & ((1 << TT_BITS) - 1) as u64) as usize;
        let mut tt_move = None;
        if let Some(e) = self.tt[idx] {
            if e.key == key {
                tt_move = e.best;
                if e.depth >= depth && ply > 0 {
                    match e.bound {
                        Bound::Exact => return e.score,
                        Bound::Lower if e.score >= beta => return e.score,
                        Bound::Upper if e.score <= alpha => return e.score,
                        _ => {}
                    }
                }
            }
        }

        if depth <= 0 {
            return self.qsearch(g, pos, alpha, beta, ply);
        }
        if self.nodes >= self.node_budget {
            return self.eval.stm(g, pos);
        }

        let mut moves = pseudo_moves(g, pos);
        self.order(g, pos, &mut moves, tt_move);

        let orig_alpha = alpha;
        let mut best_score = -MATE;
        let mut best_move = None;
        let mut any_legal = false;

        self.path.push(key);
        for mv in &moves {
            if !is_legal(g, pos, mv) {
                continue;
            }
            any_legal = true;
            let u = pos.make(g, mv);
            let s = -self.negamax(g, pos, depth - 1, -beta, -alpha, ply + 1);
            pos.unmake(g, &u);
            if s > best_score {
                best_score = s;
                best_move = Some(*mv);
            }
            if best_score > alpha {
                alpha = best_score;
            }
            if alpha >= beta {
                break;
            }
        }
        self.path.pop();

        if !any_legal {
            // Terminal: checkmate, or stalemate per the policy layer (§5).
            let in_check = pos.royal_attacked(g, pos.stm);
            return if in_check {
                -MATE + ply
            } else {
                match g.policy.stalemate {
                    StalematePolicy::Draw => 0,
                    StalematePolicy::Loss => -MATE + ply,
                }
            };
        }

        let bound = if best_score <= orig_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.tt[idx] =
            Some(TtEntry { key, depth, score: best_score, bound, best: best_move });
        best_score
    }

    /// Capture-only quiescence with stand-pat.
    fn qsearch(&mut self, g: &GameDef, pos: &mut Position, mut alpha: i32, beta: i32, ply: i32) -> i32 {
        self.nodes += 1;
        let stand = self.eval.stm(g, pos);
        if stand >= beta || ply > 64 {
            return stand;
        }
        if stand > alpha {
            alpha = stand;
        }
        let mut moves: Vec<Move> = pseudo_moves(g, pos)
            .into_iter()
            .filter(|m| m.kind != MoveKind::Drop && pos.piece_at(m.to).is_some())
            .collect();
        self.order(g, pos, &mut moves, None);
        for mv in &moves {
            if !is_legal(g, pos, mv) {
                continue;
            }
            let u = pos.make(g, mv);
            let s = -self.qsearch(g, pos, -beta, -alpha, ply + 1);
            pos.unmake(g, &u);
            if s >= beta {
                return s;
            }
            if s > alpha {
                alpha = s;
            }
        }
        alpha
    }

    /// TT move first, then captures by victim value (MVV), then the rest.
    fn order(&self, g: &GameDef, pos: &Position, moves: &mut [Move], tt_move: Option<Move>) {
        let score = |mv: &Move| -> i32 {
            if Some(*mv) == tt_move {
                return i32::MAX;
            }
            match pos.piece_at(mv.to) {
                Some(v) => 1000 + self.eval.material[v.t as usize],
                None => 0,
            }
        };
        moves.sort_by_key(|m| -score(m));
        let _ = g;
    }
}
