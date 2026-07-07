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
