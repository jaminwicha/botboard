//! The acceptance suite (spec §6): chess, xiangqi, and shogi reconstructed
//! from Bits, validated by perft against published reference node counts
//! (Fairy-Stockfish / Chess Programming Wiki values).
//!
//! Deep counts (chess d6, xiangqi d5, shogi d5) are `#[ignore]`d for CI
//! speed; run with `cargo test --release -- --ignored`.

use botboard_core::fen::chess_from_fen;
use botboard_core::perft::perft;
use botboard_core::position::Position;
use botboard_core::variants;

fn chess_fen_perft(fen: &str, expected: &[u64]) {
    let g = variants::chess::game();
    let mut pos = chess_from_fen(&g, fen).unwrap();
    for (i, &e) in expected.iter().enumerate() {
        assert_eq!(perft(&g, &mut pos, i as u32 + 1), e, "{fen} depth {}", i + 1);
    }
}

#[test]
fn chess_startpos() {
    let g = variants::chess::game();
    let mut pos = Position::startpos(&g);
    for (d, e) in [(1, 20), (2, 400), (3, 8902), (4, 197281), (5, 4865609)] {
        assert_eq!(perft(&g, &mut pos, d), e, "chess startpos depth {d}");
    }
}

#[test]
#[ignore]
fn chess_startpos_deep() {
    let g = variants::chess::game();
    let mut pos = Position::startpos(&g);
    assert_eq!(perft(&g, &mut pos, 6), 119060324);
}

#[test]
fn chess_kiwipete() {
    chess_fen_perft(
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        &[48, 2039, 97862, 4085603],
    );
}

#[test]
fn chess_position3() {
    chess_fen_perft(
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        &[14, 191, 2812, 43238, 674624],
    );
}

#[test]
fn chess_position4() {
    chess_fen_perft(
        "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        &[6, 264, 9467, 422333],
    );
}

#[test]
fn chess_position5() {
    chess_fen_perft(
        "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        &[44, 1486, 62379, 2103487],
    );
}

#[test]
fn xiangqi_startpos() {
    let g = variants::xiangqi::game();
    let mut pos = Position::startpos(&g);
    for (d, e) in [(1, 44), (2, 1920), (3, 79666), (4, 3290240)] {
        assert_eq!(perft(&g, &mut pos, d), e, "xiangqi startpos depth {d}");
    }
}

#[test]
#[ignore]
fn xiangqi_startpos_deep() {
    let g = variants::xiangqi::game();
    let mut pos = Position::startpos(&g);
    assert_eq!(perft(&g, &mut pos, 5), 133312995);
}

#[test]
fn shogi_startpos() {
    let g = variants::shogi::game();
    let mut pos = Position::startpos(&g);
    for (d, e) in [(1, 30), (2, 900), (3, 25470), (4, 719731)] {
        assert_eq!(perft(&g, &mut pos, d), e, "shogi startpos depth {d}");
    }
}

#[test]
#[ignore]
fn shogi_startpos_deep() {
    let g = variants::shogi::game();
    let mut pos = Position::startpos(&g);
    assert_eq!(perft(&g, &mut pos, 5), 19861490);
}
