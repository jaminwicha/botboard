//! The learned evaluator (§7.4, §10.6): an NNUE-style accumulator network
//! whose piece features are **Bit-derived descriptors**, not type ids —
//! Departure B: any procedural piece gets features from its compiled
//! kernels, so never-before-seen pieces evaluate sensibly.
//!
//! Shape: per piece, input = descriptor(type,side) ⊗ positional basis φ(sq)
//! summed into a two-perspective accumulator (own/enemy from the mover's
//! view) → one ReLU hidden layer → a scalar value in centipawns (per-player
//! vector derives as [v, −v] for two players).
//!
//! Two grades (§10.6):
//! - **Performance grade** (`FloatNet`): f32 training by SGD on self-play
//!   outcomes — the farm side; reproducible by seed+log, not bit-replay.
//! - **Deterministic grade** (`QuantNet`): i16/i32 fixed-point inference,
//!   integer ops with defined semantics — bit-exact across platforms; the
//!   only grade permitted on product-canonical paths.
//! **Quantization parity** is a named test obligation: see `tests/nnue_suite`.

use crate::game::{GameDef, Side, TypeId};
use crate::position::{Loc, Position};
use crate::rng::Rng;

pub const D: usize = 21; // descriptor dims (v2: SRW ability vocabulary)
pub const P: usize = 4; // positional basis dims
pub const F: usize = D * P; // per-perspective feature dims
pub const IN: usize = 2 * F; // own + enemy perspectives
pub const H: usize = 32; // hidden units

/// v1 descriptor shape (BBNET001 checkpoints): 12 dims, ability vocabulary
/// summarized as a bare `abilities.len()/4` at dim 10. Kept for backward
/// checkpoint loading only.
const D_V1: usize = 12;
const F_V1: usize = D_V1 * P;
const IN_V1: usize = 2 * F_V1;

/// Fixed-point scales for the deterministic grade.
const QIN: i32 = 64; // input fraction bits: value × 64
const QW: i32 = 64; // weight fraction bits

/// Bit-derived descriptor of a piece type (§7.4): pure function of the
/// compiled kernels and type flags — integer-derived, so both grades see
/// exactly representable values (every dim is a rational with denominator
/// dividing QIN, so ×64 quantizes without rounding error).
///
/// v2 layout: dims 0–9 are the v1 movement/armor block unchanged; dims
/// 10–15 replace v1's bare `abilities.len()/4` with one signal per Axis-B
/// ability kind (§3.2); dims 16–19 are the Appendix-B flags (stealth,
/// flight, hologram, EMP aura); dim 20 is the bias.
pub fn descriptor(g: &GameDef, t: TypeId, side: Side) -> [f32; D] {
    use crate::bits::AbilityBit;
    let ck = g.compiled(t, side);
    let ty = &g.types[t as usize];
    let nl = ck.leaps.len() as f32;
    let nr = ck.rides.len() as f32;
    let nh = ck.hops.len() as f32;
    let frac = |num: usize, den: f32| if den > 0.0 { num as f32 / den } else { 0.0 };
    let fwd_leap = frac(ck.leaps.iter().filter(|k| k.d.dy > 0).count(), nl);
    let fwd_ride = frac(ck.rides.iter().filter(|k| k.d.dy > 0).count(), nr);
    let cap_only = frac(
        ck.leaps.iter().filter(|k| !k.mode.can_move()).count()
            + ck.rides.iter().filter(|k| !k.mode.can_move()).count(),
        nl + nr,
    );
    let move_only = frac(
        ck.leaps.iter().filter(|k| !k.mode.can_capture()).count()
            + ck.rides.iter().filter(|k| !k.mode.can_capture()).count(),
        nl + nr,
    );
    // Per-ability-kind signals: reach-normalized where reach is the value
    // (heal amount, laser range + pierce bonus), presence/count elsewhere.
    let (mut heal, mut laser, mut wallpit, mut mine, mut res, mut hack) =
        (0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
    for a in &ty.abilities {
        match *a {
            AbilityBit::Heal { amount, .. } => {
                heal = heal.max((amount as f32 / 2.0).min(2.0))
            }
            AbilityBit::Laser { range, pierce, .. } => {
                // §4.1's pierce logic in feature form: an occupancy-blind
                // beam reaches like a longer one.
                let reach = range as f32 + if pierce { 2.0 } else { 0.0 };
                laser = laser.max((reach / 4.0).min(2.0));
            }
            AbilityBit::CreateWall { .. } | AbilityBit::DigPit { .. } => {
                wallpit = (wallpit + 0.5).min(1.0)
            }
            AbilityBit::MineLayer { .. } => mine = 1.0,
            AbilityBit::Resurrect { .. } => res = 1.0,
            AbilityBit::Hack { .. } => hack = 1.0,
            // Batch-2 abilities (swap/push) have no dedicated descriptor
            // dim: the 21-dim layout is frozen for checkpoint
            // compatibility; their value flows through the flat priors.
            // Swap/push/spy carry no descriptor dim: swap and push flow
            // through the cost prior only; spy is board-null entirely.
            AbilityBit::Swap { .. } | AbilityBit::Push { .. } | AbilityBit::Spy { .. } => {}
            // Stage-4 custom rows map into the frozen layout via their
            // validated stdlib-kin slot (documented approximation: a
            // fixed presence signal of 1.0 on the kin's dim — exactly
            // representable at ×64 — or the wall/pit +0.5 step; kins
            // without a dim contribute nothing here and price via the
            // cost hint's flat prior).
            AbilityBit::Custom(ci) => match g.custom_effects[ci as usize].descriptor_slot {
                10 => heal = heal.max(1.0),
                11 => laser = laser.max(1.0),
                12 => wallpit = (wallpit + 0.5).min(1.0),
                13 => mine = 1.0,
                14 => res = 1.0,
                15 => hack = 1.0,
                _ => {}
            },
        }
    }
    [
        (nl / 8.0).min(2.0),
        (nr / 8.0).min(2.0),
        (nh / 4.0).min(2.0),
        fwd_leap,
        fwd_ride,
        cap_only,
        move_only,
        ty.royal as u8 as f32,
        (ty.max_hp as f32 / 4.0).min(2.0),
        ty.overclock as u8 as f32,
        heal,
        laser,
        wallpit,
        mine,
        res,
        hack,
        ty.stealth as u8 as f32,
        ty.flight as u8 as f32,
        ty.hologram as u8 as f32,
        (ty.emp_aura as f32 / 2.0).min(2.0),
        1.0, // bias feature
    ]
}

/// Positional basis φ(sq) in the piece-owner's frame (y flips for side 1).
fn phi(g: &GameDef, sq: u16, side: Side) -> [f32; P] {
    let (x, y) = g.board.xy(sq);
    let w = (g.board.w - 1).max(1) as f32;
    let h = (g.board.h - 1).max(1) as f32;
    let xn = x as f32 / w;
    let yn = if side == 0 { y as f32 / h } else { 1.0 - y as f32 / h };
    let cx = (xn - 0.5).abs() * 2.0;
    [1.0, xn, yn, cx]
}

/// Accumulate the two-perspective feature vector for `stm`'s view.
fn features(g: &GameDef, pos: &Position, stm: Side) -> [f32; IN] {
    let mut f = [0f32; IN];
    for p in &pos.pieces {
        let Loc::Board(sq) = p.loc else { continue };
        let d = descriptor(g, p.t, p.side);
        let ph = phi(g, sq, p.side);
        let base = if p.side == stm { 0 } else { F };
        for (i, di) in d.iter().enumerate() {
            for (j, pj) in ph.iter().enumerate() {
                f[base + i * P + j] += di * pj;
            }
        }
    }
    f
}

// ---------------------------------------------------------------------------
// Performance grade: float net + SGD training
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct FloatNet {
    pub w1: Vec<f32>, // [IN][H]
    pub b1: Vec<f32>, // [H]
    pub w2: Vec<f32>, // [H]
    pub b2: f32,
}

impl FloatNet {
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut init = |n: usize, scale: f32| {
            (0..n).map(|_| (rng.unit_f64() as f32 - 0.5) * scale).collect::<Vec<f32>>()
        };
        FloatNet {
            w1: init(IN * H, 0.2),
            b1: init(H, 0.05),
            w2: init(H, 0.4),
            b2: 0.0,
        }
    }

    fn hidden(&self, f: &[f32; IN]) -> [f32; H] {
        let mut h = [0f32; H];
        for j in 0..H {
            let mut a = self.b1[j];
            for i in 0..IN {
                a += f[i] * self.w1[i * H + j];
            }
            h[j] = a.max(0.0);
        }
        h
    }

    /// Value in centipawns from `pos.stm`'s perspective.
    pub fn eval(&self, g: &GameDef, pos: &Position) -> f32 {
        let f = features(g, pos, pos.stm);
        let h = self.hidden(&f);
        let mut v = self.b2;
        for j in 0..H {
            v += h[j] * self.w2[j];
        }
        v * 100.0
    }

    /// One SGD step on (position, outcome z ∈ {0, 0.5, 1} from stm's view).
    /// Loss: cross-entropy on sigmoid(v/4) with v in pawn units.
    pub fn sgd_step(&mut self, g: &GameDef, pos: &Position, z: f32, lr: f32) -> f32 {
        let f = features(g, pos, pos.stm);
        let h = self.hidden(&f);
        let mut v = self.b2;
        for j in 0..H {
            v += h[j] * self.w2[j];
        }
        let p = 1.0 / (1.0 + (-v / 4.0).exp());
        let dv = (p - z) / 4.0; // dLoss/dv
        // Output layer.
        for j in 0..H {
            let gw2 = dv * h[j];
            let dh = if h[j] > 0.0 { dv * self.w2[j] } else { 0.0 };
            self.w2[j] -= lr * gw2;
            if dh != 0.0 {
                self.b1[j] -= lr * dh;
                for i in 0..IN {
                    if f[i] != 0.0 {
                        self.w1[i * H + j] -= lr * dh * f[i];
                    }
                }
            }
        }
        self.b2 -= lr * dv;
        -(z * p.max(1e-7).ln() + (1.0 - z) * (1.0 - p).max(1e-7).ln())
    }
}

impl FloatNet {
    /// Versioned checkpoint (Training Spec §8): little-endian f32s.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = b"BBNET002".to_vec();
        out.extend((IN as u32).to_le_bytes());
        out.extend((H as u32).to_le_bytes());
        for v in self.w1.iter().chain(self.b1.iter()).chain(self.w2.iter()) {
            out.extend(v.to_le_bytes());
        }
        out.extend(self.b2.to_le_bytes());
        out
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() >= 16 && &data[0..8] == b"BBNET001" {
            return Self::from_bytes_v1(data);
        }
        if data.len() < 16 || &data[0..8] != b"BBNET002" {
            return Err("bad magic".into());
        }
        let rd = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
        if rd(8) as usize != IN || rd(12) as usize != H {
            return Err("dimension mismatch".into());
        }
        let need = 16 + (IN * H + H + H + 1) * 4;
        if data.len() != need {
            return Err(format!("expected {need} bytes, got {}", data.len()));
        }
        let mut f = data[16..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()));
        let mut take = |n: usize| (0..n).map(|_| f.next().unwrap()).collect::<Vec<f32>>();
        Ok(FloatNet {
            w1: take(IN * H),
            b1: take(H),
            w2: take(H),
            b2: f.next().unwrap(),
        })
    }

    /// Backward loading of BBNET001 checkpoints: w1 rows are per-input-dim,
    /// so the surviving v1 dims (movement/armor 0–9 and the bias) remap to
    /// their v2 feature indices and every new dim gets a zero row — new
    /// features then contribute nothing, reproducing the v1 net exactly on
    /// any content whose v1 ability-count feature was zero (true of every
    /// shipped BBNET001 checkpoint: all chess, where no type has abilities).
    /// The dropped bare-count row is superseded by the v2 per-kind dims and
    /// was gradient-dead on ability-free training data anyway (`sgd_step`
    /// skips zero features).
    fn from_bytes_v1(data: &[u8]) -> Result<Self, String> {
        let rd = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
        if rd(8) as usize != IN_V1 || rd(12) as usize != H {
            return Err("v1 dimension mismatch".into());
        }
        let need = 16 + (IN_V1 * H + H + H + 1) * 4;
        if data.len() != need {
            return Err(format!("expected {need} bytes (v1), got {}", data.len()));
        }
        let mut f = data[16..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()));
        let mut take = |n: usize| (0..n).map(|_| f.next().unwrap()).collect::<Vec<f32>>();
        let w1_v1 = take(IN_V1 * H);
        let b1 = take(H);
        let w2 = take(H);
        let b2 = f.next().unwrap();
        let mut w1 = vec![0f32; IN * H];
        for persp in 0..2 {
            for di in 0..D_V1 {
                // v1 dim → v2 dim: 0–9 identity, 11 (bias) → D−1,
                // 10 (bare ability count) dropped.
                let ndi = match di {
                    0..=9 => di,
                    11 => D - 1,
                    _ => continue,
                };
                for j in 0..P {
                    let old = persp * F_V1 + di * P + j;
                    let new = persp * F + ndi * P + j;
                    w1[new * H..(new + 1) * H]
                        .copy_from_slice(&w1_v1[old * H..(old + 1) * H]);
                }
            }
        }
        Ok(FloatNet { w1, b1, w2, b2 })
    }
}

// ---------------------------------------------------------------------------
// Deterministic grade: integer-quantized inference (§10.6)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct QuantNet {
    /// Quantized descriptor cache per (type, side): [D] × QIN.
    desc_q: Vec<[i32; D]>,
    nsides: usize,
    w1: Vec<i16>, // × QW
    b1: Vec<i32>, // × QIN·QW
    w2: Vec<i16>, // × QW
    b2: i32,      // × QIN·QW·QW (pre-shift domain)
}

impl QuantNet {
    /// Post-training quantization of the float net (§10.6): weights round
    /// to i16 fixed-point; descriptors are integer-derived so they quantize
    /// exactly.
    pub fn from_float(g: &GameDef, net: &FloatNet) -> Self {
        let ntypes = g.types.len();
        let mut desc_q = Vec::with_capacity(ntypes * g.sides as usize);
        for t in 0..ntypes {
            for s in 0..g.sides {
                let d = descriptor(g, t as TypeId, s);
                let mut q = [0i32; D];
                for i in 0..D {
                    q[i] = (d[i] * QIN as f32).round() as i32;
                }
                desc_q.push(q);
            }
        }
        let qi16 = |v: f32| (v * QW as f32).round().clamp(-32767.0, 32767.0) as i16;
        let _ = ntypes;
        QuantNet {
            desc_q,
            nsides: g.sides as usize,
            w1: net.w1.iter().map(|&v| qi16(v)).collect(),
            b1: net.b1.iter().map(|&v| (v * (QIN * QW) as f32).round() as i32).collect(),
            w2: net.w2.iter().map(|&v| qi16(v)).collect(),
            b2: (net.b2 * (QIN * QW * QW) as f32).round() as i32,
        }
    }

    #[inline]
    fn desc(&self, g: &GameDef, t: TypeId, side: Side) -> &[i32; D] {
        let _ = g;
        &self.desc_q[t as usize * self.nsides + side as usize]
    }

    /// Integer positional basis × QIN, exact for board dims (rationals with
    /// small denominators rounded once, identically on every platform).
    fn phi_q(g: &GameDef, sq: u16, side: Side) -> [i32; P] {
        let (x, y) = g.board.xy(sq);
        let w = (g.board.w - 1).max(1) as i32;
        let h = (g.board.h - 1).max(1) as i32;
        let xn = (x as i32 * QIN) / w;
        let yn = if side == 0 {
            (y as i32 * QIN) / h
        } else {
            QIN - (y as i32 * QIN) / h
        };
        let cx = (xn - QIN / 2).abs() * 2;
        [QIN, xn, yn, cx]
    }

    /// Bit-exact integer forward pass: value in centipawns from stm's view.
    /// i64 accumulation, defined shifts — same output on every machine.
    pub fn eval(&self, g: &GameDef, pos: &Position) -> i32 {
        // Accumulator: features × QIN² (descriptor × φ).
        let mut acc = [0i64; IN];
        for p in &pos.pieces {
            let Loc::Board(sq) = p.loc else { continue };
            let d = self.desc(g, p.t, p.side);
            let ph = Self::phi_q(g, sq, p.side);
            let base = if p.side == pos.stm { 0 } else { F };
            for i in 0..D {
                let di = d[i] as i64;
                for j in 0..P {
                    acc[base + i * P + j] += di * ph[j] as i64;
                }
            }
        }
        // Hidden: Σ acc(×QIN²) · w1(×QW) → ×QIN²·QW; shift to ×QIN·QW.
        let mut v: i64 = self.b2 as i64;
        for j in 0..H {
            let mut a: i64 = self.b1[j] as i64 * QIN as i64; // ×QIN²·QW
            for i in 0..IN {
                if acc[i] != 0 {
                    a += acc[i] * self.w1[i * H + j] as i64;
                }
            }
            let h = (a >> 6).max(0); // ReLU, ×QIN·QW  (QIN=64 ⇒ >>6)
            v += h * self.w2[j] as i64; // ×QIN·QW·QW
        }
        // v ×QIN·QW·QW → pawns×100: v / (QIN·QW·QW) × 100.
        ((v * 100) >> 18) as i32 // QIN·QW·QW = 2^18
    }
}

// ---------------------------------------------------------------------------
// Training driver (performance grade, §9)
// ---------------------------------------------------------------------------

pub struct TrainReport {
    pub samples: usize,
    pub first_loss: f32,
    pub last_loss: f32,
}

/// Generate self-play samples and train the float net. Deterministic per
/// seed on one machine (performance-grade reproducibility: seed + log).
pub fn train_from_selfplay(
    g: &GameDef,
    net: &mut FloatNet,
    material_for_play: &[i32],
    games: u32,
    depth: i32,
    epochs: u32,
    lr: f32,
    seed: u64,
) -> TrainReport {
    use crate::eval::Eval;
    use crate::movegen::legal_moves;
    use crate::search::Searcher;
    use crate::selfplay::Outcome;

    // Collect (position, outcome-from-side0) samples.
    let mut samples: Vec<(Position, f32)> = Vec::new();
    for gi in 0..games {
        let mut s0 = Searcher::new(g, Eval::new(material_for_play.to_vec()));
        let mut s1 = Searcher::new(g, Eval::new(material_for_play.to_vec()));
        let mut rng = Rng::new(seed + gi as u64);
        let mut pos = Position::startpos(g);
        let mut snaps: Vec<Position> = Vec::new();
        let mut outcome = 0.5f32;
        let mut history: Vec<u64> = Vec::new();
        for ply in 0..160 {
            let moves = legal_moves(g, &mut pos);
            if moves.is_empty() {
                let stm = pos.stm;
                let in_check = pos.royal_attacked(g, stm);
                let loss = in_check
                    || g.policy.stalemate == crate::game::StalematePolicy::Loss;
                outcome = if !loss {
                    0.5
                } else if stm == 0 {
                    0.0
                } else {
                    1.0
                };
                break;
            }
            let mv = if ply < 6 {
                moves[rng.below(moves.len())]
            } else {
                let s: &mut Searcher = if pos.stm == 0 { &mut s0 } else { &mut s1 };
                s.history = history.clone();
                s.search(g, &mut pos, depth, 30_000).best.unwrap_or(moves[0])
            };
            pos.make(g, &mv);
            history.push(pos.hash);
            if history.iter().filter(|&&h| h == pos.hash).count() >= 3 {
                break; // draw, outcome stays 0.5
            }
            if ply >= 6 && ply % 3 == 0 {
                snaps.push(pos.clone());
            }
        }
        let out = Outcome::Draw; // silence unused-import pattern
        let _ = out;
        for sp in snaps {
            samples.push((sp, outcome));
        }
    }

    // SGD epochs (shuffled deterministically).
    let mut order: Vec<usize> = (0..samples.len()).collect();
    let mut rng = Rng::new(seed ^ 0xA11CE);
    let (mut first, mut last) = (0f32, 0f32);
    for e in 0..epochs {
        rng.shuffle(&mut order);
        let mut sum = 0f32;
        for &i in &order {
            let (pos, z0) = &samples[i];
            // Outcome from the sample's side-to-move perspective.
            let z = if pos.stm == 0 { *z0 } else { 1.0 - *z0 };
            sum += net.sgd_step(g, pos, z, lr);
        }
        let avg = sum / samples.len().max(1) as f32;
        if e == 0 {
            first = avg;
        }
        last = avg;
    }
    TrainReport { samples: samples.len(), first_loss: first, last_loss: last }
}

/// SRW-content training (STATUS scale rung 3): the same recipe as
/// `train_from_selfplay`, but every game draws a fresh sampled robot
/// GameDef from the full Appendix-B vocabulary (`random_robot_army`), with
/// the linear cost Eval as the teacher on both sides. The descriptors are
/// Bit-derived (§7.4), so one net trains across all the sampled defs and
/// generalizes to armies it never saw. Deterministic per seed.
pub fn train_srw_from_selfplay(
    net: &mut FloatNet,
    weights: &crate::cost::CostWeights,
    budget: f64,
    games: u32,
    depth: i32,
    max_plies: u32,
    epochs: u32,
    lr: f32,
    seed: u64,
) -> TrainReport {
    use crate::eval::Eval;
    use crate::movegen::legal_moves;
    use crate::search::Searcher;
    use crate::selfplay::random_robot_army;

    // Collect (def index, position, outcome-from-side0) samples; each
    // sample keeps its GameDef because descriptors are per-def.
    let mut defs: Vec<GameDef> = Vec::new();
    let mut samples: Vec<(usize, Position, f32)> = Vec::new();
    for gi in 0..games {
        let mut rng = Rng::new(seed + gi as u64);
        let (g, gen) = random_robot_army(budget, &mut rng, weights);
        // Material straight from the sampled costs (royal 0): the teacher.
        let mut material = vec![0i32; g.types.len()];
        for p in &gen {
            material[p.type_id as usize] = (p.cost * 100.0).round() as i32;
        }
        let mut s0 = Searcher::new(&g, Eval::new(material.clone()));
        let mut s1 = Searcher::new(&g, Eval::new(material));
        let mut pos = Position::startpos(&g);
        let mut snaps: Vec<Position> = Vec::new();
        let mut outcome = 0.5f32;
        let mut history: Vec<u64> = Vec::new();
        // The value-target horizon: games still undecided at the cap
        // label 0.5, so a horizon past the old 160 (one of the promotion
        // recipe's named levers) turns slow grinds into real outcomes.
        for ply in 0..max_plies {
            let moves = legal_moves(&g, &mut pos);
            if moves.is_empty() {
                let stm = pos.stm;
                let in_check = pos.royal_attacked(&g, stm);
                let loss = in_check
                    || g.policy.stalemate == crate::game::StalematePolicy::Loss;
                outcome = if !loss {
                    0.5
                } else if stm == 0 {
                    0.0
                } else {
                    1.0
                };
                break;
            }
            let mv = if ply < 6 {
                moves[rng.below(moves.len())]
            } else {
                let s: &mut Searcher = if pos.stm == 0 { &mut s0 } else { &mut s1 };
                s.history = history.clone();
                s.search(&g, &mut pos, depth, 30_000).best.unwrap_or(moves[0])
            };
            pos.make(&g, &mv);
            history.push(pos.hash);
            if history.iter().filter(|&&h| h == pos.hash).count() >= 3 {
                break; // draw, outcome stays 0.5
            }
            if ply >= 6 && ply % 3 == 0 {
                snaps.push(pos.clone());
            }
        }
        let di = defs.len();
        defs.push(g);
        for sp in snaps {
            samples.push((di, sp, outcome));
        }
    }

    // SGD epochs (shuffled deterministically), each sample against its def.
    let mut order: Vec<usize> = (0..samples.len()).collect();
    let mut rng = Rng::new(seed ^ 0xA11CE);
    let (mut first, mut last) = (0f32, 0f32);
    for e in 0..epochs {
        rng.shuffle(&mut order);
        let mut sum = 0f32;
        for &i in &order {
            let (di, pos, z0) = &samples[i];
            let z = if pos.stm == 0 { *z0 } else { 1.0 - *z0 };
            sum += net.sgd_step(&defs[*di], pos, z, lr);
        }
        let avg = sum / samples.len().max(1) as f32;
        if e == 0 {
            first = avg;
        }
        last = avg;
    }
    TrainReport { samples: samples.len(), first_loss: first, last_loss: last }
}
