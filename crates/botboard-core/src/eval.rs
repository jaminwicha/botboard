//! Deterministic-grade evaluation (§10.6): integer-only material + mobility.
//!
//! This is the Phase-2 seed of the shared evaluator: material values come
//! from the cost model (prior, then self-play-corrected), so evaluation
//! already generalizes to procedural pieces — any Bit-set gets a value via
//! its cost. The NNUE-with-Bit-embeddings network (§7.4) replaces this
//! linear form later behind the same interface; per-player scores are the
//! per-player value head in miniature.

use crate::game::{GameDef, Side};
use crate::movegen::pseudo_moves;
use crate::position::{Loc, Position};

pub const MOBILITY_CP: i32 = 4;

#[derive(Clone, Debug)]
pub struct Eval {
    /// Centipawn value per piece type (royals 0 — losing them ends the game).
    /// Kept even with a net: move ordering (MVV) and pivotality read it.
    pub material: Vec<i32>,
    pub mobility_cp: i32,
    /// Deterministic-grade network (§10.6): when present, replaces the
    /// linear material+mobility form on every value query.
    pub net: Option<crate::nnue::QuantNet>,
}

impl Eval {
    pub fn new(material: Vec<i32>) -> Self {
        Eval { material, mobility_cp: MOBILITY_CP, net: None }
    }

    pub fn with_net(material: Vec<i32>, net: crate::nnue::QuantNet) -> Self {
        Eval { material, mobility_cp: 0, net: Some(net) }
    }

    /// Per-player score vector (the value-head shape, §7.4). Armored pieces
    /// count HP-proportionally so strikes register as progress.
    pub fn players(&self, g: &GameDef, pos: &mut Position) -> Vec<i32> {
        let mut score = vec![0i32; g.sides as usize];
        for (i, p) in pos.pieces.iter().enumerate() {
            match p.loc {
                Loc::Board(_) => {
                    let max = g.types[p.t as usize].max_hp.max(1) as i32;
                    let v = self.material[p.t as usize];
                    score[p.side as usize] += v * pos.hp[i].max(0) as i32 / max;
                }
                Loc::Hand(s) => score[s as usize] += self.material[p.t as usize],
                Loc::Dead => {}
            }
        }
        if self.mobility_cp != 0 {
            let saved = pos.stm;
            for s in 0..g.sides {
                pos.set_stm(g, s);
                score[s as usize] += self.mobility_cp * pseudo_moves(g, pos).len() as i32;
            }
            pos.set_stm(g, saved);
        }
        score
    }

    /// Two-player scalar from the side-to-move's perspective (negamax form).
    pub fn stm(&self, g: &GameDef, pos: &mut Position) -> i32 {
        if let Some(net) = &self.net {
            return net.eval(g, pos);
        }
        let v = self.players(g, pos);
        let me = pos.stm as usize;
        let enemy = (pos.stm as usize + 1) % g.sides as usize;
        v[me] - v[enemy]
    }

    #[allow(dead_code)]
    fn _unused(_: Side) {}
}
