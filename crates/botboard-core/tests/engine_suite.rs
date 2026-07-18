//! Engine-layer tests: cost prior (Prototype 2's analytic half), rung-0
//! search sanity, belief substrate, ladder gating, and training-loop smoke.

use botboard_core::belief::Belief;
use botboard_core::cost::{fit_weights, cost_prior, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::fen::chess_from_fen;
use botboard_core::ladder::{gate, LadderConfig, Rung};
use botboard_core::league::{nash_averaging};
use botboard_core::movegen::legal_moves;
use botboard_core::position::Position;
use botboard_core::rng::Rng;
use botboard_core::search::{Searcher, MATE};
use botboard_core::selfplay::random_army;
use botboard_core::variants;

/// §4.2: with weights fitted on the anchors, the prior must recover the
/// classical value *ordering* and rough magnitudes emergently.
#[test]
fn cost_prior_recovers_classical_ordering() {
    use variants::chess as c;
    let g = c::game();
    let anchors = [
        Anchor { t: c::P, value: 1.0 },
        Anchor { t: c::N, value: 3.0 },
        Anchor { t: c::B, value: 3.0 },
        Anchor { t: c::R, value: 5.0 },
        Anchor { t: c::Q, value: 9.0 },
    ];
    let w = fit_weights(&g, &anchors);
    let cost = |t| cost_prior(&g, t, &w);
    let (p, n, b, r, q) = (cost(c::P), cost(c::N), cost(c::B), cost(c::R), cost(c::Q));
    println!("prior: P={p:.2} N={n:.2} B={b:.2} R={r:.2} Q={q:.2}");
    assert!(p < n && p < b, "pawn cheapest");
    assert!(n < r && b < r, "minors < rook");
    assert!(r < q, "rook < queen");
    assert!((2.0..=4.5).contains(&n), "knight ≈ 3, got {n:.2}");
    assert!((2.0..=4.5).contains(&b), "bishop ≈ 3, got {b:.2}");
    assert!((3.5..=6.5).contains(&r), "rook ≈ 5, got {r:.2}");
    assert!((7.0..=11.5).contains(&q), "queen ≈ 9, got {q:.2}");
}

fn chess_eval() -> (botboard_core::game::GameDef, Eval) {
    use variants::chess as c;
    let g = c::game();
    // Classical values suffice for search sanity tests.
    let mut material = vec![0i32; g.types.len()];
    material[c::Q as usize] = 900;
    material[c::R as usize] = 500;
    material[c::B as usize] = 310;
    material[c::N as usize] = 300;
    material[c::P as usize] = 100;
    (g, Eval::new(material))
}

#[test]
fn search_finds_mate_in_one() {
    let (g, e) = chess_eval();
    // Scholar's mate in hand: Qxf7#
    let mut pos = chess_from_fen(
        &g,
        "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 0 1",
    )
    .unwrap();
    let mut s = Searcher::new(&g, e);
    let r = s.search(&g, &mut pos, 3, u64::MAX);
    assert!(r.score >= MATE - 10, "mate score, got {}", r.score);
    let mv = r.best.unwrap();
    assert_eq!(botboard_core::moves::move_str(&g, &mv), "f3f7");
}

#[test]
fn search_finds_back_rank_mate_in_two() {
    let (g, e) = chess_eval();
    let mut pos = chess_from_fen(&g, "6k1/5ppp/8/8/8/8/R7/K6R w - - 0 1").unwrap();
    let mut s = Searcher::new(&g, e);
    let r = s.search(&g, &mut pos, 4, u64::MAX);
    assert!(r.score >= MATE - 10, "mate-in-2 score, got {}", r.score);
}

#[test]
fn belief_collapses_with_observation() {
    use variants::chess as c;
    let g = c::game();
    let pos = Position::startpos(&g);
    let mut b = Belief::cold_open(&g, &pos, 0);
    let h0 = b.entropy();
    assert!(h0 > 0.0);

    // Black plays g8f6 (a knight move): the mover's hypotheses must shrink
    // to types whose kernels generate it — the knight uniquely, here.
    let mut p2 = pos.clone();
    p2.set_stm(&g, 1);
    let mv = legal_moves(&g, &mut p2)
        .into_iter()
        .find(|m| botboard_core::moves::move_str(&g, m) == "g8f6")
        .unwrap();
    b.observe(&g, &p2, &mv);
    assert!(b.entropy() < h0, "entropy must drop after observation");
    let idx = p2.board[g.board.sq(6, 7) as usize] as usize;
    assert_eq!(b.candidates[idx], vec![c::N]);
}

#[test]
fn gate_spans_the_ladder() {
    use variants::chess as c;
    let g = c::game();
    let pos = Position::startpos(&g);
    let material = material_table(
        &g,
        &fit_weights(&g, &[Anchor { t: c::P, value: 1.0 }, Anchor { t: c::Q, value: 9.0 }]),
    );
    let cfg = LadderConfig::default();

    let revealed = Belief::revealed(&pos, 0);
    assert_eq!(gate(&revealed, &material, &cfg), Rung::R0PerfectInfo);

    let cold = Belief::cold_open(&g, &pos, 0);
    assert!(matches!(
        gate(&cold, &material, &cfg),
        Rung::R2Sound | Rung::R3Policy
    ));
}

#[test]
fn nash_averaging_handles_cycles() {
    // Rock-paper-scissors meta-game: the mixture must be ~uniform.
    let payoff = vec![
        vec![0.5, 1.0, 0.0],
        vec![0.0, 0.5, 1.0],
        vec![1.0, 0.0, 0.5],
    ];
    let (mix, rating) = nash_averaging(&payoff, 5000);
    for m in &mix {
        assert!((m - 1.0 / 3.0).abs() < 0.05, "RPS mixture ~uniform, got {mix:?}");
    }
    for r in &rating {
        assert!((r - 0.5).abs() < 0.05);
    }
}

#[test]
fn random_army_generation_prices_and_packs() {
    let mut rng = Rng::new(42);
    let w = botboard_core::cost::CostWeights { w_move: 0.35, w_attack: 0.35 };
    let (g, gen) = random_army(20.0, &mut rng, &w);
    assert!(!gen.is_empty());
    for p in &gen {
        assert!(p.cost >= 1.0, "cost floor (§4.1)");
    }
    // The generated game must be playable.
    let mut pos = Position::startpos(&g);
    assert!(!legal_moves(&g, &mut pos).is_empty());
}

/// The SRW-content sampler (STATUS scale rung 3): deterministic per seed,
/// budget-respecting, playable, and — across seeds — actually drawing from
/// the full Appendix-B vocabulary (flags AND ability Bits), not just
/// movement.
#[test]
fn random_robot_army_samples_the_full_vocabulary() {
    use botboard_core::selfplay::random_robot_army;
    let w = botboard_core::cost::CostWeights { w_move: 0.35, w_attack: 0.35 };
    let budget = 14.0;

    // Same seed ⇒ identical armies (start layout, costs, playable).
    let sample = |seed: u64| {
        let mut rng = Rng::new(seed);
        random_robot_army(budget, &mut rng, &w)
    };
    let (ga, gen_a) = sample(9);
    let (gb, gen_b) = sample(9);
    assert_eq!(ga.start, gb.start, "same seed ⇒ same army (§10.1)");
    for (a, b) in gen_a.iter().zip(gen_b.iter()) {
        assert_eq!(a.type_id, b.type_id);
        assert_eq!(a.cost, b.cost);
        assert!(a.cost >= 1.0, "cost floor (§4.1)");
    }
    let mut pos = Position::startpos(&ga);
    assert!(!legal_moves(&ga, &mut pos).is_empty());

    // Budget respected: placed non-royal pieces (one flank) fit under it.
    let cost_of = |gen: &[botboard_core::selfplay::GeneratedPiece], t| {
        gen.iter().find(|p| p.type_id == t).map(|p| p.cost).unwrap_or(0.0)
    };
    let spent: f64 = ga
        .start
        .iter()
        .filter(|&&(t, side, _, _)| side == 0 && t != 0)
        .map(|&(t, _, _, _)| cost_of(&gen_a, t))
        .sum();
    assert!(spent <= budget, "spent {spent} over budget {budget}");

    // Vocabulary coverage over many seeds: every flag and every ability
    // kind must appear at its palette-ish rate, and hologram decoys must
    // hold their movement-only 1-HP contract.
    use botboard_core::bits::{AbilityBit, Geometry};
    // hp>1 oc stealth flight holo emp heal laser wall/pit mine res+hack
    // swap+push hopper-bits capped-riders
    let mut seen = [0u32; 14];
    for seed in 0..80u64 {
        let (g, _) = sample(seed);
        for ty in &g.types[1..] {
            for b in &ty.bits {
                match b.geom {
                    // Batch 3: hopper landing modes at low rates…
                    Geometry::Hopper(..) => seen[12] += 1,
                    // …and range-limited riders (the R4 atom).
                    Geometry::Rider(..) if b.max_steps > 0 => seen[13] += 1,
                    _ => {}
                }
            }
            if ty.max_hp > 1 {
                seen[0] += 1;
            }
            if ty.overclock {
                seen[1] += 1;
            }
            if ty.stealth {
                seen[2] += 1;
            }
            if ty.flight {
                seen[3] += 1;
            }
            if ty.hologram {
                seen[4] += 1;
                assert_eq!(ty.max_hp, 1, "hologram decoys are 1 HP");
                assert!(ty.abilities.is_empty(), "hologram decoys are movement-only");
            }
            if ty.emp_aura > 0 {
                seen[5] += 1;
                assert_eq!(ty.emp_aura, 2);
            }
            for a in &ty.abilities {
                match a {
                    AbilityBit::Heal { .. } => seen[6] += 1,
                    AbilityBit::Laser { .. } => seen[7] += 1,
                    AbilityBit::CreateWall { .. } | AbilityBit::DigPit { .. } => {
                        seen[8] += 1
                    }
                    AbilityBit::MineLayer { .. } => seen[9] += 1,
                    AbilityBit::Resurrect { .. } | AbilityBit::Hack { .. } => {
                        seen[10] += 1
                    }
                    // Batch-2 abilities joined the palette in batch 3.
                    AbilityBit::Swap { .. } | AbilityBit::Push { .. } => seen[11] += 1,
                }
            }
        }
    }
    for (i, &n) in seen.iter().enumerate() {
        assert!(n > 0, "vocabulary slot {i} never sampled across 80 armies");
    }
}
