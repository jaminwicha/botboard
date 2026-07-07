//! The learned evaluator's obligations (§7.4, §10.6):
//! training reduces loss; the deterministic grade is bit-exact and
//! deterministic; **quantization parity** (the named §10.6 test) holds;
//! Bit-derived descriptors generalize to unseen procedural pieces.

use botboard_core::cost::{fit_weights, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::movegen::legal_moves;
use botboard_core::nnue::{train_from_selfplay, FloatNet, QuantNet};
use botboard_core::position::Position;
use botboard_core::rng::Rng;
use botboard_core::search::Searcher;
use botboard_core::selfplay::random_army;
use botboard_core::variants;

fn chess_setup() -> (botboard_core::game::GameDef, Vec<i32>) {
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
    (g, material)
}

fn trained_net(g: &botboard_core::game::GameDef, material: &[i32]) -> FloatNet {
    let mut net = FloatNet::new(7);
    let report = train_from_selfplay(g, &mut net, material, 8, 2, 4, 0.01, 11);
    assert!(report.samples > 50, "enough samples: {}", report.samples);
    assert!(
        report.last_loss < report.first_loss,
        "training reduces loss: {} -> {}",
        report.first_loss,
        report.last_loss
    );
    net
}

#[test]
fn training_learns_and_quantized_grade_is_deterministic() {
    let (g, material) = chess_setup();
    let net = trained_net(&g, &material);
    let q1 = QuantNet::from_float(&g, &net);
    let q2 = QuantNet::from_float(&g, &net);

    // Deterministic grade (§10.6): bit-identical outputs, run to run.
    let mut pos = Position::startpos(&g);
    let mut rng = Rng::new(3);
    for _ in 0..30 {
        let moves = legal_moves(&g, &mut pos);
        if moves.is_empty() {
            break;
        }
        let mv = moves[rng.below(moves.len())];
        pos.make(&g, &mv);
        assert_eq!(q1.eval(&g, &pos), q2.eval(&g, &pos), "bit-exact inference");
    }

    // A trained net must at least see a large material deficit: startpos vs
    // startpos-minus-own-queen should order correctly for the mover.
    use botboard_core::fen::chess_from_fen;
    let full = chess_from_fen(&g, "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
        .unwrap();
    let noq = chess_from_fen(&g, "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNB1KBNR w KQkq - 0 1")
        .unwrap();
    assert!(
        q1.eval(&g, &full) > q1.eval(&g, &noq),
        "losing your queen must not look better: {} vs {}",
        q1.eval(&g, &full),
        q1.eval(&g, &noq)
    );
}

/// The named §10.6 obligation: the deterministic grade must track the float
/// net — value agreement within tolerance and high chosen-move agreement.
#[test]
fn quantization_parity() {
    let (g, material) = chess_setup();
    let net = trained_net(&g, &material);
    let q = QuantNet::from_float(&g, &net);

    let mut pos = Position::startpos(&g);
    let mut rng = Rng::new(5);
    let (mut checked, mut agree) = (0, 0);
    let mut max_dv = 0i32;
    for _ in 0..40 {
        let moves = legal_moves(&g, &mut pos);
        if moves.is_empty() {
            break;
        }
        // Value parity at this position.
        let fv = net.eval(&g, &pos) as i32;
        let qv = q.eval(&g, &pos);
        max_dv = max_dv.max((fv - qv).abs());

        // Chosen-move parity: 1-ply argmax under each grade.
        let pick = |use_q: bool, pos: &mut Position| -> usize {
            let mut best = (i64::MIN, 0usize);
            for (i, mv) in moves.iter().enumerate() {
                let u = pos.make(&g, mv);
                let v = if use_q {
                    -q.eval(&g, pos) as i64
                } else {
                    -(net.eval(&g, pos) as i64)
                };
                pos.unmake(&g, &u);
                if v > best.0 {
                    best = (v, i);
                }
            }
            best.1
        };
        let (pf, pq) = (pick(false, &mut pos), pick(true, &mut pos));
        checked += 1;
        if pf == pq {
            agree += 1;
        }
        let mv = moves[rng.below(moves.len())];
        pos.make(&g, &mv);
    }
    assert!(checked >= 20);
    let rate = agree as f64 / checked as f64;
    assert!(
        rate >= 0.9,
        "cross-grade chosen-move agreement {rate:.2} below tolerance ({agree}/{checked})"
    );
    assert!(max_dv <= 20, "value drift {max_dv}cp exceeds tolerance");
}

/// Departure B (§7.4): descriptors are functions of Bits, so a
/// never-before-seen procedural piece evaluates without any retraining.
#[test]
fn generalizes_to_unseen_procedural_pieces() {
    let (g, material) = chess_setup();
    let net = trained_net(&g, &material);

    // A fresh random-army game with novel Bit-set pieces: quantize the SAME
    // float weights against the new GameDef's descriptors and evaluate.
    let mut rng = Rng::new(99);
    let w = botboard_core::cost::CostWeights { w_move: 0.7, w_attack: 0.7 };
    let (g2, _) = random_army(20.0, &mut rng, &w);
    let q2 = QuantNet::from_float(&g2, &net);
    let mut pos = Position::startpos(&g2);
    let v0 = q2.eval(&g2, &pos);
    assert!(v0.abs() < 3000, "sane range on unseen pieces: {v0}");
    // And it must still register material loss: remove one side-0 piece.
    let victim = pos
        .pieces
        .iter()
        .position(|p| {
            p.side == 0 && !g2.types[p.t as usize].royal && matches!(p.loc, botboard_core::position::Loc::Board(_))
        })
        .unwrap();
    let botboard_core::position::Loc::Board(sq) = pos.pieces[victim].loc else { unreachable!() };
    pos.board[sq as usize] = -1;
    pos.pieces[victim].loc = botboard_core::position::Loc::Dead;
    pos.rehash(&g2);
    let v1 = q2.eval(&g2, &pos);
    assert!(
        v1 < v0,
        "side 0 to move must value losing a piece lower: {v0} -> {v1}"
    );
}

/// The net plugs in behind Eval and the searcher still finds tactics.
#[test]
fn searcher_runs_on_the_deterministic_net() {
    let (g, material) = chess_setup();
    let net = trained_net(&g, &material);
    let q = QuantNet::from_float(&g, &net);
    let mut s = Searcher::new(&g, Eval::with_net(material, q));
    let mut pos = Position::startpos(&g);
    let r = s.search(&g, &mut pos, 4, 500_000);
    assert!(r.best.is_some());
    assert!(r.depth >= 3);
}
