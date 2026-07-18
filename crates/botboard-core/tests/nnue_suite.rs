//! The learned evaluator's obligations (§7.4, §10.6):
//! training reduces loss; the deterministic grade is bit-exact and
//! deterministic; **quantization parity** (the named §10.6 test) holds;
//! Bit-derived descriptors generalize to unseen procedural pieces.

use botboard_core::cost::{fit_weights, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::movegen::legal_moves;
use botboard_core::nnue::{descriptor, train_from_selfplay, FloatNet, QuantNet, D, F, H, P};
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

    // And on SRW-content armies (scale rung 3): the v2 descriptors cover
    // the ability vocabulary, so a sampled robot army — stealth, lasers,
    // holograms and all — evaluates through the same weights.
    let mut rng = Rng::new(4242);
    let (g3, _) = botboard_core::selfplay::random_robot_army(14.0, &mut rng, &w);
    let q3 = QuantNet::from_float(&g3, &net);
    let pos3 = Position::startpos(&g3);
    let v3 = q3.eval(&g3, &pos3);
    assert!(v3.abs() < 3000, "sane range on robot armies: {v3}");
}

/// Descriptor v2 (§7.4): the SRW ability vocabulary — per-kind ability
/// signals, Appendix-B flags, EMP radius — is visible to the net, and
/// every new dim quantizes exactly at ×64 (§10.6).
#[test]
fn descriptor_v2_covers_the_srw_vocabulary() {
    use botboard_core::bits::{AbilityBit, MoveBit};
    use botboard_core::game::{
        BoardDef, CaptureFate, GameDef, PassPolicy, PieceTypeDef, Policy,
        StalematePolicy, TurnPolicy,
    };
    let step = || vec![MoveBit::leaper(0, 1)];
    let types = vec![
        PieceTypeDef::new("ctrl", 'K', step()).royal(),
        PieceTypeDef::new("shade", 'S', step()).stealth(),
        PieceTypeDef::new("hover", 'H', step()).flight(),
        PieceTypeDef::new("decoy", 'Y', step()).hologram(),
        PieceTypeDef::new("pulse", 'E', step()).emp(2),
        PieceTypeDef::new("medic", 'M', step())
            .abilities(vec![AbilityBit::Heal { amount: 2, range: 1 }]),
        PieceTypeDef::new("lance", 'L', step()).abilities(vec![AbilityBit::Laser {
            range: 2,
            retreat: true,
            pierce: true,
        }]),
        PieceTypeDef::new("mason", 'B', step()).abilities(vec![
            AbilityBit::CreateWall { range: 1 },
            AbilityBit::DigPit { range: 1 },
        ]),
        PieceTypeDef::new("sapper", 'P', step())
            .abilities(vec![AbilityBit::MineLayer { range: 1 }]),
        PieceTypeDef::new("necro", 'N', step())
            .abilities(vec![AbilityBit::Resurrect { range: 2 }]),
        PieceTypeDef::new("hacker", 'X', step())
            .abilities(vec![AbilityBit::Hack { range: 1 }]),
    ];
    let g = GameDef::new(
        BoardDef { w: 8, h: 8 },
        2,
        types,
        Vec::new(),
        Policy {
            stalemate: StalematePolicy::Loss,
            capture_fate: CaptureFate::Destroy,
            turn: TurnPolicy::Alternate,
            pass: PassPolicy::Forbidden,
        },
        Vec::new(),
    );
    let d = |t: u8| descriptor(&g, t, 0);
    assert_eq!(d(0)[7], 1.0, "royal flag");
    assert_eq!(d(1)[16], 1.0, "stealth dim");
    assert_eq!(d(2)[17], 1.0, "flight dim");
    assert_eq!(d(3)[18], 1.0, "hologram dim");
    assert_eq!(d(4)[19], 1.0, "EMP radius 2 normalizes to 1");
    assert_eq!(d(5)[10], 1.0, "heal amount 2 normalizes to 1");
    assert_eq!(d(6)[11], 1.0, "laser range 2 + pierce bonus 2 → 4/4");
    assert_eq!(d(7)[12], 1.0, "wall + pit builder saturates");
    assert_eq!(d(8)[13], 1.0, "mine-layer dim");
    assert_eq!(d(9)[14], 1.0, "resurrect dim");
    assert_eq!(d(10)[15], 1.0, "hack dim");
    // A bare mover shows nothing in the vocabulary block…
    for i in 10..=19 {
        assert_eq!(d(1)[i], if i == 16 { 1.0 } else { 0.0 });
    }
    // …and every vocabulary dim is exactly representable ×64 (§10.6).
    for t in 0..g.types.len() as u8 {
        for (i, v) in d(t).iter().enumerate().skip(10) {
            let q = v * 64.0;
            assert_eq!(q, q.round(), "dim {i} of type {t} not exact ×64: {v}");
        }
    }
}

/// Training Spec §8 back-compat: BBNET001 checkpoints (12-dim descriptors)
/// load through the legacy remap — surviving rows land on their v2 feature
/// indices, new-vocabulary rows are zero, and the dropped bare ability-count
/// row (gradient-dead on all shipped chess checkpoints) is discarded. The
/// round trip against a v2 net with zeroed vocabulary rows must be exact.
#[test]
fn bbnet001_checkpoints_load_via_the_legacy_remap() {
    const D_V1: usize = 12;
    const F_V1: usize = D_V1 * P;

    // A v2 net that a v1 net could express: vocabulary rows zeroed.
    let mut a = FloatNet::new(3);
    for persp in 0..2 {
        for di in 10..(D - 1) {
            for j in 0..P {
                let idx = persp * F + di * P + j;
                a.w1[idx * H..(idx + 1) * H].fill(0.0);
            }
        }
    }

    // Its BBNET001 projection: dims 0–9 identity, dim 11 = bias (v2 D−1);
    // dim 10 (bare ability count) filled with junk the loader must drop.
    let mut w1v1 = vec![0f32; 2 * F_V1 * H];
    for persp in 0..2 {
        for di in 0..D_V1 {
            let ndi = match di {
                0..=9 => di,
                11 => D - 1,
                _ => continue,
            };
            for j in 0..P {
                let old = persp * F_V1 + di * P + j;
                let new = persp * F + ndi * P + j;
                w1v1[old * H..(old + 1) * H]
                    .copy_from_slice(&a.w1[new * H..(new + 1) * H]);
            }
        }
        for j in 0..P {
            let old = persp * F_V1 + 10 * P + j;
            w1v1[old * H..(old + 1) * H].fill(123.0);
        }
    }
    let mut bytes = b"BBNET001".to_vec();
    bytes.extend((2 * F_V1 as u32).to_le_bytes());
    bytes.extend((H as u32).to_le_bytes());
    for v in w1v1.iter().chain(a.b1.iter()).chain(a.w2.iter()) {
        bytes.extend(v.to_le_bytes());
    }
    bytes.extend(a.b2.to_le_bytes());

    let b = FloatNet::from_bytes(&bytes).expect("legacy BBNET001 must load");
    assert_eq!(a.w1, b.w1, "remapped + zero-padded w1 is exact");
    assert_eq!(a.b1, b.b1);
    assert_eq!(a.w2, b.w2);
    assert_eq!(a.b2, b.b2);

    // The v2 container round-trips bit-exact too.
    let c = FloatNet::from_bytes(&a.to_bytes()).expect("BBNET002 round trip");
    assert_eq!(a.w1, c.w1);
    assert_eq!(a.b2, c.b2);

    // And the deterministic grades agree everywhere the weights agree.
    let (g, _) = chess_setup();
    let qa = QuantNet::from_float(&g, &a);
    let qb = QuantNet::from_float(&g, &b);
    let pos = Position::startpos(&g);
    assert_eq!(qa.eval(&g, &pos), qb.eval(&g, &pos));
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
