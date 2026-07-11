//! SRW battle-surface acceptance: compose battles from JSON, drive them
//! through the C ABI exactly as the C# campaign layer will, and check the
//! spec's promises — intel collapses beliefs (SRW §10), tiers and terrain
//! take effect (§6), pricing is engine-derived (§8), determinism (§10.1).

use std::ffi::{c_char, c_int, CString};

use botboard::srw::*;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn buf_str(buf: &[c_char]) -> String {
    let bytes: Vec<u8> =
        buf.iter().take_while(|&&b| b != 0).map(|&b| b as u8).collect();
    String::from_utf8(bytes).unwrap()
}

/// A small two-army setup: controllers (royal) + a scout + an armored
/// laser skirmisher per side, with a wall choke and a pit.
fn setup(seed: u64, intel: &str, tiers: &str) -> String {
    format!(
        r#"{{
  "seed": {seed}, "max_plies": 200,
  "board": {{"w": 7, "h": 7}},
  "sides": 2,
  "types": [
    {{"name": "controller", "glyph": "C", "royal": true,
      "moves": [{{"geom": "leaper", "m": 0, "n": 1}}, {{"geom": "leaper", "m": 1, "n": 1}}],
      "abilities": [{{"kind": "wall", "range": 2}}]}},
    {{"name": "scuttler", "glyph": "S",
      "moves": [{{"geom": "leaper", "m": 1, "n": 2}}]}},
    {{"name": "lancer", "glyph": "L", "hp": 2,
      "moves": [{{"geom": "rider", "m": 0, "n": 1, "mode": "move"}}],
      "abilities": [{{"kind": "laser", "range": 2, "retreat": true}}]}}
  ],
  "placements": [
    {{"t": 0, "side": 0, "x": 3, "y": 0}},
    {{"t": 1, "side": 0, "x": 1, "y": 0}},
    {{"t": 2, "side": 0, "x": 5, "y": 0}},
    {{"t": 0, "side": 1, "x": 3, "y": 6}},
    {{"t": 1, "side": 1, "x": 1, "y": 6}},
    {{"t": 2, "side": 1, "x": 5, "y": 6}}
  ],
  "terrain": [{{"x": 3, "y": 3, "kind": "wall"}}, {{"x": 0, "y": 3, "kind": "pit"}}],
  "intel": [{intel}],
  "tiers": {tiers}
}}"#
    )
}

#[test]
fn create_inspect_destroy() {
    let s = c(&setup(1, "", "[1, 1]"));
    let b = srw_create(s.as_ptr());
    assert!(!b.is_null(), "battle should build from valid setup");
    let mut dims = [0 as c_int; 4];
    assert_eq!(srw_dims(b, dims.as_mut_ptr()), 0);
    assert_eq!(dims, [7, 7, 2, 3]);
    assert_eq!(srw_status(b), 0);
    assert_eq!(srw_stm(b), 0);
    assert_eq!(srw_piece_count(b), 6);
    // Terrain round-trip: wall at (3,3) = sq 24, pit at (0,3) = sq 21.
    assert_eq!(srw_terrain(b, 24), 1);
    assert_eq!(srw_terrain(b, 21), 2);
    assert_eq!(srw_terrain(b, 0), 0);
    // Type introspection.
    let mut nb = [0 as c_char; 32];
    assert!(srw_type_name(b, 2, nb.as_mut_ptr(), 32) > 0);
    assert_eq!(buf_str(&nb), "lancer");
    assert_eq!(srw_type_glyph(b, 1) as u8 as char, 'S');
    // Piece info: piece 0 is side-0 controller at (3,0) = sq 3.
    let mut pi = [0 as c_int; 6];
    assert_eq!(srw_piece_info(b, 0, pi.as_mut_ptr()), 0);
    assert_eq!(pi[0], 3);
    assert_eq!(pi[1], 0);
    assert_eq!(pi[2], 0);
    assert_eq!(pi[4], 1);
    srw_destroy(b);
}

#[test]
fn rejects_bad_setups() {
    for bad in [
        "not json",
        r#"{"board": {"w": 20, "h": 20}, "sides": 2, "types": [], "placements": []}"#,
        r#"{"sides": 5, "types": [{"name": "x", "moves": []}], "placements": []}"#,
        // terrain on an occupied cell
        &setup(1, "", "[1,1]").replace(r#""x": 3, "y": 3"#, r#""x": 3, "y": 0"#),
    ] {
        let s = c(bad);
        assert!(srw_create(s.as_ptr()).is_null(), "should reject: {bad}");
    }
}

#[test]
fn intel_collapses_belief_and_drops_rungs() {
    // Cold open: side 1 knows nothing about side 0's non-royals.
    let s_cold = c(&setup(3, "", "[1, 1]"));
    let b_cold = srw_create(s_cold.as_ptr());
    let h_cold = srw_entropy(b_cold, 1);
    assert!(h_cold > 0.0, "cold open must have entropy, got {h_cold}");

    // Veteran: side 1 has full recon on the player (SRW §6 dial 3).
    let s_vet = c(&setup(3, r#"{"observer": 1, "reveal_all": true}"#, "[1, 1]"));
    let b_vet = srw_create(s_vet.as_ptr());
    assert_eq!(srw_entropy(b_vet, 1), 0.0, "reconned belief must be point mass");
    // ...and the player's own view is unaffected.
    assert_eq!(srw_entropy(b_vet, 0), h_cold);

    // The engine gear follows the intel: play the same opening move, then
    // ask each side-1 AI to move. The veteran must answer on rung 0.
    let mut mv = [0 as c_char; 32];
    for b in [b_cold, b_vet] {
        assert!(srw_legal_count(b) > 0);
        assert!(srw_legal_move(b, 0, mv.as_mut_ptr(), 32) > 0);
        let first = buf_str(&mv);
        assert_eq!(srw_apply(b, c(&first).as_ptr()), 0);
        assert_eq!(srw_stm(b), 1);
    }
    let rung_vet = srw_ai_move(b_vet, mv.as_mut_ptr(), 32);
    assert_eq!(rung_vet, 0, "informed enemy must run perfect-info rung");
    let rung_cold = srw_ai_move(b_cold, mv.as_mut_ptr(), 32);
    assert!(rung_cold >= 1, "blind enemy must run a hidden-info rung, got {rung_cold}");

    // Partial intel: knowing only the lancer types leaves scuttler doubt.
    let s_part = c(&setup(3, r#"{"observer": 1, "known_types": [2]}"#, "[1, 1]"));
    let b_part = srw_create(s_part.as_ptr());
    let h_part = srw_entropy(b_part, 1);
    assert!(h_part > 0.0 && h_part < h_cold, "partial intel must narrow: {h_part} vs {h_cold}");
    // The lancer (piece 2) is revealed to side 1; the scuttler (piece 1) is not.
    assert_eq!(srw_revealed(b_part, 1, 2), 1);
    assert_eq!(srw_revealed(b_part, 1, 1), 0);

    srw_destroy(b_cold);
    srw_destroy(b_vet);
    srw_destroy(b_part);
}

#[test]
fn full_battle_runs_to_verdict_deterministically() {
    let run = |seed: u64| -> (c_int, Vec<String>) {
        let s = c(&setup(seed, "", "[0, 0]"));
        let b = srw_create(s.as_ptr());
        let mut mv = [0 as c_char; 32];
        let mut log = Vec::new();
        for _ in 0..300 {
            if srw_status(b) != 0 {
                break;
            }
            let r = srw_ai_move(b, mv.as_mut_ptr(), 32);
            assert!(r >= 0, "ai_move failed: {r}");
            let m = buf_str(&mv);
            assert_eq!(srw_apply(b, c(&m).as_ptr()), 0);
            log.push(m);
        }
        let st = srw_status(b);
        srw_destroy(b);
        (st, log)
    };
    let (st1, log1) = run(7);
    let (st2, log2) = run(7);
    assert_ne!(st1, 0, "battle must reach a verdict");
    assert_eq!(st1, st2, "same seed ⇒ same verdict");
    assert_eq!(log1, log2, "same seed ⇒ identical move log (§10.1)");
}

#[test]
fn observing_actions_reveals_identity() {
    // No intel; drive side 0's scuttler and confirm side 1's belief about
    // it collapses once its move pattern is observed (SRW §10: discovery).
    let s = c(&setup(5, "", "[1, 1]"));
    let b = srw_create(s.as_ptr());
    let mut mv = [0 as c_char; 32];
    let mut info = [0 as c_int; 6];
    let h0 = srw_entropy(b, 1);
    for _ in 0..40 {
        if srw_status(b) != 0 {
            break;
        }
        let r = srw_ai_move(b, mv.as_mut_ptr(), 32);
        assert!(r >= 0);
        let m = buf_str(&mv);
        assert_eq!(srw_apply(b, c(&m).as_ptr()), 0);
        let _ = srw_legal_info(b, 0, info.as_mut_ptr());
    }
    let h1 = srw_entropy(b, 1);
    assert!(h1 < h0, "observed play must sharpen the belief: {h0} → {h1}");
    srw_destroy(b);
}

#[test]
fn three_player_ffa_reaches_verdict() {
    let s = c(&format!(
        r#"{{
  "seed": 11, "max_plies": 300,
  "board": {{"w": 7, "h": 7}},
  "sides": 3,
  "types": [
    {{"name": "controller", "glyph": "C", "royal": true,
      "moves": [{{"geom": "leaper", "m": 0, "n": 1}}, {{"geom": "leaper", "m": 1, "n": 1}}]}},
    {{"name": "drone", "glyph": "D",
      "moves": [{{"geom": "rider", "m": 0, "n": 1}}]}}
  ],
  "placements": [
    {{"t": 0, "side": 0, "x": 0, "y": 0}}, {{"t": 1, "side": 0, "x": 1, "y": 0}},
    {{"t": 0, "side": 1, "x": 6, "y": 0}}, {{"t": 1, "side": 1, "x": 5, "y": 0}},
    {{"t": 0, "side": 2, "x": 3, "y": 6}}, {{"t": 1, "side": 2, "x": 3, "y": 5}}
  ]
}}"#
    ));
    let b = srw_create(s.as_ptr());
    assert!(!b.is_null());
    let mut dims = [0 as c_int; 4];
    srw_dims(b, dims.as_mut_ptr());
    assert_eq!(dims[2], 3);
    let mut mv = [0 as c_char; 32];
    for _ in 0..400 {
        if srw_status(b) != 0 {
            break;
        }
        assert!(srw_ai_move(b, mv.as_mut_ptr(), 32) >= 0);
        assert_eq!(srw_apply(b, c(&buf_str(&mv)).as_ptr()), 0);
    }
    let st = srw_status(b);
    assert!(st == 9 || (1..=3).contains(&st), "FFA verdict, got {st}");
    srw_destroy(b);
}

#[test]
fn pricing_is_engine_derived_and_ordered() {
    let s = c(&setup(1, "", "[1, 1]"));
    let mut costs = [0f64; 8];
    let n = srw_price(s.as_ptr(), costs.as_mut_ptr(), 8);
    assert_eq!(n, 3);
    assert_eq!(costs[0], 0.0, "royal controller is priceless, not priced");
    assert!(costs[1] >= 1.0 && costs[2] >= 1.0);
    // The armored vertical rider + laser must outprice the bare knight-leaper.
    assert!(
        costs[2] > costs[1],
        "lancer ({}) should outprice scuttler ({})",
        costs[2],
        costs[1]
    );
}

/// Regression for the veilworks vet-sweep DNF (docs/training-report.md §4):
/// heal Bits used to qualify as "captures" in quiescence (friendly-occupied
/// target), and since heal undoes armor-strike damage the damage/heal tree
/// had no monotone bound — one tier-1 ai_move ran for CPU-hours. The army
/// below is the exact sweep game (veilworks, budget 8, seed 31, game 3).
/// With the enemy-target qsearch filter the whole battle finishes in
/// well under a second; if this test hangs, that invariant regressed.
#[test]
fn heal_army_tier1_battle_terminates_deterministically() {
    let spec = include_str!("fixtures/veilworks_b8_s31_g3_t1.json");
    let run = || -> (c_int, c_int, Vec<String>) {
        let s = c(spec);
        let b = srw_create(s.as_ptr());
        assert!(!b.is_null(), "fixture must build");
        let mut mv = [0 as c_char; 32];
        let mut log = Vec::new();
        for _ in 0..300 {
            if srw_status(b) != 0 {
                break;
            }
            let r = srw_ai_move(b, mv.as_mut_ptr(), 32);
            assert!(r >= 0, "ai_move failed: {r}");
            let m = buf_str(&mv);
            assert_eq!(srw_apply(b, c(&m).as_ptr()), 0);
            log.push(m);
        }
        let st = srw_status(b);
        let reason = srw_end_reason(b);
        srw_destroy(b);
        (st, reason, log)
    };
    let (st1, reason1, log1) = run();
    let (st2, _, log2) = run();
    assert_ne!(st1, 0, "heal-army battle must reach a verdict");
    assert!(log1.len() <= 240, "must end before the ply cap, got {}", log1.len());
    assert!(
        [1, 2, 5].contains(&reason1),
        "expected a decisive/repetition end, got reason {reason1}"
    );
    assert_eq!(st1, st2, "same seed ⇒ same verdict");
    assert_eq!(log1, log2, "same seed ⇒ identical move log (§10.1)");
}

/// Appendix B presence masks over the C ABI: stealth robots are invisible
/// to the enemy observer until adjacency or damage reveals them; enemy
/// mines never render; full battles containing all three Bits stay
/// deterministic.
#[test]
fn stealth_and_mines_are_masked_per_observer() {
    let spec = r#"{
  "seed": 11, "max_plies": 60,
  "board": {"w": 7, "h": 7},
  "sides": 2,
  "types": [
    {"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]},
    {"name": "shade", "glyph": "S", "stealth": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]},
    {"name": "sapper", "glyph": "P",
     "moves": [{"geom": "leaper", "m": 0, "n": 1}],
     "abilities": [{"kind": "mine", "range": 1}]},
    {"name": "decoy", "glyph": "D", "hologram": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]}
  ],
  "placements": [
    {"t": 0, "side": 0, "x": 3, "y": 0},
    {"t": 2, "side": 0, "x": 2, "y": 1},
    {"t": 3, "side": 0, "x": 4, "y": 1},
    {"t": 0, "side": 1, "x": 3, "y": 6},
    {"t": 1, "side": 1, "x": 3, "y": 3}
  ],
  "tiers": [0, 0]
}"#;
    let b = srw_create(c(spec).as_ptr());
    assert!(!b.is_null(), "trio setup must build");

    // The shade (piece 4, side 1) is hidden from side 0, seen by side 1.
    assert_eq!(srw_visible(b, 0, 4), 0, "stealth starts concealed");
    assert_eq!(srw_visible(b, 1, 4), 1, "its own side always sees it");
    // The decoy and controllers are plain to everyone.
    assert_eq!(srw_visible(b, 0, 2), 1);
    assert_eq!(srw_visible(b, 1, 2), 1);

    // Side 0 lays a mine at c3 (sapper at c2): the enemy view shows floor.
    assert_eq!(srw_apply(b, c("c2!mine:c3").as_ptr()), 0);
    let mine_sq = 2 + 2 * 7; // (2,2) on a 7-wide board
    assert_eq!(srw_terrain(b, mine_sq), 3, "ground truth holds the mine");
    assert_eq!(srw_terrain_for(b, 0, mine_sq), 3, "the owner sees it");
    assert_eq!(srw_terrain_for(b, 1, mine_sq), 0, "the enemy sees bare floor");

    // Walk the shade next to the sapper: chebyshev adjacency reveals it.
    assert_eq!(srw_apply(b, c("d4d3").as_ptr()), 0); // shade to d3, diagonal to c2
    assert_eq!(srw_visible(b, 0, 4), 1, "adjacency blows the shade's cover");
    srw_destroy(b);
}

#[test]
fn trio_battles_run_to_verdict_deterministically() {
    // Autoplay with stealth+mine+hologram content on both sides: the
    // masked-world AI path must stay seed-deterministic and terminate.
    let spec = |seed: u64| format!(r#"{{
  "seed": {seed}, "max_plies": 120,
  "board": {{"w": 7, "h": 7}},
  "sides": 2,
  "types": [
    {{"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{{"geom": "leaper", "m": 0, "n": 1}}, {{"geom": "leaper", "m": 1, "n": 1}}]}},
    {{"name": "shade", "glyph": "S", "stealth": true,
     "moves": [{{"geom": "leaper", "m": 1, "n": 2}}]}},
    {{"name": "sapper", "glyph": "P",
     "moves": [{{"geom": "leaper", "m": 0, "n": 1}}],
     "abilities": [{{"kind": "mine", "range": 1}}]}},
    {{"name": "decoy", "glyph": "D", "hologram": true,
     "moves": [{{"geom": "leaper", "m": 0, "n": 1}}, {{"geom": "leaper", "m": 1, "n": 1}}]}}
  ],
  "placements": [
    {{"t": 0, "side": 0, "x": 3, "y": 0}},
    {{"t": 1, "side": 0, "x": 1, "y": 1}},
    {{"t": 2, "side": 0, "x": 5, "y": 1}},
    {{"t": 3, "side": 0, "x": 3, "y": 1}},
    {{"t": 0, "side": 1, "x": 3, "y": 6}},
    {{"t": 1, "side": 1, "x": 1, "y": 5}},
    {{"t": 2, "side": 1, "x": 5, "y": 5}},
    {{"t": 3, "side": 1, "x": 3, "y": 5}}
  ],
  "tiers": [1, 1]
}}"#);
    let run = |seed: u64| -> (c_int, Vec<String>) {
        let s = c(&spec(seed));
        let b = srw_create(s.as_ptr());
        assert!(!b.is_null());
        let mut mv = [0 as c_char; 32];
        let mut log = Vec::new();
        for _ in 0..200 {
            if srw_status(b) != 0 {
                break;
            }
            let r = srw_ai_move(b, mv.as_mut_ptr(), 32);
            assert!(r >= 0, "ai_move failed: {r}");
            let m = buf_str(&mv);
            assert_eq!(srw_apply(b, c(&m).as_ptr()), 0, "arbiter accepts {m}");
            log.push(m);
        }
        let st = srw_status(b);
        srw_destroy(b);
        (st, log)
    };
    let (st1, log1) = run(13);
    let (st2, log2) = run(13);
    assert_ne!(st1, 0, "trio battle reaches a verdict");
    assert_eq!(st1, st2);
    assert_eq!(log1, log2, "masked-world AI stays deterministic (§10.1)");
}

/// Tall grass (Appendix B) is live camouflage: memoryless and positional.
/// A robot in the stalks vanishes from enemy view, reappears on contact,
/// and vanishes again when the watcher walks away.
#[test]
fn tall_grass_conceals_and_reconceals() {
    let spec = r#"{
  "seed": 21, "max_plies": 60,
  "board": {"w": 7, "h": 7},
  "sides": 2,
  "types": [
    {"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]},
    {"name": "runner", "glyph": "R",
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]}
  ],
  "placements": [
    {"t": 0, "side": 0, "x": 3, "y": 0},
    {"t": 1, "side": 0, "x": 2, "y": 1},
    {"t": 0, "side": 1, "x": 3, "y": 6},
    {"t": 1, "side": 1, "x": 3, "y": 3}
  ],
  "terrain": [{"x": 3, "y": 3, "kind": "grass"}],
  "tiers": [0, 0]
}"#;
    let b = srw_create(c(spec).as_ptr());
    assert!(!b.is_null(), "grass setup must build");

    // The enemy runner starts IN the grass at d4: hidden from side 0
    // (nobody adjacent), plain to its own side.
    assert_eq!(srw_visible(b, 0, 3), 0, "grass conceals");
    assert_eq!(srw_visible(b, 1, 3), 1);

    // Walk our runner adjacent (c2 -> c3 is chebyshev-1 of d4): seen.
    assert_eq!(srw_apply(b, c("c2c3").as_ptr()), 0);
    assert_eq!(srw_apply(b, c("d7c7").as_ptr()), 0); // enemy ctrl shuffles
    assert_eq!(srw_visible(b, 0, 3), 1, "adjacency parts the stalks");

    // We step away: hidden again.
    assert_eq!(srw_apply(b, c("c3c2").as_ptr()), 0);
    assert_eq!(srw_visible(b, 0, 3), 0, "grass re-conceals — no memory");
    srw_destroy(b);
}
