//! The belief substrate (§8.6) and the belief-sharpness dial (§8.1).
//!
//! Ground truth is strictly separated from each agent's observed view. In
//! the Phase-3 hidden-identity regime (Stratego-like, the spec's structural
//! neighbor): piece *locations* are public, piece *rules* are hidden until
//! observed in play. Each hidden enemy piece carries a knowledge mask — here
//! the set of still-consistent type hypotheses, filtered by every observed
//! action (one action per turn keeps each update well-posed, §3.4).
//! Belief sharpness is the entropy of those hypothesis sets; determinization
//! samples full worlds consistent with belief + army-pool counts.

use crate::game::{GameDef, Side, TypeId};
use crate::moves::Move;
use crate::position::{Loc, Position};
use crate::rng::Rng;

#[derive(Clone, Debug)]
pub struct Belief {
    /// The observing side.
    pub observer: Side,
    /// candidates[piece_index] = still-consistent types for that enemy
    /// piece; empty vec ⇒ not tracked (own piece or revealed at setup).
    pub candidates: Vec<Vec<TypeId>>,
}

impl Belief {
    /// Cold open: every enemy non-royal piece could be any type in the
    /// enemy army's multiset (royals are public — the check rule needs it).
    pub fn cold_open(g: &GameDef, pos: &Position, observer: Side) -> Self {
        let enemy_types: Vec<TypeId> = {
            let mut v: Vec<TypeId> = pos
                .pieces
                .iter()
                .filter(|p| p.side != observer && !g.types[p.t as usize].royal)
                .map(|p| p.t)
                .collect();
            v.sort();
            v.dedup();
            v
        };
        let candidates = pos
            .pieces
            .iter()
            .map(|p| {
                if p.side != observer && !g.types[p.t as usize].royal {
                    enemy_types.clone()
                } else {
                    Vec::new()
                }
            })
            .collect();
        Belief { observer, candidates }
    }

    /// Fully revealed (point mass, §8.1): the perfect-information limit.
    pub fn revealed(pos: &Position, observer: Side) -> Self {
        Belief { observer, candidates: vec![Vec::new(); pos.pieces.len()] }
    }

    /// Belief sharpness (§8.1): Σ log2 |hypotheses|. 0 ⇒ point mass.
    pub fn entropy(&self) -> f64 {
        self.candidates
            .iter()
            .filter(|c| c.len() > 1)
            .map(|c| (c.len() as f64).log2())
            .sum()
    }

    /// Is any *pivotal* uncertainty left (§8.5)? Miniature criterion: a
    /// hidden piece whose hypotheses disagree by more than a factor-2 value
    /// spread could swing the game; the gate escalates on it.
    pub fn pivotal(&self, material: &[i32]) -> bool {
        self.candidates.iter().any(|c| {
            c.len() >= 2 && {
                let vals: Vec<i32> = c.iter().map(|&t| material[t as usize]).collect();
                let (lo, hi) = (*vals.iter().min().unwrap(), *vals.iter().max().unwrap());
                hi > 2 * lo.max(1)
            }
        })
    }

    /// Belief update from one observed enemy action (§8.6): filter the
    /// mover's hypotheses to types whose kernels could produce the move in
    /// the pre-move position. Captures/drops/specials reveal their whole
    /// script as one observation (§3.4).
    pub fn observe(&mut self, g: &GameDef, pre: &Position, mv: &Move) {
        // A hand-sourced script (drop) reveals the type directly;
        // nothing left to filter.
        if matches!(
            crate::move_defs::script(mv.kind).source,
            crate::move_defs::Source::Hand
        ) {
            return;
        }
        let Some(idx) = pre.board.get(mv.from as usize).copied() else { return };
        if idx < 0 {
            return;
        }
        let idx = idx as usize;
        if self.candidates[idx].len() <= 1 {
            return;
        }
        let mover = pre.pieces[idx];
        let consistent: Vec<TypeId> = self.candidates[idx]
            .iter()
            .copied()
            .filter(|&t| could_generate(g, pre, mover.side, t, mv))
            .collect();
        if !consistent.is_empty() {
            self.candidates[idx] = consistent;
        }
    }
}

/// Would a piece of type `t` at `mv.from` generate a pseudo-move to `mv.to`
/// in this position? (The hypothesis-consistency kernel.)
fn could_generate(g: &GameDef, pos: &Position, side: Side, t: TypeId, mv: &Move) -> bool {
    // Build a probe position identical to `pos` but with the mover's type
    // swapped to the hypothesis.
    let mut list: Vec<(TypeId, Side, u16, bool)> = Vec::new();
    for p in &pos.pieces {
        if let Loc::Board(sq) = p.loc {
            let ty = if sq == mv.from { t } else { p.t };
            list.push((ty, p.side, sq, p.moved));
        }
    }
    let probe = Position::from_pieces(g, &list, side);
    crate::movegen::pseudo_moves(g, &probe)
        .iter()
        .any(|m| m.from == mv.from && m.to == mv.to && m.kind == mv.kind)
}

/// Determinization (§8.2 rung 1): sample a full world consistent with the
/// belief and with the enemy army's type multiset. Rejection-samples up to
/// `tries`; falls back to ground truth only if sampling fails (degenerate
/// beliefs), which keeps self-play from leaking by construction elsewhere.
pub fn determinize(
    _g: &GameDef,
    truth: &Position,
    belief: &Belief,
    rng: &mut Rng,
    tries: usize,
) -> Position {
    // Pool: the true multiset of hidden enemy types (public army list).
    let hidden: Vec<usize> = (0..truth.pieces.len())
        .filter(|&i| belief.candidates[i].len() > 1)
        .collect();
    if hidden.is_empty() {
        return truth.clone();
    }
    let mut pool: Vec<TypeId> = hidden.iter().map(|&i| truth.pieces[i].t).collect();

    'attempt: for _ in 0..tries {
        let mut order = hidden.clone();
        rng.shuffle(&mut order);
        let mut remaining = pool.clone();
        let mut assign: Vec<(usize, TypeId)> = Vec::with_capacity(order.len());
        for &i in &order {
            let opts: Vec<TypeId> = belief.candidates[i]
                .iter()
                .copied()
                .filter(|t| remaining.contains(t))
                .collect();
            if opts.is_empty() {
                continue 'attempt;
            }
            let t = opts[rng.below(opts.len())];
            let slot = remaining.iter().position(|&x| x == t).unwrap();
            remaining.swap_remove(slot);
            assign.push((i, t));
        }
        let mut world = truth.clone();
        for (i, t) in assign {
            world.pieces[i].t = t;
            world.pieces[i].base = t;
        }
        world.rehash(_g);
        return world;
    }
    pool.clear();
    truth.clone()
}
