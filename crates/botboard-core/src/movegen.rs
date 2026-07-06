//! Move generation (§7.3) and legality.
//!
//! Pseudo-legal moves come from the compiled kernels plus the special-move
//! generators (§3.3) and drops; legality is make → own-royal-safe → unmake,
//! which also catches en-passant pins and xiangqi's facing generals. Drop
//! legality runs the three tiers of §3.3: empty target, derivable
//! can-act-from, and the named predicates (*nifu*, *uchifuzume*).

use crate::bits::{SpecialBit, TargetPred};
use crate::game::{GameDef, PromoTrigger, Side, StalematePolicy, TypeId};
use crate::moves::{Move, MoveKind, NO_SQ};
use crate::position::{Loc, Position};

/// Expand a landing into promotion variants per the type's promo rule.
fn push_with_promo(
    g: &GameDef,
    side: Side,
    t: TypeId,
    from: u16,
    to: u16,
    kind: MoveKind,
    aux: u16,
    out: &mut Vec<Move>,
) {
    let ty = &g.types[t as usize];
    if let Some(pr) = &ty.promo {
        let zone = &g.zones[pr.zone];
        let triggered = match pr.trigger {
            PromoTrigger::Dest => zone.contains(side, to),
            PromoTrigger::FromOrDest => zone.contains(side, from) || zone.contains(side, to),
        };
        if triggered {
            // Forced-if-immobile (§3.2): optional promotion becomes forced
            // when the unpromoted piece could never act from the destination.
            let forced = !pr.optional || !g.compiled(t, side).can_act_from[to as usize];
            for &c in &pr.choices {
                out.push(Move { from, to, kind, promo: Some(c), drop_type: 0, aux });
            }
            if forced {
                return;
            }
        }
    }
    out.push(Move { from, to, kind, promo: None, drop_type: 0, aux });
}

pub fn pseudo_moves(g: &GameDef, pos: &Position) -> Vec<Move> {
    let stm = pos.stm;
    let mut out = Vec::with_capacity(64);
    let zone_ok =
        |z: Option<usize>, at: u16| z.map_or(true, |zi| g.zones[zi].contains(stm, at));
    let target_ok = |t: TargetPred, sq: u16| match t {
        TargetPred::Any => true,
        TargetPred::EnemyRoyal => {
            pos.piece_at(sq).map_or(false, |v| g.types[v.t as usize].royal)
        }
    };

    for p in &pos.pieces {
        let Loc::Board(from) = p.loc else { continue };
        if p.side != stm {
            continue;
        }
        let ck = g.compiled(p.t, stm);

        for k in &ck.leaps {
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            let Some(to) = g.board.offset(from, k.d.dx, k.d.dy) else { continue };
            if !zone_ok(k.to_zone, to) {
                continue;
            }
            let blocked = k.blockers.iter().any(|b| {
                g.board
                    .offset(from, b.dx, b.dy)
                    .map_or(true, |bsq| pos.board[bsq as usize] >= 0)
            });
            if blocked {
                continue;
            }
            match pos.piece_at(to) {
                None => {
                    if k.mode.can_move() {
                        push_with_promo(g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out);
                    }
                }
                Some(v) => {
                    if v.side != stm && k.mode.can_capture() && target_ok(k.target, to) {
                        push_with_promo(g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out);
                    }
                }
            }
        }

        for k in &ck.rides {
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            let mut cur = from;
            while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
                if !zone_ok(k.to_zone, to) {
                    break;
                }
                match pos.piece_at(to) {
                    None => {
                        if k.mode.can_move() {
                            push_with_promo(
                                g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out,
                            );
                        }
                        cur = to;
                    }
                    Some(v) => {
                        if v.side != stm && k.mode.can_capture() && target_ok(k.target, to) {
                            push_with_promo(
                                g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out,
                            );
                        }
                        break;
                    }
                }
            }
        }

        for k in &ck.hops {
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            let mut cur = from;
            let mut screened = false;
            while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
                match pos.piece_at(to) {
                    None => cur = to,
                    Some(v) => {
                        if !screened {
                            screened = true;
                            cur = to;
                        } else {
                            if v.side != stm && zone_ok(k.to_zone, to) && target_ok(k.target, to)
                            {
                                push_with_promo(
                                    g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out,
                                );
                            }
                            break;
                        }
                    }
                }
            }
        }

        for s in &g.types[p.t as usize].specials {
            match s {
                SpecialBit::DoubleStep { start_zone } => {
                    if !g.zones[*start_zone].contains(stm, from) {
                        continue;
                    }
                    let f = Position::forward(stm);
                    let Some(mid) = g.board.offset(from, 0, f) else { continue };
                    let Some(to) = g.board.offset(from, 0, 2 * f) else { continue };
                    if pos.board[mid as usize] < 0 && pos.board[to as usize] < 0 {
                        out.push(Move::special(from, to, MoveKind::DoubleStep, NO_SQ));
                    }
                }
                SpecialBit::EnPassant => {
                    if pos.ep == NO_SQ {
                        continue;
                    }
                    let f = Position::forward(stm);
                    for k in &ck.leaps {
                        if !k.mode.can_capture() {
                            continue;
                        }
                        if g.board.offset(from, k.d.dx, k.d.dy) == Some(pos.ep) {
                            let victim = g.board.offset(pos.ep, 0, -f).unwrap();
                            out.push(Move::special(from, pos.ep, MoveKind::EnPassant, victim));
                        }
                    }
                }
                SpecialBit::Castling => {
                    if p.moved {
                        continue;
                    }
                    gen_castles(g, pos, from, &mut out);
                }
            }
        }
    }

    // Drops (§3.3). Tier 1: empty target. Tier 2: the dropped piece must be
    // able to act from the target. Tier 3 (*nifu*) here; *uchifuzume* needs
    // make/unmake, so it lives in the legality filter.
    for t in 0..g.types.len() as TypeId {
        if pos.hands[stm as usize][t as usize] == 0 || !g.types[t as usize].droppable {
            continue;
        }
        let ck = g.compiled(t, stm);
        for sq in 0..g.board.ncells() as u16 {
            if pos.board[sq as usize] >= 0 || !ck.can_act_from[sq as usize] {
                continue;
            }
            if g.types[t as usize].drop_no_dup_file {
                let (x, _) = g.board.xy(sq);
                let dup = (0..g.board.h).any(|y| {
                    pos.piece_at(g.board.sq(x as u8, y))
                        .map_or(false, |v| v.side == stm && v.t == t)
                });
                if dup {
                    continue;
                }
            }
            out.push(Move::drop(t, sq));
        }
    }

    out
}

/// Standard castling: unmoved king + unmoved partner rook on the same rank,
/// clear between, king's from/mid/dest squares unattacked.
fn gen_castles(g: &GameDef, pos: &Position, ksq: u16, out: &mut Vec<Move>) {
    let stm = pos.stm;
    let enemy = (stm + 1) % g.sides;
    let ky = g.board.xy(ksq).1;
    for r in &pos.pieces {
        let Loc::Board(rsq) = r.loc else { continue };
        if r.side != stm || r.moved || !g.types[r.t as usize].castle_partner {
            continue;
        }
        let ry = g.board.xy(rsq).1;
        if ry != ky {
            continue;
        }
        let dir: i8 = if rsq > ksq { 1 } else { -1 };
        let mut clear = true;
        let mut cur = ksq;
        loop {
            cur = g.board.offset(cur, dir, 0).unwrap();
            if cur == rsq {
                break;
            }
            if pos.board[cur as usize] >= 0 {
                clear = false;
                break;
            }
        }
        if !clear {
            continue;
        }
        let mid = (ksq as i32 + dir as i32) as u16;
        let dest = (ksq as i32 + 2 * dir as i32) as u16;
        // Dest must lie strictly before the rook (true on 8x8; guards odd boards).
        if dest == rsq && rsq != mid {
            continue;
        }
        if pos.is_attacked(g, ksq, enemy)
            || pos.is_attacked(g, mid, enemy)
            || pos.is_attacked(g, dest, enemy)
        {
            continue;
        }
        out.push(Move::special(ksq, dest, MoveKind::Castle, rsq));
    }
}

/// Full legality: make, verify own royal safety, apply *uchifuzume*, unmake.
pub fn is_legal(g: &GameDef, pos: &mut Position, mv: &Move) -> bool {
    let mover = pos.stm;
    let u = pos.make(g, mv);
    let mut ok = !pos.royal_attacked(g, mover);
    if ok && mv.kind == MoveKind::Drop && g.types[mv.drop_type as usize].drop_no_mate {
        let enemy = pos.stm;
        if pos.royal_attacked(g, enemy) && !has_any_legal(g, pos) {
            ok = false;
        }
    }
    pos.unmake(g, &u);
    ok
}

pub fn legal_moves(g: &GameDef, pos: &mut Position) -> Vec<Move> {
    pseudo_moves(g, pos)
        .into_iter()
        .filter(|mv| is_legal(g, pos, mv))
        .collect()
}

fn has_any_legal(g: &GameDef, pos: &mut Position) -> bool {
    pseudo_moves(g, pos).iter().any(|mv| is_legal(g, pos, mv))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ongoing,
    Win(Side),
    Draw,
}

/// Terminal status per the policy layer (§5). Two-player only in Phase 0;
/// N-player victory rules are the spec's open Tier-1 gap (§13).
pub fn status(g: &GameDef, pos: &mut Position) -> Status {
    if has_any_legal(g, pos) {
        return Status::Ongoing;
    }
    let stm = pos.stm;
    let enemy = (stm + 1) % g.sides;
    if pos.royal_attacked(g, stm) {
        return Status::Win(enemy);
    }
    match g.policy.stalemate {
        StalematePolicy::Draw => Status::Draw,
        StalematePolicy::Loss => Status::Win(enemy),
    }
}
