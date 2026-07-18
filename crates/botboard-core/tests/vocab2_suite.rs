//! Vocabulary batch 2 (§3.1 terrain permissions & state gates, §3.4
//! ability scripts, SRW Appendix B terrain): drill, destructible blocks,
//! conveyor belts, swap, push, and HP-gated kernels — exercised in the
//! style of the abilities suite, with apply/undo hash-parity checks
//! (debug builds also assert the incremental key inside every make).

use botboard_core::bits::{AbilityBit, MoveBit};
use botboard_core::game::*;
use botboard_core::movegen::legal_moves;
use botboard_core::moves::{move_str, Effect};
use botboard_core::position::{
    Loc, Position, T_BLOCK1, T_BLOCK2, T_BLOCK3, T_CONV_E, T_CONV_N, T_CONV_S, T_CONV_W,
    T_ICE, T_MINE0, T_NONE, T_PIT, T_WALL,
};

const KING: TypeId = 0;
const TANK: TypeId = 1; // rider(0,1), 3 HP
const DRILLER: TypeId = 2; // rider(0,1), drill
const LASERBOT: TypeId = 3; // leaper(0,1), laser r3 no retreat
const SWAPPER: TypeId = 4; // leaper(0,1), swap r2
const PUSHER: TypeId = 5; // leaper(0,1), push r2
const SCOUT: TypeId = 6; // leaper(0,1)+(1,1), 2 HP
const DRONE: TypeId = 7; // rider(0,1), flight
const GATED: TypeId = 8; // 3 HP: ride needs hp>=2, diagonal wakes at hp<=1
const RETLASER: TypeId = 9; // leaper(0,1), laser r3 with retreat

fn robot_types() -> Vec<PieceTypeDef> {
    vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        PieceTypeDef::new("tank", 'T', vec![MoveBit::rider(0, 1)]).hp(3),
        PieceTypeDef::new("driller", 'B', vec![MoveBit::rider(0, 1)]).drill(),
        PieceTypeDef::new("laserbot", 'L', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::Laser { range: 3, retreat: false, pierce: false }]),
        PieceTypeDef::new("swapper", 'W', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::Swap { range: 2 }]),
        PieceTypeDef::new("pusher", 'P', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::Push { range: 2 }]),
        PieceTypeDef::new("scout", 'S', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .hp(2),
        PieceTypeDef::new("drone", 'F', vec![MoveBit::rider(0, 1)]).flight(),
        PieceTypeDef::new(
            "gated",
            'G',
            vec![MoveBit::rider(0, 1).min_hp(2), MoveBit::leaper(1, 1).max_hp(1)],
        )
        .hp(3),
        PieceTypeDef::new("retlaser", 'R', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::Laser { range: 3, retreat: true, pierce: false }]),
    ]
}

fn robot_game(start: Vec<(TypeId, Side, u8, u8)>) -> GameDef {
    GameDef::new(
        BoardDef { w: 8, h: 8 },
        2,
        robot_types(),
        vec![],
        Policy {
            stalemate: StalematePolicy::Draw,
            capture_fate: CaptureFate::Destroy,
            turn: TurnPolicy::Alternate,
            pass: PassPolicy::Forbidden,
        },
        start,
    )
}

// ---------------------------------------------------------------------------
// 1. Drill: the second terrain permission (§3.1)
// ---------------------------------------------------------------------------

#[test]
fn drill_passes_walls_and_blocks_but_not_pits() {
    // A wall splits the d-file and a block splits the f-file: the tank
    // stops, the driller chews straight through — and may even park
    // inside the scrap. Pits still swallow drillers.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (TANK, 0, 3, 1),
        (DRILLER, 0, 5, 1),
    ]);
    let mut pos = Position::startpos(&g);
    pos.terrain[g.board.sq(3, 3) as usize] = T_WALL;
    pos.terrain[g.board.sq(5, 3) as usize] = T_BLOCK2;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    assert!(
        !moves.iter().any(|m| m.from == g.board.sq(3, 1) && m.to == g.board.sq(3, 5)),
        "grounded riders cannot pass a wall"
    );
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(5, 1) && m.to == g.board.sq(5, 5)),
        "the drill chews through a destructible block"
    );
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(5, 1) && m.to == g.board.sq(5, 3)),
        "a driller may land inside the block it is chewing"
    );

    // Attack maps agree: the driller threatens past the block.
    pos.set_stm(&g, 1);
    assert!(pos.is_attacked(&g, g.board.sq(5, 6), 0), "drill threat crosses the block");
    assert!(!pos.is_attacked(&g, g.board.sq(3, 6), 0), "walled threat is cut");
    pos.set_stm(&g, 0);

    // Walls part for the drill too…
    pos.terrain[g.board.sq(5, 3) as usize] = T_WALL;
    pos.rehash(&g);
    let moves2 = legal_moves(&g, &mut pos);
    assert!(
        moves2.iter().any(|m| m.from == g.board.sq(5, 1) && m.to == g.board.sq(5, 5)),
        "walls neither block paths nor bar landings for drillers"
    );

    // …but pits do not: no flight, no crossing.
    pos.terrain[g.board.sq(5, 3) as usize] = T_PIT;
    pos.rehash(&g);
    let moves3 = legal_moves(&g, &mut pos);
    assert!(
        !moves3.iter().any(|m| m.from == g.board.sq(5, 1) && m.to == g.board.sq(5, 5)),
        "pits still stop a driller without flight"
    );
}

#[test]
fn drill_prices_its_terrain_permission() {
    use botboard_core::cost::{cost_prior, CostWeights};
    let g = robot_game(vec![(KING, 0, 0, 0), (KING, 1, 7, 7)]);
    let w = CostWeights { w_move: 0.35, w_attack: 0.35 };
    // Same file-rider kernels; the integral board has no terrain, so the
    // ratio is exactly the ×1.15 permission multiplier — the tank's ×2
    // armor multiplier is divided back out.
    let tank = cost_prior(&g, TANK, &w) / 2.0;
    let driller = cost_prior(&g, DRILLER, &w);
    assert!(
        (driller / tank - 1.15).abs() < 0.02,
        "drill multiplies the prior by 1.15: driller {driller:.2} vs base {tank:.2}"
    );
}

// ---------------------------------------------------------------------------
// 2. Destructible blocks
// ---------------------------------------------------------------------------

#[test]
fn lasers_crack_blocks_tier_by_tier() {
    // Laserbot at e2, a tier-3 block at e4, an enemy tank hiding at e5:
    // the beam stops at the block but may target it; three shots clear
    // the lane and only then is the tank in the line of fire.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (LASERBOT, 0, 4, 1),
        (TANK, 1, 4, 4),
    ]);
    let mut pos = Position::startpos(&g);
    let bsq = g.board.sq(4, 3);
    pos.terrain[bsq as usize] = T_BLOCK3;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    let shot = moves
        .iter()
        .find(|m| m.effect == Effect::Laser && m.to == bsq)
        .copied()
        .expect("the beam may target the block square");
    assert_eq!(move_str(&g, &shot), "e2!laser:e4", "notation stays plain laser");
    assert!(
        !moves.iter().any(|m| m.effect == Effect::Laser && m.to == g.board.sq(4, 4)),
        "the beam stops at the block: nothing behind it is targetable"
    );

    // Tier by tier: BLOCK3 → BLOCK2 → BLOCK1 → floor, each step keyed.
    let h0 = pos.hash;
    let u1 = pos.make(&g, &shot);
    assert_eq!(pos.terrain[bsq as usize], T_BLOCK2, "one hit, one tier");
    assert_ne!(pos.hash, h0, "block damage keys as terrain (§7.5)");
    pos.unmake(&g, &u1);
    assert_eq!(pos.hash, h0, "undo restores the block");
    assert_eq!(pos.terrain[bsq as usize], T_BLOCK3);

    pos.make(&g, &shot);
    pos.set_stm(&g, 0);
    pos.make(&g, &shot);
    assert_eq!(pos.terrain[bsq as usize], T_BLOCK1);
    pos.set_stm(&g, 0);
    pos.make(&g, &shot);
    assert_eq!(pos.terrain[bsq as usize], T_NONE, "tier 1 clears to floor");
    pos.set_stm(&g, 0);
    assert!(
        legal_moves(&g, &mut pos)
            .iter()
            .any(|m| m.effect == Effect::Laser && m.to == g.board.sq(4, 4)),
        "with the lane cleared, the tank behind is now a target"
    );
}

#[test]
fn retreat_lasers_target_blocks_with_teeth() {
    // The coupled retreat applies to block shots too — and blocking the
    // retreat square kills the whole compound (§3.4).
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (RETLASER, 0, 4, 3),
    ]);
    let mut pos = Position::startpos(&g);
    let bsq = g.board.sq(4, 5);
    pos.terrain[bsq as usize] = T_BLOCK1;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    let shot = moves
        .iter()
        .find(|m| m.effect == Effect::Laser && m.to == bsq)
        .copied()
        .expect("retreat laser targets the block");
    assert_eq!(shot.aux, g.board.sq(4, 2), "forced retreat square");
    let h0 = pos.hash;
    let u = pos.make(&g, &shot);
    assert_eq!(pos.terrain[bsq as usize], T_NONE, "tier-1 block cleared");
    assert!(pos.piece_at(g.board.sq(4, 2)).is_some(), "and the bot retreated");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);

    // Friendly on the retreat square: no block shot on offer.
    let g2 = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (RETLASER, 0, 4, 3),
        (TANK, 0, 4, 2),
    ]);
    let mut pos2 = Position::startpos(&g2);
    pos2.terrain[bsq as usize] = T_BLOCK1;
    pos2.rehash(&g2);
    assert!(
        !legal_moves(&g2, &mut pos2)
            .iter()
            .any(|m| m.effect == Effect::Laser && m.to == bsq),
        "blocked retreat kills the block shot too"
    );
}

// ---------------------------------------------------------------------------
// 3. Conveyor belts
// ---------------------------------------------------------------------------

#[test]
fn belts_carry_landings_across_chains() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (PUSHER, 0, 3, 3), // a plain orthogonal stepper
    ]);
    let mut pos = Position::startpos(&g);
    // Two eastbound belts starting one step north of the stepper.
    pos.terrain[g.board.sq(3, 4) as usize] = T_CONV_E;
    pos.terrain[g.board.sq(4, 4) as usize] = T_CONV_E;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(5, 4)),
        "landing on the belt chains across it to the floor past the end"
    );
    assert!(
        !moves
            .iter()
            .any(|m| m.from == g.board.sq(3, 3)
                && (m.to == g.board.sq(3, 4) || m.to == g.board.sq(4, 4))),
        "no landing may rest mid-belt"
    );
    // Hash parity through the rewritten landing.
    let mv = moves
        .iter()
        .find(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(5, 4))
        .copied()
        .unwrap();
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert!(pos.piece_at(g.board.sq(5, 4)).is_some());
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);

    // A blocked exit parks the piece on the belt.
    let g2 = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (PUSHER, 0, 3, 3),
        (TANK, 0, 4, 4), // stands where the belt would dump
    ]);
    let mut pos2 = Position::startpos(&g2);
    pos2.terrain[g2.board.sq(3, 4) as usize] = T_CONV_E;
    pos2.rehash(&g2);
    assert!(
        legal_moves(&g2, &mut pos2)
            .iter()
            .any(|m| m.from == g2.board.sq(3, 3) && m.to == g2.board.sq(3, 4)),
        "occupied exit: the piece parks on the belt"
    );
}

#[test]
fn belt_cycles_are_bounded_by_revisit_detection() {
    // A 2×2 belt cycle: b6→c6→c5→b5→(b6 again). The carry stops where
    // the loop would close instead of spinning forever.
    let g = robot_game(vec![(KING, 0, 0, 0), (KING, 1, 7, 7), (PUSHER, 0, 0, 5)]);
    let mut pos = Position::startpos(&g);
    pos.terrain[g.board.sq(1, 5) as usize] = T_CONV_E;
    pos.terrain[g.board.sq(2, 5) as usize] = T_CONV_S;
    pos.terrain[g.board.sq(2, 4) as usize] = T_CONV_W;
    pos.terrain[g.board.sq(1, 4) as usize] = T_CONV_N;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(0, 5) && m.to == g.board.sq(1, 4)),
        "the cycle carries b6→c6→c5→b5 and stops at the revisit"
    );
    for stop in [g.board.sq(1, 5), g.board.sq(2, 5), g.board.sq(2, 4)] {
        assert!(
            !moves.iter().any(|m| m.from == g.board.sq(0, 5) && m.to == stop),
            "no rest mid-cycle"
        );
    }
}

#[test]
fn belt_dumps_onto_a_mine_and_it_bites() {
    // The move's destination IS the final square, so the landing hazard
    // fires exactly once — in make, with full undo.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (TANK, 0, 3, 3),
    ]);
    let mut pos = Position::startpos(&g);
    pos.terrain[g.board.sq(3, 4) as usize] = T_CONV_E;
    pos.terrain[g.board.sq(4, 4) as usize] = T_MINE0 + 1; // enemy mine
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    let mv = moves
        .iter()
        .find(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(4, 4))
        .copied()
        .expect("the belt dumps the tank onto the mine square");
    let ti = pos.board[g.board.sq(3, 3) as usize] as usize;
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert_eq!(pos.hp[ti], 2, "the mine bites at the final square");
    assert_eq!(pos.terrain[g.board.sq(4, 4) as usize], T_NONE, "and is spent");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0, "undo re-arms the mine and heals the scar");
    assert_eq!(pos.hp[ti], 3);
    assert_eq!(pos.terrain[g.board.sq(4, 4) as usize], T_MINE0 + 1);
}

#[test]
fn ice_resolves_before_belts_in_one_pass() {
    // Documented interleave: slide the sheet first, then one belt pass,
    // no re-entry into ice logic. Passing OVER a belt is not a landing.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (TANK, 0, 3, 1),
    ]);
    let mut pos = Position::startpos(&g);
    pos.terrain[g.board.sq(3, 3) as usize] = T_ICE;
    pos.terrain[g.board.sq(3, 4) as usize] = T_ICE;
    pos.terrain[g.board.sq(3, 5) as usize] = T_CONV_E;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(3, 1) && m.to == g.board.sq(4, 5)),
        "slide off the sheet onto the belt, then ride the belt east"
    );
    assert!(
        !moves.iter().any(|m| m.from == g.board.sq(3, 1) && m.to == g.board.sq(3, 5)),
        "no landing rests on the belt at the sheet's end"
    );
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(3, 1) && m.to == g.board.sq(3, 6)),
        "riding past the belt is not a landing: no carry"
    );
}

// ---------------------------------------------------------------------------
// 4. Swap
// ---------------------------------------------------------------------------

#[test]
fn swap_exchanges_friendlies_atomically() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (SWAPPER, 0, 3, 3),
        (TANK, 0, 5, 5),   // chebyshev 2: in range
        (SCOUT, 0, 6, 6),  // chebyshev 3: out of range
        (SCOUT, 1, 3, 4),  // enemy: never a swap partner
    ]);
    let mut pos = Position::startpos(&g);
    let moves = legal_moves(&g, &mut pos);
    let swaps: Vec<_> = moves.iter().filter(|m| m.effect == Effect::Swap).collect();
    assert_eq!(swaps.len(), 1, "exactly the in-range friendly is swappable");
    let mv = *swaps[0];
    assert_eq!(mv.to, g.board.sq(5, 5));
    assert_eq!(move_str(&g, &mv), "d4!swap:f6");

    let si = pos.board[g.board.sq(3, 3) as usize] as usize;
    let ti = pos.board[g.board.sq(5, 5) as usize] as usize;
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert_eq!(pos.pieces[si].loc, Loc::Board(g.board.sq(5, 5)), "caster teleported");
    assert_eq!(pos.pieces[ti].loc, Loc::Board(g.board.sq(3, 3)), "partner teleported");
    assert_eq!(pos.stm, 1, "swap consumed the turn (§3.4)");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0, "atomic unmake restores the full state key");
    assert_eq!(pos.pieces[si].loc, Loc::Board(g.board.sq(3, 3)));
    assert_eq!(pos.pieces[ti].loc, Loc::Board(g.board.sq(5, 5)));
}

#[test]
fn swap_respects_terrain_permissions() {
    // The drone hovers on a pit; the grounded swapper must not be
    // exchanged into it — each half of the swap is a landing.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (SWAPPER, 0, 3, 3),
        (DRONE, 0, 4, 4),
    ]);
    let mut pos = Position::startpos(&g);
    assert!(
        legal_moves(&g, &mut pos).iter().any(|m| m.effect == Effect::Swap),
        "on plain floor the pair swaps"
    );
    pos.terrain[g.board.sq(4, 4) as usize] = T_PIT;
    pos.rehash(&g);
    assert!(
        !legal_moves(&g, &mut pos).iter().any(|m| m.effect == Effect::Swap),
        "no swap that would land a grounded bot in a pit"
    );
}

// ---------------------------------------------------------------------------
// 5. Push
// ---------------------------------------------------------------------------

#[test]
fn push_shoves_enemies_directly_away() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (PUSHER, 0, 3, 3),
        (SCOUT, 1, 4, 4), // pushed to f6 (away along +1,+1)
        (TANK, 1, 3, 5),  // pushed to d7 (away along 0,+1)
    ]);
    let mut pos = Position::startpos(&g);
    let moves = legal_moves(&g, &mut pos);
    let shove = moves
        .iter()
        .find(|m| m.effect == Effect::Push && m.to == g.board.sq(4, 4))
        .copied()
        .expect("in-range enemy is pushable");
    assert_eq!(shove.aux, g.board.sq(5, 5), "shoved one square directly away");
    assert_eq!(move_str(&g, &shove), "d4!push:e5>f6");
    assert!(
        moves
            .iter()
            .any(|m| m.effect == Effect::Push
                && m.to == g.board.sq(3, 5)
                && m.aux == g.board.sq(3, 6)),
        "orthogonal push uses the chebyshev direction sign"
    );

    let si = pos.board[g.board.sq(4, 4) as usize] as usize;
    let h0 = pos.hash;
    let u = pos.make(&g, &shove);
    assert_eq!(pos.pieces[si].loc, Loc::Board(g.board.sq(5, 5)), "enemy displaced");
    assert!(
        pos.piece_at(g.board.sq(3, 3)).is_some(),
        "the caster never moves on a push"
    );
    assert_eq!(pos.stm, 1, "push consumed the turn (§3.4)");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);
    assert_eq!(pos.pieces[si].loc, Loc::Board(g.board.sq(4, 4)));

    // Blocked destination: the push is simply not generated.
    pos.terrain[g.board.sq(5, 5) as usize] = T_WALL;
    pos.rehash(&g);
    assert!(
        !legal_moves(&g, &mut pos)
            .iter()
            .any(|m| m.effect == Effect::Push && m.to == g.board.sq(4, 4)),
        "no push into blocking terrain"
    );
}

#[test]
fn push_onto_acid_kills_with_clean_undo() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (PUSHER, 0, 3, 3),
        (SCOUT, 1, 4, 4),
    ]);
    let mut pos = Position::startpos(&g);
    pos.terrain[g.board.sq(5, 5) as usize] = botboard_core::position::T_ACID;
    let si = pos.board[g.board.sq(4, 4) as usize] as usize;
    pos.hp[si] = 1;
    pos.rehash(&g);

    let mv = legal_moves(&g, &mut pos)
        .iter()
        .find(|m| m.effect == Effect::Push && m.to == g.board.sq(4, 4))
        .copied()
        .expect("push onto acid is generated (the hazard is open terrain)");
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert!(matches!(pos.pieces[si].loc, Loc::Dead), "the pool dissolves the 1-HP scout");
    assert!(pos.piece_at(g.board.sq(5, 5)).is_none(), "the crater is empty");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0, "undo climbs out of the pool");
    assert_eq!(pos.pieces[si].loc, Loc::Board(g.board.sq(4, 4)));
    assert_eq!(pos.hp[si], 1);
}

#[test]
fn push_onto_a_mine_springs_it() {
    // Pushing an enemy onto your own mine both spends the mine and
    // damages (here: kills) the victim — one terrain change, fully undone.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (PUSHER, 0, 3, 3),
        (SCOUT, 1, 4, 4),
    ]);
    let mut pos = Position::startpos(&g);
    pos.terrain[g.board.sq(5, 5) as usize] = T_MINE0; // side 0's mine
    pos.rehash(&g);

    let mv = legal_moves(&g, &mut pos)
        .iter()
        .find(|m| m.effect == Effect::Push && m.to == g.board.sq(4, 4))
        .copied()
        .expect("push onto a mine square is generated");
    let si = pos.board[g.board.sq(4, 4) as usize] as usize;
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert_eq!(pos.hp[si], 1, "the trap bites the shoved scout");
    assert_eq!(pos.terrain[g.board.sq(5, 5) as usize], T_NONE, "the mine is spent");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);
    assert_eq!(pos.hp[si], 2);
    assert_eq!(pos.terrain[g.board.sq(5, 5) as usize], T_MINE0, "undo re-arms the mine");
}

// ---------------------------------------------------------------------------
// 6. HP-gated kernels (§3.1 state condition)
// ---------------------------------------------------------------------------

#[test]
fn hp_gates_switch_kernels_with_current_hp() {
    let g = robot_game(vec![(KING, 0, 0, 0), (KING, 1, 7, 7), (GATED, 0, 3, 3)]);
    let mut pos = Position::startpos(&g);
    let gi = pos.board[g.board.sq(3, 3) as usize] as usize;

    // Full HP (3): the ride runs, the desperation diagonal sleeps.
    let full = legal_moves(&g, &mut pos);
    assert!(
        full.iter().any(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(3, 6)),
        "min_hp 2 ride active at full HP"
    );
    assert!(
        !full.iter().any(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(4, 4)),
        "max_hp 1 diagonal dormant at full HP"
    );
    assert!(pos.is_attacked(&g, g.board.sq(3, 6), 0), "attack map sees the ride");
    assert!(!pos.is_attacked(&g, g.board.sq(4, 4), 0), "and not the dormant diagonal");

    // Worn to 1 HP: the ride dies, the desperation Bit wakes.
    pos.hp[gi] = 1;
    pos.rehash(&g);
    let low = legal_moves(&g, &mut pos);
    assert!(
        !low.iter().any(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(3, 6)),
        "min_hp 2 ride suppressed at 1 HP"
    );
    assert!(
        low.iter().any(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(4, 4)),
        "max_hp 1 diagonal wakes at 1 HP"
    );
    assert!(!pos.is_attacked(&g, g.board.sq(3, 6), 0), "attack map loses the ride");
    assert!(pos.is_attacked(&g, g.board.sq(4, 4), 0), "and gains the diagonal");

    // Apply/undo across a gated landscape stays hash-clean (the debug
    // make also asserts the incremental key).
    let mv = low
        .iter()
        .find(|m| m.from == g.board.sq(3, 3) && m.to == g.board.sq(4, 4))
        .copied()
        .unwrap();
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);
}

#[test]
fn desperation_bits_price_discounted() {
    use botboard_core::cost::{cost_prior, CostWeights};
    // Two 3-HP file riders; one carries an extra desperation kernel that
    // only wakes below max HP. The full-HP mobility integral cannot see
    // it, and the documented ×0.85 conditional-vocabulary discount
    // applies — so the ratio is exactly 0.85.
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1)]).royal(),
        PieceTypeDef::new("plain", 'A', vec![MoveBit::rider(0, 1)]).hp(3),
        PieceTypeDef::new(
            "desperate",
            'B',
            vec![MoveBit::rider(0, 1), MoveBit::leaper(1, 1).max_hp(1)],
        )
        .hp(3),
    ];
    let g = GameDef::new(
        BoardDef { w: 8, h: 8 },
        2,
        types,
        vec![],
        Policy {
            stalemate: StalematePolicy::Draw,
            capture_fate: CaptureFate::Destroy,
            turn: TurnPolicy::Alternate,
            pass: PassPolicy::Forbidden,
        },
        vec![(0, 0, 0, 0), (0, 1, 7, 7)],
    );
    let w = CostWeights { w_move: 0.35, w_attack: 0.35 };
    let plain = cost_prior(&g, 1, &w);
    let desperate = cost_prior(&g, 2, &w);
    assert!(
        (desperate / plain - 0.85).abs() < 0.02,
        "gates excluding full-hp discount ×0.85: {desperate:.2} vs {plain:.2}"
    );
}

// ---------------------------------------------------------------------------
// 7. Board-class equivalence with the new vocabulary
// ---------------------------------------------------------------------------

#[test]
fn board_classes_agree_with_batch2_terrain() {
    // The wide-bitboard path must yield to the mailbox for drillers when
    // walls/blocks exist (wall_count gating) and must never emit HP-gated
    // kernels from its plain tables: perft equality is the proof.
    use botboard_core::perft::perft;
    let mut counts = Vec::new();
    for use_bb in [false, true] {
        let mut g = robot_game(vec![
            (KING, 0, 0, 0),
            (KING, 1, 7, 7),
            (TANK, 0, 3, 1),
            (DRILLER, 0, 5, 1),
            (GATED, 0, 1, 1),
            (SCOUT, 1, 4, 6),
            (DRONE, 1, 6, 6),
        ]);
        g.use_bitboards = use_bb;
        let mut pos = Position::startpos(&g);
        pos.terrain[g.board.sq(3, 3) as usize] = T_WALL;
        pos.terrain[g.board.sq(5, 3) as usize] = T_BLOCK2;
        pos.terrain[g.board.sq(6, 3) as usize] = T_PIT;
        pos.terrain[g.board.sq(1, 4) as usize] = T_CONV_E;
        let gi = pos.board[g.board.sq(1, 1) as usize] as usize;
        pos.hp[gi] = 1; // wake the desperation kernel on one piece
        pos.rehash(&g);
        counts.push(perft(&g, &mut pos, 3));
    }
    assert_eq!(counts[0], counts[1], "mailbox and bitboard classes agree");
    assert!(counts[0] > 0);
}
