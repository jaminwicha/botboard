//! Bits 2.0 Stage 4 — custom abilities and terrains as first-class
//! GameDef-owned registry rows (docs/bits2-effects-as-data.md).
//!
//! Core-side acceptance: a custom vampiric strike (enemy damage + self
//! heal) generates, applies, undoes exactly (hash + full state parity),
//! classifies as a qsearch capture, prices via its cost hint, and maps
//! into the NNUE descriptor via its stdlib-kin slot; a custom lava
//! terrain bites landers through the on-land interpreter (lethal at 0),
//! and custom blocking rows obstruct movement through the per-Position
//! custom masks. The FFI half (JSON authoring, validation, wire codes)
//! lives in the srw_suite's stage4 section.

use botboard_core::bits::{AbilityBit, MoveBit};
use botboard_core::cost::{cost_prior, CostWeights};
use botboard_core::effects::{CostHint, CustomAbility, CustomReach, Pred, Who};
use botboard_core::game::{
    BoardDef, CaptureFate, GameDef, PassPolicy, PieceTypeDef, Policy, StalematePolicy, TurnPolicy,
};
use botboard_core::move_defs::script;
use botboard_core::movegen::legal_moves;
use botboard_core::moves::{move_str, Effect, MoveKind};
use botboard_core::nnue::descriptor;
use botboard_core::ops::{HpAmt, MicroOp};
use botboard_core::position::{Loc, Position};
use botboard_core::terrain_defs::{
    Blocks, Carry, Conceal, CustomOnLand, CustomTerrain, LandGate, NT,
};

fn policy() -> Policy {
    Policy {
        stalemate: StalematePolicy::Loss,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    }
}

fn vamp_strike() -> CustomAbility {
    CustomAbility {
        id: "vamp-strike".into(),
        who: Who::Enemy,
        reach: CustomReach::Point { range: 2 },
        preds: vec![Pred::NonRoyal],
        ops: vec![MicroOp::HpAdd { n: HpAmt::Lit(-1), cap: false }],
        self_ops: vec![MicroOp::HpAdd { n: HpAmt::Lit(1), cap: true }],
        cost: CostHint { flat: 3.0, mult: 1.0 },
        descriptor_slot: 11, // stdlib kin: laser
    }
}

/// 6x6, 2 sides: royal controller, a vampire (HP 3, king-step + the
/// custom strike), an enemy grunt (HP 2).
fn vamp_game() -> GameDef {
    let ctrl = PieceTypeDef::new("ctrl", 'C', vec![MoveBit::leaper(0, 1)]).royal();
    let vamp = PieceTypeDef::new("vamp", 'V', vec![MoveBit::leaper(0, 1)])
        .hp(3)
        .abilities(vec![AbilityBit::Custom(0)]);
    let grunt = PieceTypeDef::new("grunt", 'G', vec![MoveBit::leaper(0, 1)]).hp(2);
    GameDef::with_customs(
        BoardDef { w: 6, h: 6 },
        2,
        vec![ctrl, vamp, grunt],
        Vec::new(),
        policy(),
        vec![(0, 0, 0, 0), (1, 0, 2, 2), (0, 1, 5, 5), (2, 1, 3, 3)],
        vec![vamp_strike()],
        Vec::new(),
    )
}

#[test]
fn vamp_strike_generates_applies_and_undoes_exactly() {
    let g = vamp_game();
    let mut pos = Position::startpos(&g);
    // Wound the vampire so the self-heal is observable under the cap.
    let vi = pos.board[g.board.sq(2, 2) as usize] as usize;
    pos.hp[vi] = 1;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    let strike = moves
        .iter()
        .find(|m| move_str(&g, m) == "c3!vamp-strike:d4")
        .copied()
        .expect("vamp strike generates against the in-range grunt");
    assert_eq!(strike.kind, MoveKind::Ability);
    assert_eq!(strike.effect, Effect::Custom(0));
    // The royal enemy controller is pred-excluded (nonroyal) and out of
    // range anyway; no strike targets it.
    assert!(
        !moves.iter().any(|m| m.effect == Effect::Custom(0) && m.to == g.board.sq(5, 5)),
        "royal target must be excluded"
    );

    let before = pos.clone();
    let gi = pos.board[g.board.sq(3, 3) as usize] as usize;
    let u = pos.make(&g, &strike); // debug builds assert incremental-hash parity here
    assert_eq!(pos.hp[gi], 1, "enemy grunt takes 1 damage");
    assert_eq!(pos.hp[vi], 2, "vampire drinks 1 HP back");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, before.hash, "undo restores the exact key");
    assert_eq!(pos.hash, g.zobrist.full_hash(&pos), "restored key matches recompute");
    assert_eq!(pos.hp, before.hp);
    assert_eq!(pos.board, before.board);
    for (a, b) in pos.pieces.iter().zip(before.pieces.iter()) {
        assert_eq!(a.loc, b.loc);
        assert_eq!(a.side, b.side);
        assert_eq!(a.t, b.t);
        assert_eq!(a.moved, b.moved);
    }
}

#[test]
fn vamp_strike_damage_is_lethal_at_zero_and_undoes() {
    let g = vamp_game();
    let mut pos = Position::startpos(&g);
    let gi = pos.board[g.board.sq(3, 3) as usize] as usize;
    pos.hp[gi] = 1; // defenses down: the next point of damage kills
    pos.rehash(&g);
    let strike = legal_moves(&g, &mut pos)
        .into_iter()
        .find(|m| move_str(&g, m) == "c3!vamp-strike:d4")
        .expect("strike generates");
    let before = pos.clone();
    let u = pos.make(&g, &strike);
    assert_eq!(pos.pieces[gi].loc, Loc::Dead, "1-HP target falls through the capture fate");
    assert!(pos.board[g.board.sq(3, 3) as usize] < 0);
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, before.hash);
    assert_eq!(pos.pieces[gi].loc, Loc::Board(g.board.sq(3, 3)));
    assert_eq!(pos.hp[gi], 1);
}

#[test]
fn vamp_strike_counts_as_capture_for_qsearch() {
    let g = vamp_game();
    let mut pos = Position::startpos(&g);
    let strike = legal_moves(&g, &mut pos)
        .into_iter()
        .find(|m| m.effect == Effect::Custom(0))
        .expect("strike generates");
    // The kind row's derived class: an enemy stands on the destination,
    // so the custom strike joins quiescence (enemy HP strictly drops —
    // the qsearch monotonicity argument holds: the self-heal only adds
    // HP to the CASTER'S side).
    let row = script(strike.kind);
    assert!(row.counts_as_capture(&pos, &strike), "vamp strike is qsearch-visible");
    assert!(!row.is_quiet(&pos, &strike), "and never LMR-quiet");
}

#[test]
fn custom_cost_hint_and_descriptor_slot_flow_downstream() {
    let g = vamp_game();
    let w = CostWeights { w_move: 0.35, w_attack: 0.35 };
    // The identical composition with the custom bit stripped from the
    // vampire (same type INDEX, so the mobility integral's seeded
    // occupancies match): the cost delta is exactly the row's flat hint
    // (mult 1.0).
    let ctrl = PieceTypeDef::new("ctrl", 'C', vec![MoveBit::leaper(0, 1)]).royal();
    let bare_vamp = PieceTypeDef::new("vamp", 'V', vec![MoveBit::leaper(0, 1)]).hp(3);
    let grunt = PieceTypeDef::new("grunt", 'G', vec![MoveBit::leaper(0, 1)]).hp(2);
    let g_bare = GameDef::with_customs(
        BoardDef { w: 6, h: 6 },
        2,
        vec![ctrl, bare_vamp, grunt],
        Vec::new(),
        policy(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let with = cost_prior(&g, 1, &w);
    let without = cost_prior(&g_bare, 1, &w);
    assert!(
        (with - without - 3.0).abs() < 1e-9,
        "custom flat hint prices: {with} vs {without}"
    );
    // Descriptor: the vampire's kin slot (laser, dim 11) carries the
    // 1.0 presence signal; the bare twin shows 0.
    let d = descriptor(&g, 1, 0);
    assert_eq!(d[11], 1.0, "custom row maps into its stdlib kin's dim");
    let d_bare = descriptor(&g_bare, 1, 0);
    assert_eq!(d_bare[11], 0.0);
}

#[test]
fn custom_ray_reach_walks_like_a_beam() {
    // A ray custom: max 3, no pierce — first enemy on each queen line.
    let zap = CustomAbility {
        id: "zap".into(),
        who: Who::Enemy,
        reach: CustomReach::Ray { max: 3, pierce: false },
        preds: Vec::new(),
        ops: vec![MicroOp::HpAdd { n: HpAmt::Lit(-1), cap: false }],
        self_ops: Vec::new(),
        cost: CostHint { flat: 1.0, mult: 1.0 },
        descriptor_slot: 11,
    };
    let ctrl = PieceTypeDef::new("ctrl", 'C', vec![MoveBit::leaper(0, 1)]).royal();
    let zapper = PieceTypeDef::new("zapper", 'Z', vec![MoveBit::leaper(0, 1)])
        .abilities(vec![AbilityBit::Custom(0)]);
    let grunt = PieceTypeDef::new("grunt", 'G', vec![MoveBit::leaper(0, 1)]).hp(2);
    // Zapper b2; enemies on the b-file at b4 (in reach) and b5 (shadowed
    // by b4 — no pierce); enemy at e5 (diagonal, distance 3, in reach).
    let g = GameDef::with_customs(
        BoardDef { w: 6, h: 6 },
        2,
        vec![ctrl, zapper, grunt],
        Vec::new(),
        policy(),
        vec![
            (0, 0, 0, 0),
            (1, 0, 1, 1),
            (0, 1, 5, 0),
            (2, 1, 1, 3),
            (2, 1, 1, 4),
            (2, 1, 4, 4),
        ],
        vec![zap],
        Vec::new(),
    );
    let mut pos = Position::startpos(&g);
    let strs: Vec<String> =
        legal_moves(&g, &mut pos).iter().map(|m| move_str(&g, m)).collect();
    assert!(strs.contains(&"b2!zap:b4".to_string()), "first enemy on the file");
    assert!(!strs.contains(&"b2!zap:b5".to_string()), "non-piercing beam shadows");
    assert!(strs.contains(&"b2!zap:e5".to_string()), "diagonal reach 3");
}

/// Lava: open custom terrain biting every lander for 1 (lethal at 0).
fn lava() -> CustomTerrain {
    CustomTerrain {
        id: "lava".into(),
        blocks: Blocks { ground: false, flight: false, drill: false },
        on_land: Some(CustomOnLand {
            ops: vec![MicroOp::HpAdd { n: HpAmt::Lit(-1), cap: false }],
            gate: LandGate::Anyone,
            consumed: false,
        }),
        carry: Carry::None,
        conceal: Conceal::None,
        owner: None,
        tiers: 0,
    }
}

/// A solid custom crystal: blocks ground and flight, drill passes.
fn crystal() -> CustomTerrain {
    CustomTerrain {
        id: "crystal".into(),
        blocks: Blocks { ground: true, flight: true, drill: false },
        on_land: None,
        carry: Carry::None,
        conceal: Conceal::None,
        owner: None,
        tiers: 1,
    }
}

fn terrain_game() -> GameDef {
    let ctrl = PieceTypeDef::new("ctrl", 'C', vec![MoveBit::leaper(0, 1)]).royal();
    let runner = PieceTypeDef::new("runner", 'R', vec![MoveBit::leaper(0, 1)]).hp(2);
    GameDef::with_customs(
        BoardDef { w: 6, h: 6 },
        2,
        vec![ctrl, runner],
        Vec::new(),
        policy(),
        vec![(0, 0, 0, 0), (1, 0, 2, 2), (0, 1, 5, 5)],
        Vec::new(),
        vec![lava(), crystal()],
    )
}

#[test]
fn custom_lava_bites_landers_and_undoes() {
    let g = terrain_game();
    let mut pos = Position::startpos(&g);
    let lava_sq = g.board.sq(2, 3);
    pos.terrain[lava_sq as usize] = NT as u8; // custom row 0 = lava
    pos.rehash(&g);
    let ri = pos.board[g.board.sq(2, 2) as usize] as usize;
    let step = legal_moves(&g, &mut pos)
        .into_iter()
        .find(|m| move_str(&g, m) == "c3c4")
        .expect("runner can step onto lava (open terrain)");
    let before = pos.clone();
    let u = pos.make(&g, &step);
    assert_eq!(pos.hp[ri], 1, "lava bites the lander for 1");
    assert_eq!(
        pos.terrain[lava_sq as usize],
        NT as u8,
        "lava persists (consumed: false)"
    );
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, before.hash);
    assert_eq!(pos.hp[ri], 2);

    // Lethal at 0: a 1-HP lander dies in the pool.
    pos.hp[ri] = 1;
    pos.rehash(&g);
    let step = legal_moves(&g, &mut pos)
        .into_iter()
        .find(|m| move_str(&g, m) == "c3c4")
        .expect("step regenerates");
    let before = pos.clone();
    let u = pos.make(&g, &step);
    assert_eq!(pos.pieces[ri].loc, Loc::Dead, "the pool claims a 1-HP lander");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, before.hash);
    assert_eq!(pos.pieces[ri].loc, Loc::Board(g.board.sq(2, 2)));
}

#[test]
fn custom_blocking_terrain_obstructs_through_the_masks() {
    let g = terrain_game();
    let mut pos = Position::startpos(&g);
    let crystal_sq = g.board.sq(2, 3);
    pos.terrain[crystal_sq as usize] = NT as u8 + 1; // custom row 1 = crystal
    pos.rehash(&g);
    assert!(!pos.terrain_open(crystal_sq), "crystal blocks ground");
    assert!(pos.terrain_open_for(crystal_sq, false, true), "drill passes crystal");
    assert!(!pos.terrain_open_for(crystal_sq, true, false), "flight does not");
    let strs: Vec<String> =
        legal_moves(&g, &mut pos).iter().map(|m| move_str(&g, m)).collect();
    assert!(!strs.contains(&"c3c4".to_string()), "runner cannot enter the crystal");
    // The g-aware block predicates see the custom tier.
    assert!(g.terrain_is_block(NT as u8 + 1), "tier-1 crystal is destructible");
    assert_eq!(g.terrain_erode(NT as u8 + 1), 0, "custom erosion clears to floor");
}

#[test]
fn custom_battles_replay_deterministically() {
    // Same seed, same moves: drive both vamp armies with the searcher-free
    // arbiter path (legal_moves first choice) and compare the hash trail —
    // the deterministic-replay half of the Stage-4 gate at the core layer.
    let run = || {
        let g = vamp_game();
        let mut pos = Position::startpos(&g);
        let mut trail = Vec::new();
        for _ in 0..24 {
            let moves = legal_moves(&g, &mut pos);
            let Some(mv) = moves.first().copied() else { break };
            pos.make(&g, &mv);
            trail.push((move_str(&g, &mv), pos.hash));
        }
        trail
    };
    assert_eq!(run(), run(), "same composition ⇒ identical move/hash trail");
}
