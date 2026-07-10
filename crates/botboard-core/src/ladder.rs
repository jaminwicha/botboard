//! The belief-gated search ladder (§8.2–8.5): one shared evaluator under
//! four dispatch modes, cheapest-sound-first.
//!
//! Rung 0: perfect-information alpha-beta (the point-mass cap).
//! Rung 1: determinization / PIMC — sample consistent worlds, solve each
//!         with the rung-0 engine, aggregate root votes.
//! Rung 2: sound game-theoretic search — depth-limited external-sampling
//!         MCCFR/OOS over information sets (`oos.rs`); the rung that can
//!         bluff and value information-gathering correctly.
//! Rung 3: search-free softmax policy over the shared eval (the R-NaD-
//!         shaped cap; its NeuRD refinement lives in the training loop).
//!
//! The gate dispatches on belief sharpness + pivotality, biased toward the
//! next-sounder rung when uncertain (§8.5); the full GT-CFR interpolation
//! between rungs is the Phase-2+ growth path.

use crate::belief::{determinize, Belief};
use crate::game::GameDef;
use crate::movegen::legal_moves;
use crate::moves::Move;
use crate::position::Position;
use crate::rng::Rng;
use crate::search::Searcher;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Rung {
    R0PerfectInfo,
    R1Determinize,
    R2Sound,
    R3Policy,
}

pub struct LadderConfig {
    pub depth: i32,
    pub node_budget: u64,
    pub determinizations: usize,
    pub oos_iterations: u32,
    pub oos_depth: u32,
    /// Entropy thresholds: below `sharp` → rung 1, below `broad` → rung 2,
    /// else rung 3.
    pub sharp: f64,
    pub broad: f64,
}

impl Default for LadderConfig {
    fn default() -> Self {
        LadderConfig {
            depth: 4,
            node_budget: 200_000,
            determinizations: 8,
            oos_iterations: 256,
            oos_depth: 4,
            sharp: 6.0,
            broad: 24.0,
        }
    }
}

/// The gate (§8.5): entropy → rung, with pivotality escalating one rung
/// (bias toward the sounder side when uncertain).
pub fn gate(belief: &Belief, material: &[i32], cfg: &LadderConfig) -> Rung {
    let h = belief.entropy();
    let mut rung = if h == 0.0 {
        Rung::R0PerfectInfo
    } else if h < cfg.sharp {
        Rung::R1Determinize
    } else if h < cfg.broad {
        Rung::R2Sound
    } else {
        Rung::R3Policy
    };
    if rung == Rung::R1Determinize && belief.pivotal(material) {
        rung = Rung::R2Sound;
    }
    rung
}

/// What one ladder decision cost (for profiling/attribution; `oos` is
/// populated only when the gate dispatched to rung 2).
#[derive(Clone, Copy, Debug)]
pub struct LadderTrace {
    pub rung: Rung,
    pub oos: Option<crate::oos::OosStats>,
}

/// Choose a move for `pos.stm` given its belief. `truth` is ground truth;
/// hidden information reaches the chooser only through `belief`-consistent
/// determinizations (§8.6's no-leakage discipline).
pub fn choose_move(
    g: &GameDef,
    truth: &mut Position,
    belief: &Belief,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    rng: &mut Rng,
) -> Option<(Move, Rung)> {
    choose_move_traced(g, truth, belief, searcher, cfg, rng).map(|(m, t)| (m, t.rung))
}

/// `choose_move` plus per-decision counters. Identical move selection —
/// rung 2 runs through the same `Oos::choose`, only the stats are kept.
pub fn choose_move_traced(
    g: &GameDef,
    truth: &mut Position,
    belief: &Belief,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    rng: &mut Rng,
) -> Option<(Move, LadderTrace)> {
    let rung = gate(belief, &searcher.eval.material, cfg);
    let mut trace = LadderTrace { rung, oos: None };
    let mv = if rung == Rung::R2Sound {
        let mut oos = crate::oos::Oos::new();
        let ocfg = crate::oos::OosConfig {
            iterations: cfg.oos_iterations,
            depth: cfg.oos_depth,
            ..Default::default()
        };
        let mv = oos.choose(g, truth, belief, searcher, &ocfg, rng).map(|(mv, _)| mv);
        trace.oos = Some(oos.stats);
        mv
    } else {
        choose_move_with_rung(g, truth, belief, searcher, cfg, rng, rung)
    };
    mv.map(|m| (m, trace))
}

/// Run a specific rung directly (gate bypass — used by gate training,
/// Training Spec §6, and rung-consistency checks, §11).
pub fn choose_move_with_rung(
    g: &GameDef,
    truth: &mut Position,
    belief: &Belief,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    rng: &mut Rng,
    rung: Rung,
) -> Option<Move> {
    match rung {
        Rung::R0PerfectInfo => searcher.search(g, truth, cfg.depth, cfg.node_budget).best,
        Rung::R1Determinize => pimc(g, truth, belief, searcher, cfg, rng),
        Rung::R2Sound => oos_move(g, truth, belief, searcher, cfg, rng),
        Rung::R3Policy => policy_move(g, truth, belief, searcher, rng),
    }
}

/// Rung 1 — PIMC: root moves voted by average score across sampled worlds.
fn pimc(
    g: &GameDef,
    truth: &mut Position,
    belief: &Belief,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    rng: &mut Rng,
) -> Option<Move> {
    let root_moves = legal_moves(g, truth);
    if root_moves.is_empty() {
        return None;
    }
    let mut totals = vec![0i64; root_moves.len()];
    let mut counts = vec![0i64; root_moves.len()];
    for _ in 0..cfg.determinizations {
        let mut world = determinize(g, truth, belief, rng, 32);
        let world_moves = legal_moves(g, &mut world);
        for (i, mv) in root_moves.iter().enumerate() {
            if !world_moves.contains(mv) {
                continue;
            }
            let u = world.make(g, mv);
            let r = searcher.search(g, &mut world, cfg.depth - 1, cfg.node_budget / 8);
            world.unmake(g, &u);
            totals[i] += -r.score as i64;
            counts[i] += 1;
        }
    }
    (0..root_moves.len())
        .filter(|&i| counts[i] > 0)
        .max_by_key(|&i| totals[i] / counts[i])
        .or(Some(0))
        .map(|i| root_moves[i])
}

/// Rung 2 — sound search: OOS average strategy at the root (§8.2).
fn oos_move(
    g: &GameDef,
    truth: &mut Position,
    belief: &Belief,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    rng: &mut Rng,
) -> Option<Move> {
    let mut oos = crate::oos::Oos::new();
    let ocfg = crate::oos::OosConfig {
        iterations: cfg.oos_iterations,
        depth: cfg.oos_depth,
        ..Default::default()
    };
    oos.choose(g, truth, belief, searcher, &ocfg, rng).map(|(mv, _)| mv)
}

/// Rung 3 — the search-free policy cap: softmax over one-ply eval on a
/// sampled world, sampled with the core RNG (stochastic like DeepNash).
/// Candidates come from the arbiter's legal set (rules-legality can depend
/// on hidden enemy kernels — e.g. check — so the arbiter publishes what is
/// playable, Kriegspiel-referee style); the *scoring* sees only the world.
fn policy_move(
    g: &GameDef,
    truth: &mut Position,
    belief: &Belief,
    searcher: &mut Searcher,
    rng: &mut Rng,
) -> Option<Move> {
    let mut world = determinize(g, truth, belief, rng, 32);
    let moves = legal_moves(g, truth);
    if moves.is_empty() {
        return None;
    }
    let world_moves = legal_moves(g, &mut world);
    let scores: Vec<f64> = moves
        .iter()
        .map(|mv| {
            // A move the sampled world disallows scores neutrally.
            if !world_moves.contains(mv) {
                return 0.0;
            }
            let u = world.make(g, mv);
            let s = -searcher.eval.stm(g, &mut world) as f64;
            world.unmake(g, &u);
            s / 150.0
        })
        .collect();
    let max = scores.iter().cloned().fold(f64::MIN, f64::max);
    let exps: Vec<f64> = scores.iter().map(|s| (s - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    let mut r = rng.unit_f64() * sum;
    for (i, e) in exps.iter().enumerate() {
        r -= e;
        if r <= 0.0 {
            return Some(moves[i]);
        }
    }
    moves.last().copied()
}
