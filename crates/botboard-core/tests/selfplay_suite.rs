//! Self-play and training-loop tests (§9; Training Spec §4, §5, §7).
//! Fast smoke versions in CI; the heavier statistical runs are `#[ignore]`d
//! (`cargo test --release -- --ignored`).

use botboard_core::cost::{fit_weights, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::ladder::LadderConfig;
use botboard_core::league::{default_population, nash_averaging, profiles_json, round_robin};
use botboard_core::search::Searcher;
use botboard_core::selfplay::{correct_values, play_game, play_hidden_game, Outcome};
use botboard_core::variants;

fn chess_material() -> (botboard_core::game::GameDef, Vec<i32>) {
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
    let material = material_table(&g, &w);
    (g, material)
}

#[test]
fn selfplay_game_terminates_and_is_deterministic() {
    let (g, material) = chess_material();
    let run = |seed| {
        let mut s0 = Searcher::new(&g, Eval::new(material.clone()));
        let mut s1 = Searcher::new(&g, Eval::new(material.clone()));
        let rec =
            play_game(&g, &mut s0, &mut s1, (2, 2), (20_000, 20_000), seed, 4, 120, 8);
        (rec.outcome, rec.plies, rec.samples.len())
    };
    let a = run(7);
    let b = run(7);
    assert_eq!(a, b, "same seed ⇒ bit-identical game (determinism, §10.1)");
    let c = run(8);
    // Different seed usually differs; plies>0 always.
    assert!(a.1 > 0 && c.1 > 0);
}

#[test]
fn correction_loop_moves_values_toward_outcomes() {
    let (g, material) = chess_material();
    // Tiny record set: correction must run, stay positive, keep anchors.
    let mut records = Vec::new();
    for seed in 0..4u64 {
        let mut s0 = Searcher::new(&g, Eval::new(material.clone()));
        let mut s1 = Searcher::new(&g, Eval::new(material.clone()));
        records.push(play_game(
            &g,
            &mut s0,
            &mut s1,
            (2, 2),
            (10_000, 10_000),
            seed,
            6,
            100,
            6,
        ));
    }
    let corrected = correct_values(
        &g,
        &material,
        &records,
        (variants::chess::P, 1.0),
        3,
        0.02,
    );
    assert_eq!(corrected[variants::chess::P as usize], 100, "anchor pinned");
    for (t, &v) in corrected.iter().enumerate() {
        if !g.types[t].royal {
            assert!(v > 0, "type {t} positive value");
        }
    }
}

#[test]
fn hidden_game_runs_the_ladder_and_reveals() {
    let (g, material) = chess_material();
    let mut s = Searcher::new(&g, Eval::new(material));
    let cfg = LadderConfig { depth: 2, node_budget: 8_000, determinizations: 3, ismcts_iters: 40, ..Default::default() };
    let (outcome, rungs) = play_hidden_game(&g, &mut s, &cfg, 11, 60);
    assert!(!rungs.is_empty());
    // Cold open must start on an imperfect-info rung…
    assert_ne!(rungs[0], botboard_core::ladder::Rung::R0PerfectInfo);
    // …and belief must collapse toward cheaper rungs as play reveals (§8.8).
    let last = *rungs.last().unwrap();
    let first_cost = rungs[0] as u8;
    assert!(
        (last as u8) <= first_cost,
        "disambiguation must not escalate over a game: {rungs:?}"
    );
    let _ = outcome;
}

#[test]
fn league_round_robin_and_profiles() {
    let (g, material) = chess_material();
    let pop = default_population();
    let payoff = round_robin(&g, &pop, &material, 2, 1, 3_000, 99);
    let (mix, rating) = nash_averaging(&payoff, 2000);
    assert!((mix.iter().sum::<f64>() - 1.0).abs() < 1e-6);
    let json = profiles_json(&pop, &mix, &rating, &payoff);
    assert!(json.contains("\"profiles\""));
    assert!(json.contains("berserk"));
}

/// Heavier statistical check: a deeper searcher must beat a shallower one.
#[test]
#[ignore]
fn depth_advantage_wins_matches() {
    let (g, material) = chess_material();
    let eval = Eval::new(material);
    let (mut w, mut l, mut d) = (0, 0, 0);
    for i in 0..10u64 {
        let mut a = Searcher::new(&g, eval.clone());
        let mut b = Searcher::new(&g, eval.clone());
        let deep_is_0 = i % 2 == 0;
        let (depths, budgets) = if deep_is_0 {
            ((4, 2), (400_000, 20_000))
        } else {
            ((2, 4), (20_000, 400_000))
        };
        let rec = play_game(&g, &mut a, &mut b, depths, budgets, 1000 + i, 4, 200, 0);
        match rec.outcome {
            Outcome::Draw => d += 1,
            Outcome::Win(side) => {
                if (side == 0) == deep_is_0 {
                    w += 1
                } else {
                    l += 1
                }
            }
        }
    }
    println!("deep vs shallow: +{w} -{l} ={d}");
    assert!(w > l, "depth must win the match: +{w} -{l} ={d}");
}
