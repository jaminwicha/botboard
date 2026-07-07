//! Prototype 1 (spec §7.1/§12): the two board classes — mailbox+piece-list
//! and the wide-bitboard (u128) kernel path — must generate identical game
//! trees. Perft equality at depth is the differential proof.

use botboard_core::perft::perft;
use botboard_core::position::Position;
use botboard_core::variants;

#[test]
fn board_classes_agree_on_perft() {
    for (name, mut g, depth, expected) in [
        ("chess", variants::chess::game(), 4, 197281u64),
        ("xiangqi", variants::xiangqi::game(), 3, 79666),
        ("shogi", variants::shogi::game(), 3, 25470),
    ] {
        for use_bb in [false, true] {
            g.use_bitboards = use_bb;
            let mut pos = Position::startpos(&g);
            assert_eq!(
                perft(&g, &mut pos, depth),
                expected,
                "{name} perft({depth}) with use_bitboards={use_bb}"
            );
        }
    }
}
