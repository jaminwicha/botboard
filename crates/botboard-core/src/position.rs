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

use crate::game::{CaptureFate, GameDef, Side, TypeId};
use crate::moves::{Effect, Move, MoveKind, NO_SQ};

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

/// Terrain that blocks entry, rays, screens, and legs (mines do not).
#[inline]
pub fn terrain_blocks(t: u8) -> bool {
    t == T_WALL || t == T_PIT
}

/// `terrain_blocks` under a terrain permission: flight ignores pits.
#[inline]
pub fn terrain_stops(t: u8, flies: bool) -> bool {
    if flies {
        t == T_WALL
    } else {
        terrain_blocks(t)
    }
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
    /// Per-cell terrain (T_NONE / T_WALL / T_PIT). Blocks entry, rays,
    /// screens, and lame-leaper legs; flight/drill permissions are the
    /// later path-interaction refinement (§3.1).
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
    wide: bool,
}

pub struct Undo {
    mv: Move,
    moving: usize,
    prior_t: TypeId,
    prior_moved: bool,
    /// Where the mover actually stood before the move (strikes don't move).
    prior_sq: u16,
    captured: Option<(usize, Piece)>,
    /// Second-step capture of a compound.
    captured2: Option<(usize, Piece)>,
    /// Castle partner: (index, prior square, prior moved).
    partner: Option<(usize, u16, bool)>,
    /// (piece index, prior hp) — strikes, heals, overclock self-damage.
    hp_changes: Vec<(usize, i16)>,
    terrain_change: Option<(u16, u8)>,
    /// Resurrect: (revived piece index, its prior `moved` flag).
    revived: Option<(usize, bool)>,
    /// Hack: (flipped piece index, its prior side).
    hacked: Option<(usize, Side)>,
    /// Mine kill: (piece index, the square it died on).
    mine_death: Option<(usize, u16)>,
    prior_ep: u16,
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

    /// `cell_obstructed` with a terrain permission: flyers pass over pits
    /// (Appendix B hover/flight); walls stop everyone.
    #[inline]
    pub fn cell_obstructed_for(&self, sq: u16, flies: bool) -> bool {
        if self.board[sq as usize] >= 0 {
            return true;
        }
        let t = self.terrain[sq as usize];
        if flies {
            t == T_WALL
        } else {
            terrain_blocks(t)
        }
    }

    /// May a piece land here (ignoring occupancy)? Mines are open — that
    /// is their whole point.
    #[inline]
    pub fn terrain_open(&self, sq: u16) -> bool {
        !terrain_blocks(self.terrain[sq as usize])
    }

    /// `terrain_open` with a terrain permission: a flyer may hover on a
    /// pit square.
    #[inline]
    pub fn terrain_open_for(&self, sq: u16, flies: bool) -> bool {
        let t = self.terrain[sq as usize];
        if flies {
            t != T_WALL
        } else {
            !terrain_blocks(t)
        }
    }

    /// Any pits on the board? Gates the wide-bitboard fast path for
    /// flyers (whose kernels must ignore pit obstruction).
    #[inline]
    pub fn has_pit(&self) -> bool {
        self.pit_count > 0
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
    fn xor_piece(&mut self, g: &GameDef, i: usize) {
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
    fn xor_hand(&mut self, g: &GameDef, s: usize, t: usize) {
        self.hash ^= g.zobrist.hand_key(s, t, self.hands[s][t] as usize);
    }

    #[inline]
    fn xor_terrain(&mut self, g: &GameDef, sq: u16) {
        self.hash ^= g.zobrist.terrain_key(self.terrain[sq as usize], sq as usize);
    }

    /// Put piece `i` on `sq` (mailbox + masks + loc).
    #[inline]
    fn place(&mut self, i: usize, sq: u16) {
        self.board[sq as usize] = i as i32;
        self.pieces[i].loc = Loc::Board(sq);
        if self.wide {
            self.occ_all |= 1u128 << sq;
            self.occ_side[self.pieces[i].side as usize] |= 1u128 << sq;
        }
    }

    /// Remove piece `i` from `sq` (mailbox + masks; caller sets loc).
    #[inline]
    fn lift(&mut self, i: usize, sq: u16) {
        self.board[sq as usize] = -1;
        if self.wide {
            self.occ_all &= !(1u128 << sq);
            self.occ_side[self.pieces[i].side as usize] &= !(1u128 << sq);
        }
    }

    #[inline]
    fn set_terrain(&mut self, sq: u16, t: u8) {
        if self.terrain[sq as usize] == T_PIT {
            self.pit_count -= 1;
        }
        if t == T_PIT {
            self.pit_count += 1;
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

    /// Landing hazards (SRW Appendix B): spring an enemy mine (spent on
    /// trigger) or wade into acid (persistent, bites every side). Either
    /// way the lander takes 1 HP — lethal at 1.
    fn trigger_mine(&mut self, g: &GameDef, mi: usize, u: &mut Undo) {
        let Loc::Board(sq) = self.pieces[mi].loc else { return };
        let t = self.terrain[sq as usize];
        let bites = match mine_owner(t) {
            Some(owner) => {
                if owner == self.pieces[mi].side {
                    return;
                }
                // The mine is spent.
                debug_assert!(u.terrain_change.is_none(), "one terrain change per move");
                u.terrain_change = Some((sq, t));
                self.xor_terrain(g, sq);
                self.set_terrain(sq, T_NONE);
                self.xor_terrain(g, sq);
                true
            }
            None => t == T_ACID,
        };
        if !bites {
            return;
        }
        if self.hp[mi] > 1 {
            u.hp_changes.push((mi, self.hp[mi]));
            self.xor_piece(g, mi);
            self.hp[mi] -= 1;
            self.xor_piece(g, mi);
        } else {
            self.xor_piece(g, mi);
            self.lift(mi, sq);
            self.pieces[mi].loc = Loc::Dead;
            u.mine_death = Some((mi, sq));
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

    /// Apply a capture-or-strike against the occupant of `sq`.
    /// Returns (captured record, killed).
    fn hit(
        &mut self,
        g: &GameDef,
        sq: u16,
        undo_hp: &mut Vec<(usize, i16)>,
    ) -> (Option<(usize, Piece)>, bool) {
        let ci = self.board[sq as usize];
        if ci < 0 {
            return (None, false);
        }
        let ci = ci as usize;
        if self.hp[ci] > 1 {
            // Armor strike (§3.2): decrement HP, victim stays.
            undo_hp.push((ci, self.hp[ci]));
            self.xor_piece(g, ci);
            self.hp[ci] -= 1;
            self.xor_piece(g, ci);
            return (None, false);
        }
        let prior = self.pieces[ci];
        self.xor_piece(g, ci);
        self.lift(ci, sq);
        match g.policy.capture_fate {
            CaptureFate::Destroy => self.pieces[ci].loc = Loc::Dead,
            CaptureFate::ToHand => {
                let base = self.pieces[ci].base;
                self.pieces[ci].t = base;
                self.pieces[ci].side = self.stm;
                self.pieces[ci].loc = Loc::Hand(self.stm);
                self.xor_hand(g, self.stm as usize, base as usize);
                self.hands[self.stm as usize][base as usize] += 1;
                self.xor_hand(g, self.stm as usize, base as usize);
            }
        }
        // Off-board now: xor_piece contributes nothing (paired naturally).
        (Some((ci, prior)), true)
    }

    fn move_piece(&mut self, mi: usize, from: u16, to: u16) {
        self.lift(mi, from);
        self.place(mi, to);
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

    fn make_impl(&mut self, g: &GameDef, mv: &Move) -> Undo {
        let prior_ep = self.ep;
        let prior_hash = self.hash;
        self.hash ^= g.zobrist.ep_key(self.ep);
        self.ep = NO_SQ;

        if mv.kind == MoveKind::Drop {
            let idx = self
                .pieces
                .iter()
                .position(|p| p.loc == Loc::Hand(self.stm) && p.t == mv.drop_type)
                .expect("drop with empty hand");
            let mut u = Undo {
                mv: *mv,
                moving: idx,
                prior_t: self.pieces[idx].t,
                prior_moved: self.pieces[idx].moved,
                prior_sq: NO_SQ,
                captured: None,
                captured2: None,
                partner: None,
                hp_changes: Vec::new(),
                terrain_change: None,
                revived: None,
                hacked: None,
                mine_death: None,
                prior_ep,
                prior_hash,
            };
            self.xor_hand(g, self.stm as usize, mv.drop_type as usize);
            self.hands[self.stm as usize][mv.drop_type as usize] -= 1;
            self.xor_hand(g, self.stm as usize, mv.drop_type as usize);
            // From hand (no key) to board (keyed): single trailing XOR.
            self.place(idx, mv.to);
            self.pieces[idx].moved = true;
            // A dropped piece re-enters at full HP for its type.
            if self.hp[idx] != g.types[mv.drop_type as usize].max_hp {
                u.hp_changes.push((idx, self.hp[idx]));
                self.hp[idx] = g.types[mv.drop_type as usize].max_hp;
            }
            self.xor_piece(g, idx);
            self.trigger_mine(g, idx, &mut u);
            self.advance_stm(g);
            return u;
        }

        let mi = self.board[mv.from as usize] as usize;
        let mut u = Undo {
            mv: *mv,
            moving: mi,
            prior_t: self.pieces[mi].t,
            prior_moved: self.pieces[mi].moved,
            prior_sq: mv.from,
            captured: None,
            captured2: None,
            partner: None,
            hp_changes: Vec::new(),
            terrain_change: None,
            revived: None,
            hacked: None,
            mine_death: None,
            prior_ep,
            prior_hash,
        };

        match mv.kind {
            MoveKind::Ability => {
                match mv.effect {
                    Effect::Heal(amount) => {
                        let ti = self.board[mv.to as usize] as usize;
                        u.hp_changes.push((ti, self.hp[ti]));
                        let max = g.types[self.pieces[ti].t as usize].max_hp;
                        self.xor_piece(g, ti);
                        self.hp[ti] = (self.hp[ti] + amount).min(max);
                        self.xor_piece(g, ti);
                    }
                    Effect::Wall | Effect::Pit | Effect::Mine => {
                        u.terrain_change = Some((mv.to, self.terrain[mv.to as usize]));
                        self.xor_terrain(g, mv.to);
                        self.set_terrain(
                            mv.to,
                            match mv.effect {
                                Effect::Wall => T_WALL,
                                Effect::Pit => T_PIT,
                                _ => T_MINE0 + self.stm,
                            },
                        );
                        self.xor_terrain(g, mv.to);
                    }
                    Effect::Laser => {
                        let (cap, _killed) = self.hit(g, mv.to, &mut u.hp_changes);
                        u.captured = cap;
                        if mv.aux != NO_SQ {
                            // Coupled retreat (§3.4): part of the atomic script.
                            self.xor_piece(g, mi);
                            self.move_piece(mi, mv.from, mv.aux);
                            self.pieces[mi].moved = true;
                            self.xor_piece(g, mi);
                            self.trigger_mine(g, mi, &mut u);
                        }
                    }
                    Effect::Hack => {
                        // Flip the target to the caster's side. Masks track
                        // side, so lift under the old side and re-place
                        // under the new; the hash brackets the whole flip.
                        let ti = self.board[mv.to as usize] as usize;
                        u.hacked = Some((ti, self.pieces[ti].side));
                        self.xor_piece(g, ti);
                        self.lift(ti, mv.to);
                        self.pieces[ti].side = self.stm;
                        self.place(ti, mv.to);
                        self.xor_piece(g, ti);
                    }
                    Effect::Resurrect => {
                        // Revive the first dead friendly of the named type
                        // (identity within a type is interchangeable, so
                        // lowest index keeps it deterministic) at 1 HP.
                        let ri = self
                            .pieces
                            .iter()
                            .position(|p| {
                                p.side == self.stm
                                    && p.t == mv.drop_type
                                    && p.loc == Loc::Dead
                            })
                            .expect("resurrect with no dead piece of that type");
                        u.revived = Some((ri, self.pieces[ri].moved));
                        u.hp_changes.push((ri, self.hp[ri]));
                        // Dead pieces are unkeyed: mutate fully, then one
                        // trailing XOR keys the revived state.
                        self.hp[ri] = 1;
                        self.place(ri, mv.to);
                        self.pieces[ri].moved = true;
                        self.xor_piece(g, ri);
                    }
                    _ => {}
                }
                self.advance_stm(g);
                return u;
            }
            MoveKind::Compound => {
                // Overclock ⟨move, move, self −1 HP⟩ (§3.4): step 1 is a
                // non-capture move from→to; step 2 to→aux may capture/strike.
                self.xor_piece(g, mi);
                self.move_piece(mi, mv.from, mv.to);
                self.xor_piece(g, mi);
                let (cap2, killed2) = self.hit(g, mv.aux, &mut u.hp_changes);
                u.captured2 = cap2;
                self.xor_piece(g, mi);
                if killed2 || self.board[mv.aux as usize] < 0 {
                    self.move_piece(mi, mv.to, mv.aux);
                }
                self.pieces[mi].moved = true;
                u.hp_changes.push((mi, self.hp[mi]));
                self.hp[mi] -= 1;
                self.xor_piece(g, mi);
                self.trigger_mine(g, mi, &mut u);
                self.advance_stm(g);
                return u;
            }
            _ => {}
        }

        let victim_sq = if mv.kind == MoveKind::EnPassant { mv.aux } else { mv.to };
        let mut moved_to_dest = true;
        if mv.kind != MoveKind::Castle && self.board[victim_sq as usize] >= 0 {
            let (cap, killed) = self.hit(g, victim_sq, &mut u.hp_changes);
            u.captured = cap;
            if !killed && victim_sq == mv.to {
                // Strike on an armored piece: the attacker stays put.
                moved_to_dest = false;
            }
        }

        if moved_to_dest {
            self.xor_piece(g, mi);
            self.move_piece(mi, mv.from, mv.to);
            self.pieces[mi].moved = true;
            if let Some(pt) = mv.promo {
                self.pieces[mi].t = pt;
            }
            self.xor_piece(g, mi);
        }

        self.trigger_mine(g, u.moving, &mut u);

        match mv.kind {
            MoveKind::DoubleStep => {
                let (x, y1) = g.board.xy(mv.from);
                let (_, y2) = g.board.xy(mv.to);
                self.ep = g.board.sq(x as u8, ((y1 + y2) / 2) as u8);
                self.hash ^= g.zobrist.ep_key(self.ep);
            }
            MoveKind::Castle => {
                let ri = self.board[mv.aux as usize] as usize;
                let rook_to = (mv.from + mv.to) / 2;
                u.partner = Some((ri, mv.aux, self.pieces[ri].moved));
                self.xor_piece(g, ri);
                self.move_piece(ri, mv.aux, rook_to);
                self.pieces[ri].moved = true;
                self.xor_piece(g, ri);
            }
            _ => {}
        }

        self.advance_stm(g);
        u
    }

    pub fn unmake(&mut self, g: &GameDef, u: &Undo) {
        self.stm = (self.stm + g.sides - 1) % g.sides;
        self.ep = u.prior_ep;
        let mv = &u.mv;

        if mv.kind == MoveKind::Drop {
            if let Some((sq, prior)) = u.terrain_change {
                self.set_terrain(sq, prior); // the spent mine returns
            }
            self.lift(u.moving, mv.to);
            self.pieces[u.moving].loc = Loc::Hand(self.stm);
            self.pieces[u.moving].moved = u.prior_moved;
            self.hands[self.stm as usize][mv.drop_type as usize] += 1;
            for &(i, hp) in u.hp_changes.iter().rev() {
                self.hp[i] = hp;
            }
            self.hash = u.prior_hash;
            return;
        }

        if let Some((sq, prior)) = u.terrain_change {
            self.set_terrain(sq, prior);
        }

        if let Some((ri, rsq, rmoved)) = u.partner {
            let rook_to = (mv.from + mv.to) / 2;
            self.lift(ri, rook_to);
            self.place(ri, rsq);
            self.pieces[ri].moved = rmoved;
        }

        if let Some((ri, rmoved)) = u.revived {
            self.lift(ri, mv.to);
            self.pieces[ri].loc = Loc::Dead;
            self.pieces[ri].moved = rmoved;
        }

        if let Some((hi, side)) = u.hacked {
            self.lift(hi, mv.to);
            self.pieces[hi].side = side;
            self.place(hi, mv.to);
        }

        if let Some((mi, dsq)) = u.mine_death {
            // Back from the crater; the mover-back block below (or the
            // captured-restore for victims) finishes the rewind.
            self.place(mi, dsq);
        }

        // Put the mover back wherever it now stands.
        if let Loc::Board(cur) = self.pieces[u.moving].loc {
            if cur != u.prior_sq {
                self.lift(u.moving, cur);
                self.place(u.moving, u.prior_sq);
            }
        }
        self.pieces[u.moving].moved = u.prior_moved;
        self.pieces[u.moving].t = u.prior_t;

        for &(i, hp) in u.hp_changes.iter().rev() {
            self.hp[i] = hp;
        }

        for cap in [&u.captured2, &u.captured].into_iter().flatten() {
            let (ci, prior) = *cap;
            if let Loc::Hand(s) = self.pieces[ci].loc {
                self.hands[s as usize][self.pieces[ci].t as usize] -= 1;
            }
            self.pieces[ci] = prior;
            let Loc::Board(vsq) = prior.loc else { unreachable!() };
            self.place(ci, vsq);
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
        for p in &self.pieces {
            let Loc::Board(psq) = p.loc else { continue };
            if p.side != by || psq == sq {
                continue;
            }
            // A hologram projects no threat: it cannot capture or check.
            if g.types[p.t as usize].hologram {
                continue;
            }
            let aflies = g.types[p.t as usize].flight;
            let (px, py) = g.board.xy(psq);
            let (dx, dy) = ((tx - px) as i32, (ty - py) as i32);
            let ck = g.compiled(p.t, p.side);
            let zone_ok = |z: Option<usize>, s: Side, at: u16| {
                z.map_or(true, |zi| g.zones[zi].contains(s, at))
            };
            let bb_active = use_bb && ck.bb.is_some() && !(aflies && self.has_pit());
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
                        .map_or(true, |bsq| self.cell_obstructed_for(bsq, aflies))
                });
                if !blocked {
                    return true;
                }
            }
            'ride: for (ki, k) in ck.rides.iter().enumerate() {
                if bb_active && ck.ride_plain[ki] {
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
                let mut cur = psq;
                for _ in 1..steps {
                    cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                    if self.cell_obstructed_for(cur, aflies) {
                        continue 'ride;
                    }
                }
                return true;
            }
            'hop: for k in &ck.hops {
                if !pred_ok(k.target) {
                    continue;
                }
                let Some(steps) = ray_steps(dx, dy, k.d.dx as i32, k.d.dy as i32) else {
                    continue;
                };
                if !zone_ok(k.from_zone, p.side, psq) || !zone_ok(k.to_zone, p.side, sq) {
                    continue;
                }
                let mut screens = 0;
                let mut cur = psq;
                for _ in 1..steps {
                    cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                    if terrain_stops(self.terrain[cur as usize], aflies) {
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
