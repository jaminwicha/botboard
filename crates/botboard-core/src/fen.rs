//! Chess FEN parsing — test tooling for the acceptance suite's reference
//! positions (§6). Castling rights map onto the engine's `moved` flags.

use crate::game::{GameDef, TypeId};
use crate::moves::NO_SQ;
use crate::position::{Loc, Position};

/// Parse a standard chess FEN against the chess `GameDef`. Supports the
/// piece-placement, side-to-move, castling, and en-passant fields.
pub fn chess_from_fen(g: &GameDef, fen: &str) -> Result<Position, String> {
    let mut parts = fen.split_whitespace();
    let placement = parts.next().ok_or("empty FEN")?;
    let stm = match parts.next().unwrap_or("w") {
        "w" => 0,
        "b" => 1,
        s => return Err(format!("bad side: {s}")),
    };
    let castling = parts.next().unwrap_or("-");
    let ep_str = parts.next().unwrap_or("-");

    let glyph_type = |c: char| -> Option<TypeId> {
        g.types
            .iter()
            .position(|t| t.glyph == c.to_ascii_uppercase())
            .map(|i| i as TypeId)
    };

    let mut list: Vec<(TypeId, u8, u16, bool)> = Vec::new();
    let mut y = g.board.h as i16 - 1;
    let mut x: i16 = 0;
    for c in placement.chars() {
        match c {
            '/' => {
                y -= 1;
                x = 0;
            }
            '1'..='9' => x += c.to_digit(10).unwrap() as i16,
            _ => {
                let t = glyph_type(c).ok_or_else(|| format!("bad glyph: {c}"))?;
                let side = if c.is_ascii_uppercase() { 0 } else { 1 };
                if x < 0 || y < 0 || x >= g.board.w as i16 || y >= g.board.h as i16 {
                    return Err("placement out of bounds".into());
                }
                // `moved: true` by default; castling rights clear it below.
                list.push((t, side, g.board.sq(x as u8, y as u8), true));
                x += 1;
            }
        }
    }

    let mut pos = Position::from_pieces(g, &list, stm);

    let mut clear_moved = |sq: u16| {
        if let Some(i) = pos.pieces.iter().position(|p| p.loc == Loc::Board(sq)) {
            pos.pieces[i].moved = false;
        }
    };
    let (back0, back1) = (0u8, g.board.h - 1);
    for c in castling.chars() {
        match c {
            'K' => {
                clear_moved(g.board.sq(4, back0));
                clear_moved(g.board.sq(7, back0));
            }
            'Q' => {
                clear_moved(g.board.sq(4, back0));
                clear_moved(g.board.sq(0, back0));
            }
            'k' => {
                clear_moved(g.board.sq(4, back1));
                clear_moved(g.board.sq(7, back1));
            }
            'q' => {
                clear_moved(g.board.sq(4, back1));
                clear_moved(g.board.sq(0, back1));
            }
            _ => {}
        }
    }

    pos.ep = if ep_str == "-" {
        NO_SQ
    } else {
        let b = ep_str.as_bytes();
        g.board.sq(b[0] - b'a', b[1] - b'1')
    };

    Ok(pos)
}
