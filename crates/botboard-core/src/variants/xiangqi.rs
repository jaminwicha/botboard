//! Xiangqi from Bits (spec Appendix A): lame leapers, the mode-split cannon,
//! palace/river zones, and the flying-general target predicate.

use crate::bits::{Mode, MoveBit, PathRule, TargetPred};
use crate::game::*;
use crate::geometry::DirFilter;

pub const GENERAL: TypeId = 0;
pub const ADVISOR: TypeId = 1;
pub const ELEPHANT: TypeId = 2;
pub const HORSE: TypeId = 3;
pub const CHARIOT: TypeId = 4;
pub const CANNON: TypeId = 5;
pub const SOLDIER: TypeId = 6;

const Z_PALACE: usize = 0;
const Z_OWN_HALF: usize = 1;
const Z_ENEMY_HALF: usize = 2;

pub fn game() -> GameDef {
    let board = BoardDef { w: 9, h: 10 };

    let mut palace = Zone::new(2);
    palace.add_rect(0, &board, 3, 0, 5, 2);
    palace.add_rect(1, &board, 3, 7, 5, 9);
    let mut own_half = Zone::new(2);
    own_half.add_rect(0, &board, 0, 0, 8, 4);
    own_half.add_rect(1, &board, 0, 5, 8, 9);
    let mut enemy_half = Zone::new(2);
    enemy_half.add_rect(0, &board, 0, 5, 8, 9);
    enemy_half.add_rect(1, &board, 0, 0, 8, 4);

    let types = vec![
        PieceTypeDef::new(
            "general",
            'K',
            vec![
                MoveBit::leaper(0, 1).to_zone(Z_PALACE),
                // Flying general: a capture-only file rider whose target must
                // be the enemy royal — also what makes facing generals illegal.
                MoveBit::rider(0, 1)
                    .dirs(DirFilter::Vertical)
                    .mode(Mode::Capture)
                    .target(TargetPred::EnemyRoyal),
            ],
        )
        .royal(),
        PieceTypeDef::new("advisor", 'A', vec![MoveBit::leaper(1, 1).to_zone(Z_PALACE)]),
        PieceTypeDef::new(
            "elephant",
            'E',
            vec![MoveBit::leaper(2, 2).path(PathRule::Midpoint).to_zone(Z_OWN_HALF)],
        ),
        PieceTypeDef::new("horse", 'H', vec![MoveBit::leaper(1, 2).path(PathRule::Leg)]),
        PieceTypeDef::new("chariot", 'R', vec![MoveBit::rider(0, 1)]),
        PieceTypeDef::new(
            "cannon",
            'C',
            vec![
                MoveBit::rider(0, 1).mode(Mode::Move),
                MoveBit::hopper(0, 1), // capture-only, one screen
            ],
        ),
        PieceTypeDef::new(
            "soldier",
            'P',
            vec![
                MoveBit::leaper(0, 1).dirs(DirFilter::Forward),
                MoveBit::leaper(0, 1).dirs(DirFilter::Sideways).from_zone(Z_ENEMY_HALF),
            ],
        ),
    ];

    let policy = Policy {
        stalemate: StalematePolicy::Loss,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };

    let mut start = Vec::new();
    let back = [CHARIOT, HORSE, ELEPHANT, ADVISOR, GENERAL, ADVISOR, ELEPHANT, HORSE, CHARIOT];
    for (x, &t) in back.iter().enumerate() {
        start.push((t, 0, x as u8, 0));
        start.push((t, 1, x as u8, 9));
    }
    for x in [1u8, 7] {
        start.push((CANNON, 0, x, 2));
        start.push((CANNON, 1, x, 7));
    }
    for x in [0u8, 2, 4, 6, 8] {
        start.push((SOLDIER, 0, x, 3));
        start.push((SOLDIER, 1, x, 6));
    }

    GameDef::new(board, 2, types, vec![palace, own_half, enemy_half], policy, start)
}
