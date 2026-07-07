//! Move generation (§7.3) and legality.
//!
//! Pseudo-legal moves come from the compiled kernels plus the special-move
//! generators (§3.3) and drops; legality is make → own-royal-safe → unmake,
//! which also catches en-passant pins and xiangqi's facing generals. Drop
//! legality runs the three tiers of §3.3: empty target, derivable
//! can-act-from, and the named predicates (*nifu*, *uchifuzume*).

use crate::bits::{AbilityBit, SpecialBit, TargetPred};
use crate::game::{GameDef, PromoTrigger, Side, StalematePolicy, TypeId};
use crate::moves::{Effect, Move, MoveKind, NO_SQ};
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
                out.push(Move {
                    from,
                    to,
                    kind,
                    promo: Some(c),
                    drop_type: 0,
                    aux,
                    effect: Effect::None,
                });
            }
            if forced {
                return;
            }
        }
    }
    out.push(Move { from, to, kind, promo: None, drop_type: 0, aux, effect: Effect::None });
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

    // Occupancy masks for the wide-bitboard path (§7.1): incrementally
    // maintained mirrors — the mailbox stays the source of truth.
    let use_bb = g.use_bitboards && g.board.ncells() <= 128;
    let (occ_all, occ_own, terrain_mask) =
        (pos.occ_all, if use_bb { pos.occ_side[stm as usize] } else { 0 }, pos.terrain_mask);
    let obstructed = occ_all | terrain_mask;

    for p in &pos.pieces {
        let Loc::Board(from) = p.loc else { continue };
        if p.side != stm {
            continue;
        }
        let ck = g.compiled(p.t, stm);

        let bb_active = use_bb && ck.bb.is_some();
        if bb_active {
            let bb = ck.bb.as_ref().unwrap();
            for e in &bb.leaps[from as usize] {
                if e.blockers & obstructed != 0 {
                    continue;
                }
                let tobit = 1u128 << e.to;
                if terrain_mask & tobit != 0 {
                    continue;
                }
                if occ_all & tobit == 0 {
                    if e.mode.can_move() {
                        push_with_promo(g, stm, p.t, from, e.to, MoveKind::Normal, NO_SQ, &mut out);
                    }
                } else if occ_own & tobit == 0 && e.mode.can_capture() {
                    push_with_promo(g, stm, p.t, from, e.to, MoveKind::Normal, NO_SQ, &mut out);
                }
            }
            for r in &bb.rides {
                let ray = r.rays[from as usize];
                let blk = ray & obstructed;
                let (reach, stop) = if blk == 0 {
                    (ray, None)
                } else {
                    let b = if r.positive {
                        blk.trailing_zeros() as u16
                    } else {
                        127 - blk.leading_zeros() as u16
                    };
                    (ray & !r.rays[b as usize], Some(b))
                };
                if r.mode.can_move() {
                    let mut quiet = reach & !obstructed;
                    while quiet != 0 {
                        let to = quiet.trailing_zeros() as u16;
                        quiet &= quiet - 1;
                        push_with_promo(g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out);
                    }
                }
                if r.mode.can_capture() {
                    if let Some(b) = stop {
                        let bbit = 1u128 << b;
                        if terrain_mask & bbit == 0 && occ_own & bbit == 0 && occ_all & bbit != 0
                        {
                            push_with_promo(
                                g, stm, p.t, from, b, MoveKind::Normal, NO_SQ, &mut out,
                            );
                        }
                    }
                }
            }
        }

        for (ki, k) in ck.leaps.iter().enumerate() {
            if bb_active && ck.leap_plain[ki] {
                continue;
            }
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            let Some(to) = g.board.offset(from, k.d.dx, k.d.dy) else { continue };
            if !zone_ok(k.to_zone, to) || !pos.terrain_open(to) {
                continue;
            }
            let blocked = k.blockers.iter().any(|b| {
                g.board
                    .offset(from, b.dx, b.dy)
                    .map_or(true, |bsq| pos.cell_obstructed(bsq))
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

        for (ki, k) in ck.rides.iter().enumerate() {
            if bb_active && ck.ride_plain[ki] {
                continue;
            }
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            let mut cur = from;
            while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
                if !zone_ok(k.to_zone, to) || !pos.terrain_open(to) {
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
                if !pos.terrain_open(to) {
                    break; // terrain blocks; it is never a screen
                }
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

        gen_abilities(g, pos, p.t, from, &mut out);
        if g.types[p.t as usize].overclock && pos.hp[pos.board[from as usize] as usize] > 1 {
            gen_overclock(g, pos, from, &mut out);
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
                    if !pos.cell_obstructed(mid) && !pos.cell_obstructed(to) {
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
            if pos.cell_obstructed(sq) || !ck.can_act_from[sq as usize] {
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
            if pos.cell_obstructed(cur) {
                clear = false;
                break;
            }
        }
        if !clear {
            continue;
        }
        let mid = (ksq as i32 + dir as i32) as u16;
        let dest = (ksq as i32 + 2 * dir as i32) as u16;
        // The king's crossing and landing squares must be free (the rook's
        // own square counts as free for `mid` — it vacates). With standard
        // rook placement the between-check already covers these; arbitrary
        // Bit-worlds can put an unmoved partner adjacent to the king.
        if dest == rsq {
            continue;
        }
        if (pos.board[mid as usize] >= 0 && mid != rsq) || !pos.terrain_open(mid) {
            continue;
        }
        if pos.board[dest as usize] >= 0 || !pos.terrain_open(dest) {
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

/// Ability actions (§3.4): one Axis-B effect as the turn's single action.
fn gen_abilities(g: &GameDef, pos: &Position, t: TypeId, from: u16, out: &mut Vec<Move>) {
    let stm = pos.stm;
    let (fx, fy) = g.board.xy(from);
    for a in &g.types[t as usize].abilities {
        match *a {
            AbilityBit::Heal { amount, range } => {
                for p in &pos.pieces {
                    let Loc::Board(sq) = p.loc else { continue };
                    if p.side != stm {
                        continue;
                    }
                    let idx = pos.board[sq as usize] as usize;
                    if pos.hp[idx] >= g.types[p.t as usize].max_hp {
                        continue;
                    }
                    let (x, y) = g.board.xy(sq);
                    if (x - fx).abs().max((y - fy).abs()) as u8 <= range {
                        out.push(Move::ability(from, sq, Effect::Heal(amount), NO_SQ));
                    }
                }
            }
            AbilityBit::CreateWall { range } | AbilityBit::DigPit { range } => {
                let eff = if matches!(a, AbilityBit::CreateWall { .. }) {
                    Effect::Wall
                } else {
                    Effect::Pit
                };
                for sq in 0..g.board.ncells() as u16 {
                    if sq == from || pos.cell_obstructed(sq) {
                        continue;
                    }
                    let (x, y) = g.board.xy(sq);
                    if (x - fx).abs().max((y - fy).abs()) as u8 <= range {
                        out.push(Move::ability(from, sq, eff, NO_SQ));
                    }
                }
            }
            AbilityBit::Laser { range, retreat } => {
                // A bounded capture-at-range rider that never vacates the
                // origin (§3.2 ranged effects).
                for (dx, dy) in
                    [(0i8, 1i8), (0, -1), (1, 0), (-1, 0), (1, 1), (1, -1), (-1, 1), (-1, -1)]
                {
                    let mut cur = from;
                    for _ in 0..range {
                        let Some(nsq) = g.board.offset(cur, dx, dy) else { break };
                        if !pos.terrain_open(nsq) {
                            break;
                        }
                        if let Some(v) = pos.piece_at(nsq) {
                            if v.side != stm {
                                let aux = if retreat {
                                    // Coupled nerf: forced 1-step retreat away
                                    // from the target; illegal if blocked —
                                    // "the nerf has teeth" (§3.4).
                                    match g.board.offset(from, -dx, -dy) {
                                        Some(r)
                                            if pos.board[r as usize] < 0
                                                && pos.terrain_open(r) =>
                                        {
                                            r
                                        }
                                        _ => break,
                                    }
                                } else {
                                    NO_SQ
                                };
                                out.push(Move::ability(from, nsq, Effect::Laser, aux));
                            }
                            break;
                        }
                        cur = nsq;
                    }
                }
            }
        }
    }
}

/// Overclock compound moves (§3.4): enumerate legal ⟨move, move⟩ pairs by
/// actually applying step 1 — branching inflation is paid exactly where the
/// Bit occurs (piece-local), never globally. Step 1 is a quiet move; step 2
/// may capture or strike. Skipped at 1 HP (the self-damage would be lethal).
fn gen_overclock(g: &GameDef, pos: &Position, from: u16, out: &mut Vec<Move>) {
    let mut tmp = pos.clone();
    let firsts: Vec<Move> = {
        let all = pseudo_kernel_moves_of(g, &tmp, from);
        all.into_iter()
            .filter(|m| m.kind == MoveKind::Normal && tmp.board[m.to as usize] < 0)
            .collect()
    };
    for f in firsts {
        let u = tmp.make(g, &f);
        tmp.set_stm(g, pos.stm); // same piece acts again within the compound
        for s in pseudo_kernel_moves_of(g, &tmp, f.to) {
            if s.kind == MoveKind::Normal {
                out.push(Move::compound(from, f.to, s.to));
            }
        }
        tmp.set_stm(g, (pos.stm + 1) % g.sides);
        tmp.unmake(g, &u);
    }
}

/// Kernel-only pseudo moves of the piece standing on `from` (no specials,
/// abilities, drops, or nested compounds).
fn pseudo_kernel_moves_of(g: &GameDef, pos: &Position, from: u16) -> Vec<Move> {
    let Some(p) = pos.piece_at(from) else { return Vec::new() };
    if p.side != pos.stm {
        return Vec::new();
    }
    let stm = pos.stm;
    let ck = g.compiled(p.t, stm);
    let zone_ok = |z: Option<usize>, at: u16| z.map_or(true, |zi| g.zones[zi].contains(stm, at));
    let mut out = Vec::new();
    for k in &ck.leaps {
        if !zone_ok(k.from_zone, from) {
            continue;
        }
        let Some(to) = g.board.offset(from, k.d.dx, k.d.dy) else { continue };
        if !zone_ok(k.to_zone, to) || !pos.terrain_open(to) {
            continue;
        }
        if k.blockers.iter().any(|b| {
            g.board.offset(from, b.dx, b.dy).map_or(true, |bsq| pos.cell_obstructed(bsq))
        }) {
            continue;
        }
        match pos.piece_at(to) {
            None if k.mode.can_move() => out.push(Move::normal(from, to)),
            Some(v) if v.side != stm && k.mode.can_capture() => {
                out.push(Move::normal(from, to))
            }
            _ => {}
        }
    }
    for k in &ck.rides {
        if !zone_ok(k.from_zone, from) {
            continue;
        }
        let mut cur = from;
        while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
            if !zone_ok(k.to_zone, to) || !pos.terrain_open(to) {
                break;
            }
            match pos.piece_at(to) {
                None => {
                    if k.mode.can_move() {
                        out.push(Move::normal(from, to));
                    }
                    cur = to;
                }
                Some(v) => {
                    if v.side != stm && k.mode.can_capture() {
                        out.push(Move::normal(from, to));
                    }
                    break;
                }
            }
        }
    }
    out
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
