//! Shogi from Bits (spec Appendix A): capture-to-hand, drops with the three
//! legality tiers, optional/forced promotion with base-type de-promotion.

use crate::bits::MoveBit;
use crate::game::*;
use crate::geometry::{Delta, DirFilter};

pub const K: TypeId = 0;
pub const R: TypeId = 1;
pub const B: TypeId = 2;
pub const G: TypeId = 3;
pub const S: TypeId = 4;
pub const N: TypeId = 5;
pub const L: TypeId = 6;
pub const P: TypeId = 7;
pub const PR: TypeId = 8; // dragon
pub const PB: TypeId = 9; // horse
pub const PS: TypeId = 10;
pub const PN: TypeId = 11;
pub const PL: TypeId = 12;
pub const PP: TypeId = 13; // tokin

const Z_PROMO: usize = 0;

fn gold_bits() -> Vec<MoveBit> {
    vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1).dirs(DirFilter::Forward)]
}

fn promo(to: TypeId) -> PromoRule {
    PromoRule { zone: Z_PROMO, choices: vec![to], trigger: PromoTrigger::FromOrDest, optional: true }
}

pub fn game() -> GameDef {
    let board = BoardDef { w: 9, h: 9 };

    let mut pz = Zone::new(2);
    pz.add_rect(0, &board, 0, 6, 8, 8);
    pz.add_rect(1, &board, 0, 0, 8, 2);

    // The shogi knight jumps only to the two narrow-forward squares.
    let knight_dirs = DirFilter::Custom(vec![Delta::new(1, 2), Delta::new(-1, 2)]);

    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)]).royal(),
        PieceTypeDef::new("rook", 'R', vec![MoveBit::rider(0, 1)])
            .promo(promo(PR))
            .droppable(),
        PieceTypeDef::new("bishop", 'B', vec![MoveBit::rider(1, 1)])
            .promo(promo(PB))
            .droppable(),
        PieceTypeDef::new("gold", 'G', gold_bits()).droppable(),
        PieceTypeDef::new(
            "silver",
            'S',
            vec![MoveBit::leaper(1, 1), MoveBit::leaper(0, 1).dirs(DirFilter::Forward)],
        )
        .promo(promo(PS))
        .droppable(),
        PieceTypeDef::new("knight", 'N', vec![MoveBit::leaper(1, 2).dirs(knight_dirs)])
            .promo(promo(PN))
            .droppable(),
        PieceTypeDef::new("lance", 'L', vec![MoveBit::rider(0, 1).dirs(DirFilter::Forward)])
            .promo(promo(PL))
            .droppable(),
        PieceTypeDef::new("pawn", 'P', vec![MoveBit::leaper(0, 1).dirs(DirFilter::Forward)])
            .promo(promo(PP))
            .droppable()
            .nifu()
            .no_drop_mate(),
        PieceTypeDef::new(
            "dragon",
            'D',
            vec![MoveBit::rider(0, 1), MoveBit::leaper(1, 1)],
        ),
        PieceTypeDef::new(
            "promoted-bishop",
            'H',
            vec![MoveBit::rider(1, 1), MoveBit::leaper(0, 1)],
        ),
        PieceTypeDef::new("promoted-silver", 'T', gold_bits()),
        PieceTypeDef::new("promoted-knight", 'U', gold_bits()),
        PieceTypeDef::new("promoted-lance", 'V', gold_bits()),
        PieceTypeDef::new("tokin", 'W', gold_bits()),
    ];

    let policy = Policy {
        stalemate: StalematePolicy::Loss,
        capture_fate: CaptureFate::ToHand,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };

    let mut start = Vec::new();
    let back = [L, N, S, G, K, G, S, N, L];
    for (x, &t) in back.iter().enumerate() {
        start.push((t, 0, x as u8, 0));
        start.push((t, 1, x as u8, 8));
    }
    start.push((B, 0, 1, 1));
    start.push((R, 0, 7, 1));
    start.push((R, 1, 1, 7));
    start.push((B, 1, 7, 7));
    for x in 0..9 {
        start.push((P, 0, x, 2));
        start.push((P, 1, x, 6));
    }

    GameDef::new(board, 2, types, vec![pz], policy, start)
}
