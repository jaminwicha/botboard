//! Systems obligations closed in the fully-realized pass: codex rematch
//! warm-starts (§8.8), the synergy term's fitting loop (§4.1/§4.3), the
//! N-player FFA baseline ruling (§8.7), and the parallel actor pool
//! (Training Spec §10).

use botboard_core::belief::Belief;
use botboard_core::bits::MoveBit;
use botboard_core::codex::{belief_from_json, belief_to_json, warm_start};
use botboard_core::cost::{bit_categories, SynergyModel, K_CATS};
use botboard_core::cost::{fit_weights, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::ffa::{play_ffa, FfaOutcome};
use botboard_core::game::*;
use botboard_core::geometry::DirFilter;
use botboard_core::movegen::legal_moves;
use botboard_core::position::Position;
use botboard_core::selfplay::parallel_selfplay;
use botboard_core::variants;

fn chess_material() -> (GameDef, Vec<i32>) {
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

/// §8.8: reconnaissance persists — the rematch belief is strictly sharper
/// than a cold open, so the gate starts on a cheaper rung.
#[test]
fn codex_roundtrip_and_rematch_warm_start() {
    let (g, _) = chess_material();
    let pos = Position::startpos(&g);
    let mut b = Belief::cold_open(&g, &pos, 0);
    let cold_entropy = b.entropy();

    // First encounter: observe a few enemy moves.
    let mut p = pos.clone();
    p.set_stm(&g, 1);
    for mv_str in ["g8f6", "b8c6", "e7e5"] {
        let pre = p.clone();
        let mv = legal_moves(&g, &mut p)
            .into_iter()
            .find(|m| botboard_core::moves::move_str(&g, m) == *mv_str)
            .unwrap();
        p.make(&g, &mv);
        b.observe(&g, &pre, &mv);
        p.set_stm(&g, 1);
    }
    assert!(b.entropy() < cold_entropy);

    // Persist → parse → warm-start the rematch.
    let json = belief_to_json(&b);
    let codex = belief_from_json(&json).expect("parse codex");
    assert_eq!(codex.candidates, b.candidates, "lossless roundtrip");

    let rematch = warm_start(&g, &pos, 0, &codex);
    assert!(
        rematch.entropy() < cold_entropy,
        "rematch must start sharper: {} < {cold_entropy}",
        rematch.entropy()
    );
}

/// §4.1/§4.3: the pairwise synergy fitter recovers a planted interaction.
#[test]
fn synergy_fitter_recovers_planted_interaction() {
    // Plant: rider(cat 1) × forward-only(cat 3) interact at +1.5; all other
    // residuals are 0. The fitter must find that weight and stay near zero
    // elsewhere on identifiable pairs.
    let mut samples: Vec<([bool; K_CATS], f64)> = Vec::new();
    let mk = |idxs: &[usize]| {
        let mut x = [false; K_CATS];
        for &i in idxs {
            x[i] = true;
        }
        x
    };
    for _ in 0..50 {
        samples.push((mk(&[1, 3]), 1.5)); // rider+forward → the interaction
        samples.push((mk(&[1]), 0.0));
        samples.push((mk(&[3]), 0.0));
        samples.push((mk(&[0, 1]), 0.0)); // leaper+rider → nothing
        samples.push((mk(&[0]), 0.0));
    }
    let m = SynergyModel::fit(&samples, 200, 0.05);
    let planted = m.term(&mk(&[1, 3]));
    let control = m.term(&mk(&[0, 1]));
    assert!(
        (planted - 1.5).abs() < 0.2,
        "planted interaction recovered: {planted:.2}"
    );
    assert!(control.abs() < 0.2, "control pair near zero: {control:.2}");

    // Categories derive from real Bits: chess rook = rider, not leaper.
    let (g, _) = chess_material();
    let cats = bit_categories(&g, variants::chess::R);
    assert!(cats[1] && !cats[0]);
}

/// §8.7 committed baseline: a 3-player FFA runs under strict rotation to a
/// single survivor (or a counted draw), deterministically per seed.
#[test]
fn three_player_ffa_reaches_a_verdict() {
    const K: TypeId = 0;
    const R: TypeId = 1;
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        PieceTypeDef::new("rook", 'R', vec![MoveBit::rider(0, 1)]),
        PieceTypeDef::new(
            "guard",
            'G',
            vec![MoveBit::leaper(1, 1).dirs(DirFilter::All)],
        ),
    ];
    let policy = Policy {
        stalemate: StalematePolicy::Loss,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };
    // Three corners of a 7x7.
    let start = vec![
        (K, 0, 0, 0),
        (R, 0, 1, 0),
        (2, 0, 0, 1),
        (K, 1, 6, 0),
        (R, 1, 5, 0),
        (2, 1, 6, 1),
        (K, 2, 3, 6),
        (R, 2, 4, 6),
        (2, 2, 3, 5),
    ];
    let g = GameDef::new(BoardDef { w: 7, h: 7 }, 3, types, vec![], policy, start);
    let eval = Eval::new(vec![0, 500, 200]);

    let (o1, plies1) = play_ffa(&g, &eval, 42, 300);
    let (o2, _) = play_ffa(&g, &eval, 42, 300);
    assert_eq!(o1, o2, "seeded determinism in FFA");
    assert!(plies1 > 0);
    match o1 {
        FfaOutcome::Win(s) => assert!(s < 3),
        FfaOutcome::DrawAmong(n) => assert!(n >= 1 && n <= 3),
    }
}

/// Training Spec §10: the actor pool produces the same records as the
/// sequential runner for the same seeds, at any thread count.
#[test]
fn parallel_farm_matches_sequential() {
    use botboard_core::search::Searcher;
    use botboard_core::selfplay::play_game;
    let (g, material) = chess_material();
    let eval = Eval::new(material);

    let par = parallel_selfplay(&g, &eval, 6, 4, 2, 8_000, 500, 100, 10);
    for (i, rec) in par.iter().enumerate() {
        let mut s0 = Searcher::new(&g, eval.clone());
        let mut s1 = Searcher::new(&g, eval.clone());
        let seq = play_game(
            &g,
            &mut s0,
            &mut s1,
            (2, 2),
            (8_000, 8_000),
            500 + i as u64,
            6,
            100,
            10,
        );
        assert_eq!(rec.outcome, seq.outcome, "game {i} outcome");
        assert_eq!(rec.plies, seq.plies, "game {i} plies");
        assert_eq!(rec.samples, seq.samples, "game {i} samples");
    }
}
