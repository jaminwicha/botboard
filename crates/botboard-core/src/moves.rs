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
    /// An Axis-B effect invoked as the turn's single action (§3.4).
    Ability,
    /// A multi-step effect script generated as ONE move (§3.4): atomic,
    /// piece-local. Overclock: from→to then to→aux, self −1 HP.
    Compound,
    /// Locust hop (§3.1 landing mode): capture the screen at `aux`, land
    /// on the empty square `to` immediately beyond — en-passant-style
    /// aux-victim machinery, checkers atom.
    Locust,
}

/// Which Axis-B effect an Ability/Compound move applies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    Heal(i16),
    Wall,
    Pit,
    /// Capture at range without vacating the origin; `aux` holds the forced
    /// retreat square (NO_SQ when the laser has no coupled retreat). When
    /// `to` holds no piece but a destructible block (T_BLOCK*), the shot is
    /// terrain damage: the block drops one tier (T_BLOCK1 clears).
    Laser,
    /// Act-twice: second-step destination in `aux`, self −1 HP.
    Twice,
    /// Revive a dead friendly piece at `to`; `drop_type` names which type
    /// re-enters (piece identity within a type is interchangeable).
    Resurrect,
    /// Flip the enemy piece at `to` to the caster's side.
    Hack,
    /// Lay an owner-tagged mine on the empty square at `to`.
    Mine,
    /// Exchange the caster (`from`) with the friendly piece at `to` —
    /// an atomic two-piece relocation.
    Swap,
    /// Shove the enemy at `to` one square directly away from the caster;
    /// `aux` holds the shoved piece's destination.
    Push,
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
    /// Auxiliary square: en-passant victim, castling rook origin, compound
    /// second step, laser retreat, or push destination; `NO_SQ` otherwise.
    pub aux: u16,
    pub effect: Effect,
}

impl Move {
    pub fn normal(from: u16, to: u16) -> Self {
        Move {
            from,
            to,
            kind: MoveKind::Normal,
            promo: None,
            drop_type: 0,
            aux: NO_SQ,
            effect: Effect::None,
        }
    }
    pub fn promo(from: u16, to: u16, t: TypeId) -> Self {
        Move { promo: Some(t), ..Self::normal(from, to) }
    }
    pub fn special(from: u16, to: u16, kind: MoveKind, aux: u16) -> Self {
        Move { kind, aux, ..Self::normal(from, to) }
    }
    pub fn drop(t: TypeId, to: u16) -> Self {
        Move { from: NO_SQ, kind: MoveKind::Drop, drop_type: t, ..Self::normal(NO_SQ, to) }
    }
    pub fn ability(from: u16, to: u16, effect: Effect, aux: u16) -> Self {
        Move { kind: MoveKind::Ability, effect, aux, ..Self::normal(from, to) }
    }
    pub fn resurrect(from: u16, to: u16, t: TypeId) -> Self {
        Move {
            kind: MoveKind::Ability,
            effect: Effect::Resurrect,
            drop_type: t,
            ..Self::normal(from, to)
        }
    }
    pub fn compound(from: u16, to: u16, second: u16) -> Self {
        Move { kind: MoveKind::Compound, effect: Effect::Twice, aux: second, ..Self::normal(from, to) }
    }
}

pub fn sq_name(g: &GameDef, sq: u16) -> String {
    let (x, y) = g.board.xy(sq);
    format!("{}{}", (b'a' + x as u8) as char, y + 1)
}

/// UCI-ish notation for perft divide and the CLI: "e2e4", "e7e8q", "P@e5",
/// abilities "e2!heal:e3", compounds "e2e4+e4e5".
pub fn move_str(g: &GameDef, mv: &Move) -> String {
    match mv.kind {
        MoveKind::Drop => format!(
            "{}@{}",
            g.types[mv.drop_type as usize].glyph,
            sq_name(g, mv.to)
        ),
        MoveKind::Ability => {
            let name = match mv.effect {
                Effect::Heal(_) => "heal".to_string(),
                Effect::Wall => "wall".to_string(),
                Effect::Pit => "pit".to_string(),
                Effect::Laser => "laser".to_string(),
                // The revived type's glyph disambiguates ("e2!rezT:d3").
                Effect::Resurrect => {
                    format!("rez{}", g.types[mv.drop_type as usize].glyph)
                }
                Effect::Hack => "hack".to_string(),
                Effect::Mine => "mine".to_string(),
                Effect::Swap => "swap".to_string(),
                Effect::Push => "push".to_string(),
                _ => "fx".to_string(),
            };
            let mut s = format!("{}!{}:{}", sq_name(g, mv.from), name, sq_name(g, mv.to));
            if mv.aux != NO_SQ {
                s.push_str(&format!(">{}", sq_name(g, mv.aux)));
            }
            s
        }
        // Locust: destination plus the captured screen ("e2e5xe4") — kept
        // distinct from plain "e2e5" so a piece owning both a leaper and a
        // locust hopper never aliases two different moves to one string.
        MoveKind::Locust => format!(
            "{}{}x{}",
            sq_name(g, mv.from),
            sq_name(g, mv.to),
            sq_name(g, mv.aux)
        ),
        MoveKind::Compound => format!(
            "{}{}+{}{}",
            sq_name(g, mv.from),
            sq_name(g, mv.to),
            sq_name(g, mv.to),
            sq_name(g, mv.aux)
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
