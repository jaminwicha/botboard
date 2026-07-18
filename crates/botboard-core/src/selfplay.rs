//! Self-play and the training loops (§9; Training Spec §4, §7).
//!
//! Implements, in deterministic miniature: the game runner with full-state
//! repetition + ply-cap termination; self-play match play; the cost-model
//! correction loop (Texel-style logistic regression folding realized value
//! back into per-type material, Training Spec §7's measure→correct step);
//! and random army generation from the costed prior (the generate step).
//! The heavyweight CVPN/GT-CFR/R-NaD farm is the scale-up of these loops.

use crate::cost::{cost_prior, CostWeights};
use crate::eval::Eval;
use crate::game::{GameDef, Side, StalematePolicy, TypeId};
use crate::ladder::{choose_move, LadderConfig};
use crate::belief::Belief;
use crate::movegen::{legal_moves};
use crate::position::{Loc, Position};
use crate::rng::Rng;
use crate::search::Searcher;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Win(Side),
    Draw,
}

pub struct GameRecord {
    pub outcome: Outcome,
    pub plies: u32,
    /// Sampled (material-count-per-type diff vectors, from side-0 view).
    pub samples: Vec<Vec<i32>>,
}

fn material_counts(g: &GameDef, pos: &Position) -> Vec<i32> {
    let mut v = vec![0i32; g.types.len()];
    for p in &pos.pieces {
        let sign = if p.side == 0 { 1 } else { -1 };
        match p.loc {
            Loc::Board(_) => v[p.t as usize] += sign,
            Loc::Hand(s) => v[p.t as usize] += if s == 0 { 1 } else { -1 },
            Loc::Dead => {}
        }
    }
    v
}

/// Play one seeded self-play game: `opening_random` random plies for
/// diversity, then both sides on the given searchers. Termination: mate/
/// stalemate via the policy layer, threefold repetition of the full state
/// key, or the ply cap (§5's termination obligations).
pub fn play_game(
    g: &GameDef,
    s0: &mut Searcher,
    s1: &mut Searcher,
    depth: (i32, i32),
    node_budget: (u64, u64),
    seed: u64,
    opening_random: u32,
    max_plies: u32,
    sample_every: u32,
) -> GameRecord {
    let mut rng = Rng::new(seed);
    let mut pos = Position::startpos(g);
    let mut history: Vec<u64> = Vec::new();
    let mut samples = Vec::new();
    let mut plies = 0;

    loop {
        let moves = {
            let searcher: &mut Searcher = if pos.stm == 0 { &mut *s0 } else { &mut *s1 };
            let _ = &searcher;
            legal_moves(g, &mut pos)
        };
        if moves.is_empty() {
            let stm = pos.stm;
            let enemy = (stm + 1) % g.sides;
            let in_check = pos.royal_attacked(g, stm);
            let outcome = if in_check || g.policy.stalemate == StalematePolicy::Loss {
                Outcome::Win(enemy)
            } else {
                Outcome::Draw
            };
            return GameRecord { outcome, plies, samples };
        }

        let mv = if plies < opening_random {
            moves[rng.below(moves.len())]
        } else {
            let (searcher, d, nb): (&mut Searcher, i32, u64) = if pos.stm == 0 {
                (&mut *s0, depth.0, node_budget.0)
            } else {
                (&mut *s1, depth.1, node_budget.1)
            };
            searcher.history = history.clone();
            searcher.search(g, &mut pos, d, nb).best.unwrap_or(moves[0])
        };

        let _u = pos.make(g, &mv);
        plies += 1;
        let key = pos.hash;
        history.push(key);
        if history.iter().filter(|&&h| h == key).count() >= 3 {
            return GameRecord { outcome: Outcome::Draw, plies, samples };
        }
        if plies >= max_plies {
            return GameRecord { outcome: Outcome::Draw, plies, samples };
        }
        if sample_every > 0 && plies % sample_every == 0 && plies > opening_random {
            samples.push(material_counts(g, &pos));
        }
    }
}

/// W/L/D for side 0 over `n` seeded games.
pub fn play_match(
    g: &GameDef,
    eval0: &Eval,
    eval1: &Eval,
    depth: i32,
    node_budget: u64,
    n: u32,
    seed: u64,
) -> (u32, u32, u32) {
    let (mut w, mut l, mut d) = (0, 0, 0);
    for i in 0..n {
        // Alternate colors: even games eval0 plays side 0.
        let (ea, eb) = if i % 2 == 0 { (eval0, eval1) } else { (eval1, eval0) };
        let mut sa = Searcher::new(g, ea.clone());
        let mut sb = Searcher::new(g, eb.clone());
        let rec = play_game(
            g,
            &mut sa,
            &mut sb,
            (depth, depth),
            (node_budget, node_budget),
            seed + i as u64,
            6,
            400,
            0,
        );
        match rec.outcome {
            Outcome::Win(0) => {
                if i % 2 == 0 {
                    w += 1
                } else {
                    l += 1
                }
            }
            Outcome::Win(_) => {
                if i % 2 == 0 {
                    l += 1
                } else {
                    w += 1
                }
            }
            Outcome::Draw => d += 1,
        }
    }
    (w, l, d)
}

/// The correction loop (§4.3, Training Spec §7): logistic regression of
/// game outcomes on material-count diffs, warm-started at the prior,
/// anchored by rescaling to the anchor type's value. Returns corrected
/// per-type values (centipawns).
pub fn correct_values(
    g: &GameDef,
    prior_material: &[i32],
    records: &[GameRecord],
    anchor: (TypeId, f64),
    epochs: u32,
    lr: f64,
) -> Vec<i32> {
    let ntypes = g.types.len();
    let mut w: Vec<f64> = prior_material.iter().map(|&m| m as f64 / 100.0).collect();
    let k = 0.7; // logistic scale in pawn units

    for _ in 0..epochs {
        for rec in records {
            let y = match rec.outcome {
                Outcome::Win(0) => 1.0,
                Outcome::Win(_) => 0.0,
                Outcome::Draw => 0.5,
            };
            for x in &rec.samples {
                let s: f64 = (0..ntypes).map(|t| w[t] * x[t] as f64).sum();
                let p = 1.0 / (1.0 + (-k * s).exp());
                let grad = (p - y) * k * p * (1.0 - p);
                for t in 0..ntypes {
                    if g.types[t].royal {
                        continue;
                    }
                    w[t] -= lr * grad * x[t] as f64;
                    w[t] = w[t].max(0.1);
                }
            }
        }
    }
    // Anchor the scale (§4.2): pin the anchor type to its reference value.
    let (at, av) = anchor;
    let scale = av / w[at as usize].max(1e-6);
    w.iter()
        .enumerate()
        .map(|(t, &v)| {
            if g.types[t].royal {
                0
            } else {
                (v * scale * 100.0).round() as i32
            }
        })
        .collect()
}

/// One generated army member: a Bit-set piece with its prior cost.
pub struct GeneratedPiece {
    pub bits_desc: String,
    pub type_id: TypeId,
    pub cost: f64,
}

/// The generate step (Training Spec §7): random Bit-set pieces from a
/// vocabulary, priced by the prior, packed under an army budget.
pub fn random_army(
    g_budget: f64,
    rng: &mut Rng,
    weights: &CostWeights,
) -> (GameDef, Vec<GeneratedPiece>) {
    use crate::bits::{Mode, MoveBit};
    use crate::game::*;
    use crate::geometry::DirFilter;

    let board = BoardDef { w: 8, h: 8 };
    let mut last_rank = Zone::new(2);
    last_rank.add_rect(0, &board, 0, 7, 7, 7);
    last_rank.add_rect(1, &board, 0, 0, 7, 0);

    // Vocabulary sampler: 1–2 random Bits per piece.
    let mut types = vec![PieceTypeDef::new(
        "king",
        'K',
        vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)],
    )
    .royal()];
    let glyphs = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H'];
    let nbits_choices = [1usize, 2];
    for i in 0..4usize {
        let mut bits = Vec::new();
        for _ in 0..nbits_choices[rng.below(2)] {
            let (m, n) = [(0i8, 1i8), (1, 1), (1, 2), (0, 2), (2, 2), (1, 3), (2, 3)]
                [rng.below(7)];
            let mut b = if rng.unit_f64() < 0.5 {
                MoveBit::leaper(m, n)
            } else {
                MoveBit::rider(m, n)
            };
            if rng.unit_f64() < 0.25 {
                b = b.dirs(DirFilter::Forward);
            }
            if rng.unit_f64() < 0.15 {
                b = b.mode(if rng.unit_f64() < 0.5 { Mode::Move } else { Mode::Capture });
            }
            bits.push(b);
        }
        types.push(PieceTypeDef::new("gen", glyphs[i], bits));
    }

    let policy = Policy {
        stalemate: StalematePolicy::Draw,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };

    // Provisional def to price the generated types.
    let gdef =
        GameDef::new(board, 2, types.clone(), vec![last_rank.clone()], policy.clone(), vec![]);
    let mut gen: Vec<GeneratedPiece> = (1..types.len())
        .map(|t| GeneratedPiece {
            bits_desc: format!("{:?}", types[t].bits),
            type_id: t as TypeId,
            cost: cost_prior(&gdef, t as TypeId, weights),
        })
        .collect();

    // Pack a symmetric army under budget: kings + affordable pieces.
    let mut start = vec![(0 as TypeId, 0 as Side, 4u8, 0u8), (0, 1, 4, 7)];
    let mut spent = 0.0;
    let cols = [3u8, 5, 2, 6, 1, 7, 0];
    let mut ci = 0;
    let mut guard = 0;
    while ci < cols.len() && guard < 64 {
        guard += 1;
        let pick = rng.below(gen.len());
        if spent + gen[pick].cost > g_budget {
            // Try any affordable piece before giving up this slot.
            match gen.iter().position(|p| spent + p.cost <= g_budget) {
                Some(j) => {
                    spent += gen[j].cost;
                    start.push((gen[j].type_id, 0, cols[ci], 0));
                    start.push((gen[j].type_id, 1, cols[ci], 7));
                    ci += 1;
                }
                None => break,
            }
        } else {
            spent += gen[pick].cost;
            start.push((gen[pick].type_id, 0, cols[ci], 0));
            start.push((gen[pick].type_id, 1, cols[ci], 7));
            ci += 1;
        }
    }

    let def = GameDef::new(board, 2, types, vec![last_rank], policy, start);
    gen.sort_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap());
    (def, gen)
}

/// SRW-content generate step (STATUS scale rung 3; SRW Appendix B): sample
/// the full robot vocabulary — armor 1–3, overclock, stealth, flight, EMP
/// auras, hologram decoys (movement-only, 1 HP), and Axis-B ability Bits
/// (heal / wall / pit / laser ±pierce ±retreat / mine / resurrect / hack /
/// swap / push) — at roughly the SRW clan palettes' rates (abilities ~30%,
/// flags 5–20%), priced by the cost prior and packed symmetric under budget
/// like `random_army`. Batch-3 movement vocabulary at low rates: hopper Bits
/// with the three landing modes (~8% of movement Bits) and range-limited
/// riders (max_steps 2–4 on ~10% of riders). Deterministic per rng seed.
pub fn random_robot_army(
    g_budget: f64,
    rng: &mut Rng,
    weights: &CostWeights,
) -> (GameDef, Vec<GeneratedPiece>) {
    use crate::bits::{AbilityBit, HopMode, Mode, MoveBit};
    use crate::game::*;
    use crate::geometry::DirFilter;

    let board = BoardDef { w: 8, h: 8 };
    let mut types = vec![PieceTypeDef::new(
        "controller",
        'K',
        vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)],
    )
    .royal()];
    let glyphs = ['A', 'B', 'C', 'D'];
    for i in 0..4usize {
        // Movement: 1–2 Bits from the shared vocabulary.
        let mut bits = Vec::new();
        for _ in 0..[1usize, 2][rng.below(2)] {
            let (m, n) = [(0i8, 1i8), (1, 1), (1, 2), (0, 2), (2, 2), (1, 3), (2, 3)]
                [rng.below(7)];
            let geom = rng.unit_f64();
            let mut b = if geom < 0.46 {
                MoveBit::leaper(m, n)
            } else if geom < 0.92 {
                let mut r = MoveBit::rider(m, n);
                if rng.unit_f64() < 0.10 {
                    // Range-limited rider (the R4 atom): 2–4 steps.
                    r = r.max_steps(2 + rng.below(3) as u8);
                }
                r
            } else {
                // Hopper family, one landing mode each (batch 3).
                let hop = MoveBit::hopper(m, n);
                match rng.below(3) {
                    0 => hop, // cannon-at-range (capture-only default)
                    1 => hop.landing(HopMode::BeyondScreen), // grasshopper
                    _ => hop.landing(HopMode::Locust),
                }
            };
            if rng.unit_f64() < 0.25 {
                b = b.dirs(DirFilter::Forward);
            }
            if rng.unit_f64() < 0.15 {
                b = b.mode(if rng.unit_f64() < 0.5 { Mode::Move } else { Mode::Capture });
            }
            bits.push(b);
        }
        let mut def = PieceTypeDef::new("bot", glyphs[i], bits);
        if rng.unit_f64() < 0.05 {
            // Hologram decoy: movement-only bluff at 1 HP, nothing else.
            types.push(def.hologram());
            continue;
        }
        let hp = {
            let r = rng.unit_f64();
            if r < 0.55 {
                1
            } else if r < 0.85 {
                2
            } else {
                3
            }
        };
        if hp > 1 {
            def = def.hp(hp);
        }
        if rng.unit_f64() < 0.10 {
            def = def.overclock();
        }
        if rng.unit_f64() < 0.10 {
            def = def.stealth();
        }
        if rng.unit_f64() < 0.10 {
            def = def.flight();
        }
        if rng.unit_f64() < 0.05 {
            def = def.emp(2);
        }
        if rng.unit_f64() < 0.30 {
            let range12 = 1 + rng.below(2) as u8;
            let ability = match rng.below(9) {
                0 => AbilityBit::Heal { amount: 1, range: range12 },
                1 => AbilityBit::CreateWall { range: range12 },
                2 => AbilityBit::DigPit { range: range12 },
                3 => AbilityBit::Laser {
                    range: 2 + rng.below(2) as u8,
                    retreat: rng.unit_f64() < 0.5,
                    pierce: rng.unit_f64() < 0.25,
                },
                4 => AbilityBit::MineLayer { range: 1 },
                5 => AbilityBit::Resurrect { range: 2 },
                6 => AbilityBit::Hack { range: range12 },
                // Batch-2 abilities join the palette.
                7 => AbilityBit::Swap { range: range12 },
                _ => AbilityBit::Push { range: range12 },
            };
            def = def.abilities(vec![ability]);
        }
        types.push(def);
    }

    let policy = Policy {
        stalemate: StalematePolicy::Loss, // a stuck robot army powers down
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };

    // Provisional def to price the generated types.
    let gdef = GameDef::new(board, 2, types.clone(), Vec::new(), policy.clone(), vec![]);
    let mut gen: Vec<GeneratedPiece> = (1..types.len())
        .map(|t| GeneratedPiece {
            bits_desc: format!("{:?} {:?}", types[t].bits, types[t].abilities),
            type_id: t as TypeId,
            cost: cost_prior(&gdef, t as TypeId, weights),
        })
        .collect();

    // Pack a symmetric (mirrored) army under budget: controllers + pieces.
    let mut start = vec![(0 as TypeId, 0 as Side, 4u8, 0u8), (0, 1, 4, 7)];
    let mut spent = 0.0;
    let cols = [3u8, 5, 2, 6, 1, 7, 0];
    let mut ci = 0;
    let mut guard = 0;
    while ci < cols.len() && guard < 64 {
        guard += 1;
        let pick = rng.below(gen.len());
        if spent + gen[pick].cost > g_budget {
            match gen.iter().position(|p| spent + p.cost <= g_budget) {
                Some(j) => {
                    spent += gen[j].cost;
                    start.push((gen[j].type_id, 0, cols[ci], 0));
                    start.push((gen[j].type_id, 1, cols[ci], 7));
                    ci += 1;
                }
                None => break,
            }
        } else {
            spent += gen[pick].cost;
            start.push((gen[pick].type_id, 0, cols[ci], 0));
            start.push((gen[pick].type_id, 1, cols[ci], 7));
            ci += 1;
        }
    }

    let def = GameDef::new(board, 2, types, Vec::new(), policy, start);
    gen.sort_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap());
    (def, gen)
}

/// The self-play farm's actor pool (Training Spec §10) in-process:
/// deterministic per seed regardless of thread count — each seeded game is
/// independent, so records assemble in seed order. Scales the correction
/// loop, league round-robins, and net-training sample generation.
pub fn parallel_selfplay(
    g: &GameDef,
    eval: &Eval,
    n_games: u32,
    threads: usize,
    depth: i32,
    node_budget: u64,
    seed: u64,
    max_plies: u32,
    sample_every: u32,
) -> Vec<GameRecord> {
    let threads = threads.max(1);
    let mut slots: Vec<Option<GameRecord>> = Vec::new();
    slots.resize_with(n_games as usize, || None);
    let slots_ref = std::sync::Mutex::new(&mut slots);
    let next = std::sync::atomic::AtomicU32::new(0);

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= n_games {
                    break;
                }
                let mut s0 = Searcher::new(g, eval.clone());
                let mut s1 = Searcher::new(g, eval.clone());
                let rec = play_game(
                    g,
                    &mut s0,
                    &mut s1,
                    (depth, depth),
                    (node_budget, node_budget),
                    seed + i as u64,
                    6,
                    max_plies,
                    sample_every,
                );
                slots_ref.lock().unwrap()[i as usize] = Some(rec);
            });
        }
    });
    slots.into_iter().map(|r| r.expect("all games played")).collect()
}

/// Hidden-information self-play through the ladder (§8.6 discipline: each
/// side chooses on its own belief; ground truth never leaks).
pub fn play_hidden_game(
    g: &GameDef,
    searcher: &mut Searcher,
    cfg: &LadderConfig,
    seed: u64,
    max_plies: u32,
) -> (Outcome, Vec<crate::ladder::Rung>) {
    let mut rng = Rng::new(seed);
    let mut pos = Position::startpos(g);
    let mut beliefs = [Belief::cold_open(g, &pos, 0), Belief::cold_open(g, &pos, 1)];
    let mut rungs = Vec::new();
    let mut history: Vec<u64> = Vec::new();
    let mut plies = 0;

    loop {
        if legal_moves(g, &mut pos).is_empty() {
            let stm = pos.stm;
            let enemy = (stm + 1) % g.sides;
            let in_check = pos.royal_attacked(g, stm);
            let outcome = if in_check || g.policy.stalemate == StalematePolicy::Loss {
                Outcome::Win(enemy)
            } else {
                Outcome::Draw
            };
            return (outcome, rungs);
        }
        let stm = pos.stm as usize;
        let pre = pos.clone();
        let Some((mv, rung)) =
            choose_move(g, &mut pos, &beliefs[stm], searcher, cfg, &mut rng)
        else {
            return (Outcome::Draw, rungs);
        };
        rungs.push(rung);
        pos.make(g, &mv);
        plies += 1;
        // Both observers update on the public action (§3.4: one observed
        // action per turn keeps this well-posed).
        beliefs[0].observe(g, &pre, &mv);
        beliefs[1].observe(g, &pre, &mv);
        let key = pos.hash;
        history.push(key);
        if history.iter().filter(|&&h| h == key).count() >= 3 || plies >= max_plies {
            return (Outcome::Draw, rungs);
        }
    }
}
