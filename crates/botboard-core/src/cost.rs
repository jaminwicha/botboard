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
                // Spy is board-null: its reveal ball is information, not
                // board power — the flat prior term prices it, so it
                // must not masquerade as attack coverage here.
                if mv.effect == crate::moves::Effect::Spy {
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
/// Utility multipliers per §4.1: armor 1 + 0.5(HP−1); multi-action
/// (overclock compound) ×1.8 initial prior. Coupled nerfs (the laser's
/// forced retreat) price via S_nerfs; ability mobility (laser reach, wall/
/// pit/heal targets) already flows through the measured integral because
/// abilities are generated moves. Synergy weights ship 0 until self-play
/// fits them (§4.3).
pub fn cost_prior(g: &GameDef, t: TypeId, w: &CostWeights) -> f64 {
    if g.types[t as usize].royal {
        return 0.0; // royalty is priceless, not priced (§5 royalty policy)
    }
    let ty = &g.types[t as usize];
    let m = mobility_integral(g, t, 0);
    let c_base = m.avg_moved * w.w_move + m.avg_attacked * w.w_attack;
    let mut m_utility = 1.0;
    if ty.max_hp > 1 {
        m_utility *= 1.0 + 0.5 * (ty.max_hp as f64 - 1.0);
    }
    if ty.overclock {
        m_utility *= 1.8;
    }
    if ty.stealth {
        m_utility *= 1.4; // Appendix B: concealment multiplier
    }
    if ty.flight {
        m_utility *= 1.1; // Appendix B: terrain-permission multiplier
    }
    if ty.drill {
        m_utility *= 1.15; // §3.1's second terrain permission: wall-chewing
    }
    // HP-gated kernels (§3.1 state condition): the mobility integral
    // prices at full HP, so a Bit-set with a kernel that only wakes below
    // max HP (a desperation Bit) carries conditional vocabulary the piece
    // cannot use at full strength — discounted once, ×0.85, documented
    // as a simple prior until self-play refits it (§4.3).
    if ty.max_hp > 1
        && ty
            .bits
            .iter()
            .any(|b| b.max_hp != 0 && b.max_hp < ty.max_hp)
    {
        m_utility *= 0.85;
    }
    let mut s_nerfs = 0.0;
    let mut s_flat = 0.0;
    for a in &ty.abilities {
        match a {
            // Spec §4.1: a piercing beam prices ×1.3 — occupancy-blind rays
            // reach targets the mobility integral's screens would hide.
            crate::bits::AbilityBit::Laser { pierce: true, .. } => {
                m_utility *= 1.3;
                if let crate::bits::AbilityBit::Laser { retreat: true, .. } = a {
                    s_nerfs += 0.5;
                }
            }
            crate::bits::AbilityBit::Laser { retreat: true, .. } => s_nerfs += 0.5,
            // Board-state-dependent abilities generate no moves on the
            // mobility-integral board, so they carry Appendix-B-style flat
            // priors until self-play refits them (§4.3).
            crate::bits::AbilityBit::Resurrect { .. } => s_flat += 4.0,
            crate::bits::AbilityBit::Hack { .. } => s_flat += 5.0,
            crate::bits::AbilityBit::MineLayer { .. } => s_flat += 2.0,
            // Batch-2 flat priors: the exchange trick and the shove.
            crate::bits::AbilityBit::Swap { .. } => s_flat += 1.5,
            crate::bits::AbilityBit::Push { .. } => s_flat += 2.0,
            // Spy is board-null; its value is informational (belief
            // collapse at the SRW layer), priced as a small flat term.
            crate::bits::AbilityBit::Spy { .. } => s_flat += 0.5,
            // Stage-4 custom rows price via their REQUIRED cost hint:
            // the flat term joins the board-state-dependent abilities'
            // S_flat; the multiplier joins M_utility (pierce-style
            // utility scaling). Point/ray target mobility still flows
            // through the measured integral because custom abilities
            // are generated moves.
            crate::bits::AbilityBit::Custom(ci) => {
                let hint = &g.custom_effects[*ci as usize].cost;
                m_utility *= hint.mult;
                s_flat += hint.flat;
            }
            _ => {}
        }
    }
    if ty.emp_aura > 0 {
        s_flat += 6.5; // Appendix B EMP aura prior
    }
    if ty.hologram {
        // A decoy's value is the bluff, not the body (Appendix B +2.5).
        s_flat += 2.5;
    }
    (c_base * m_utility - s_nerfs + s_flat).max(1.0)
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

/// Bit-category flags for the synergy term (§4.1): K interpretable
/// categories per type, from the compiled kernels and type flags.
pub const K_CATS: usize = 8;

pub fn bit_categories(g: &GameDef, t: TypeId) -> [bool; K_CATS] {
    let ck = g.compiled(t, 0);
    let ty = &g.types[t as usize];
    [
        !ck.leaps.is_empty(),
        !ck.rides.is_empty(),
        !ck.hops.is_empty(),
        ck.leaps.iter().all(|k| k.d.dy > 0) && !ck.leaps.is_empty()
            || ck.rides.iter().all(|k| k.d.dy > 0) && !ck.rides.is_empty(),
        ty.max_hp > 1,
        ty.overclock,
        !ty.abilities.is_empty(),
        ck.leaps.iter().any(|k| !k.mode.can_move())
            || ck.rides.iter().any(|k| !k.mode.can_move()),
    ]
}

/// The synergy term (§4.1): Σ_{i<j} S_ij·x_i·x_j over Bit categories —
/// a structured pairwise prior, O(K²), fitted from self-play residuals
/// (measured value − analytic prior) per §4.3.
#[derive(Clone, Debug)]
pub struct SynergyModel {
    /// Upper-triangular pairwise weights, row-major over i<j.
    pub s: Vec<f64>,
}

impl SynergyModel {
    pub fn zeros() -> Self {
        SynergyModel { s: vec![0.0; K_CATS * (K_CATS - 1) / 2] }
    }

    fn idx(i: usize, j: usize) -> usize {
        debug_assert!(i < j);
        i * (2 * K_CATS - i - 1) / 2 + (j - i - 1)
    }

    pub fn term(&self, x: &[bool; K_CATS]) -> f64 {
        let mut sum = 0.0;
        for i in 0..K_CATS {
            if !x[i] {
                continue;
            }
            for j in (i + 1)..K_CATS {
                if x[j] {
                    sum += self.s[Self::idx(i, j)];
                }
            }
        }
        sum
    }

    /// Fit pairwise weights by SGD on (categories, residual) samples —
    /// the §4.3 correction loop's synergy half. L2-regularized so sparse
    /// data can't inflate the structured prior.
    pub fn fit(samples: &[([bool; K_CATS], f64)], epochs: u32, lr: f64) -> Self {
        let mut m = Self::zeros();
        for _ in 0..epochs {
            for (x, y) in samples {
                let err = m.term(x) - y;
                for i in 0..K_CATS {
                    if !x[i] {
                        continue;
                    }
                    for j in (i + 1)..K_CATS {
                        if x[j] {
                            let w = &mut m.s[Self::idx(i, j)];
                            *w -= lr * (err + 0.01 * *w);
                        }
                    }
                }
            }
        }
        m
    }
}

/// C_prior with the fitted synergy term (§4.1's full form).
pub fn cost_prior_with_synergy(
    g: &GameDef,
    t: TypeId,
    w: &CostWeights,
    synergy: &SynergyModel,
) -> f64 {
    let base = cost_prior(g, t, w);
    if g.types[t as usize].royal {
        return base;
    }
    (base + synergy.term(&bit_categories(g, t))).max(1.0)
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
