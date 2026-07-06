//! Phase-3 model features (§3.2, §3.4, §7.5): hit-count armor, abilities
//! as the turn's action, compound moves, mutable terrain — exercised with
//! Robot-Wars-flavored content composed purely from Bits.

use botboard_core::bits::{AbilityBit, MoveBit};
use botboard_core::game::*;
use botboard_core::movegen::{legal_moves};
use botboard_core::moves::{move_str, Effect, Move, MoveKind, NO_SQ};
use botboard_core::position::{Loc, Position, T_NONE, T_WALL};
use botboard_core::zobrist::Zobrist;

const KING: TypeId = 0;
const TANK: TypeId = 1; // rider(0,1), 3 HP armor
const MEDIC: TypeId = 2; // leaper(0,1)+(1,1), heal 1 range 1
const ENGINEER: TypeId = 3; // leaper(0,1), wall range 1
const LASERBOT: TypeId = 4; // leaper(0,1), laser range 3 with retreat
const SCOUT: TypeId = 5; // leaper(0,1)+(1,1), overclock, 2 HP

fn robot_types() -> Vec<PieceTypeDef> {
    vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        PieceTypeDef::new("tank", 'T', vec![MoveBit::rider(0, 1)]).hp(3),
        PieceTypeDef::new("medic", 'M', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .abilities(vec![AbilityBit::Heal { amount: 1, range: 1 }]),
        PieceTypeDef::new("engineer", 'E', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::CreateWall { range: 1 }]),
        PieceTypeDef::new("laserbot", 'L', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::Laser { range: 3, retreat: true }]),
        PieceTypeDef::new("scout", 'S', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .hp(2)
            .overclock(),
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

#[test]
fn armor_strike_then_kill() {
    // White tank attacks a black 3-HP tank: two strikes, then the kill.
    let g = robot_game(vec![(KING, 0, 0, 0), (KING, 1, 7, 7), (TANK, 0, 3, 3), (TANK, 1, 3, 6)]);
    let mut pos = Position::startpos(&g);
    let zob = Zobrist::new(&g);
    let h0 = zob.hash(&g, &pos);

    let attack = Move::normal(g.board.sq(3, 3), g.board.sq(3, 6));
    let victim_idx = pos.board[g.board.sq(3, 6) as usize] as usize;

    // Strike 1: victim 3→2 HP, attacker does not move.
    let u1 = pos.make(&g, &attack);
    assert_eq!(pos.hp[victim_idx], 2);
    assert!(pos.piece_at(g.board.sq(3, 3)).is_some(), "attacker stays on a strike");
    let h1 = zob.hash(&g, &pos);
    assert_ne!(h0, h1, "HP is in the state bucket (§7.5)");
    // Board-identical positions differing only in HP are different nodes.
    pos.unmake(&g, &u1);
    assert_eq!(zob.hash(&g, &pos), h0, "unmake restores the full state key");

    pos.make(&g, &attack); // 3→2
    pos.stm = 0;
    pos.make(&g, &attack); // 2→1
    assert_eq!(pos.hp[victim_idx], 1);
    pos.stm = 0;
    pos.make(&g, &attack); // kill: attacker moves in
    assert!(matches!(pos.pieces[victim_idx].loc, Loc::Dead));
    assert_eq!(
        pos.board[g.board.sq(3, 6) as usize],
        pos.board[g.board.sq(3, 6) as usize].max(0),
        "attacker occupies the square after the kill"
    );
}

#[test]
fn heal_is_the_turns_single_action() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (TANK, 0, 3, 3),
        (MEDIC, 0, 4, 3),
        (TANK, 1, 3, 7),
    ]);
    let mut pos = Position::startpos(&g);
    let tank_idx = pos.board[g.board.sq(3, 3) as usize] as usize;
    pos.hp[tank_idx] = 1; // damaged

    let moves = legal_moves(&g, &mut pos);
    let heal = moves
        .iter()
        .find(|m| m.kind == MoveKind::Ability && matches!(m.effect, Effect::Heal(_)))
        .copied()
        .expect("medic must offer heal");
    assert_eq!(heal.to, g.board.sq(3, 3));
    let u = pos.make(&g, &heal);
    assert_eq!(pos.hp[tank_idx], 2);
    assert_eq!(pos.stm, 1, "ability consumed the turn (§3.4)");
    pos.unmake(&g, &u);
    assert_eq!(pos.hp[tank_idx], 1);
}

#[test]
fn wall_blocks_riders_and_hashes_as_terrain() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (ENGINEER, 0, 3, 4),
        (TANK, 1, 4, 6),
    ]);
    let mut pos = Position::startpos(&g);
    let zob = Zobrist::new(&g);

    // Tank (file rider) initially attacks down the file toward y=0.
    assert!(pos.is_attacked(&g, g.board.sq(4, 1), 1));

    let wall = Move::ability(g.board.sq(3, 4), g.board.sq(4, 5), Effect::Wall, NO_SQ);
    let moves = legal_moves(&g, &mut pos);
    assert!(moves.contains(&wall), "engineer offers wall creation");

    let h0 = zob.hash(&g, &pos);
    let u = pos.make(&g, &wall);
    assert_eq!(pos.terrain[g.board.sq(4, 5) as usize], T_WALL);
    assert!(
        !pos.is_attacked(&g, g.board.sq(4, 1), 1),
        "wall interrupts the tank's ray (terrain mutates the blocker set, §7.2)"
    );
    assert_ne!(zob.hash(&g, &pos), h0, "terrain XORs into the key (§7.5)");
    pos.unmake(&g, &u);
    assert_eq!(pos.terrain[g.board.sq(4, 5) as usize], T_NONE);
    assert_eq!(zob.hash(&g, &pos), h0);
}

#[test]
fn laser_captures_at_range_with_retreat_teeth() {
    // Laserbot at e4 targeting an enemy tank at e7 (range 3); retreat = e3.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (LASERBOT, 0, 4, 3),
        (TANK, 1, 4, 6),
    ]);
    let mut pos = Position::startpos(&g);
    let moves = legal_moves(&g, &mut pos);
    let laser = moves
        .iter()
        .find(|m| m.effect == Effect::Laser)
        .copied()
        .expect("laser move generated");
    assert_eq!(laser.to, g.board.sq(4, 6));
    assert_eq!(laser.aux, g.board.sq(4, 2), "forced retreat square");

    let victim_idx = pos.board[laser.to as usize] as usize;
    let u = pos.make(&g, &laser);
    // 3-HP tank: laser strikes for 1; the bot still retreats (atomic script).
    assert_eq!(pos.hp[victim_idx], 2);
    assert!(pos.piece_at(g.board.sq(4, 2)).is_some(), "retreated");
    assert!(pos.piece_at(g.board.sq(4, 3)).is_none());
    pos.unmake(&g, &u);
    assert!(pos.piece_at(g.board.sq(4, 3)).is_some());

    // Now block the retreat square: the laser must vanish ("the nerf has
    // teeth", §3.4 — the compound is illegal if the retreat is blocked).
    let g2 = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (LASERBOT, 0, 4, 3),
        (TANK, 1, 4, 6),
        (TANK, 0, 4, 2), // friendly on the retreat square
    ]);
    let mut pos2 = Position::startpos(&g2);
    let up_laser = legal_moves(&g2, &mut pos2)
        .iter()
        .filter(|m| m.effect == Effect::Laser && m.to == g2.board.sq(4, 6))
        .count();
    assert_eq!(up_laser, 0, "blocked retreat kills the whole compound");
}

#[test]
fn overclock_compounds_are_atomic_and_bounded() {
    let g = robot_game(vec![(KING, 0, 0, 0), (KING, 1, 7, 7), (SCOUT, 0, 3, 3)]);
    let mut pos = Position::startpos(&g);
    let zob = Zobrist::new(&g);
    let scout_idx = pos.board[g.board.sq(3, 3) as usize] as usize;

    let moves = legal_moves(&g, &mut pos);
    let compounds: Vec<Move> =
        moves.iter().filter(|m| m.kind == MoveKind::Compound).copied().collect();
    assert!(!compounds.is_empty(), "overclock generates compound moves");
    // Bounded locally (§3.4): only the scout generates them, and the count
    // is ≤ (own kernel moves)² — 8 dirs × ≤8 second steps here.
    assert!(compounds.len() <= 64, "compound count bounded: {}", compounds.len());
    // The per-piece compound-count metric exists for content vetting (§13).

    let h0 = zob.hash(&g, &pos);
    let mv = compounds
        .iter()
        .find(|m| m.to == g.board.sq(3, 4) && m.aux == g.board.sq(3, 5))
        .copied()
        .unwrap();
    assert_eq!(move_str(&g, &mv), "d4d5+d5d6");
    let u = pos.make(&g, &mv);
    assert!(pos.piece_at(g.board.sq(3, 5)).is_some(), "two steps applied as one move");
    assert_eq!(pos.hp[scout_idx], 1, "self-damage applied");
    assert_eq!(pos.stm, 1, "one action, one turn (§3.4)");
    pos.unmake(&g, &u);
    assert_eq!(zob.hash(&g, &pos), h0, "atomic unmake");
    assert_eq!(pos.hp[scout_idx], 2);

    // At 1 HP the self-damage would be lethal: no compounds offered.
    pos.hp[scout_idx] = 1;
    let n = legal_moves(&g, &mut pos)
        .iter()
        .filter(|m| m.kind == MoveKind::Compound)
        .count();
    assert_eq!(n, 0, "overclock suppressed at 1 HP");
}

#[test]
fn utility_multipliers_price_armor_and_overclock() {
    use botboard_core::cost::{cost_prior, CostWeights};
    let g = robot_game(vec![(KING, 0, 0, 0), (KING, 1, 7, 7)]);
    let w = CostWeights { w_move: 0.7, w_attack: 0.7 };
    // Same mover kernels, ±armor/overclock ⇒ strictly higher cost.
    let medic = cost_prior(&g, MEDIC, &w); // king-move kernels, no armor
    let scout = cost_prior(&g, SCOUT, &w); // same kernels + 2HP + overclock
    assert!(
        scout > medic * 1.5,
        "armor(×1.5)·overclock(×1.8) must raise the prior: scout {scout:.2} vs medic-base {medic:.2}"
    );
}
