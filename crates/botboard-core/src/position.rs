//! Position state, make/unmake, and attack detection.
//!
//! Mailbox array + piece list (§7.1, Phase-0 board class). Pieces never
//! leave the list: capture sends them to `Dead` or to a hand (§3.2 capture
//! fate); shogi drops re-activate hand pieces, so unmake is a pure reversal.
//!
//! Per-instance state (HP) lives in a parallel SoA array (§7.3), and
//! mutable terrain in a per-cell array — both first-class in the Zobrist
//! key (§7.5). Hit-count armor (§3.2): a capture against a piece with
//! HP > 1 is a *strike* — the victim loses 1 HP and the attacker does not
//! move. Abilities and compound moves apply atomically (§3.4).
//!
//! `hash` is the full ground-truth Zobrist key, maintained incrementally:
//! every mutation cluster XORs the affected entity out and back in; unmake
//! restores the recorded prior key in O(1). Debug builds assert equality
//! with the full recompute after every make.

use crate::game::{GameDef, Side, TypeId};
use crate::moves::{Effect, Move, NO_SQ};
use crate::ops::{OpCtx, OpLog};

pub const T_NONE: u8 = 0;
pub const T_WALL: u8 = 1;
pub const T_PIT: u8 = 2;
/// Owner-tagged mines (SRW Appendix B): T_MINE0 + side. Passable and
/// non-blocking; an enemy landing takes 1 HP (lethal at 1) and spends it.
pub const T_MINE0: u8 = 3;
/// Ice (Appendix B): a rider entering keeps sliding until it hits a
/// blocker or leaves the sheet — realized as destination rewriting in
/// movegen, so make/unmake never see it.
pub const T_ICE: u8 = 7;
/// Tall grass (Appendix B): positional concealment — masking lives at
/// the battle layer; the engine only stores the tile.
pub const T_GRASS: u8 = 8;
/// Acid (Appendix B): passable; any piece landing here takes 1 HP
/// (lethal at 1). The pool persists.
pub const T_ACID: u8 = 9;
/// Destructible blocks (SRW Appendix B walls a driller chews): blocking
/// like walls for everyone except drillers, who pass/enter them as if
/// absent. Three damage tiers; a laser hit lowers the tier by one
/// (T_BLOCK1 clears to floor).
pub const T_BLOCK1: u8 = 10;
pub const T_BLOCK2: u8 = 11;
pub const T_BLOCK3: u8 = 12;
/// Conveyor belts: a piece LANDING on one is carried a square per belt in
/// the belt's direction (chains, bounded) — realized as destination
/// rewriting in movegen like ice, so make/unmake never see them.
/// N is +y (side 0's forward), E is +x, S is −y, W is −x.
pub const T_CONV_N: u8 = 13;
pub const T_CONV_E: u8 = 14;
pub const T_CONV_S: u8 = 15;
pub const T_CONV_W: u8 = 16;

/// Is this a destructible block tier?
#[inline]
pub fn is_block(t: u8) -> bool {
    (T_BLOCK1..=T_BLOCK3).contains(&t)
}

/// The belt direction, if this terrain is a conveyor.
#[inline]
pub fn conv_dir(t: u8) -> Option<(i8, i8)> {
    match t {
        T_CONV_N => Some((0, 1)),
        T_CONV_E => Some((1, 0)),
        T_CONV_S => Some((0, -1)),
        T_CONV_W => Some((-1, 0)),
        _ => None,
    }
}

/// Terrain that blocks entry, rays, screens, and legs (mines do not).
#[inline]
pub fn terrain_blocks(t: u8) -> bool {
    t == T_WALL || t == T_PIT || is_block(t)
}

/// `terrain_blocks` under the terrain permissions (§3.1 path
/// interaction): flight ignores pits; drill ignores walls and
/// destructible blocks (the drill chews scrap). Pits still stop a
/// driller unless it also flies.
#[inline]
pub fn terrain_stops(t: u8, flies: bool, drills: bool) -> bool {
    ((t == T_WALL || is_block(t)) && !drills) || (t == T_PIT && !flies)
}

/// The mine's owner, if this cell holds one (mines occupy 3..=6).
#[inline]
pub fn mine_owner(t: u8) -> Option<Side> {
    (T_MINE0..T_MINE0 + 4).contains(&t).then(|| (t - T_MINE0) as Side)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loc {
    Board(u16),
    Hand(Side),
    Dead,
}

#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub t: TypeId,
    /// De-promotion target: the type this piece reverts to in hand (§3.2).
    pub base: TypeId,
    pub side: Side,
    pub loc: Loc,
    pub moved: bool,
}

#[derive(Clone, Debug)]
pub struct Position {
    /// Mailbox: piece index or -1.
    pub board: Vec<i32>,
    pub pieces: Vec<Piece>,
    /// SoA per-instance state (§7.3): hit points, parallel to `pieces`.
    pub hp: Vec<i16>,
    /// Per-cell terrain (the T_* codes above). Blocking kinds (walls,
    /// pits, destructible blocks) stop entry, rays, screens, and
    /// lame-leaper legs, moderated by the §3.1 terrain permissions:
    /// flight ignores pits, drill ignores walls and blocks.
    pub terrain: Vec<u8>,
    pub stm: Side,
    /// En-passant target square, or NO_SQ.
    pub ep: u16,
    /// hands[side][type] = count of that type held.
    pub hands: Vec<Vec<u8>>,
    /// Incrementally-maintained full ground-truth Zobrist key (§7.5).
    pub hash: u64,
    /// Wide-bitboard occupancy mirrors (boards ≤ 128 cells): the mailbox
    /// stays the source of truth; these are maintained at the same
    /// mutation sites and debug-asserted against it.
    pub occ_all: u128,
    pub occ_side: Vec<u128>,
    pub terrain_mask: u128,
    /// Number of pit cells (kept in sync by set_terrain/rehash).
    pit_count: u16,
    /// Number of wall + destructible-block cells (same maintenance).
    wall_count: u16,
    wide: bool,
}

/// Per-move undo: the micro-op log (Bits 2.0 — every mutation a move
/// makes is an op record in application order; unmake replays it
/// backwards) plus the O(1) hash snapshot. Stage 2 folded the last
/// move-level state snapshot (`prior_ep`) into the log as
/// `OpUndo::Ephemeral` records (the prologue clear and the
/// `SetEphemeral` producer both push one).
pub struct Undo {
    ops: OpLog,
    prior_hash: u64,
}

impl Position {
    pub fn from_pieces(g: &GameDef, list: &[(TypeId, Side, u16, bool)], stm: Side) -> Self {
        let mut pos = Position {
            board: vec![-1; g.board.ncells()],
            pieces: Vec::with_capacity(list.len()),
            hp: Vec::with_capacity(list.len()),
            terrain: vec![T_NONE; g.board.ncells()],
            stm,
            ep: NO_SQ,
            hands: vec![vec![0; g.types.len()]; g.sides as usize],
            hash: 0,
            occ_all: 0,
            occ_side: vec![0; g.sides as usize],
            terrain_mask: 0,
            pit_count: 0,
            wall_count: 0,
            wide: g.use_bitboards && g.board.ncells() <= 128,
        };
        for &(t, side, sq, moved) in list {
            pos.board[sq as usize] = pos.pieces.len() as i32;
            pos.pieces.push(Piece { t, base: t, side, loc: Loc::Board(sq), moved });
            pos.hp.push(g.types[t as usize].max_hp);
            if pos.wide {
                pos.occ_all |= 1u128 << sq;
                pos.occ_side[side as usize] |= 1u128 << sq;
            }
        }
        pos.hash = g.zobrist.full_hash(&pos);
        pos
    }

    pub fn startpos(g: &GameDef) -> Self {
        let list: Vec<_> = g
            .start
            .iter()
            .map(|&(t, side, x, y)| (t, side, g.board.sq(x, y), false))
            .collect();
        Self::from_pieces(g, &list, 0)
    }

    /// Re-derive the incremental key and occupancy mirrors after
    /// out-of-band edits (tests, determinization, manual setup).
    pub fn rehash(&mut self, g: &GameDef) {
        self.hash = g.zobrist.full_hash(self);
        self.pit_count =
            self.terrain.iter().filter(|&&t| t == T_PIT).count() as u16;
        self.wall_count = self
            .terrain
            .iter()
            .filter(|&&t| t == T_WALL || is_block(t))
            .count() as u16;
        if self.wide {
            self.occ_all = 0;
            self.occ_side.iter_mut().for_each(|m| *m = 0);
            self.terrain_mask = 0;
            for p in self.pieces.iter() {
                if let Loc::Board(sq) = p.loc {
                    self.occ_all |= 1u128 << sq;
                }
            }
            for (i, p) in self.pieces.iter().enumerate() {
                let _ = i;
                if let Loc::Board(sq) = p.loc {
                    self.occ_side[p.side as usize] |= 1u128 << sq;
                }
            }
            for sq in 0..self.terrain.len() {
                if terrain_blocks(self.terrain[sq]) {
                    self.terrain_mask |= 1u128 << sq;
                }
            }
        }
    }

    pub fn piece_at(&self, sq: u16) -> Option<&Piece> {
        let i = self.board[sq as usize];
        if i < 0 {
            None
        } else {
            Some(&self.pieces[i as usize])
        }
    }

    /// A cell a ray/screen/leg cannot pass and a piece cannot enter.
    #[inline]
    pub fn cell_obstructed(&self, sq: u16) -> bool {
        self.board[sq as usize] >= 0 || terrain_blocks(self.terrain[sq as usize])
    }

    /// `cell_obstructed` with terrain permissions: flyers pass over pits
    /// (Appendix B hover/flight); drillers pass through walls and
    /// destructible blocks (§3.1).
    #[inline]
    pub fn cell_obstructed_for(&self, sq: u16, flies: bool, drills: bool) -> bool {
        self.board[sq as usize] >= 0
            || terrain_stops(self.terrain[sq as usize], flies, drills)
    }

    /// May a piece land here (ignoring occupancy)? Mines are open — that
    /// is their whole point.
    #[inline]
    pub fn terrain_open(&self, sq: u16) -> bool {
        !terrain_blocks(self.terrain[sq as usize])
    }

    /// `terrain_open` with terrain permissions: a flyer may hover on a
    /// pit square; a driller may stand inside a wall or block.
    #[inline]
    pub fn terrain_open_for(&self, sq: u16, flies: bool, drills: bool) -> bool {
        !terrain_stops(self.terrain[sq as usize], flies, drills)
    }

    /// Any pits on the board? Gates the wide-bitboard fast path for
    /// flyers (whose kernels must ignore pit obstruction).
    #[inline]
    pub fn has_pit(&self) -> bool {
        self.pit_count > 0
    }

    /// Any walls or destructible blocks? Gates the wide-bitboard fast
    /// path for drillers (whose kernels must ignore wall obstruction).
    #[inline]
    pub fn has_wall(&self) -> bool {
        self.wall_count > 0
    }

    fn fwd(side: Side) -> i8 {
        if side == 0 {
            1
        } else {
            -1
        }
    }

    // -- incremental-hash brackets -------------------------------------

    /// XOR piece `i`'s key (state-dependent) into the hash. Call once
    /// before and once after mutating any of its keyed state; off-board
    /// pieces contribute nothing, so hand/dead transitions pair naturally.
    #[inline]
    pub(crate) fn xor_piece(&mut self, g: &GameDef, i: usize) {
        if let Loc::Board(sq) = self.pieces[i].loc {
            let p = &self.pieces[i];
            self.hash ^= g.zobrist.piece_key(
                p.t as usize,
                p.side as usize,
                p.moved,
                self.hp[i],
                sq as usize,
            );
        }
    }

    #[inline]
    pub(crate) fn xor_hand(&mut self, g: &GameDef, s: usize, t: usize) {
        self.hash ^= g.zobrist.hand_key(s, t, self.hands[s][t] as usize);
    }

    #[inline]
    pub(crate) fn xor_terrain(&mut self, g: &GameDef, sq: u16) {
        self.hash ^= g.zobrist.terrain_key(self.terrain[sq as usize], sq as usize);
    }

    /// Put piece `i` on `sq` (mailbox + masks + loc).
    #[inline]
    pub(crate) fn place(&mut self, i: usize, sq: u16) {
        self.board[sq as usize] = i as i32;
        self.pieces[i].loc = Loc::Board(sq);
        if self.wide {
            self.occ_all |= 1u128 << sq;
            self.occ_side[self.pieces[i].side as usize] |= 1u128 << sq;
        }
    }

    /// Remove piece `i` from `sq` (mailbox + masks; caller sets loc).
    #[inline]
    pub(crate) fn lift(&mut self, i: usize, sq: u16) {
        self.board[sq as usize] = -1;
        if self.wide {
            self.occ_all &= !(1u128 << sq);
            self.occ_side[self.pieces[i].side as usize] &= !(1u128 << sq);
        }
    }

    #[inline]
    pub(crate) fn set_terrain(&mut self, sq: u16, t: u8) {
        let old = self.terrain[sq as usize];
        if old == T_PIT {
            self.pit_count -= 1;
        }
        if t == T_PIT {
            self.pit_count += 1;
        }
        if old == T_WALL || is_block(old) {
            self.wall_count -= 1;
        }
        if t == T_WALL || is_block(t) {
            self.wall_count += 1;
        }
        self.terrain[sq as usize] = t;
        if self.wide {
            if terrain_blocks(t) {
                self.terrain_mask |= 1u128 << sq;
            } else {
                self.terrain_mask &= !(1u128 << sq);
            }
        }
    }

    #[inline]
    fn advance_stm(&mut self, g: &GameDef) {
        self.hash ^= g.zobrist.stm_key(self.stm as usize);
        self.stm = (self.stm + 1) % g.sides;
        self.hash ^= g.zobrist.stm_key(self.stm as usize);
    }

    /// Set the side to move out-of-band, keeping the incremental key valid
    /// (evaluation probes, compound-step enumeration, tests).
    #[inline]
    pub fn set_stm(&mut self, g: &GameDef, s: Side) {
        self.hash ^= g.zobrist.stm_key(self.stm as usize);
        self.stm = s;
        self.hash ^= g.zobrist.stm_key(self.stm as usize);
    }

    // -------------------------------------------------------------------

    /// Debug-only invariant: mailbox and piece list must agree.
    #[cfg(debug_assertions)]
    pub fn assert_consistent(&self, ctx: &str) {
        for (i, p) in self.pieces.iter().enumerate() {
            if let Loc::Board(sq) = p.loc {
                assert_eq!(
                    self.board[sq as usize], i as i32,
                    "{ctx}: piece {i} thinks it is on {sq} but the mailbox disagrees"
                );
            }
        }
        for (sq, &pi) in self.board.iter().enumerate() {
            if pi >= 0 {
                assert_eq!(
                    self.pieces[pi as usize].loc,
                    Loc::Board(sq as u16),
                    "{ctx}: mailbox {sq} points at piece {pi} which is elsewhere"
                );
            }
        }
    }

    pub fn make(&mut self, g: &GameDef, mv: &Move) -> Undo {
        #[cfg(debug_assertions)]
        self.assert_consistent("make-entry");
        let u = self.make_impl(g, mv);
        debug_assert_eq!(
            self.hash,
            g.zobrist.full_hash(self),
            "incremental hash diverged on {mv:?}"
        );
        #[cfg(debug_assertions)]
        if self.wide {
            let mut occ = 0u128;
            for p in &self.pieces {
                if let Loc::Board(sq) = p.loc {
                    occ |= 1u128 << sq;
                }
            }
            debug_assert_eq!(self.occ_all, occ, "occupancy mirror diverged on {mv:?}");
        }
        u
    }

    /// One move = one stdlib script (Bits 2.0 Stage 2). The one
    /// `MoveKind` branch is the script lookup; everything after is
    /// driven by the row's data. The dominant Mover shape
    /// `[CaptureAt(victim), Relocate(+TransformType)]` keeps its
    /// specialized non-interpreted path parameterized by the row
    /// (victim square, strike rule, trailing ops); abilities interpret
    /// their effect registry row; drops interpret the drop row's ops;
    /// the overclock keeps its kill-conditional sequencing. Every
    /// mutation lands in the op log in application order,
    /// hash-bracketed inside the op.
    fn make_impl(&mut self, g: &GameDef, mv: &Move) -> Undo {
        let mut u = Undo { ops: OpLog::new(), prior_hash: self.hash };
        // Expire the ephemeral marker (logged, not snapshotted).
        self.op_clear_ephemeral(g, &mut u.ops);
        // Select the script (the ONE MoveKind branch, jump-threaded to
        // each row's constant-folded entry point) and apply it — no
        // further branching on the kind anywhere in make/unmake.
        crate::move_defs::apply_script(self, g, mv, &mut u.ops);
        self.advance_stm(g);
        u
    }

    /// The dominant Mover shape, parameterized by the row: optional
    /// `CaptureAt(victim)` (aux for the ep/locust scripts, dest
    /// otherwise; none for castle) with the row's strike rule — a strike
    /// on an armored victim leaves the mover in place when the row says
    /// so — then `Relocate(From→To)` with any promotion (the appended
    /// `TransformType` op) fused into the same bracket, the
    /// landing-hazard hook, and the row's trailing ops (the double-step's
    /// `SetEphemeral(mid)`, the castle's partner relocation).
    #[inline(always)]
    pub(crate) fn mover_apply(
        &mut self,
        g: &GameDef,
        mv: &Move,
        victim: Option<crate::ops::SqRef>,
        strike_stops_mover: bool,
        trailing: &[crate::ops::MicroOp],
        log: &mut OpLog,
    ) {
        let mi = self.board[mv.from as usize] as usize;
        let mut moved_to_dest = true;
        if let Some(v) = victim {
            let victim_sq = match v {
                crate::ops::SqRef::Aux => mv.aux,
                _ => mv.to,
            };
            if self.board[victim_sq as usize] >= 0 {
                let killed = self.op_capture_at(g, victim_sq, log);
                if !killed && strike_stops_mover {
                    moved_to_dest = false;
                }
            }
        }
        if moved_to_dest {
            self.op_relocate_promo(g, mi, mv.to, mv.promo, log);
        }
        self.hazard_landing(g, mi, log);
        if !trailing.is_empty() {
            self.apply_move_ops(g, mv, trailing, log);
        }
    }

    /// Interpret an op list against the move. `inline(always)`: the
    /// per-row wrappers call this with CONSTANT op slices, so the loop
    /// unrolls and the unused context fields fall away per row.
    #[inline(always)]
    pub(crate) fn apply_move_ops(
        &mut self,
        g: &GameDef,
        mv: &Move,
        ops: &[crate::ops::MicroOp],
        log: &mut OpLog,
    ) {
        let ctx = OpCtx {
            from: mv.from,
            to: mv.to,
            aux: mv.aux,
            amount: match mv.effect {
                Effect::Heal(n) => n,
                _ => 0,
            },
            side: self.stm,
            drop_type: mv.drop_type,
            promo: mv.promo,
        };
        for op in ops {
            self.apply_op(g, op, &ctx, log);
        }
    }

    /// The ability interpreter: the move's effect names its registry
    /// row; target ops then self ops apply.
    pub(crate) fn apply_effect_row(&mut self, g: &GameDef, mv: &Move, log: &mut OpLog) {
        let row = crate::effects::row(mv.effect);
        self.apply_move_ops(g, mv, row.ops, log);
        self.apply_move_ops(g, mv, row.self_ops, log);
    }

    /// Overclock ⟨move, move, self −1 HP⟩ (§3.4): step 1 is a
    /// non-capture move from→to; step 2 to→aux may capture or strike (a
    /// strike leaves the mover on `to`).
    pub(crate) fn apply_sequenced(&mut self, g: &GameDef, mv: &Move, log: &mut OpLog) {
        let mi = self.board[mv.from as usize] as usize;
        self.op_relocate(g, mi, mv.to, log);
        let killed2 = self.op_capture_at(g, mv.aux, log);
        if killed2 || self.board[mv.aux as usize] < 0 {
            self.op_relocate(g, mi, mv.aux, log);
        }
        self.op_hp_add(g, mi, -1, false, log);
        self.hazard_landing(g, mi, log);
    }

    /// Replay the move's op log backwards — each record exactly inverts
    /// its op (including ephemeral-marker changes) — then restore the
    /// O(1) hash snapshot.
    pub fn unmake(&mut self, g: &GameDef, u: &Undo) {
        self.stm = (self.stm + g.sides - 1) % g.sides;
        for op in u.ops.rev_iter() {
            self.revert_op(op);
        }
        self.hash = u.prior_hash;
    }

    /// Null move (for null-move pruning): pass the turn, clearing ep.
    /// Returns (prior_ep, prior_hash) for `unmake_null`.
    pub fn make_null(&mut self, g: &GameDef) -> (u16, u64) {
        let prior = (self.ep, self.hash);
        self.hash ^= g.zobrist.ep_key(self.ep);
        self.ep = NO_SQ;
        self.advance_stm(g);
        prior
    }

    pub fn unmake_null(&mut self, g: &GameDef, prior: (u16, u64)) {
        self.stm = (self.stm + g.sides - 1) % g.sides;
        self.ep = prior.0;
        self.hash = prior.1;
    }

    /// Is `sq` attacked by any piece of `by`? Target predicates see the
    /// occupant of `sq` (empty ⇒ predicates like EnemyRoyal fail). Terrain
    /// obstructs rays, screens, and lame-leaper legs.
    ///
    /// Plain kernels (no zones/predicates) run on the wide-bitboard mirrors:
    /// blocker checks are single mask intersections instead of walks.
    pub fn is_attacked(&self, g: &GameDef, sq: u16, by: Side) -> bool {
        use crate::bits::TargetPred;
        let (tx, ty) = g.board.xy(sq);
        let occ_royal = self.piece_at(sq).map_or(false, |p| g.types[p.t as usize].royal);
        let pred_ok = |t: TargetPred| match t {
            TargetPred::Any => true,
            TargetPred::EnemyRoyal => occ_royal,
        };
        let use_bb = self.wide && g.use_bitboards;
        let obstructed = self.occ_all | self.terrain_mask;
        let sq_bit = 1u128 << sq;
        for (pi, p) in self.pieces.iter().enumerate() {
            let Loc::Board(psq) = p.loc else { continue };
            if p.side != by || psq == sq {
                continue;
            }
            // A hologram projects no threat: it cannot capture or check.
            if g.types[p.t as usize].hologram {
                continue;
            }
            let aflies = g.types[p.t as usize].flight;
            let adrills = g.types[p.t as usize].drill;
            let ahp = self.hp[pi];
            let (px, py) = g.board.xy(psq);
            let (dx, dy) = ((tx - px) as i32, (ty - py) as i32);
            let ck = g.compiled(p.t, p.side);
            let zone_ok = |z: Option<usize>, s: Side, at: u16| {
                z.map_or(true, |zi| g.zones[zi].contains(s, at))
            };
            let bb_active = use_bb
                && ck.bb.is_some()
                && !(aflies && self.has_pit())
                && !(adrills && self.has_wall());
            if bb_active {
                let bb = ck.bb.as_ref().unwrap();
                for e in &bb.leaps[psq as usize] {
                    if e.to == sq && e.mode.can_capture() && e.blockers & obstructed == 0 {
                        return true;
                    }
                }
                for r in &bb.rides {
                    if !r.mode.can_capture() {
                        continue;
                    }
                    let ray = r.rays[psq as usize];
                    if ray & sq_bit == 0 {
                        continue;
                    }
                    // Cells strictly between attacker and target.
                    let between = ray & !r.rays[sq as usize] & !sq_bit;
                    if between & obstructed == 0 {
                        return true;
                    }
                }
            }
            for (ki, k) in ck.leaps.iter().enumerate() {
                if bb_active && ck.leap_plain[ki] {
                    continue;
                }
                if !hp_gate_ok(k.min_hp, k.max_hp, ahp) {
                    continue;
                }
                if !k.mode.can_capture() || k.d.dx as i32 != dx || k.d.dy as i32 != dy {
                    continue;
                }
                if !pred_ok(k.target)
                    || !zone_ok(k.from_zone, p.side, psq)
                    || !zone_ok(k.to_zone, p.side, sq)
                {
                    continue;
                }
                let blocked = k.blockers.iter().any(|b| {
                    g.board
                        .offset(psq, b.dx, b.dy)
                        .map_or(true, |bsq| self.cell_obstructed_for(bsq, aflies, adrills))
                });
                if !blocked {
                    return true;
                }
            }
            'ride: for (ki, k) in ck.rides.iter().enumerate() {
                if bb_active && ck.ride_plain[ki] {
                    continue;
                }
                if !hp_gate_ok(k.min_hp, k.max_hp, ahp) {
                    continue;
                }
                if !k.mode.can_capture() || !pred_ok(k.target) {
                    continue;
                }
                let Some(steps) = ray_steps(dx, dy, k.d.dx as i32, k.d.dy as i32) else {
                    continue;
                };
                if !zone_ok(k.from_zone, p.side, psq) || !zone_ok(k.to_zone, p.side, sq) {
                    continue;
                }
                if k.max_steps != 0 && steps > k.max_steps as i32 {
                    continue; // range-limited rider: target beyond its reach
                }
                let mut cur = psq;
                for _ in 1..steps {
                    cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                    if self.cell_obstructed_for(cur, aflies, adrills) {
                        continue 'ride;
                    }
                }
                return true;
            }
            'hop: for k in &ck.hops {
                if !hp_gate_ok(k.min_hp, k.max_hp, ahp) {
                    continue;
                }
                if !k.mode.can_capture() {
                    continue;
                }
                let Some(steps) = ray_steps(dx, dy, k.d.dx as i32, k.d.dy as i32) else {
                    continue;
                };
                if !zone_ok(k.from_zone, p.side, psq) {
                    continue;
                }
                match k.landing {
                    // Cannon: `sq` is capturable at any range past exactly
                    // one screen strictly between.
                    crate::bits::HopMode::CannonAtRange => {
                        if !pred_ok(k.target) || !zone_ok(k.to_zone, p.side, sq) {
                            continue;
                        }
                        let mut screens = 0;
                        let mut cur = psq;
                        for _ in 1..steps {
                            cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                            if terrain_stops(self.terrain[cur as usize], aflies, adrills) {
                                continue 'hop; // blocking terrain is not a screen
                            }
                            if self.board[cur as usize] >= 0 {
                                screens += 1;
                                if screens > 1 {
                                    continue 'hop;
                                }
                            }
                        }
                        if screens == 1 {
                            return true;
                        }
                    }
                    // Grasshopper: `sq` is capturable iff it lies exactly
                    // one step beyond the ray's first screen — the cell
                    // just before `sq` occupied, everything nearer empty.
                    crate::bits::HopMode::BeyondScreen => {
                        if steps < 2 || !pred_ok(k.target) || !zone_ok(k.to_zone, p.side, sq) {
                            continue;
                        }
                        let mut cur = psq;
                        for i in 1..steps {
                            cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                            if terrain_stops(self.terrain[cur as usize], aflies, adrills) {
                                continue 'hop;
                            }
                            let occupied = self.board[cur as usize] >= 0;
                            if occupied != (i == steps - 1) {
                                continue 'hop; // screen must sit exactly there
                            }
                        }
                        return true;
                    }
                    // Locust: the capture geometry targets the SCREEN —
                    // `sq` itself is the first piece on the ray and the
                    // square beyond it must be an open, empty landing.
                    crate::bits::HopMode::Locust => {
                        if self.board[sq as usize] < 0 || !pred_ok(k.target) {
                            continue; // an empty square cannot be the victim
                        }
                        // A victim tucked inside terrain the attacker's ray
                        // cannot enter (a driller in a block) is unreachable
                        // — mirror of movegen's screen-cell terrain check.
                        if terrain_stops(self.terrain[sq as usize], aflies, adrills) {
                            continue;
                        }
                        let mut cur = psq;
                        let mut clear = true;
                        for _ in 1..steps {
                            cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                            if terrain_stops(self.terrain[cur as usize], aflies, adrills)
                                || self.board[cur as usize] >= 0
                            {
                                clear = false;
                                break;
                            }
                        }
                        if !clear {
                            continue;
                        }
                        let Some(land) = g.board.offset(sq, k.d.dx, k.d.dy) else { continue };
                        if self.board[land as usize] >= 0
                            || terrain_stops(self.terrain[land as usize], aflies, adrills)
                            || !zone_ok(k.to_zone, p.side, land)
                        {
                            continue;
                        }
                        return true;
                    }
                }
            }
        }
        false
    }

    /// True if any royal piece of `side` is attacked by **any** other side
    /// (N-player-correct; includes xiangqi's facing-generals rule via the
    /// flying-general target-predicate Bit).
    pub fn royal_attacked(&self, g: &GameDef, side: Side) -> bool {
        self.pieces.iter().any(|p| {
            g.types[p.t as usize].royal
                && p.side == side
                && matches!(p.loc, Loc::Board(sq)
                    if (0..g.sides).any(|e| e != side && self.is_attacked(g, sq, e)))
        })
    }

    pub fn forward(side: Side) -> i8 {
        Self::fwd(side)
    }
}

/// Is the kernel's HP gate (§3.1 state condition) satisfied at `hp`?
/// 0 means unbounded on that end.
#[inline]
pub fn hp_gate_ok(min_hp: i16, max_hp: i16, hp: i16) -> bool {
    (min_hp == 0 || hp >= min_hp) && (max_hp == 0 || hp <= max_hp)
}

/// If (dx,dy) = k·(sx,sy) for integer k ≥ 1, return k.
fn ray_steps(dx: i32, dy: i32, sx: i32, sy: i32) -> Option<i32> {
    let k = if sx != 0 {
        if dx % sx != 0 {
            return None;
        }
        dx / sx
    } else if dx != 0 {
        return None;
    } else if sy != 0 {
        if dy % sy != 0 {
            return None;
        }
        dy / sy
    } else {
        return None;
    };
    if k >= 1 && dx == k * sx && dy == k * sy {
        Some(k)
    } else {
        None
    }
}
