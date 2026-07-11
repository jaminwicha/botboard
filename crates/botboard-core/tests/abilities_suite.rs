//! Phase-3 model features (§3.2, §3.4, §7.5): hit-count armor, abilities
//! as the turn's action, compound moves, mutable terrain — exercised with
//! Robot-Wars-flavored content composed purely from Bits.

use botboard_core::bits::{AbilityBit, MoveBit};
use botboard_core::game::*;
use botboard_core::movegen::{legal_moves};
use botboard_core::moves::{move_str, Effect, Move, MoveKind, NO_SQ};
use botboard_core::position::{Loc, Position, T_ACID, T_ICE, T_NONE, T_WALL};

const KING: TypeId = 0;
const TANK: TypeId = 1; // rider(0,1), 3 HP armor
const MEDIC: TypeId = 2; // leaper(0,1)+(1,1), heal 1 range 1
const ENGINEER: TypeId = 3; // leaper(0,1), wall range 1
const LASERBOT: TypeId = 4; // leaper(0,1), laser range 3 with retreat
const SCOUT: TypeId = 5; // leaper(0,1)+(1,1), overclock, 2 HP
const NECRO: TypeId = 6; // leaper(0,1), resurrect range 1
const HACKER: TypeId = 7; // leaper(0,1), hack range 2
const DECOY: TypeId = 8; // hologram: leaper(0,1)+(1,1), no threat
const SAPPER: TypeId = 9; // leaper(0,1), mine-layer range 1

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
        PieceTypeDef::new("necro", 'N', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::Resurrect { range: 1 }]),
        PieceTypeDef::new("hacker", 'H', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::Hack { range: 2 }]),
        PieceTypeDef::new("decoy", 'D', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .hologram(),
        PieceTypeDef::new("sapper", 'P', vec![MoveBit::leaper(0, 1)])
            .abilities(vec![AbilityBit::MineLayer { range: 1 }]),
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
    let h0 = pos.hash;

    let attack = Move::normal(g.board.sq(3, 3), g.board.sq(3, 6));
    let victim_idx = pos.board[g.board.sq(3, 6) as usize] as usize;

    // Strike 1: victim 3→2 HP, attacker does not move.
    let u1 = pos.make(&g, &attack);
    assert_eq!(pos.hp[victim_idx], 2);
    assert!(pos.piece_at(g.board.sq(3, 3)).is_some(), "attacker stays on a strike");
    let h1 = pos.hash;
    assert_ne!(h0, h1, "HP is in the state bucket (§7.5)");
    // Board-identical positions differing only in HP are different nodes.
    pos.unmake(&g, &u1);
    assert_eq!(pos.hash, h0, "unmake restores the full state key");

    pos.make(&g, &attack); // 3→2
    pos.set_stm(&g, 0);
    pos.make(&g, &attack); // 2→1
    assert_eq!(pos.hp[victim_idx], 1);
    pos.set_stm(&g, 0);
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
    pos.rehash(&g);

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

    // Tank (file rider) initially attacks down the file toward y=0.
    assert!(pos.is_attacked(&g, g.board.sq(4, 1), 1));

    let wall = Move::ability(g.board.sq(3, 4), g.board.sq(4, 5), Effect::Wall, NO_SQ);
    let moves = legal_moves(&g, &mut pos);
    assert!(moves.contains(&wall), "engineer offers wall creation");

    let h0 = pos.hash;
    let u = pos.make(&g, &wall);
    assert_eq!(pos.terrain[g.board.sq(4, 5) as usize], T_WALL);
    assert!(
        !pos.is_attacked(&g, g.board.sq(4, 1), 1),
        "wall interrupts the tank's ray (terrain mutates the blocker set, §7.2)"
    );
    assert_ne!(pos.hash, h0, "terrain XORs into the key (§7.5)");
    pos.unmake(&g, &u);
    assert_eq!(pos.terrain[g.board.sq(4, 5) as usize], T_NONE);
    assert_eq!(pos.hash, h0);
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
    let scout_idx = pos.board[g.board.sq(3, 3) as usize] as usize;

    let moves = legal_moves(&g, &mut pos);
    let compounds: Vec<Move> =
        moves.iter().filter(|m| m.kind == MoveKind::Compound).copied().collect();
    assert!(!compounds.is_empty(), "overclock generates compound moves");
    // Bounded locally (§3.4): only the scout generates them, and the count
    // is ≤ (own kernel moves)² — 8 dirs × ≤8 second steps here.
    assert!(compounds.len() <= 64, "compound count bounded: {}", compounds.len());
    // The per-piece compound-count metric exists for content vetting (§13).

    let h0 = pos.hash;
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
    assert_eq!(pos.hash, h0, "atomic unmake");
    assert_eq!(pos.hp[scout_idx], 2);

    // At 1 HP the self-damage would be lethal: no compounds offered.
    pos.hp[scout_idx] = 1;
    pos.rehash(&g);
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

#[test]
fn resurrect_revives_a_dead_friendly_at_one_hp() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (NECRO, 0, 4, 4),
        (MEDIC, 0, 3, 3),
        (TANK, 1, 3, 6),
    ]);
    let mut pos = Position::startpos(&g);
    // Nobody dead: no resurrect on offer.
    assert!(
        !legal_moves(&g, &mut pos).iter().any(|m| m.effect == Effect::Resurrect),
        "resurrect needs a dead friendly"
    );

    // The medic falls (out-of-band, as a battle would leave it).
    let mi = pos.board[g.board.sq(3, 3) as usize] as usize;
    pos.board[g.board.sq(3, 3) as usize] = -1;
    pos.pieces[mi].loc = Loc::Dead;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    let rez: Vec<_> = moves.iter().filter(|m| m.effect == Effect::Resurrect).collect();
    assert!(!rez.is_empty(), "necro must offer resurrect once a friendly is dead");
    assert!(
        rez.iter().all(|m| m.drop_type == MEDIC),
        "only the dead medic is revivable (royals never are)"
    );
    // Notation is unambiguous about which type re-enters.
    assert!(move_str(&g, rez[0]).contains("!rezM:"));

    let mv = rez
        .iter()
        .find(|m| m.to == g.board.sq(4, 5))
        .copied()
        .copied()
        .expect("an in-range empty square");
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert!(matches!(pos.pieces[mi].loc, Loc::Board(_)), "medic back on board");
    assert_eq!(pos.hp[mi], 1, "revived at 1 HP");
    assert_eq!(pos.stm, 1, "resurrect consumed the turn (§3.4)");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0, "unmake restores the full state key");
    assert!(matches!(pos.pieces[mi].loc, Loc::Dead), "medic dead again");
}

#[test]
fn resurrect_offers_one_candidate_per_dead_type() {
    // Two dead medics must not double the move list: piece identity within
    // a type is interchangeable.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (NECRO, 0, 4, 4),
        (MEDIC, 0, 2, 2),
        (MEDIC, 0, 2, 4),
        (TANK, 1, 3, 6),
    ]);
    let mut pos = Position::startpos(&g);
    for sq in [g.board.sq(2, 2), g.board.sq(2, 4)] {
        let i = pos.board[sq as usize] as usize;
        pos.board[sq as usize] = -1;
        pos.pieces[i].loc = Loc::Dead;
    }
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    let rez: Vec<_> = moves.iter().filter(|m| m.effect == Effect::Resurrect).collect();
    let unique: std::collections::HashSet<_> =
        rez.iter().map(|m| (m.to, m.drop_type)).collect();
    assert_eq!(rez.len(), unique.len(), "no duplicate resurrect candidates");
    // Reviving twice brings back both bodies, lowest index first.
    let first = rez[0];
    let u1 = pos.make(&g, first);
    let again: Vec<_> = {
        pos.set_stm(&g, 0);
        let ms = legal_moves(&g, &mut pos);
        ms.into_iter().filter(|m| m.effect == Effect::Resurrect).collect()
    };
    assert!(!again.is_empty(), "second body still revivable");
    pos.set_stm(&g, 1);
    pos.unmake(&g, &u1);
}

#[test]
fn hack_flips_a_weakened_enemy() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (HACKER, 0, 3, 3),
        (TANK, 1, 4, 4),   // 3 HP: defended, unhackable
        (SCOUT, 1, 2, 4),  // 2 HP: unhackable until damaged
    ]);
    let mut pos = Position::startpos(&g);
    assert!(
        !legal_moves(&g, &mut pos).iter().any(|m| m.effect == Effect::Hack),
        "healthy targets resist the hack (predicate: exactly 1 HP)"
    );

    // Wear the scout down to 1 HP: its defenses are open.
    let si = pos.board[g.board.sq(2, 4) as usize] as usize;
    pos.hp[si] = 1;
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    let hacks: Vec<_> = moves.iter().filter(|m| m.effect == Effect::Hack).collect();
    assert_eq!(hacks.len(), 1, "only the weakened scout is hackable");
    assert_eq!(hacks[0].to, g.board.sq(2, 4));
    assert!(move_str(&g, hacks[0]).contains("!hack:"));

    let mv = *hacks[0];
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert_eq!(pos.pieces[si].side, 0, "the scout fights for us now");
    assert_eq!(pos.stm, 1, "hack consumed the turn (§3.4)");
    pos.unmake(&g, &u);
    assert_eq!(pos.pieces[si].side, 1, "unmake gives it back");
    assert_eq!(pos.hash, h0, "unmake restores the full state key");
}

#[test]
fn hack_never_touches_royals() {
    // Enemy king wandered adjacent at 1 HP: still not a legal hack target.
    let g = robot_game(vec![(KING, 0, 0, 0), (HACKER, 0, 3, 3), (KING, 1, 4, 4)]);
    let mut pos = Position::startpos(&g);
    assert!(
        !legal_moves(&g, &mut pos).iter().any(|m| m.effect == Effect::Hack),
        "royals cannot be flipped — capture-the-controller is its own system"
    );
}

#[test]
fn holograms_move_but_never_threaten() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (DECOY, 0, 3, 3),
        (TANK, 1, 3, 4), // adjacent enemy: a real piece would capture it
    ]);
    let mut pos = Position::startpos(&g);

    let moves = legal_moves(&g, &mut pos);
    let decoy_moves: Vec<_> =
        moves.iter().filter(|m| m.from == g.board.sq(3, 3)).collect();
    assert!(!decoy_moves.is_empty(), "the decoy still moves");
    assert!(
        decoy_moves.iter().all(|m| pos.piece_at(m.to).is_none()),
        "a hologram never captures"
    );
    // And it projects no threat: the adjacent enemy square is not attacked.
    assert!(!pos.is_attacked(&g, g.board.sq(3, 4), 0));

    // It vanishes to a single hit.
    pos.set_stm(&g, 1);
    let di = pos.board[g.board.sq(3, 3) as usize] as usize;
    pos.make(&g, &Move::normal(g.board.sq(3, 4), g.board.sq(3, 3)));
    assert!(matches!(pos.pieces[di].loc, Loc::Dead), "decoys pop when hit");
}

#[test]
fn mines_arm_spend_and_kill() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (SAPPER, 0, 3, 3),
        (SCOUT, 1, 4, 5),  // 2 HP: survives a mine with a scar
        (TANK, 0, 3, 5),   // friendly: walks over its own side's mine
    ]);
    let mut pos = Position::startpos(&g);

    // Lay a mine at d5 (adjacent to the sapper at d4).
    let moves = legal_moves(&g, &mut pos);
    let lay = moves
        .iter()
        .find(|m| m.effect == Effect::Mine && m.to == g.board.sq(3, 4))
        .copied()
        .expect("sapper offers mine-laying");
    assert!(move_str(&g, &lay).contains("!mine:"));
    let h0 = pos.hash;
    let u = pos.make(&g, &lay);
    assert!(pos.terrain[g.board.sq(3, 4) as usize] >= 3, "mine terrain laid");
    assert_ne!(pos.hash, h0, "mines hash as terrain");
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);
    pos.make(&g, &lay);

    // The enemy scout steps on it: 2 HP -> 1, mine spent.
    let si = pos.board[g.board.sq(4, 5) as usize] as usize;
    let u2 = pos.make(&g, &Move::normal(g.board.sq(4, 5), g.board.sq(3, 4)));
    assert_eq!(pos.hp[si], 1, "the trap bites for 1 HP");
    assert_eq!(pos.terrain[g.board.sq(3, 4) as usize], 0, "the mine is spent");
    pos.unmake(&g, &u2);
    assert_eq!(pos.hp[si], 2, "unmake heals the scar");
    assert!(pos.terrain[g.board.sq(3, 4) as usize] >= 3, "and re-arms the mine");

    // At 1 HP the trap is lethal — and fully reversible.
    pos.hp[si] = 1;
    pos.rehash(&g);
    let h1 = pos.hash;
    let u3 = pos.make(&g, &Move::normal(g.board.sq(4, 5), g.board.sq(3, 4)));
    assert!(matches!(pos.pieces[si].loc, Loc::Dead), "lethal at 1 HP");
    assert!(pos.piece_at(g.board.sq(3, 4)).is_none(), "the crater is empty");
    pos.unmake(&g, &u3);
    assert_eq!(pos.hash, h1, "unmake climbs out of the crater");
    assert!(matches!(pos.pieces[si].loc, Loc::Board(_)));

    // Friendly pieces stroll over their own minefield.
    pos.set_stm(&g, 0);
    let ti = pos.board[g.board.sq(3, 5) as usize] as usize;
    pos.make(&g, &Move::normal(g.board.sq(3, 5), g.board.sq(3, 4)));
    assert!(matches!(pos.pieces[ti].loc, Loc::Board(_)));
    assert_eq!(pos.hp[ti], 3, "own mines do not bite");
    assert!(pos.terrain[g.board.sq(3, 4) as usize] >= 3, "and stay armed");
}

#[test]
fn ice_slides_riders_but_not_leapers() {
    // Tank is a file-rider; scout leaps. Ice sheet down the d-file.
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (TANK, 0, 3, 1),
        (SCOUT, 0, 5, 1),
    ]);
    let mut pos = Position::startpos(&g);
    for y in [3u8, 4, 5] {
        pos.terrain[g.board.sq(3, y) as usize] = T_ICE;
        pos.terrain[g.board.sq(5, y - 1) as usize] = T_ICE;
    }
    pos.rehash(&g);

    let moves = legal_moves(&g, &mut pos);
    // The tank aiming onto the sheet skids to the first floor past it (d7).
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(3, 1) && m.to == g.board.sq(3, 6)),
        "rider slides across the whole sheet"
    );
    // No rider move may STOP on open mid-sheet ice.
    assert!(
        !moves
            .iter()
            .any(|m| m.from == g.board.sq(3, 1)
                && (m.to == g.board.sq(3, 3) || m.to == g.board.sq(3, 4))),
        "a slider cannot stop mid-sheet"
    );
    // The leaper lands on ice and just stands there.
    assert!(
        moves.iter().any(|m| m.from == g.board.sq(5, 1) && m.to == g.board.sq(5, 2)),
        "leapers do not skid"
    );

    // A wall at the sheet's end parks the slider on the last ice cell.
    pos.terrain[g.board.sq(3, 6) as usize] = T_WALL;
    pos.rehash(&g);
    let moves2 = legal_moves(&g, &mut pos);
    assert!(
        moves2.iter().any(|m| m.from == g.board.sq(3, 1) && m.to == g.board.sq(3, 5)),
        "blocked slide stops on the final ice cell"
    );
}

#[test]
fn acid_bites_anyone_who_wades_in() {
    let g = robot_game(vec![
        (KING, 0, 0, 0),
        (KING, 1, 7, 7),
        (SCOUT, 0, 3, 3), // 2 HP
        (MEDIC, 1, 5, 5), // 1 HP
    ]);
    let mut pos = Position::startpos(&g);
    pos.terrain[g.board.sq(3, 4) as usize] = T_ACID;
    pos.terrain[g.board.sq(5, 4) as usize] = T_ACID;
    pos.rehash(&g);

    // Friendly-to-nobody: the pool bites your own side too.
    let si = pos.board[g.board.sq(3, 3) as usize] as usize;
    let h0 = pos.hash;
    let u = pos.make(&g, &Move::normal(g.board.sq(3, 3), g.board.sq(3, 4)));
    assert_eq!(pos.hp[si], 1, "wading in costs 1 HP");
    assert_eq!(
        pos.terrain[g.board.sq(3, 4) as usize],
        T_ACID,
        "the pool persists"
    );
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);
    assert_eq!(pos.hp[si], 2);

    // Lethal at 1 HP, fully reversible.
    pos.set_stm(&g, 1);
    let mi = pos.board[g.board.sq(5, 5) as usize] as usize;
    let h1 = pos.hash;
    let u2 = pos.make(&g, &Move::normal(g.board.sq(5, 5), g.board.sq(5, 4)));
    assert!(matches!(pos.pieces[mi].loc, Loc::Dead), "the pool dissolves 1-HP waders");
    pos.unmake(&g, &u2);
    assert_eq!(pos.hash, h1);
    assert!(matches!(pos.pieces[mi].loc, Loc::Board(_)));
}

