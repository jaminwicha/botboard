//! The training reconciliation in miniature (Training Spec §3, §6).
//!
//! Two committed decisions are prototyped here, deterministically:
//!
//! 1. **The rung-3 net-sharing assumption** (Training Spec §13's
//!    load-bearing prototype): the search-free policy is the *same* shared
//!    evaluator's move scores refined by **NeuRD-style regularized
//!    updates** — softmax-CFR form: logits += lr · advantage, with
//!    Follow-the-Regularized-Leader pull toward a slowly-updated
//!    regularization policy (R-NaD's trick). One substrate serves both the
//!    search rungs (which consume values) and the policy cap (which
//!    consumes the refined logits).
//!
//! 2. **Gate training** (Training Spec §6): during self-play, sampled
//!    positions record the cheapest rung whose chosen move matches the
//!    soundest rung's — the cheapest-*sufficient* rung. A threshold fit on
//!    belief entropy turns those labels into the gate's dispatch bounds,
//!    biased toward the sounder side when uncertain.

use crate::belief::{determinize, Belief};
use crate::eval::Eval;
use crate::game::GameDef;
use crate::ladder::{LadderConfig, Rung};
use crate::movegen::legal_moves;
use crate::moves::Move;
use crate::position::Position;
use crate::rng::Rng;
use crate::search::Searcher;

// ---------------------------------------------------------------------------
// NeuRD-lite policy refinement
// ---------------------------------------------------------------------------

/// A linear policy head over shared-evaluator move features. The features
/// are the evaluator's one-ply scores (the "shared substrate"): NeuRD
/// refines *weights over that substrate*, it does not grow a second brain.
#[derive(Clone, Debug)]
pub struct PolicyHead {
    /// Scale on the shared eval score (the substrate feature).
    pub w_eval: f64,
    /// Per-move-kind bias: [normal, capture, ability/compound].
    pub bias: [f64; 3],
    /// Regularization target (FoReL): a slow copy pulled toward by `eta`.
    reg: Option<Box<PolicyHead>>,
}

impl PolicyHead {
    pub fn new() -> Self {
        PolicyHead { w_eval: 1.0, bias: [0.0; 3], reg: None }
    }

    fn feature_class(pos: &Position, mv: &Move) -> usize {
        use crate::moves::MoveKind;
        match mv.kind {
            MoveKind::Ability | MoveKind::Compound => 2,
            _ if pos.piece_at(mv.to).is_some() => 1,
            _ => 0,
        }
    }

    /// Logits over legal moves from one-ply shared-eval scores.
    pub fn logits(
        &self,
        g: &GameDef,
        pos: &mut Position,
        moves: &[Move],
        eval: &Eval,
    ) -> Vec<f64> {
        moves
            .iter()
            .map(|mv| {
                let u = pos.make(g, mv);
                let s = -eval.stm(g, pos) as f64 / 300.0;
                pos.unmake(g, &u);
                self.w_eval * s + self.bias[Self::feature_class(pos, mv)]
            })
            .collect()
    }

    pub fn softmax(logits: &[f64]) -> Vec<f64> {
        let max = logits.iter().cloned().fold(f64::MIN, f64::max);
        let exps: Vec<f64> = logits.iter().map(|l| (l - max).exp()).collect();
        let sum: f64 = exps.iter().sum();
        exps.iter().map(|e| e / sum).collect()
    }

    /// One NeuRD update from a sampled episode step: logit gradients follow
    /// the counterfactual advantage (score of the played move against the
    /// policy's expectation), plus the FoReL pull toward the regularization
    /// policy. `reward` is from the mover's perspective in [-1, 1].
    pub fn neurd_update(
        &mut self,
        g: &GameDef,
        pos: &mut Position,
        moves: &[Move],
        played: usize,
        reward: f64,
        eval: &Eval,
        lr: f64,
        forel_eta: f64,
    ) {
        let logits = self.logits(g, pos, moves, eval);
        let probs = Self::softmax(&logits);
        // Expected value baseline under the current policy.
        let scores: Vec<f64> = moves
            .iter()
            .map(|mv| {
                let u = pos.make(g, mv);
                let s = -eval.stm(g, pos) as f64 / 300.0;
                pos.unmake(g, &u);
                s
            })
            .collect();
        let baseline: f64 = probs.iter().zip(&scores).map(|(p, s)| p * s).sum();
        // Advantage of the played action, shaped by the episode reward
        // (outcome-corrected counterfactual advantage in miniature).
        let adv = (scores[played] - baseline) + reward;
        // NeuRD: gradient on the logit of the played action, distributed
        // through the shared features (no regret-matching projection).
        let cls = Self::feature_class(pos, &moves[played]);
        self.w_eval += lr * adv * scores[played].clamp(-2.0, 2.0);
        self.bias[cls] += lr * adv;
        // FoReL regularization toward the slow policy (R-NaD's fixed-point
        // iteration in miniature).
        if let Some(reg) = &self.reg {
            self.w_eval += forel_eta * (reg.w_eval - self.w_eval);
            for i in 0..3 {
                self.bias[i] += forel_eta * (reg.bias[i] - self.bias[i]);
            }
        }
    }

    /// Advance the regularization policy to the current one (R-NaD's outer
    /// iteration).
    pub fn advance_regularization(&mut self) {
        self.reg = Some(Box::new(PolicyHead { reg: None, ..self.clone() }));
    }

    /// Sample a move (stochastic policy, DeepNash-style) with the core RNG.
    pub fn sample(
        &self,
        g: &GameDef,
        pos: &mut Position,
        moves: &[Move],
        eval: &Eval,
        rng: &mut Rng,
    ) -> usize {
        let probs = Self::softmax(&self.logits(g, pos, moves, eval));
        let mut r = rng.unit_f64();
        for (i, p) in probs.iter().enumerate() {
            r -= p;
            if r <= 0.0 {
                return i;
            }
        }
        probs.len() - 1
    }
}

impl Default for PolicyHead {
    fn default() -> Self {
        Self::new()
    }
}

/// Train the policy head by hidden-information self-play episodes (both
/// sides sample from the same head — R-NaD self-play). Returns the trained
/// head; the caller owns evaluation (§11).
pub fn train_policy_selfplay(
    g: &GameDef,
    eval: &Eval,
    episodes: u32,
    max_plies: u32,
    seed: u64,
) -> PolicyHead {
    let mut head = PolicyHead::new();
    let mut rng = Rng::new(seed);
    for ep in 0..episodes {
        if ep % 8 == 0 {
            head.advance_regularization();
        }
        let mut pos = Position::startpos(g);
        // (position snapshot restored by replay; store move index + moves)
        let mut steps: Vec<(Position, Vec<Move>, usize, u8)> = Vec::new();
        let mut outcome: f64 = 0.0; // side-0 perspective
        for _ in 0..max_plies {
            let moves = legal_moves(g, &mut pos);
            if moves.is_empty() {
                let stm = pos.stm;
                let in_check = pos.royal_attacked(g, stm);
                outcome = if in_check {
                    if stm == 0 {
                        -1.0
                    } else {
                        1.0
                    }
                } else {
                    0.0
                };
                break;
            }
            let pick = head.sample(g, &mut pos, &moves, eval, &mut rng);
            steps.push((pos.clone(), moves.clone(), pick, pos.stm));
            let mv = moves[pick];
            pos.make(g, &mv);
        }
        for (mut spos, moves, pick, stm) in steps {
            let reward = if stm == 0 { outcome } else { -outcome };
            head.neurd_update(g, &mut spos, &moves, pick, reward * 0.5, eval, 0.05, 0.02);
        }
    }
    head
}

// ---------------------------------------------------------------------------
// Gate training (Training Spec §6)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct GateSample {
    pub entropy: f64,
    pub pivotal: bool,
    /// The cheapest rung whose chosen move matched the soundest rung's.
    pub cheapest_sufficient: Rung,
}

/// Collect gate-training labels: on sampled hidden positions, run every
/// rung and record the cheapest one that agrees with the soundest tried.
pub fn collect_gate_samples(
    g: &GameDef,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    positions: u32,
    seed: u64,
) -> Vec<GateSample> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::new();
    for i in 0..positions {
        // A fresh cold-open game advanced a few random plies gives a
        // position with partially-collapsed belief.
        let mut pos = Position::startpos(g);
        let mut belief = Belief::cold_open(g, &pos, 0);
        let plies = rng.below(12) as u32;
        let mut dead = false;
        for _ in 0..plies {
            let pre = pos.clone();
            let moves = legal_moves(g, &mut pos);
            if moves.is_empty() {
                dead = true;
                break;
            }
            let mv = moves[rng.below(moves.len())];
            pos.make(g, &mv);
            belief.observe(g, &pre, &mv);
        }
        if dead || legal_moves(g, &mut pos).is_empty() {
            continue;
        }

        // The "soundest" reference at this scale: ISMCTS (rung 2).
        let reference = run_rung(g, &mut pos, &belief, searcher, cfg, Rung::R2Ismcts, &mut rng);
        let Some(reference) = reference else { continue };

        let mut cheapest = Rung::R2Ismcts;
        for rung in [Rung::R0PerfectInfo, Rung::R1Determinize] {
            let mv = run_rung(g, &mut pos, &belief, searcher, cfg, rung, &mut rng);
            if mv == Some(reference) {
                cheapest = rung;
                break;
            }
        }
        out.push(GateSample {
            entropy: belief.entropy(),
            pivotal: belief.pivotal(&searcher.eval.material),
            cheapest_sufficient: cheapest,
        });
        let _ = i;
    }
    out
}

fn run_rung(
    g: &GameDef,
    pos: &mut Position,
    belief: &Belief,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    rung: Rung,
    rng: &mut Rng,
) -> Option<Move> {
    match rung {
        Rung::R0PerfectInfo => {
            // Rung 0 on imperfect info = search one determinized world.
            let mut world = determinize(g, pos, belief, rng, 32);
            searcher.search(g, &mut world, cfg.depth, cfg.node_budget).best
        }
        _ => crate::ladder::choose_move_with_rung(g, pos, belief, searcher, cfg, rng, rung),
    }
}

/// Fit entropy thresholds from labeled samples: `sharp` = the largest
/// entropy at which rung 0 was sufficient in ≥ `quantile` of samples below
/// it (bias toward the sounder side: we take a conservative low quantile).
pub fn fit_gate_thresholds(samples: &[GateSample], default: &LadderConfig) -> (f64, f64) {
    let mut r0_ok: Vec<f64> = samples
        .iter()
        .filter(|s| s.cheapest_sufficient == Rung::R0PerfectInfo)
        .map(|s| s.entropy)
        .collect();
    let mut r1_ok: Vec<f64> = samples
        .iter()
        .filter(|s| s.cheapest_sufficient <= Rung::R1Determinize)
        .map(|s| s.entropy)
        .collect();
    r0_ok.sort_by(|a, b| a.partial_cmp(b).unwrap());
    r1_ok.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Conservative: the 25th percentile of entropies where the cheap rung
    // sufficed (when uncertain, escalate — Training Spec §6 safe default).
    let q = |v: &[f64], def: f64| {
        if v.is_empty() {
            def
        } else {
            v[(v.len() - 1) / 4]
        }
    };
    (q(&r0_ok, default.sharp), q(&r1_ok, default.broad))
}
