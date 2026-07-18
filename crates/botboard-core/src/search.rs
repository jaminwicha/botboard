//! Rung 0 of the search ladder (§8.2): perfect-information alpha-beta over
//! the incrementally-maintained full ground-truth key (§7.5), with the
//! classical toolkit the spec adopts (§7.5, §8.2): transposition table,
//! iterative deepening with aspiration windows, TT-move/MVV/killer/history
//! ordering, null-move pruning, late-move reductions, and a capture-only
//! quiescence. Repetition is judged on full state-key equality.

use crate::eval::Eval;
use crate::game::{GameDef, StalematePolicy};
use crate::move_defs::script;
use crate::movegen::{is_legal, pseudo_moves};
use crate::moves::Move;
use crate::position::{Loc, Position};

pub const MATE: i32 = 1_000_000;
const TT_BITS: usize = 20;
const MAX_PLY: usize = 128;

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
    pub eval: Eval,
    tt: Vec<Option<TtEntry>>,
    /// Hashes of positions already seen in the actual game (repetition).
    pub history: Vec<u64>,
    path: Vec<u64>,
    killers: Vec<[Option<Move>; 2]>,
    /// History heuristic: [from][to] accumulated cutoff credit.
    hist: Vec<u32>,
    ncells: usize,
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
        let ncells = g.board.ncells();
        Searcher {
            eval,
            tt: vec![None; 1 << TT_BITS],
            history: Vec::new(),
            path: Vec::new(),
            killers: vec![[None; 2]; MAX_PLY],
            hist: vec![0; ncells * ncells],
            ncells,
            nodes: 0,
            node_budget: u64::MAX,
        }
    }

    pub fn clear_tt(&mut self) {
        self.tt.iter_mut().for_each(|e| *e = None);
        self.hist.iter_mut().for_each(|h| *h = 0);
    }

    /// Iterative deepening with aspiration windows.
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
        let mut prev_score = 0;
        for d in 1..=max_depth {
            let (mut alpha, mut beta) = if d >= 3 {
                (prev_score - 60, prev_score + 60)
            } else {
                (-MATE, MATE)
            };
            let mut score;
            loop {
                score = self.negamax(g, pos, d, alpha, beta, 0);
                if score <= alpha && alpha > -MATE {
                    alpha = -MATE; // fail low: widen down
                } else if score >= beta && beta < MATE {
                    beta = MATE; // fail high: widen up
                } else {
                    break;
                }
                if self.nodes >= self.node_budget {
                    break;
                }
            }
            if self.nodes >= self.node_budget && d > 1 {
                break; // partial iteration: keep the previous depth's move
            }
            let h = pos.hash;
            let best = self.tt[(h & ((1 << TT_BITS) - 1) as u64) as usize]
                .filter(|e| e.key == h)
                .and_then(|e| e.best);
            result = SearchResult { best, score, depth: d, nodes: self.nodes };
            prev_score = score;
            if score.abs() >= MATE - 1000 {
                break;
            }
        }
        result
    }

    /// Any non-royal material for the side to move (null-move guard).
    fn has_material(&self, g: &GameDef, pos: &Position) -> bool {
        pos.pieces.iter().any(|p| {
            p.side == pos.stm
                && !g.types[p.t as usize].royal
                && matches!(p.loc, Loc::Board(_))
        })
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
        let key = pos.hash;

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

        let in_check = pos.royal_attacked(g, pos.stm);

        // Null-move pruning: skip a turn; if the reduced search still fails
        // high, the position is good enough to cut. Guarded against check
        // and bare-royal zugzwang; two-player only (N>2 rotation differs).
        if depth >= 3
            && !in_check
            && ply > 0
            && g.sides == 2
            && beta < MATE - 1000
            && self.has_material(g, pos)
        {
            let prior = pos.make_null(g);
            self.path.push(key);
            let r = 2 + depth / 4;
            let s = -self.negamax(g, pos, depth - 1 - r, -beta, -beta + 1, ply + 1);
            self.path.pop();
            pos.unmake_null(g, prior);
            if s >= beta {
                return beta;
            }
        }

        let mut moves = pseudo_moves(g, pos);
        self.order(g, pos, &mut moves, tt_move, ply);

        let orig_alpha = alpha;
        let mut best_score = -MATE;
        let mut best_move = None;
        let mut any_legal = false;
        let mut searched = 0;

        self.path.push(key);
        for mv in &moves {
            if !is_legal(g, pos, mv) {
                continue;
            }
            any_legal = true;
            // Quietness is a derived property of the generating script
            // row (empty landing, not from a hand, no aux-victim class).
            let quiet = script(mv.kind).is_quiet(pos, mv);
            let u = pos.make(g, mv);
            let mut s;
            // Late-move reduction: late quiet moves get a reduced search,
            // re-searched at full depth only if they beat alpha.
            let reduce = depth >= 3 && searched >= 4 && quiet && !in_check;
            if reduce {
                s = -self.negamax(g, pos, depth - 2, -alpha - 1, -alpha, ply + 1);
                if s > alpha {
                    s = -self.negamax(g, pos, depth - 1, -beta, -alpha, ply + 1);
                }
            } else {
                s = -self.negamax(g, pos, depth - 1, -beta, -alpha, ply + 1);
            }
            pos.unmake(g, &u);
            searched += 1;
            if s > best_score {
                best_score = s;
                best_move = Some(*mv);
            }
            if best_score > alpha {
                alpha = best_score;
            }
            if alpha >= beta {
                // Killer + history credit for quiet cutoffs.
                if quiet && (ply as usize) < MAX_PLY {
                    let ks = &mut self.killers[ply as usize];
                    if ks[0] != Some(*mv) {
                        ks[1] = ks[0];
                        ks[0] = Some(*mv);
                    }
                    if mv.from != crate::moves::NO_SQ {
                        let h = &mut self.hist
                            [mv.from as usize * self.ncells + mv.to as usize];
                        *h = h.saturating_add((depth * depth) as u32);
                    }
                }
                break;
            }
        }
        self.path.pop();

        if !any_legal {
            // Terminal: checkmate, or stalemate per the policy layer (§5).
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
        if stand >= beta || ply > 64 || self.nodes >= self.node_budget {
            return stand;
        }
        if stand > alpha {
            alpha = stand;
        }
        // Enemy-occupied targets only: every recursion strictly reduces the
        // opponent's total HP (armor strike or kill), so the line is bounded.
        // Friendly-target abilities (Heal) would undo that progress and make
        // damage/heal cycles explode toward the ply cap — they are position
        // maintenance, not tactics, and stand-pat covers them.
        // "Counts as a capture" is the generating script row's derived
        // property: the aux-victim capture class (locust — enemy HP still
        // strictly drops) or an enemy on the destination.
        let mut moves: Vec<Move> = pseudo_moves(g, pos)
            .into_iter()
            .filter(|m| script(m.kind).counts_as_capture(pos, m))
            .collect();
        self.order(g, pos, &mut moves, None, ply);
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

    /// TT move, then captures by victim value (MVV), then killers, then
    /// history credit.
    fn order(&self, g: &GameDef, pos: &Position, moves: &mut [Move], tt_move: Option<Move>, ply: i32) {
        let ks = if (ply as usize) < MAX_PLY {
            self.killers[ply as usize]
        } else {
            [None; 2]
        };
        let score = |mv: &Move| -> i64 {
            if Some(*mv) == tt_move {
                return i64::MAX;
            }
            if let Some(v) = pos.piece_at(mv.to) {
                return 1_000_000_000 + self.eval.material[v.t as usize] as i64;
            }
            // Aux-victim capture class (locust): MVV credit via `aux`.
            if script(mv.kind).capture_class {
                if let Some(v) = pos.piece_at(mv.aux) {
                    return 1_000_000_000 + self.eval.material[v.t as usize] as i64;
                }
            }
            if Some(*mv) == ks[0] {
                return 900_000_000;
            }
            if Some(*mv) == ks[1] {
                return 800_000_000;
            }
            if mv.from != crate::moves::NO_SQ {
                return self.hist[mv.from as usize * self.ncells + mv.to as usize] as i64;
            }
            0
        };
        moves.sort_by_key(|m| -score(m));
        let _ = g;
    }
}
