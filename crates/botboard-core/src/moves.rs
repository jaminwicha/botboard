//! Move representation. One move = one action (§3.4); compounds (later
//! phases) stay a single `Move` whose application script has several steps.

use crate::game::{GameDef, TypeId};

pub const NO_SQ: u16 = u16::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveKind {
    Normal,
    DoubleStep,
    EnPassant,
    Castle,
    Drop,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    /// `NO_SQ` for drops.
    pub from: u16,
    pub to: u16,
    pub kind: MoveKind,
    pub promo: Option<TypeId>,
    /// Only meaningful when `kind == Drop`.
    pub drop_type: TypeId,
    /// Auxiliary square: the victim square for en passant, the rook's
    /// origin square for castling; `NO_SQ` otherwise.
    pub aux: u16,
}

impl Move {
    pub fn normal(from: u16, to: u16) -> Self {
        Move { from, to, kind: MoveKind::Normal, promo: None, drop_type: 0, aux: NO_SQ }
    }
    pub fn promo(from: u16, to: u16, t: TypeId) -> Self {
        Move { from, to, kind: MoveKind::Normal, promo: Some(t), drop_type: 0, aux: NO_SQ }
    }
    pub fn special(from: u16, to: u16, kind: MoveKind, aux: u16) -> Self {
        Move { from, to, kind, promo: None, drop_type: 0, aux }
    }
    pub fn drop(t: TypeId, to: u16) -> Self {
        Move { from: NO_SQ, to, kind: MoveKind::Drop, promo: None, drop_type: t, aux: NO_SQ }
    }
}

pub fn sq_name(g: &GameDef, sq: u16) -> String {
    let (x, y) = g.board.xy(sq);
    format!("{}{}", (b'a' + x as u8) as char, y + 1)
}

/// UCI-ish notation for perft divide and the CLI: "e2e4", "e7e8q", "P@e5".
pub fn move_str(g: &GameDef, mv: &Move) -> String {
    match mv.kind {
        MoveKind::Drop => format!(
            "{}@{}",
            g.types[mv.drop_type as usize].glyph,
            sq_name(g, mv.to)
        ),
        _ => {
            let mut s = format!("{}{}", sq_name(g, mv.from), sq_name(g, mv.to));
            if let Some(t) = mv.promo {
                s.push(g.types[t as usize].glyph.to_ascii_lowercase());
            }
            s
        }
    }
}
