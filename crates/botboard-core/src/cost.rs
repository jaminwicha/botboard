//! The cost model (§4): an analytic prior over Bit-sets, calibrated on
//! anchor pieces, corrected by self-play (§4.3, wired in `selfplay`).
//!
//! The mobility integral is measured empirically with the *real* compiled
//! kernels against seeded random occupancies — so path rules, zones,
//! hoppers, and direction masks all price themselves; no per-Bit special
//! cases. Self-play later folds realized value back into per-type values.

use crate::game::{GameDef, Side, TypeId};
use crate::position::{Loc, Piece, Position};
use crate::rng::Rng;

/// Occupancy density used for the integral; roughly a midgame board.
const DENSITY: f64 = 0.30;
const SAMPLES: usize = 48;

#[derive(Clone, Copy, Debug, Default)]
pub struct Mobility {
    pub avg_moved: f64,
    pub avg_attacked: f64,
}

/// Average move/attack destination counts for a type over all squares and
/// seeded random occupancies (deterministic: one fixed seed).
pub fn mobility_integral(g: &GameDef, t: TypeId, side: Side) -> Mobility {
    let mut rng = Rng::new(0xC057_0000 + t as u64 * 7919 + side as u64);
    let ncells = g.board.ncells();
    let (mut moved, mut attacked, mut placements) = (0f64, 0f64, 0usize);

    for _ in 0..SAMPLES {
        // A synthetic position: random filler pieces of an inert type would
        // change movegen; instead we build occupancy from real pieces of the
        // *first* type on both sides, which blocks/screens like any piece.
        let mut list: Vec<(TypeId, Side, u16, bool)> = Vec::new();
        for sq in 0..ncells as u16 {
            if rng.unit_f64() < DENSITY {
                let s = if rng.unit_f64() < 0.5 { 0 } else { 1 };
                list.push((0, s, sq, true));
            }
        }
        for sq in 0..ncells as u16 {
            // Place the probe on `sq`, displacing any filler.
            let mut l: Vec<_> =
                list.iter().copied().filter(|&(_, _, s, _)| s != sq).collect();
            l.push((t, side, sq, true));
            let mut pos = Position::from_pieces(g, &l, side);
            // Count destinations of the probe piece only.
            let probe_from = sq;
            let moves = crate::movegen::pseudo_moves(g, &pos);
            // Dedupe by destination so promotion-choice expansion doesn't
            // inflate mobility.
            let mut dests: Vec<u16> = Vec::new();
            let mut m = 0f64;
            let mut a = 0f64;
            for mv in &moves {
                if mv.from != probe_from || dests.contains(&mv.to) {
                    continue;
                }
                dests.push(mv.to);
                if pos.piece_at(mv.to).is_some() {
                    a += 1.0;
                } else {
                    m += 1.0;
                }
            }
            // Zone-confined pieces score zero off-zone; that *is* their
            // confinement discount, so keep all placements in the average.
            moved += m;
            attacked += a;
            placements += 1;
            // Silence unused-mut lint paths on Position.
            let _ = &mut pos;
        }
    }
    Mobility {
        avg_moved: moved / placements as f64,
        avg_attacked: attacked / placements as f64,
    }
}

/// Calibration weights fitted on the anchor set (§4.2).
#[derive(Clone, Copy, Debug)]
pub struct CostWeights {
    pub w_move: f64,
    pub w_attack: f64,
}

/// An anchor: a piece type pinned to an external reference value.
pub struct Anchor {
    pub t: TypeId,
    pub value: f64,
}

/// C_prior = max(1, C_base × M_utility − S_nerfs) + Σ synergy (§4.1).
/// Phase-1 content has no utility/nerf Bits yet, so those terms are the
/// identity; the hooks stay so later Bits (armor, multi-action ×1.8) price in.
pub fn cost_prior(g: &GameDef, t: TypeId, w: &CostWeights) -> f64 {
    if g.types[t as usize].royal {
        return 0.0; // royalty is priceless, not priced (§5 royalty policy)
    }
    let m = mobility_integral(g, t, 0);
    let c_base = m.avg_moved * w.w_move + m.avg_attacked * w.w_attack;
    let m_utility = 1.0;
    let s_nerfs = 0.0;
    (c_base * m_utility - s_nerfs).max(1.0)
}

/// Fit (w_move, w_attack) by least squares against the anchors:
/// minimize Σ (m_i·w − v_i)², i.e. a 2×2 normal-equations solve.
pub fn fit_weights(g: &GameDef, anchors: &[Anchor]) -> CostWeights {
    let (mut a11, mut a12, mut a22, mut b1, mut b2) = (0f64, 0f64, 0f64, 0f64, 0f64);
    for an in anchors {
        let m = mobility_integral(g, an.t, 0);
        a11 += m.avg_moved * m.avg_moved;
        a12 += m.avg_moved * m.avg_attacked;
        a22 += m.avg_attacked * m.avg_attacked;
        b1 += m.avg_moved * an.value;
        b2 += m.avg_attacked * an.value;
    }
    let det = a11 * a22 - a12 * a12;
    if det.abs() > 1e-9 {
        let w = CostWeights {
            w_move: (b1 * a22 - b2 * a12) / det,
            w_attack: (b2 * a11 - b1 * a12) / det,
        };
        // Mobility features are near-collinear; an unconstrained fit can go
        // negative and underprice capture-only pieces. Require both ≥ 0.
        if w.w_move >= 0.0 && w.w_attack >= 0.0 {
            return w;
        }
    }
    // Single-parameter fallback: one weight on (moved + attacked).
    let w = (b1 + b2) / (a11 + 2.0 * a12 + a22).max(1e-9);
    CostWeights { w_move: w, w_attack: w }
}

/// Integer piece values for the deterministic-grade evaluator (§10.6):
/// centi-pawn scale, derived from the prior, overridable by self-play
/// corrected values.
pub fn material_table(g: &GameDef, w: &CostWeights) -> Vec<i32> {
    (0..g.types.len() as TypeId)
        .map(|t| {
            if g.types[t as usize].royal {
                0
            } else {
                (cost_prior(g, t, w) * 100.0).round() as i32
            }
        })
        .collect()
}

// Convenience for tests: silence unused import if Piece/Loc unused later.
#[allow(dead_code)]
fn _t(_: &Piece, _: &Loc) {}
