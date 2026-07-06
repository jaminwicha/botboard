//! Western chess from Bits (spec Appendix A).

use crate::bits::{Mode, MoveBit, SpecialBit};
use crate::game::*;
use crate::geometry::DirFilter;

pub const K: TypeId = 0;
pub const Q: TypeId = 1;
pub const R: TypeId = 2;
pub const B: TypeId = 3;
pub const N: TypeId = 4;
pub const P: TypeId = 5;

const Z_PAWN_START: usize = 0;
const Z_LAST_RANK: usize = 1;

pub fn game() -> GameDef {
    let board = BoardDef { w: 8, h: 8 };

    let mut pawn_start = Zone::new(2);
    pawn_start.add_rect(0, &board, 0, 1, 7, 1);
    pawn_start.add_rect(1, &board, 0, 6, 7, 6);
    let mut last_rank = Zone::new(2);
    last_rank.add_rect(0, &board, 0, 7, 7, 7);
    last_rank.add_rect(1, &board, 0, 0, 7, 0);

    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal()
            .specials(vec![SpecialBit::Castling]),
        PieceTypeDef::new("queen", 'Q', vec![MoveBit::rider(0, 1), MoveBit::rider(1, 1)]),
        PieceTypeDef::new("rook", 'R', vec![MoveBit::rider(0, 1)]).castle_partner(),
        PieceTypeDef::new("bishop", 'B', vec![MoveBit::rider(1, 1)]),
        PieceTypeDef::new("knight", 'N', vec![MoveBit::leaper(1, 2)]),
        PieceTypeDef::new(
            "pawn",
            'P',
            vec![
                MoveBit::leaper(0, 1).dirs(DirFilter::Forward).mode(Mode::Move),
                MoveBit::leaper(1, 1).dirs(DirFilter::Forward).mode(Mode::Capture),
            ],
        )
        .specials(vec![
            SpecialBit::DoubleStep { start_zone: Z_PAWN_START },
            SpecialBit::EnPassant,
        ])
        .promo(PromoRule {
            zone: Z_LAST_RANK,
            choices: vec![Q, R, B, N],
            trigger: PromoTrigger::Dest,
            optional: false,
        }),
    ];

    let policy = Policy {
        stalemate: StalematePolicy::Draw,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };

    let mut start = Vec::new();
    let back = [R, N, B, Q, K, B, N, R];
    for (x, &t) in back.iter().enumerate() {
        start.push((t, 0, x as u8, 0));
        start.push((t, 1, x as u8, 7));
    }
    for x in 0..8 {
        start.push((P, 0, x, 1));
        start.push((P, 1, x, 6));
    }

    GameDef::new(board, 2, types, vec![pawn_start, last_rank], policy, start)
}
