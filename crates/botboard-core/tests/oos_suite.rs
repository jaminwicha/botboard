//! Rung-2 soundness properties (§8.2): OOS regret matching must converge
//! toward the right answers where they are checkable.

use botboard_core::belief::Belief;
use botboard_core::cost::{fit_weights, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::fen::chess_from_fen;
use botboard_core::moves::move_str;
use botboard_core::oos::{Oos, OosConfig};
use botboard_core::position::Position;
use botboard_core::rng::Rng;
use botboard_core::search::Searcher;
use botboard_core::variants;

fn chess_setup() -> (botboard_core::game::GameDef, Searcher) {
    use variants::chess as c;
    let g = c::game();
    let anchors = [
        Anchor { t: c::P, value: 1.0 },
        Anchor { t: c::N, value: 3.0 },
        Anchor { t: c::B, value: 3.0 },
        Anchor { t: c::R, value: 5.0 },
        Anchor { t: c::Q, value: 9.0 },
    ];
    let material = material_table(&g, &fit_weights(&g, &anchors));
    let searcher = Searcher::new(&g, Eval::new(material));
    (g, searcher)
}

/// With a point-mass belief (perfect information), OOS must concentrate on
/// a mate-in-one — the degenerate soundness check.
#[test]
fn oos_finds_forced_win_under_point_mass() {
    let (g, mut searcher) = chess_setup();
    let mut pos = chess_from_fen(
        &g,
        "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 0 1",
    )
    .unwrap();
    let belief = Belief::revealed(&pos, 0);
    let mut rng = Rng::new(1);
    let mut oos = Oos::new();
    let cfg = OosConfig { iterations: 300, depth: 2, ..Default::default() };
    let (mv, strat) = oos
        .choose(&g, &mut pos, &belief, &mut searcher, &cfg, &mut rng)
        .expect("move");
    assert_eq!(move_str(&g, &mv), "f3f7", "OOS must find Qxf7#; strat={strat:?}");
}

/// Under a cold-open belief OOS still returns an arbiter-legal move and a
/// proper distribution, deterministically per seed.
#[test]
fn oos_is_deterministic_and_legal_under_uncertainty() {
    let (g, mut searcher) = chess_setup();
    let mut run = |seed: u64| {
        let mut pos = Position::startpos(&g);
        let belief = Belief::cold_open(&g, &pos, 0);
        let mut rng = Rng::new(seed);
        let mut oos = Oos::new();
        let cfg = OosConfig { iterations: 120, depth: 3, ..Default::default() };
        let (mv, strat) = oos
            .choose(&g, &mut pos, &belief, &mut searcher, &cfg, &mut rng)
            .expect("move");
        let legal = botboard_core::movegen::legal_moves(&g, &mut pos);
        assert!(legal.contains(&mv));
        let sum: f64 = strat.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "strategy normalized: {sum}");
        (mv, strat)
    };
    let (m1, s1) = run(9);
    let (m2, s2) = run(9);
    assert_eq!(m1, m2, "seeded determinism");
    assert_eq!(s1, s2);
}
