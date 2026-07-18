//! Move generation (§7.3) and legality.
//!
//! Pseudo-legal moves come from the compiled kernels plus the compiled
//! move-script references (Bits 2.0 Stage 2): specials and drops walk
//! `move_defs` stdlib rows — origin gates interpreted as data, one
//! hand-optimized walker per DERIVED generation shape, each parameterized
//! by its row. Legality is make → own-royal-safe → unmake, which also
//! catches en-passant pins and xiangqi's facing generals; drop legality
//! runs the three tiers of §3.3 (empty target, derivable can-act-from,
//! and the drop row's named predicate gates *nifu* / *uchifuzume*).

use crate::bits::{AbilityBit, HopMode, TargetPred};
use crate::game::{Compiled, GameDef, PromoTrigger, Side, StalematePolicy, TypeId};
use crate::move_defs::{Binding, Gate, GenShape, MoveScript, PartnerFlag};
use crate::moves::{Effect, Move, MoveKind, NO_SQ};
use crate::position::{hp_gate_ok, Loc, Piece, Position};

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
        let flies = g.types[p.t as usize].flight;
        let drills = g.types[p.t as usize].drill;
        let hp = pos.hp[pos.board[from as usize] as usize];

        // Flyers ignore pit obstruction and drillers ignore wall/block
        // obstruction, which the precompiled bitboard masks cannot
        // express — those pieces walk the mailbox path whenever the
        // relevant terrain is on the board.
        let bb_active = use_bb
            && ck.bb.is_some()
            && !(flies && pos.has_pit())
            && !(drills && pos.has_wall());
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
            if !crate::position::hp_gate_ok(k.min_hp, k.max_hp, hp) {
                continue;
            }
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            let Some(to) = g.board.offset(from, k.d.dx, k.d.dy) else { continue };
            if !zone_ok(k.to_zone, to) || !pos.terrain_open_for(to, flies, drills) {
                continue;
            }
            let blocked = k.blockers.iter().any(|b| {
                g.board
                    .offset(from, b.dx, b.dy)
                    .map_or(true, |bsq| pos.cell_obstructed_for(bsq, flies, drills))
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
            if !crate::position::hp_gate_ok(k.min_hp, k.max_hp, hp) {
                continue;
            }
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            let mut cur = from;
            let mut steps = 0u8;
            while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
                steps += 1;
                if k.max_steps != 0 && steps > k.max_steps {
                    break; // range-limited rider (R4 atom): reach exhausted
                }
                if !zone_ok(k.to_zone, to) || !pos.terrain_open_for(to, flies, drills) {
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
            if !crate::position::hp_gate_ok(k.min_hp, k.max_hp, hp) {
                continue;
            }
            if !zone_ok(k.from_zone, from) {
                continue;
            }
            match k.landing {
                // Xiangqi cannon: after one screen, capture at any range.
                HopMode::CannonAtRange => {
                    if !k.mode.can_capture() {
                        continue;
                    }
                    let mut cur = from;
                    let mut screened = false;
                    while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
                        if !pos.terrain_open_for(to, flies, drills) {
                            break; // blocking terrain is never a screen
                        }
                        match pos.piece_at(to) {
                            None => cur = to,
                            Some(v) => {
                                if !screened {
                                    screened = true;
                                    cur = to;
                                } else {
                                    if v.side != stm
                                        && zone_ok(k.to_zone, to)
                                        && target_ok(k.target, to)
                                    {
                                        push_with_promo(
                                            g, stm, p.t, from, to, MoveKind::Normal, NO_SQ,
                                            &mut out,
                                        );
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                // Grasshopper: land IMMEDIATELY beyond the screen — onto
                // empty (move) or capturing an enemy there (capture).
                HopMode::BeyondScreen => {
                    let Some((_screen, to)) = first_screen_and_beyond(g, pos, from, k, flies, drills)
                    else {
                        continue;
                    };
                    if !zone_ok(k.to_zone, to) || !pos.terrain_open_for(to, flies, drills) {
                        continue;
                    }
                    match pos.piece_at(to) {
                        None => {
                            if k.mode.can_move() {
                                push_with_promo(
                                    g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out,
                                );
                            }
                        }
                        Some(v) => {
                            if v.side != stm && k.mode.can_capture() && target_ok(k.target, to) {
                                push_with_promo(
                                    g, stm, p.t, from, to, MoveKind::Normal, NO_SQ, &mut out,
                                );
                            }
                        }
                    }
                }
                // Locust: capture the screen itself (must be an enemy),
                // landing on the empty square just beyond — the aux-victim
                // machinery en passant already exercises.
                HopMode::Locust => {
                    if !k.mode.can_capture() {
                        continue;
                    }
                    let Some((screen, to)) = first_screen_and_beyond(g, pos, from, k, flies, drills)
                    else {
                        continue;
                    };
                    let victim = pos.piece_at(screen).expect("screen is occupied");
                    if victim.side == stm || !target_ok(k.target, screen) {
                        continue;
                    }
                    if !zone_ok(k.to_zone, to)
                        || !pos.terrain_open_for(to, flies, drills)
                        || pos.piece_at(to).is_some()
                    {
                        continue;
                    }
                    out.push(Move::special(from, to, MoveKind::Locust, screen));
                }
            }
        }

        gen_abilities(g, pos, p.t, from, &mut out);
        if let Some(oc) = &ck.overclock {
            // The overclock row's HpBand gate (min 2: the compound's
            // self −1 HP must not be lethal).
            if oc.gates.pass(g, pos, from, p.moved, hp) {
                gen_overclock(g, pos, from, &mut out);
            }
        }

        gen_specials(g, pos, ck, p, from, hp, &mut out);
    }

    // Drops — the compiled Drop script row (source: Hand). Tier 1: empty
    // target. Tier 2: the dropped piece must be able to act from the
    // target. Tier 3 is the row's NAMED gates: *nifu* (`NoDupFile`) walks
    // the file here; *uchifuzume* (`NoDropMate`) needs make/unmake, so it
    // is interpreted in the legality filter.
    for t in 0..g.types.len() as TypeId {
        if pos.hands[stm as usize][t as usize] == 0 {
            continue;
        }
        let ck = g.compiled(t, stm);
        let Some(dr) = &ck.drop else { continue };
        for sq in 0..g.board.ncells() as u16 {
            if pos.cell_obstructed(sq) || !ck.can_act_from[sq as usize] {
                continue;
            }
            if dr.no_dup_file {
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

    // Appendix B ice: a rider landing on an empty sheet keeps sliding
    // along its ray — destinations are rewritten here so make/unmake and
    // hashing never know ice existed. Leapers land normally.
    let any_ice = pos.terrain.iter().any(|&t| t == crate::position::T_ICE);
    if any_ice {
        for m in out.iter_mut() {
            slide_on_ice(g, pos, m);
        }
    }
    // Conveyor belts: any quiet landing on a belt is carried along it —
    // also destination rewriting, applied to ALL landings (leapers too;
    // belts carry everyone). Documented interleave rule: belts resolve
    // AFTER the ice slide, one pass, no re-entry into ice logic — a belt
    // dumping onto ice does not restart the skid.
    let any_belt = pos
        .terrain
        .iter()
        .any(|&t| crate::position::conv_dir(t).is_some());
    if any_belt {
        for m in out.iter_mut() {
            carry_on_belt(g, pos, m);
        }
    }
    if any_ice || any_belt {
        // Two moves can now share a destination: keep the first.
        let mut seen: Vec<Move> = Vec::with_capacity(out.len());
        out.retain(|m| {
            if seen.contains(m) {
                false
            } else {
                seen.push(*m);
                true
            }
        });
    }

    // Holograms move but never threaten (SRW Appendix B): keep only their
    // quiet kernel steps — no captures, abilities, compounds, or specials.
    out.retain(|m| {
        if m.kind == MoveKind::Drop || m.from == NO_SQ {
            return true;
        }
        let Some(p) = pos.piece_at(m.from) else { return true };
        if !g.types[p.t as usize].hologram {
            return true;
        }
        m.kind == MoveKind::Normal && pos.board[m.to as usize] < 0
    });

    out
}

/// Walk a hop ray from `from`: find the first occupied square (the screen)
/// and the square immediately beyond it. `None` when the ray hits blocking
/// terrain or the board edge first, or when the screen is on the edge.
/// Callers apply their own zone/occupancy/terrain rules to the landing.
fn first_screen_and_beyond(
    g: &GameDef,
    pos: &Position,
    from: u16,
    k: &crate::game::HopK,
    flies: bool,
    drills: bool,
) -> Option<(u16, u16)> {
    let mut cur = from;
    while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
        if !pos.terrain_open_for(to, flies, drills) {
            return None; // blocking terrain is never a screen
        }
        if pos.piece_at(to).is_some() {
            let beyond = g.board.offset(to, k.d.dx, k.d.dy)?;
            return Some((to, beyond));
        }
        cur = to;
    }
    None
}

/// Special-move generation (§3.3): walk the type's compiled script
/// references; check each row's compiled origin gates
/// (`OriginGates::pass`), then run the walker for the row's derived
/// generation shape — hand-optimized per shape, but parameterized by the
/// row's data (step counts, gates, binding).
#[inline]
fn gen_specials(
    g: &GameDef,
    pos: &Position,
    ck: &Compiled,
    p: &Piece,
    from: u16,
    hp: i16,
    out: &mut Vec<Move>,
) {
    let stm = pos.stm;
    for sr in &ck.specials {
        if !sr.gates.pass(g, pos, from, p.moved, hp) {
            continue;
        }
        match sr.shape {
            // Stepped quiet advance laying the ephemeral marker: walk
            // `gen_steps` straight ahead (PathClear: every crossed
            // square, and the quiet landing, unobstructed).
            GenShape::TwoStepEphemeral => {
                let f = Position::forward(stm);
                let mut cur = from;
                let mut open = true;
                for _ in 0..sr.gen_steps {
                    match g.board.offset(cur, 0, f) {
                        Some(nx) if !pos.cell_obstructed(nx) => cur = nx,
                        _ => {
                            open = false;
                            break;
                        }
                    }
                }
                if open {
                    out.push(Move::special(from, cur, sr.kind, NO_SQ));
                }
            }
            // Capture onto the live marker (`EphemeralMatch` held at the
            // origin gates): any capture-capable leap kernel reaching the
            // marker fires; the aux victim is the passed-over pawn.
            GenShape::EphemeralCapture => {
                let f = Position::forward(stm);
                for k in &ck.leaps {
                    if !k.mode.can_capture() {
                        continue;
                    }
                    if g.board.offset(from, k.d.dx, k.d.dy) == Some(pos.ep) {
                        let victim = g.board.offset(pos.ep, 0, -f).unwrap();
                        out.push(Move::special(from, pos.ep, sr.kind, victim));
                    }
                }
            }
            GenShape::PartnerCompound => gen_partner_compound(g, pos, sr.script, from, out),
        }
    }
}

/// Two-piece compound over a partner binding (castle): select the
/// partner per the binding row (type flag, same rank, unmoved), then
/// interpret the PathClear gate (squares strictly between actor and
/// partner), the actor's `gen_steps` travel with its crossing/landing
/// occupancy rules, and the PathSafe gate (origin, crossings, landing
/// unattacked).
fn gen_partner_compound(
    g: &GameDef,
    pos: &Position,
    sc: &'static MoveScript,
    ksq: u16,
    out: &mut Vec<Move>,
) {
    let Binding::Partner { flag, same_rank, unmoved } = sc.binding else {
        return;
    };
    let path_clear = sc.has_gate(Gate::PathClear);
    let path_safe = sc.has_gate(Gate::PathSafe);
    let stm = pos.stm;
    let enemy = (stm + 1) % g.sides;
    let ky = g.board.xy(ksq).1;
    for r in &pos.pieces {
        let Loc::Board(rsq) = r.loc else { continue };
        if r.side != stm
            || (unmoved && r.moved)
            || !match flag {
                PartnerFlag::CastlePartner => g.types[r.t as usize].castle_partner,
            }
        {
            continue;
        }
        if same_rank && g.board.xy(rsq).1 != ky {
            continue;
        }
        let dir: i8 = if rsq > ksq { 1 } else { -1 };
        if path_clear {
            // Squares strictly between actor and partner unobstructed.
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
        }
        // The actor travels `gen_steps` toward the partner; the partner
        // lands on the midpoint (the row's `Relocate(Aux→Mid)` op). The
        // crossing and landing squares must be free (the partner's own
        // square counts as free — it vacates). With standard placement
        // the between-check already covers these; arbitrary Bit-worlds
        // can put an unmoved partner adjacent to the actor.
        let dest = (ksq as i32 + sc.gen_steps as i32 * dir as i32) as u16;
        if dest == rsq {
            continue;
        }
        let mut open = true;
        for i in 1..sc.gen_steps {
            let c = (ksq as i32 + i as i32 * dir as i32) as u16;
            if (pos.board[c as usize] >= 0 && c != rsq) || !pos.terrain_open(c) {
                open = false;
                break;
            }
        }
        if !open {
            continue;
        }
        if pos.board[dest as usize] >= 0 || !pos.terrain_open(dest) {
            continue;
        }
        if path_safe {
            // Origin, crossings, landing: none may be attacked.
            let mut safe = !pos.is_attacked(g, ksq, enemy);
            for i in 1..=sc.gen_steps {
                if !safe {
                    break;
                }
                let c = (ksq as i32 + i as i32 * dir as i32) as u16;
                safe = !pos.is_attacked(g, c, enemy);
            }
            if !safe {
                continue;
            }
        }
        out.push(Move::special(ksq, dest, sc.kind, rsq));
    }
}

/// Ice slide (Appendix B): rewrite a quiet rider move that lands on ice
/// to its slide-out endpoint — the last ice cell before a blocker/edge,
/// or the first open non-ice cell past the sheet.
fn slide_on_ice(g: &GameDef, pos: &Position, mv: &mut Move) {
    use crate::position::T_ICE;
    if mv.kind != MoveKind::Normal || mv.promo.is_some() {
        return;
    }
    if pos.board[mv.to as usize] >= 0 || pos.terrain[mv.to as usize] != T_ICE {
        return;
    }
    let mover = pos.piece_at(mv.from).expect("mover exists");
    let (fx, fy) = g.board.xy(mv.from);
    let (tx, ty) = g.board.xy(mv.to);
    let (dx, dy) = ((tx - fx) as i32, (ty - fy) as i32);
    let gd = gcd(dx.unsigned_abs(), dy.unsigned_abs()) as i32;
    if gd == 0 {
        return;
    }
    let (sx, sy) = ((dx / gd) as i8, (dy / gd) as i8);
    // Only sliders skid: the mover must ride in exactly this direction.
    let ck = g.compiled(mover.t, mover.side);
    let is_ride = ck
        .rides
        .iter()
        .any(|r| r.mode.can_move() && r.d.dx == sx && r.d.dy == sy);
    if !is_ride {
        return;
    }
    let flies = g.types[mover.t as usize].flight;
    let drills = g.types[mover.t as usize].drill;
    let mut cur = mv.to;
    loop {
        let Some(next) = g.board.offset(cur, sx, sy) else { break };
        if pos.board[next as usize] >= 0 || !pos.terrain_open_for(next, flies, drills) {
            break; // stop on the last ice cell before the obstruction
        }
        if pos.terrain[next as usize] == T_ICE {
            cur = next;
            continue;
        }
        cur = next; // first open floor past the sheet: stop there
        break;
    }
    mv.to = cur;
}

/// Conveyor carry (destination rewriting, like the ice pass): a quiet
/// landing on a belt is carried one square in the belt's direction when
/// that square is open (terrain-permission-aware for the mover) and
/// empty, chaining across consecutive belts — bounded at 8 carries, with
/// revisit detection so belt cycles cannot spin forever. A blocked exit
/// parks the piece on the belt. Captures and strikes are not rewritten
/// (the scuffle jams the rollers), nor are drops, specials, abilities,
/// compounds, or promotion landings — the same guards as the ice pass.
fn carry_on_belt(g: &GameDef, pos: &Position, mv: &mut Move) {
    use crate::position::conv_dir;
    if mv.kind != MoveKind::Normal || mv.promo.is_some() {
        return;
    }
    if pos.board[mv.to as usize] >= 0 {
        return;
    }
    let Some(mover) = pos.piece_at(mv.from) else { return };
    let flies = g.types[mover.t as usize].flight;
    let drills = g.types[mover.t as usize].drill;
    let mut cur = mv.to;
    let mut visited: Vec<u16> = vec![cur];
    for _ in 0..8 {
        let Some((dx, dy)) = conv_dir(pos.terrain[cur as usize]) else { break };
        let Some(next) = g.board.offset(cur, dx, dy) else { break };
        if pos.board[next as usize] >= 0 || !pos.terrain_open_for(next, flies, drills) {
            break; // blocked exit: the piece parks on the belt
        }
        if visited.contains(&next) {
            break; // belt cycle: stop where the loop closes
        }
        visited.push(next);
        cur = next;
    }
    mv.to = cur;
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Ability actions (§3.4): one Axis-B effect as the turn's single action.
fn gen_abilities(g: &GameDef, pos: &Position, t: TypeId, from: u16, out: &mut Vec<Move>) {
    let stm = pos.stm;
    let (fx, fy) = g.board.xy(from);
    // EMP aura (Appendix B): inside an enemy emitter's radius, ability
    // Bits are dead circuitry — no action Bits generate at all.
    for e in &pos.pieces {
        let Loc::Board(esq) = e.loc else { continue };
        let r = g.types[e.t as usize].emp_aura;
        if e.side == stm || r == 0 {
            continue;
        }
        let (ex, ey) = g.board.xy(esq);
        if (ex - fx).abs().max((ey - fy).abs()) as u8 <= r {
            return;
        }
    }
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
            AbilityBit::CreateWall { range }
            | AbilityBit::DigPit { range }
            | AbilityBit::MineLayer { range } => {
                let eff = match a {
                    AbilityBit::CreateWall { .. } => Effect::Wall,
                    AbilityBit::DigPit { .. } => Effect::Pit,
                    _ => Effect::Mine,
                };
                for sq in 0..g.board.ncells() as u16 {
                    // Bare floor only: no stacking terrain on mines.
                    if sq == from
                        || pos.cell_obstructed(sq)
                        || pos.terrain[sq as usize] != crate::position::T_NONE
                    {
                        continue;
                    }
                    let (x, y) = g.board.xy(sq);
                    if (x - fx).abs().max((y - fy).abs()) as u8 <= range {
                        out.push(Move::ability(from, sq, eff, NO_SQ));
                    }
                }
            }
            AbilityBit::Hack { range } => {
                // The target predicate (§7 hack): a non-royal enemy at
                // exactly 1 HP in range — flip it, deterministically.
                // The HP check is the row's `HpEq1` pred, evaluated via
                // the same hp-band predicate the kernel and script
                // `HpBand` gates use (band [1, 1]).
                for p in &pos.pieces {
                    let Loc::Board(sq) = p.loc else { continue };
                    if p.side == stm || g.types[p.t as usize].royal {
                        continue;
                    }
                    let idx = pos.board[sq as usize] as usize;
                    if !hp_gate_ok(1, 1, pos.hp[idx]) {
                        continue;
                    }
                    let (x, y) = g.board.xy(sq);
                    if (x - fx).abs().max((y - fy).abs()) as u8 <= range {
                        out.push(Move::ability(from, sq, Effect::Hack, NO_SQ));
                    }
                }
            }
            AbilityBit::Resurrect { range } => {
                // One candidate per dead friendly type (piece identity
                // within a type is interchangeable): revive onto any empty
                // open square in range, at 1 HP. Royals stay dead — their
                // loss decided something.
                for t2 in 0..g.types.len() {
                    if g.types[t2].royal {
                        continue;
                    }
                    let has_dead = pos.pieces.iter().any(|p| {
                        p.side == stm && p.t == t2 as TypeId && p.loc == Loc::Dead
                    });
                    if !has_dead {
                        continue;
                    }
                    for sq in 0..g.board.ncells() as u16 {
                        if pos.cell_obstructed(sq) {
                            continue;
                        }
                        let (x, y) = g.board.xy(sq);
                        if (x - fx).abs().max((y - fy).abs()) as u8 <= range {
                            out.push(Move::resurrect(from, sq, t2 as TypeId));
                        }
                    }
                }
            }
            AbilityBit::Laser { range, retreat, pierce } => {
                // A bounded capture-at-range rider that never vacates the
                // origin (§3.2 ranged effects). A piercing beam ignores
                // occupancy on its ray (Appendix B) — every enemy along it
                // is a target; terrain still stops the beam.
                for (dx, dy) in
                    [(0i8, 1i8), (0, -1), (1, 0), (-1, 0), (1, 1), (1, -1), (-1, 1), (-1, -1)]
                {
                    let mut cur = from;
                    for _ in 0..range {
                        let Some(nsq) = g.board.offset(cur, dx, dy) else { break };
                        if !pos.terrain_open(nsq) {
                            // A destructible block is blocking terrain the
                            // beam may TARGET: each hit drops it one tier
                            // (make handles the empty-victim square as
                            // terrain damage). The beam still stops here,
                            // pierce or not.
                            if crate::position::is_block(pos.terrain[nsq as usize])
                                && pos.board[nsq as usize] < 0
                            {
                                let aux = if retreat {
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
                            if !pierce {
                                break;
                            }
                        }
                        cur = nsq;
                    }
                }
            }
            AbilityBit::Swap { range } => {
                // Exchange with a friendly in chebyshev range. Each half
                // of the exchange is a landing: the swapped-onto squares
                // must be terrain-open for the piece arriving there.
                let cflies = g.types[t as usize].flight;
                let cdrills = g.types[t as usize].drill;
                for p in &pos.pieces {
                    let Loc::Board(sq) = p.loc else { continue };
                    if p.side != stm || sq == from {
                        continue;
                    }
                    let (x, y) = g.board.xy(sq);
                    if (x - fx).abs().max((y - fy).abs()) as u8 > range {
                        continue;
                    }
                    let pflies = g.types[p.t as usize].flight;
                    let pdrills = g.types[p.t as usize].drill;
                    if !pos.terrain_open_for(sq, cflies, cdrills)
                        || !pos.terrain_open_for(from, pflies, pdrills)
                    {
                        continue;
                    }
                    out.push(Move::ability(from, sq, Effect::Swap, NO_SQ));
                }
            }
            AbilityBit::Push { range } => {
                // Shove an in-range enemy one square directly away from
                // the caster (chebyshev direction sign); generated only
                // when that square is open for the PUSHED piece and empty.
                for p in &pos.pieces {
                    let Loc::Board(sq) = p.loc else { continue };
                    if p.side == stm {
                        continue;
                    }
                    let (x, y) = g.board.xy(sq);
                    let (cx, cy) = (x - fx, y - fy);
                    if cx.abs().max(cy.abs()) as u8 > range {
                        continue;
                    }
                    let (dx, dy) = (cx.signum() as i8, cy.signum() as i8);
                    let Some(dest) = g.board.offset(sq, dx, dy) else { continue };
                    let vflies = g.types[p.t as usize].flight;
                    let vdrills = g.types[p.t as usize].drill;
                    if pos.board[dest as usize] >= 0
                        || !pos.terrain_open_for(dest, vflies, vdrills)
                    {
                        continue;
                    }
                    out.push(Move::ability(from, sq, Effect::Push, dest));
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
    let flies = g.types[p.t as usize].flight;
    let drills = g.types[p.t as usize].drill;
    let hp = pos.hp[pos.board[from as usize] as usize];
    if p.side != pos.stm {
        return Vec::new();
    }
    let stm = pos.stm;
    let ck = g.compiled(p.t, stm);
    let zone_ok = |z: Option<usize>, at: u16| z.map_or(true, |zi| g.zones[zi].contains(stm, at));
    let mut out = Vec::new();
    for k in &ck.leaps {
        if !crate::position::hp_gate_ok(k.min_hp, k.max_hp, hp) {
            continue;
        }
        if !zone_ok(k.from_zone, from) {
            continue;
        }
        let Some(to) = g.board.offset(from, k.d.dx, k.d.dy) else { continue };
        if !zone_ok(k.to_zone, to) || !pos.terrain_open_for(to, flies, drills) {
            continue;
        }
        if k.blockers.iter().any(|b| {
            g.board
                .offset(from, b.dx, b.dy)
                .map_or(true, |bsq| pos.cell_obstructed_for(bsq, flies, drills))
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
        if !crate::position::hp_gate_ok(k.min_hp, k.max_hp, hp) {
            continue;
        }
        if !zone_ok(k.from_zone, from) {
            continue;
        }
        let mut cur = from;
        let mut steps = 0u8;
        while let Some(to) = g.board.offset(cur, k.d.dx, k.d.dy) {
            steps += 1;
            if k.max_steps != 0 && steps > k.max_steps {
                break;
            }
            if !zone_ok(k.to_zone, to) || !pos.terrain_open_for(to, flies, drills) {
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

/// Full legality: make, verify own royal safety, interpret the named
/// `NoDropMate` gate (*uchifuzume*) from the generating script row, unmake.
pub fn is_legal(g: &GameDef, pos: &mut Position, mv: &Move) -> bool {
    let mover = pos.stm;
    let u = pos.make(g, mv);
    let mut ok = !pos.royal_attacked(g, mover);
    if ok
        && g.types[mv.drop_type as usize].drop_no_mate
        && crate::move_defs::script(mv.kind).has_gate(Gate::NoDropMate)
    {
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
