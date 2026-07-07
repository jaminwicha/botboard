//! Training-reconciliation tests (Training Spec §3, §6, §11): the rung-3
//! net-sharing prototype and gate-threshold fitting.

use botboard_core::cost::{fit_weights, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::ladder::{LadderConfig, Rung};
use botboard_core::search::Searcher;
use botboard_core::training::{
    collect_gate_samples, fit_gate_thresholds, train_policy_selfplay, PolicyHead,
};
use botboard_core::movegen::legal_moves;
use botboard_core::position::Position;
use botboard_core::variants;

fn chess_eval() -> (botboard_core::game::GameDef, Eval) {
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
    (g, Eval::new(material))
}

/// The load-bearing prototype (Training Spec §13): NeuRD refinement of the
/// shared substrate must run, stay finite, and keep the policy sound
/// (probabilities normalized, deterministic per seed).
#[test]
fn neurd_refinement_runs_deterministically_on_shared_substrate() {
    let (g, eval) = chess_eval();
    let h1 = train_policy_selfplay(&g, &eval, 6, 40, 42);
    let h2 = train_policy_selfplay(&g, &eval, 6, 40, 42);
    assert_eq!(h1.w_eval.to_bits(), h2.w_eval.to_bits(), "seeded determinism");
    for i in 0..3 {
        assert_eq!(h1.bias[i].to_bits(), h2.bias[i].to_bits());
        assert!(h1.bias[i].is_finite());
    }
    assert!(h1.w_eval.is_finite() && h1.w_eval != 0.0);

    // The refined head still yields a proper distribution over legal moves.
    let mut pos = Position::startpos(&g);
    let moves = legal_moves(&g, &mut pos);
    let logits = h1.logits(&g, &mut pos, &moves, &eval);
    let probs = PolicyHead::softmax(&logits);
    let sum: f64 = probs.iter().sum();
    assert!((sum - 1.0).abs() < 1e-9);
    assert!(probs.iter().all(|p| *p >= 0.0));
}

/// Gate training (Training Spec §6): labels collected from rung agreement,
/// thresholds fitted with the escalate-when-uncertain bias.
#[test]
fn gate_labels_and_threshold_fit() {
    let (g, eval) = chess_eval();
    let mut searcher = Searcher::new(&g, eval);
    let cfg = LadderConfig {
        depth: 2,
        node_budget: 5_000,
        determinizations: 3,
        ismcts_iters: 60,
        ..Default::default()
    };
    let samples = collect_gate_samples(&g, &mut searcher, &cfg, 12, 7);
    assert!(!samples.is_empty(), "gate sampling produced labels");
    for s in &samples {
        assert!(s.entropy >= 0.0);
    }
    let (sharp, broad) = fit_gate_thresholds(&samples, &cfg);
    assert!(sharp >= 0.0 && broad >= sharp, "monotone thresholds: {sharp} ≤ {broad}");

    // Rung consistency (§11): every rung must produce a legal move from the
    // same shared searcher on a hidden position.
    use botboard_core::belief::Belief;
    use botboard_core::ladder::choose_move_with_rung;
    use botboard_core::rng::Rng;
    let mut pos = Position::startpos(&g);
    let belief = Belief::cold_open(&g, &pos, 0);
    let mut rng = Rng::new(3);
    let legal = legal_moves(&g, &mut pos);
    for rung in [Rung::R0PerfectInfo, Rung::R1Determinize, Rung::R2Ismcts, Rung::R3Policy] {
        let mv = choose_move_with_rung(&g, &mut pos, &belief, &mut searcher, &cfg, &mut rng, rung);
        let mv = mv.expect("every rung yields a move");
        if rung != Rung::R0PerfectInfo {
            assert!(legal.contains(&mv), "{rung:?} move must be arbiter-legal");
        }
    }
}
