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

/// The optional `"net"` setup field (§10.6 into the battle surface): a
/// checkpoint loads, quantizes against the battle's GameDef, powers every
/// side's searcher, and same-seed battles still replay identically. A
/// missing or corrupt checkpoint fails the build — never a silent
/// fallback to the linear eval.
#[test]
fn net_checkpoint_powers_battles_deterministically() {
    let dir = std::env::temp_dir().join("botboard_srw_net_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tiny_net.bin");
    let net = botboard_core::nnue::FloatNet::new(7);
    std::fs::write(&path, net.to_bytes()).unwrap();
    let net_path = path.to_str().unwrap().to_string();

    let spec = |seed: u64, net: &str| {
        format!(
            r#"{{
  "seed": {seed}, "max_plies": 80,
  "board": {{"w": 6, "h": 6}},
  "sides": 2,
  "types": [
    {{"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{{"geom": "leaper", "m": 0, "n": 1}}, {{"geom": "leaper", "m": 1, "n": 1}}]}},
    {{"name": "lance", "glyph": "L", "hp": 2,
     "moves": [{{"geom": "rider", "m": 0, "n": 1, "mode": "move"}}],
     "abilities": [{{"kind": "laser", "range": 2, "retreat": true}}]}}
  ],
  "placements": [
    {{"t": 0, "side": 0, "x": 3, "y": 0}},
    {{"t": 1, "side": 0, "x": 1, "y": 0}},
    {{"t": 0, "side": 1, "x": 3, "y": 5}},
    {{"t": 1, "side": 1, "x": 1, "y": 5}}
  ],
  "tiers": [0, 0],
  "net": "{net}"
}}"#
        )
    };

    let run = |seed: u64| -> (c_int, Vec<String>) {
        let s = c(&spec(seed, &net_path));
        let b = srw_create(s.as_ptr());
        assert!(!b.is_null(), "battle with net checkpoint must build");
        let mut mv = [0 as c_char; 32];
        let mut log = Vec::new();
        for _ in 0..200 {
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
    assert_ne!(st1, 0, "net-powered battle reaches a verdict");
    assert_eq!(st1, st2, "same seed ⇒ same verdict with net on");
    assert_eq!(log1, log2, "same seed ⇒ identical move log with net on (§10.1)");

    // Nonexistent path: build error, not silent fallback.
    let bad = c(&spec(7, "/nonexistent/dir/net.bin"));
    assert!(srw_create(bad.as_ptr()).is_null(), "missing checkpoint must fail build");

    // Corrupt bytes: same.
    let junk = dir.join("junk_net.bin");
    std::fs::write(&junk, b"not a checkpoint").unwrap();
    let bad2 = c(&spec(7, junk.to_str().unwrap()));
    assert!(srw_create(bad2.as_ptr()).is_null(), "corrupt checkpoint must fail build");
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

/// Vocabulary batch 2 over the C ABI: drill types, destructible blocks,
/// conveyor belts, swap/push abilities, and HP-gated move kernels — all
/// additive schema, with the extended terrain and effect code tables.
#[test]
fn batch2_vocabulary_over_the_abi() {
    let spec = r#"{
  "seed": 5, "max_plies": 60,
  "board": {"w": 6, "h": 6},
  "sides": 2,
  "types": [
    {"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]},
    {"name": "driller", "glyph": "D", "drill": true,
     "moves": [{"geom": "rider", "m": 0, "n": 1}]},
    {"name": "swapper", "glyph": "W",
     "moves": [{"geom": "leaper", "m": 0, "n": 1}],
     "abilities": [{"kind": "swap", "range": 2}]},
    {"name": "pusher", "glyph": "P",
     "moves": [{"geom": "leaper", "m": 0, "n": 1}],
     "abilities": [{"kind": "push", "range": 2}]},
    {"name": "limper", "glyph": "M", "hp": 2,
     "moves": [{"geom": "leaper", "m": 0, "n": 1},
                {"geom": "rider", "m": 0, "n": 1, "max_hp": 1}]},
    {"name": "runner", "glyph": "R",
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]}
  ],
  "placements": [
    {"t": 0, "side": 0, "x": 0, "y": 0},
    {"t": 1, "side": 0, "x": 2, "y": 0},
    {"t": 2, "side": 0, "x": 1, "y": 0},
    {"t": 3, "side": 0, "x": 4, "y": 0},
    {"t": 4, "side": 0, "x": 0, "y": 2},
    {"t": 0, "side": 1, "x": 5, "y": 5},
    {"t": 5, "side": 1, "x": 4, "y": 2}
  ],
  "terrain": [
    {"x": 2, "y": 2, "kind": "wall"},
    {"x": 5, "y": 0, "kind": "block1"},
    {"x": 5, "y": 1, "kind": "block2"},
    {"x": 5, "y": 2, "kind": "block3"},
    {"x": 3, "y": 3, "kind": "conv_n"},
    {"x": 3, "y": 4, "kind": "conv_e"},
    {"x": 0, "y": 5, "kind": "conv_s"},
    {"x": 1, "y": 5, "kind": "conv_w"}
  ],
  "tiers": [0, 0]
}"#;
    let b = srw_create(c(spec).as_ptr());
    assert!(!b.is_null(), "batch-2 setup must build (additive schema)");

    // Extended terrain code table: 7/8/9 block tiers, 10..=13 conv NESW.
    assert_eq!(srw_terrain(b, 14), 1, "wall at c3");
    assert_eq!(srw_terrain(b, 5), 7, "block1 at f1");
    assert_eq!(srw_terrain(b, 11), 8, "block2 at f2");
    assert_eq!(srw_terrain(b, 17), 9, "block3 at f3");
    assert_eq!(srw_terrain(b, 21), 10, "conv_n at d4");
    assert_eq!(srw_terrain(b, 27), 11, "conv_e at d5");
    assert_eq!(srw_terrain(b, 30), 12, "conv_s at a6");
    assert_eq!(srw_terrain(b, 31), 13, "conv_w at b6");
    assert_eq!(srw_terrain_for(b, 1, 5), 7, "blocks are public terrain");

    // Collect side 0's legal moves (strings + effect codes).
    let n = srw_legal_count(b);
    assert!(n > 0);
    let mut strs = Vec::new();
    let mut effects = Vec::new();
    let mut mv = [0 as c_char; 32];
    let mut info = [0 as c_int; 6];
    for i in 0..n {
        assert!(srw_legal_move(b, i, mv.as_mut_ptr(), 32) > 0);
        strs.push(buf_str(&mv));
        assert_eq!(srw_legal_info(b, i, info.as_mut_ptr()), 0);
        effects.push(info[4]);
    }

    // Drill: the ride passes the wall at c3 and may even land inside it.
    assert!(strs.contains(&"c1c3".to_string()), "driller lands in the wall");
    assert!(strs.contains(&"c1c4".to_string()), "driller passes the wall");
    // Swap and push surface with their new effect codes (9 and 10).
    let swap_i = strs.iter().position(|s| s == "b1!swap:c1").expect("swap move");
    assert_eq!(effects[swap_i], 9, "swap effect code");
    let push_i = strs.iter().position(|s| s == "e1!push:e3>e4").expect("push move");
    assert_eq!(effects[push_i], 10, "push effect code");
    // HP gate: the limper's rider only wakes below max HP.
    assert!(strs.contains(&"a3a4".to_string()), "ungated step generates");
    assert!(!strs.contains(&"a3a5".to_string()), "max_hp-gated ride dormant at full HP");

    // Apply the swap through the ABI: driller and swapper trade squares.
    assert_eq!(srw_apply(b, c("b1!swap:c1").as_ptr()), 0);
    let mut pi = [0 as c_int; 6];
    assert_eq!(srw_piece_info(b, 1, pi.as_mut_ptr()), 0);
    assert_eq!(pi[0], 1, "driller now on b1");
    assert_eq!(srw_piece_info(b, 2, pi.as_mut_ptr()), 0);
    assert_eq!(pi[0], 2, "swapper now on c1");
    srw_destroy(b);
}

/// Vocabulary batch 3 over the C ABI: hopper landing modes (`"landing"`:
/// grasshopper `"beyond"`, checkers-style `"locust"`) and range-limited
/// riders (`"max_steps"`) reach the battle surface as additive schema —
/// composed here as a grasshopper+locust robot skirmish with a
/// deterministic full-battle replay.
#[test]
fn batch3_grasshopper_locust_battle_over_the_abi() {
    let spec = |seed: u64| format!(r#"{{
  "seed": {seed}, "max_plies": 120,
  "board": {{"w": 7, "h": 7}},
  "sides": 2,
  "types": [
    {{"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{{"geom": "leaper", "m": 0, "n": 1}}, {{"geom": "leaper", "m": 1, "n": 1}}]}},
    {{"name": "ghopper", "glyph": "G",
     "moves": [{{"geom": "hopper", "m": 0, "n": 1, "landing": "beyond"}},
                {{"geom": "hopper", "m": 1, "n": 1, "landing": "beyond"}}]}},
    {{"name": "locust", "glyph": "L",
     "moves": [{{"geom": "hopper", "m": 0, "n": 1, "landing": "locust"}},
                {{"geom": "hopper", "m": 1, "n": 1, "landing": "locust"}}]}},
    {{"name": "runner", "glyph": "R", "moves": [{{"geom": "leaper", "m": 0, "n": 1}}]}},
    {{"name": "shortr", "glyph": "S",
     "moves": [{{"geom": "rider", "m": 0, "n": 1, "max_steps": 2}}]}}
  ],
  "placements": [
    {{"t": 0, "side": 0, "x": 0, "y": 0}},
    {{"t": 2, "side": 0, "x": 3, "y": 1}},
    {{"t": 1, "side": 0, "x": 2, "y": 0}},
    {{"t": 3, "side": 0, "x": 2, "y": 1}},
    {{"t": 4, "side": 0, "x": 5, "y": 0}},
    {{"t": 0, "side": 1, "x": 6, "y": 6}},
    {{"t": 2, "side": 1, "x": 3, "y": 5}},
    {{"t": 1, "side": 1, "x": 4, "y": 6}},
    {{"t": 3, "side": 1, "x": 3, "y": 3}},
    {{"t": 4, "side": 1, "x": 1, "y": 6}}
  ],
  "tiers": [0, 0]
}}"#);
    let b = srw_create(c(&spec(23)).as_ptr());
    assert!(!b.is_null(), "batch-3 setup must build (additive schema)");

    // Collect side 0's legal moves with kind codes.
    let n = srw_legal_count(b);
    assert!(n > 0);
    let mut strs = Vec::new();
    let mut kinds = Vec::new();
    let mut mv = [0 as c_char; 32];
    let mut info = [0 as c_int; 6];
    for i in 0..n {
        assert!(srw_legal_move(b, i, mv.as_mut_ptr(), 32) > 0);
        strs.push(buf_str(&mv));
        assert_eq!(srw_legal_info(b, i, info.as_mut_ptr()), 0);
        kinds.push([info[0], info[1], info[2], info[3]]);
    }
    // Locust d2: hops the enemy runner at d4, capturing IT and landing d5
    // — surfaced as kind 7 with the screen in aux.
    let li = strs.iter().position(|s| s == "d2d5xd4").expect("locust move over the ABI");
    assert_eq!(kinds[li][2], 7, "locust kind code");
    assert_eq!(kinds[li][0], 3 + 7, "from d2");
    assert_eq!(kinds[li][1], 3 + 4 * 7, "to d5");
    assert_eq!(kinds[li][3], 3 + 3 * 7, "aux = the captured screen d4");
    // Grasshopper c1: over the friendly runner at c2, landing exactly
    // beyond on c3 (a plain landing, kind 0).
    let gi = strs.iter().position(|s| s == "c1c3").expect("grasshopper hop");
    assert_eq!(kinds[gi][2], 0);
    // Short rider f1: two steps up the f-file and no further.
    assert!(strs.contains(&"f1f3".to_string()), "max_steps 2 reaches step 2");
    assert!(!strs.contains(&"f1f4".to_string()), "max_steps 2 stops there");

    // Apply the locust capture: the screen dies, the locust lands beyond.
    assert_eq!(srw_apply(b, c("d2d5xd4").as_ptr()), 0);
    let mut pi = [0 as c_int; 6];
    assert_eq!(srw_piece_info(b, 1, pi.as_mut_ptr()), 0);
    assert_eq!(pi[0], 3 + 4 * 7, "locust stands on d5");
    assert_eq!(srw_piece_info(b, 8, pi.as_mut_ptr()), 0);
    assert_eq!(pi[4], 0, "the screened runner is dead");
    srw_destroy(b);

    // Deterministic full-battle replay with all three batch-3 atoms live.
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
    let (st1, log1) = run(29);
    let (st2, log2) = run(29);
    assert_ne!(st1, 0, "grasshopper+locust battle reaches a verdict");
    assert_eq!(st1, st2, "same seed ⇒ same verdict");
    assert_eq!(log1, log2, "same seed ⇒ identical move log (§10.1)");
}

// ---------------------------------------------------------------------------
// Bits 2.0 Stage 4: custom abilities and terrains authored in the setup
// JSON — schema, validation (loud NULL + srw_last_error), wire-code
// allocation, and full-battle determinism with customs live.
// ---------------------------------------------------------------------------

fn last_err() -> String {
    let mut buf = [0 as c_char; 512];
    let n = srw_last_error(buf.as_mut_ptr(), 512);
    assert!(n >= 0, "srw_last_error buffer error: {n}");
    buf_str(&buf)
}

/// The vampiric-strike army: a custom enemy-damage + self-heal ability
/// referenced from a piece's ability list by id.
fn vamp_setup(seed: u64) -> String {
    format!(
        r#"{{
  "seed": {seed}, "max_plies": 120,
  "board": {{"w": 6, "h": 6}},
  "sides": 2,
  "abilities": [
    {{"id": "vamp-strike",
      "target": {{"who": "enemy", "range": 2, "pred": ["nonroyal"]}},
      "ops": [{{"op": "hp_add", "n": -1}}],
      "self_ops": [{{"op": "hp_add", "n": 1, "cap": true}}],
      "cost": {{"flat": 3.0, "mult": 1.0}},
      "descriptor_slot": "laser"}}
  ],
  "types": [
    {{"name": "ctrl", "glyph": "C", "royal": true,
      "moves": [{{"geom": "leaper", "m": 0, "n": 1}}, {{"geom": "leaper", "m": 1, "n": 1}}]}},
    {{"name": "vamp", "glyph": "V", "hp": 3,
      "moves": [{{"geom": "leaper", "m": 0, "n": 1}}],
      "abilities": [{{"kind": "vamp-strike"}}]}},
    {{"name": "grunt", "glyph": "G", "hp": 2,
      "moves": [{{"geom": "leaper", "m": 0, "n": 1}}]}}
  ],
  "placements": [
    {{"t": 0, "side": 0, "x": 0, "y": 0}},
    {{"t": 1, "side": 0, "x": 2, "y": 2}},
    {{"t": 0, "side": 1, "x": 5, "y": 5}},
    {{"t": 2, "side": 1, "x": 3, "y": 3}}
  ],
  "tiers": [0, 0]
}}"#
    )
}

/// A custom vampiric strike over the C ABI: generation surfaces the id
/// notation and the upward-allocated effect code (11), application
/// drains the enemy and feeds the caster (capped), lethal damage kills
/// through the capture fate, and same-seed battles replay identically
/// (make/unmake hash parity is debug-asserted under every make).
#[test]
fn stage4_vamp_strike_over_the_abi() {
    let b = srw_create(c(&vamp_setup(3)).as_ptr());
    assert!(!b.is_null(), "vamp setup must build: {}", last_err());
    assert_eq!(last_err(), "", "successful create clears the error slot");

    // Generation: the strike surfaces with id notation and effect 11.
    let n = srw_legal_count(b);
    let mut mv = [0 as c_char; 48];
    let mut info = [0 as c_int; 6];
    let mut strike_idx = None;
    for i in 0..n {
        assert!(srw_legal_move(b, i, mv.as_mut_ptr(), 48) > 0);
        if buf_str(&mv) == "c3!vamp-strike:d4" {
            strike_idx = Some(i);
        }
    }
    let si = strike_idx.expect("vamp strike generates over the ABI");
    assert_eq!(srw_legal_info(b, si, info.as_mut_ptr()), 0);
    assert_eq!(info[2], 5, "ability kind code");
    assert_eq!(info[4], 32, "custom effect codes live in the band above the stdlib codes");

    // Apply: grunt (piece 3) 2 → 1 HP; vamp (piece 1) capped at 3.
    let mut pi = [0 as c_int; 6];
    assert_eq!(srw_apply(b, c("c3!vamp-strike:d4").as_ptr()), 0);
    assert_eq!(srw_piece_info(b, 3, pi.as_mut_ptr()), 0);
    assert_eq!(pi[3], 1, "grunt takes 1 damage");
    assert_eq!(srw_piece_info(b, 1, pi.as_mut_ptr()), 0);
    assert_eq!(pi[3], 3, "full-HP vampire's drink caps at type max");

    // Enemy shuffles; the second strike kills through the capture fate.
    assert_eq!(srw_apply(b, c("f6f5").as_ptr()), 0);
    assert_eq!(srw_apply(b, c("c3!vamp-strike:d4").as_ptr()), 0);
    assert_eq!(srw_piece_info(b, 3, pi.as_mut_ptr()), 0);
    assert_eq!(pi[4], 0, "1-HP grunt falls to the custom strike");
    srw_destroy(b);

    // Determinism: same seed, AI-driven, identical logs (§10.1).
    let run = |seed: u64| -> (c_int, Vec<String>) {
        let b = srw_create(c(&vamp_setup(seed)).as_ptr());
        assert!(!b.is_null());
        let mut mv = [0 as c_char; 48];
        let mut log = Vec::new();
        for _ in 0..200 {
            if srw_status(b) != 0 {
                break;
            }
            let r = srw_ai_move(b, mv.as_mut_ptr(), 48);
            assert!(r >= 0, "ai_move failed: {r}");
            let m = buf_str(&mv);
            assert_eq!(srw_apply(b, c(&m).as_ptr()), 0, "arbiter accepts {m}");
            log.push(m);
        }
        let st = srw_status(b);
        srw_destroy(b);
        (st, log)
    };
    let (st1, log1) = run(17);
    let (st2, log2) = run(17);
    assert_ne!(st1, 0, "vamp battle reaches a verdict");
    assert_eq!(st1, st2, "same seed ⇒ same verdict");
    assert_eq!(log1, log2, "same seed ⇒ identical move log with a custom effect live");
}

/// Custom terrains over the C ABI: a lava row (on-land bite, lethal at
/// 0) and an owner-secret glow row — wire codes allocate upward (14+),
/// the concealment masks read the custom rows, and the hazard persists
/// (consumed: false).
#[test]
fn stage4_custom_lava_terrain_over_the_abi() {
    let spec = r#"{
  "seed": 7, "max_plies": 80,
  "board": {"w": 6, "h": 6},
  "sides": 2,
  "terrains": [
    {"id": "lava", "code": "auto",
     "blocks": {"ground": false, "flight": false, "drill": false},
     "on_land": {"ops": [{"op": "hp_add", "n": -1}], "gate": "anyone", "consumed": false},
     "carry": null, "conceal": null, "tiers": 0},
    {"id": "glow", "conceal": "owner_secret", "owner": 0}
  ],
  "types": [
    {"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]},
    {"name": "runner", "glyph": "R", "hp": 2,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]}
  ],
  "placements": [
    {"t": 0, "side": 0, "x": 0, "y": 0},
    {"t": 1, "side": 0, "x": 2, "y": 2},
    {"t": 0, "side": 1, "x": 5, "y": 5}
  ],
  "terrain": [{"x": 2, "y": 3, "kind": "lava"}, {"x": 4, "y": 0, "kind": "glow"}],
  "tiers": [0, 0]
}"#;
    let b = srw_create(c(spec).as_ptr());
    assert!(!b.is_null(), "lava setup must build: {}", last_err());

    // Wire codes: custom rows surface as 14 + index, in setup order.
    let lava_sq = 2 + 3 * 6;
    let glow_sq = 4;
    assert_eq!(srw_terrain(b, lava_sq), 14, "lava wire code");
    assert_eq!(srw_terrain(b, glow_sq), 15, "glow wire code");
    // Owner-secret custom conceal: the owner sees it, the enemy floor.
    assert_eq!(srw_terrain_for(b, 0, glow_sq), 15, "owner sees the glow");
    assert_eq!(srw_terrain_for(b, 1, glow_sq), 0, "enemy sees bare floor");
    assert_eq!(srw_terrain_for(b, 1, lava_sq), 14, "unconcealed lava is public");

    // Landing bites: runner (piece 1) 2 → 1 HP; the pool persists.
    let mut pi = [0 as c_int; 6];
    assert_eq!(srw_apply(b, c("c3c4").as_ptr()), 0);
    assert_eq!(srw_piece_info(b, 1, pi.as_mut_ptr()), 0);
    assert_eq!(pi[3], 1, "lava bites the lander");
    assert_eq!(srw_terrain(b, lava_sq), 14, "lava persists (consumed: false)");

    // Off and back on at 1 HP: the pool claims the runner.
    assert_eq!(srw_apply(b, c("f6f5").as_ptr()), 0);
    assert_eq!(srw_apply(b, c("c4c3").as_ptr()), 0);
    assert_eq!(srw_apply(b, c("f5f6").as_ptr()), 0);
    assert_eq!(srw_apply(b, c("c3c4").as_ptr()), 0);
    assert_eq!(srw_piece_info(b, 1, pi.as_mut_ptr()), 0);
    assert_eq!(pi[4], 0, "lava is lethal at 0");
    srw_destroy(b);
}

/// Every validation failure is loud: srw_create returns NULL and
/// srw_last_error names the fault precisely.
#[test]
fn stage4_validation_failures_name_the_fault() {
    // A minimal valid skeleton the cases below corrupt.
    let base = |abilities: &str, terrains: &str| {
        format!(
            r#"{{
  "seed": 1, "board": {{"w": 6, "h": 6}}, "sides": 2,
  {abilities}{terrains}
  "types": [
    {{"name": "ctrl", "glyph": "C", "royal": true,
      "moves": [{{"geom": "leaper", "m": 0, "n": 1}}]}}
  ],
  "placements": [
    {{"t": 0, "side": 0, "x": 0, "y": 0}},
    {{"t": 0, "side": 1, "x": 5, "y": 5}}
  ]
}}"#
        )
    };
    let ok_ability = |body: &str| {
        format!(r#""abilities": [{body}],"#)
    };
    let good = r#"{"id": "zap", "target": {"who": "enemy", "range": 2},
        "ops": [{"op": "hp_add", "n": -1}],
        "cost": {"flat": 1.0, "mult": 1.0}, "descriptor_slot": "laser"}"#;

    // Sanity: the skeleton with a good custom builds.
    let ok = srw_create(c(&base(&ok_ability(good), "")).as_ptr());
    assert!(!ok.is_null(), "skeleton must build: {}", last_err());
    srw_destroy(ok);

    let cases: Vec<(String, &str)> = vec![
        // -- ability validation ------------------------------------------
        (ok_ability(&good.replace("hp_add", "teleport")), "unknown op"),
        (
            ok_ability(&good.replace(r#""who": "enemy", "range": 2"#,
                r#""who": "enemy", "range": 2, "pred": ["sleepy"]"#)),
            "unknown pred",
        ),
        (ok_ability(&good.replace(r#""who": "enemy""#, r#""who": "anything""#)), "unknown target.who"),
        (
            ok_ability(&good.replace(
                r#""ops": [{"op": "hp_add", "n": -1}]"#,
                r#""ops": [{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1},{"op": "hp_add", "n": -1}]"#,
            )),
            "op list length",
        ),
        (
            ok_ability(&good.replace(r#""ops": [{"op": "hp_add", "n": -1}]"#, r#""ops": []"#)),
            "at least one op",
        ),
        (
            ok_ability(&good.replace(r#""cost": {"flat": 1.0, "mult": 1.0}, "#, "")),
            "cost hint required",
        ),
        (
            ok_ability(&good.replace(r#", "descriptor_slot": "laser""#, "")),
            "descriptor_slot required",
        ),
        (
            ok_ability(&good.replace(r#""descriptor_slot": "laser""#, r#""descriptor_slot": "plasma""#)),
            "not a stdlib kin",
        ),
        (ok_ability(&good.replace(r#""id": "zap""#, r#""id": "heal""#)), "collides with a stdlib ability"),
        (ok_ability(&format!("{good}, {good}")), "duplicate id"),
        (ok_ability(&good.replace(r#""n": -1"#, r#""n": 0"#)), "hp_add n must be"),
        (ok_ability(&good.replace(r#""n": -1"#, r#""n": -12"#)), "hp_add n must be"),
        (ok_ability(&good.replace(r#""range": 2"#, r#""range": 0"#)), "target.range must be"),
        (
            ok_ability(&good.replace(r#""who": "enemy", "range": 2"#,
                r#""who": "friendly", "ray": {"max": 3}"#)),
            "ray reach requires who",
        ),
        (
            ok_ability(&good.replace(r#"{"op": "hp_add", "n": -1}"#,
                r#"{"op": "set_terrain", "kind": "wall"}"#)),
            "set_terrain needs who",
        ),
        (ok_ability(&good.replace(r#""id": "zap""#, r#""id": "Bad Id!""#)), "bad id"),
        (
            ok_ability(&good.replace(r#""cost": {"flat": 1.0, "mult": 1.0}"#,
                r#""cost": {"flat": 1.0, "mult": 99.0}"#)),
            "cost.mult must be",
        ),
        // -- terrain validation ------------------------------------------
        (
            format!(
                r#""terrains": [{}],"#,
                (0..17)
                    .map(|i| format!(r#"{{"id": "t{i}"}}"#))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "at most 16 custom rows",
        ),
        (r#""terrains": [{"id": "wall"}],"#.to_string(), "collides with a stdlib terrain"),
        (r#""terrains": [{"id": "x"}, {"id": "x"}],"#.to_string(), "duplicate id"),
        (r#""terrains": [{"id": "x", "tiers": 2}],"#.to_string(), "tiers must be 0 or 1"),
        (r#""terrains": [{"id": "x", "code": 40}],"#.to_string(), "code must be \"auto\""),
        (
            r#""terrains": [{"id": "x", "carry": {"belt": {"dx": 0, "dy": 0}}}],"#.to_string(),
            "unit step",
        ),
        (
            r#""terrains": [{"id": "x", "conceal": "owner_secret"}],"#.to_string(),
            "requires \"owner\"",
        ),
        (
            r#""terrains": [{"id": "x", "on_land": {"ops": [{"op": "hp_add", "n": -1}], "gate": "enemy_of_owner"}}],"#
                .to_string(),
            "requires \"owner\"",
        ),
        (
            r#""terrains": [{"id": "x", "on_land": {"ops": [{"op": "burninate", "n": -1}]}}],"#
                .to_string(),
            "unknown on_land op",
        ),
        (
            r#""terrains": [{"id": "x", "on_land": {"ops": [{"op": "hp_add", "n": 9}]}}],"#
                .to_string(),
            "on_land hp_add n must be",
        ),
    ];
    for (frag, want) in cases {
        let spec = if frag.starts_with(r#""terrains""#) {
            base("", &frag)
        } else {
            base(&frag, "")
        };
        let b = srw_create(c(&spec).as_ptr());
        assert!(b.is_null(), "must reject: {frag}");
        let err = last_err();
        assert!(
            err.contains(want),
            "error must name the fault: wanted {want:?} in {err:?} for {frag}"
        );
    }
}

#[test]
fn spy_collapses_belief_and_pierces_stealth_over_the_abi() {
    // SRW §7/§10: the active recon verb. A spy-capable controller reveals
    // an ambiguous, stealthed enemy: identity collapses for the caster's
    // side only, stealth pierces permanently, the board doesn't move.
    let spec = r#"{
  "seed": 21, "max_plies": 60,
  "board": {"w": 7, "h": 7},
  "sides": 2,
  "types": [
    {"name": "ctrl", "glyph": "C", "royal": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}],
     "abilities": [{"kind": "spy", "range": 4}]},
    {"name": "scuttler", "glyph": "S",
     "moves": [{"geom": "leaper", "m": 1, "n": 2}]},
    {"name": "shade", "glyph": "H", "stealth": true,
     "moves": [{"geom": "leaper", "m": 0, "n": 1}]}
  ],
  "placements": [
    {"t": 0, "side": 0, "x": 3, "y": 0},
    {"t": 0, "side": 1, "x": 3, "y": 6},
    {"t": 1, "side": 1, "x": 0, "y": 6},
    {"t": 2, "side": 1, "x": 3, "y": 3}
  ],
  "tiers": [0, 0]
}"#;
    let b = srw_create(c(spec).as_ptr());
    assert!(!b.is_null(), "spy setup must build: {}", last_err());

    // Piece 3 = the shade at d4: ambiguous and invisible to side 0.
    assert_eq!(srw_revealed(b, 0, 3), 0, "shade identity starts ambiguous");
    assert_eq!(srw_visible(b, 0, 3), 0, "shade starts concealed");
    let h0 = srw_entropy(b, 0);
    assert!(h0 > 0.0);

    // The spy move generates (ground truth — the arbiter sees the cloak)
    // and surfaces effect code 11 in srw_legal_info.
    let mut mv = [0 as c_char; 32];
    let mut info = [0 as c_int; 6];
    let n = srw_legal_count(b);
    let mut spy_at = -1;
    for i in 0..n {
        assert!(srw_legal_move(b, i, mv.as_mut_ptr(), 32) > 0);
        if buf_str(&mv) == "d1!spy:d4" {
            spy_at = i;
        }
    }
    assert!(spy_at >= 0, "d1!spy:d4 must be legal");
    assert_eq!(srw_legal_info(b, spy_at, info.as_mut_ptr()), 0);
    assert_eq!(info[2], 5, "spy is an ability-kind move");
    assert_eq!(info[4], 11, "spy's stable effect code");

    // Apply: board-null, but side 0 now knows and sees the shade.
    let mut pi_before = [0 as c_int; 6];
    assert_eq!(srw_piece_info(b, 3, pi_before.as_mut_ptr()), 0);
    assert_eq!(srw_apply(b, c("d1!spy:d4").as_ptr()), 0);
    let mut pi_after = [0 as c_int; 6];
    assert_eq!(srw_piece_info(b, 3, pi_after.as_mut_ptr()), 0);
    assert_eq!(pi_before, pi_after, "spy mutates nothing on the board");
    assert_eq!(srw_revealed(b, 0, 3), 1, "identity collapses for the caster's side");
    assert_eq!(srw_visible(b, 0, 3), 1, "stealth is pierced for the caster's side");
    assert!(srw_entropy(b, 0) < h0, "the caster's belief sharpens");
    assert_eq!(srw_stm(b), 1, "the turn was the price");
    srw_destroy(b);
}

#[test]
fn codex_export_warm_starts_rematch_over_the_abi() {
    // §8.8/§10–§11 end to end: export a battle's accumulated belief,
    // feed it back verbatim in a rematch setup, start strictly sharper.
    let s_cold = c(&setup(3, "", "[1, 1]"));
    let b_cold = srw_create(s_cold.as_ptr());
    let h_cold = srw_entropy(b_cold, 1);

    // Battle A: side 1 learned the lancers (partial intel stands in for
    // a played battle's observations).
    let s_a = c(&setup(3, r#"{"observer": 1, "known_types": [2]}"#, "[1, 1]"));
    let b_a = srw_create(s_a.as_ptr());
    let h_a = srw_entropy(b_a, 1);
    assert!(h_a > 0.0 && h_a < h_cold);
    let mut cbuf = [0 as c_char; 4096];
    let n = srw_codex(b_a, 1, cbuf.as_mut_ptr(), 4096);
    assert!(n > 0, "codex export must write, got {n}");
    let codex_json = buf_str(&cbuf);
    assert!(codex_json.contains("\"candidates\""));

    // Battle B: a cold rematch, codex fed back verbatim.
    let s_b = setup(3, "", "[1, 1]")
        .replace("\"tiers\"", &format!("\"codex\": [{codex_json}],\n  \"tiers\""));
    let b_b = srw_create(c(&s_b).as_ptr());
    assert!(!b_b.is_null(), "codex rematch must build: {}", last_err());
    let h_b = srw_entropy(b_b, 1);
    assert!(
        (h_b - h_a).abs() < 1e-12,
        "warm start restores the learned sharpness: {h_b} vs {h_a}"
    );
    assert!(h_b < h_cold, "the rematch starts strictly sharper than cold");
    assert_eq!(srw_revealed(b_b, 1, 2), 1, "the scouted lancer stays known");
    // The other side's belief is untouched by side 1's codex.
    assert_eq!(srw_entropy(b_b, 0), srw_entropy(b_cold, 0));

    // Validation is loud: an out-of-range observer names the fault.
    let bad = setup(3, "", "[1, 1]").replace(
        "\"tiers\"",
        "\"codex\": [{\"observer\": 5, \"candidates\": [[0]]}],\n  \"tiers\"",
    );
    assert!(srw_create(c(&bad).as_ptr()).is_null());
    assert!(last_err().contains("codex"), "fault names codex: {}", last_err());

    srw_destroy(b_cold);
    srw_destroy(b_a);
    srw_destroy(b_b);
}
