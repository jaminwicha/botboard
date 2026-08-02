//! Bits 2.0 Stage 1 — the micro-op layer (docs/bits2-effects-as-data.md).
//!
//! Effects and move-application scripts compose a small, closed,
//! engine-owned set of atomic micro-operations. Each op mutates the
//! position inside its own incremental-hash bracket (xor-out / mutate /
//! xor-in — exactly the discipline the old per-kind `make_impl` arms
//! used), pushes an exact-undo record onto the move's `OpLog`, and is
//! reverted by replaying the log backwards. Ops never compose ops; the
//! one hook that runs *after* an op is the landing-hazard check — as of
//! Stage 3 an interpreter over the terrain registry row's `on_land`
//! script (gate → consumed → ops), its mutations flowing through the
//! same undo records.
//!
//! Op set as built (deviations from the design table are documented in
//! the design doc's Stage-1 notes):
//! - `Relocate`        mover/partner/target relocation (dest `Aux` no-ops
//!                     when the move carries no aux — the retreat-less
//!                     laser shares one stdlib row with the retreat form)
//! - `CaptureAt`       strike-or-kill at a square (≠ dest ok: ep, locust);
//!                     no-op on an empty square
//! - `PlaceFrom`       enter from hand (full HP) or dead pool (1 HP)
//! - `HpAdd`           heal/damage; heal caps at type max
//! - `SetTerrain`      lay wall / pit / owner-tagged mine
//! - `TerrainStep`     block erosion: an *unoccupied* destructible block
//!                     drops one tier (T_BLOCK1 clears) — the emptiness
//!                     guard is inherent, so a laser row can sequence
//!                     [TerrainStep, CaptureAt] without a conditional
//! - `FlipSide`        hack: re-side the piece at the target
//! - `SwapWithCaster`  atomic two-piece exchange. Kept as ONE op (not two
//!                     `Relocate`s): a sequential decomposition would need
//!                     a temp square mid-transaction.
//!
//! Stage 2 promoted the last two deviations to first-class ops:
//! - `SetEphemeral`    lay the one-ply marker (en-passant square) the
//!                     `EphemeralMatch`-gated scripts consume; the
//!                     make-prologue clear logs the same `Ephemeral`
//!                     undo record (no move-level snapshot remains)
//! - `TransformType`   promotion/demotion as an interpreted op; the
//!                     dominant-script fast path still fuses it into the
//!                     mover's relocate bracket (`op_relocate_promo`) —
//!                     identical final hash, one bracket per plain move

use crate::game::{GameDef, Side, TypeId};
use crate::moves::NO_SQ;
use crate::position::{mine_owner, Loc, Piece, Position, T_MINE0, T_NONE, T_PIT, T_WALL};
use crate::terrain_defs::{self, LandGate};

/// Square reference, resolved against the applying move's fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SqRef {
    From,
    To,
    Aux,
    /// Midpoint of `from` and `to` — exact on a rectangular board whenever
    /// the two share a rank (castle rook destination) or a file two ranks
    /// apart (the double-step's passed-over square).
    Mid,
}

/// Source pool for `PlaceFrom` (§3.2 capture fate, §7 resurrect).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pool {
    /// Shogi-style hand: enters at full HP for its type.
    Hand,
    /// The dead pool: the lowest-index dead friendly of the move's named
    /// type re-enters at 1 HP (identity within a type is interchangeable,
    /// so lowest index keeps it deterministic).
    Dead,
}

/// Terrain laid by `SetTerrain`. Owner-tagged kinds resolve against the
/// caster's side at apply time (the T_MINE0 code band). `Code` lays a
/// fixed internal code — Stage-4 custom ability rows may lay any
/// authorable stdlib kind or a custom terrain row by its code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerrainKind {
    Wall,
    Pit,
    OwnerMine,
    Code(u8),
}

/// HP delta source for `HpAdd`: a literal, or the amount carried by the
/// move (`Effect::Heal(n)` — authoring parameters stay on `AbilityBit`
/// this stage, so the registry row is amount-generic).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HpAmt {
    Lit(i16),
    FromMove,
}

/// The closed micro-op set (design doc §"Micro-op set"). Registry rows
/// are ordered lists of these; the move fast path calls the same
/// underlying `op_*` functions directly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MicroOp {
    /// Relocate the piece standing at `who` to `dest`, marking it moved.
    /// `hazard` runs the landing-hazard hook (mine/acid) afterwards.
    /// A `dest` of `Aux` no-ops when the move carries no aux square.
    Relocate { who: SqRef, dest: SqRef, hazard: bool },
    /// Strike-or-kill the occupant of `at` (§3.2 hit-count armor):
    /// HP > 1 loses 1 HP and stays; HP 1 falls per the capture fate.
    /// No-op when `at` is empty.
    CaptureAt { at: SqRef },
    /// Enter the board at the move's `to` from a source pool. `hazard`
    /// runs the landing-hazard hook: drops spring mines, resurrected
    /// pieces do not (the Stage-1 behavior quirks, preserved as data).
    PlaceFrom { pool: Pool, hazard: bool },
    /// Add HP to the occupant of the move's `to`; `cap` clamps at the
    /// target type's max (heal never overfills). Stage-1 users never
    /// drive HP to 0 through this op (generation guards it), so
    /// floor-death lives only in `CaptureAt` and the hazard hook.
    HpAdd { n: HpAmt, cap: bool },
    /// Lay terrain on the move's `to` (movegen only targets bare floor).
    SetTerrain { kind: TerrainKind },
    /// Erode a destructible block at the move's `to` by one tier
    /// (T_BLOCK1 clears to floor). Only fires when the square is
    /// UNOCCUPIED and holds a block — a piece standing inside the block
    /// (a driller) shields it, matching the old laser arm's conditional.
    TerrainStep,
    /// Flip the occupant of the move's `to` to the caster's side (hack).
    FlipSide,
    /// Exchange the caster (at `from`) with the piece at `to`; both
    /// relocations are landings, so hazards bite both (caster first).
    SwapWithCaster,
    /// Set the ephemeral one-ply marker (the en-passant square) at `at`
    /// — the state `EphemeralMatch`-gated scripts consume. First-class
    /// as of Stage 2: the make-prologue clear and this producer both log
    /// `OpUndo::Ephemeral` records instead of a move-level snapshot.
    SetEphemeral { at: SqRef },
    /// Promotion/demotion: re-type the occupant of the move's `to` to the
    /// move's carried promotion type. First-class as of Stage 2; the
    /// dominant-script fast path still fuses it into the mover's relocate
    /// bracket (`op_relocate_promo`) — same final hash, one bracket.
    TransformType,
}

/// Move-derived context the interpreter resolves op parameters against.
pub struct OpCtx {
    pub from: u16,
    pub to: u16,
    pub aux: u16,
    /// Heal amount (`Effect::Heal(n)`); 0 otherwise.
    pub amount: i16,
    /// The caster's side (side to move at apply time).
    pub side: Side,
    /// Named type for `PlaceFrom` (drop or resurrect target type).
    pub drop_type: TypeId,
    /// Promotion target for `TransformType` (the move's `promo` field).
    pub promo: Option<TypeId>,
}

/// Exact-undo record for one applied micro-op. Reverting a move replays
/// its log backwards; each record fully inverts its op's state mutation
/// (the hash is restored O(1) from the move-level snapshot, and
/// `set_terrain`/`place`/`lift` maintain the masks and counts).
#[derive(Clone, Copy, Debug)]
pub enum OpUndo {
    /// A piece relocated: it now stands wherever its `loc` says; put it
    /// back on `from`. Covers mover, castle rook, push victim, retreat.
    Moved { i: u32, from: u16, prior_moved: bool },
    /// Promotion/demotion of piece `i` (fused into the mover's bracket).
    Transformed { i: u32, prior_t: TypeId },
    /// A piece fell to `CaptureAt`: restore the full prior record (side,
    /// type, loc — a ToHand fate re-sided and re-based it) and un-bank
    /// the hand credit if it went to a hand.
    Captured { i: u32, prior: Piece },
    /// HP change (strike, heal, overclock self-damage, hazard bite).
    Hp { i: u32, prior: i16 },
    /// Terrain change (lay/spend/erode), in application order.
    Terrain { sq: u16, prior: u8 },
    /// Entered from a hand: return there (side/type are on the piece).
    PlacedFromHand { i: u32, prior_moved: bool },
    /// Revived from the dead pool: back to `Loc::Dead`.
    PlacedFromDead { i: u32, prior_moved: bool },
    /// Hack: restore the prior side (re-keying the occupancy masks).
    Flipped { i: u32, prior_side: Side },
    /// Atomic exchange: both pieces back to their prior squares.
    Swapped { a: u32, asq: u16, am: bool, b: u32, bsq: u16, bm: bool },
    /// Killed by a landing hazard on `sq`: back from the crater (HP was
    /// already 1 and is unchanged; the spent mine is its own record).
    HazardDeath { i: u32, sq: u16 },
    /// The ephemeral marker (en-passant square) changed: restore the
    /// prior value. Logged by the make-prologue clear and by the
    /// `SetEphemeral` producer (Stage 2 — the move-level `prior_ep`
    /// snapshot folded into the op log).
    Ephemeral { prior: u16 },
}

const FILL: OpUndo = OpUndo::Hp { i: 0, prior: 0 };
/// Inline capacity: every fast-path move (quiet 1, capture 2, promotion
/// capture 3, castle 2) fits without touching the heap; only long ability
/// scripts and hazard pileups spill.
const INLINE: usize = 4;

/// Per-move undo log: a small inline buffer with heap spill, so the
/// dominant `[Relocate(+CaptureAt(to))]` script never allocates.
pub struct OpLog {
    len: u8,
    inline: [OpUndo; INLINE],
    spill: Vec<OpUndo>,
}

impl OpLog {
    #[inline]
    pub fn new() -> Self {
        OpLog { len: 0, inline: [FILL; INLINE], spill: Vec::new() }
    }

    #[inline]
    pub fn push(&mut self, u: OpUndo) {
        let l = self.len as usize;
        if l < INLINE {
            self.inline[l] = u;
            self.len += 1;
        } else {
            self.spill.push(u);
        }
    }

    /// Records in reverse application order (spill first — it holds the
    /// latest ops — then the inline buffer).
    #[inline]
    pub fn rev_iter(&self) -> impl Iterator<Item = &OpUndo> {
        self.spill.iter().rev().chain(self.inline[..self.len as usize].iter().rev())
    }
}

impl Default for OpLog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Apply / revert. These are Position methods (they own the mutation
// clusters) but live here: ops.rs is the single home of every state
// mutation a move can make, and position.rs keeps only the bracket
// primitives (xor/lift/place/set_terrain) they are built from.
// ---------------------------------------------------------------------------

impl Position {
    /// Interpret one registry op against the move context. `inline` so
    /// call sites with a CONSTANT op (the unrolled row-wrapper loops)
    /// fold the match down to the single live arm.
    #[inline]
    pub(crate) fn apply_op(&mut self, g: &GameDef, op: &MicroOp, ctx: &OpCtx, log: &mut OpLog) {
        let sq = |r: SqRef| match r {
            SqRef::From => ctx.from,
            SqRef::To => ctx.to,
            SqRef::Aux => ctx.aux,
            SqRef::Mid => (ctx.from + ctx.to) / 2,
        };
        match *op {
            MicroOp::Relocate { who, dest, hazard } => {
                let dsq = sq(dest);
                if dsq == NO_SQ {
                    return; // e.g. the laser row without the retreat nerf
                }
                let i = self.board[sq(who) as usize] as usize;
                self.op_relocate(g, i, dsq, log);
                if hazard {
                    self.hazard_landing(g, i, log);
                }
            }
            MicroOp::CaptureAt { at } => {
                self.op_capture_at(g, sq(at), log);
            }
            MicroOp::PlaceFrom { pool, hazard } => {
                let i = match pool {
                    Pool::Dead => {
                        let ri = self
                            .pieces
                            .iter()
                            .position(|p| {
                                p.side == ctx.side && p.t == ctx.drop_type && p.loc == Loc::Dead
                            })
                            .expect("resurrect with no dead piece of that type");
                        self.op_place_from_dead(g, ri, ctx.to, log);
                        ri
                    }
                    Pool::Hand => {
                        let i = self
                            .pieces
                            .iter()
                            .position(|p| p.loc == Loc::Hand(ctx.side) && p.t == ctx.drop_type)
                            .expect("drop with empty hand");
                        self.op_place_from_hand(g, i, ctx.to, log);
                        i
                    }
                };
                if hazard {
                    self.hazard_landing(g, i, log);
                }
            }
            MicroOp::HpAdd { n, cap } => {
                let i = self.board[ctx.to as usize] as usize;
                let amt = match n {
                    HpAmt::Lit(v) => v,
                    HpAmt::FromMove => ctx.amount,
                };
                self.op_hp_add(g, i, amt, cap, log);
            }
            MicroOp::SetTerrain { kind } => {
                let t = match kind {
                    TerrainKind::Wall => T_WALL,
                    TerrainKind::Pit => T_PIT,
                    TerrainKind::OwnerMine => T_MINE0 + ctx.side,
                    TerrainKind::Code(c) => c,
                };
                self.op_set_terrain(g, ctx.to, t, log);
            }
            MicroOp::TerrainStep => self.op_terrain_step(g, ctx.to, log),
            MicroOp::FlipSide => self.op_flip_side(g, ctx.to, ctx.side, log),
            MicroOp::SwapWithCaster => {
                let a = self.board[ctx.from as usize] as usize;
                let b = self.board[ctx.to as usize] as usize;
                self.op_swap(g, a, b, log);
                // Both relocations are landings: hazards bite, caster first.
                self.hazard_landing(g, a, log);
                self.hazard_landing(g, b, log);
            }
            MicroOp::SetEphemeral { at } => self.op_set_ephemeral(g, sq(at), log),
            MicroOp::TransformType => {
                let i = self.board[ctx.to as usize] as usize;
                let pt = ctx.promo.expect("TransformType with no promotion type on the move");
                self.op_transform_type(g, i, pt, log);
            }
        }
    }

    /// Exactly invert one op record. Each revert reads the piece's
    /// CURRENT location, so reverse-order replay is robust to later ops
    /// (already reverted) having moved or killed the same piece.
    /// `inline(always)`: unmake's replay loop must absorb this whole
    /// match (an out-of-line call per record measurably stalls unmake).
    #[inline(always)]
    pub(crate) fn revert_op(&mut self, u: &OpUndo) {
        match *u {
            OpUndo::Moved { i, from, prior_moved } => {
                let i = i as usize;
                let Loc::Board(cur) = self.pieces[i].loc else {
                    unreachable!("relocated piece is off-board at revert")
                };
                self.lift(i, cur);
                self.place(i, from);
                self.pieces[i].moved = prior_moved;
            }
            OpUndo::Transformed { i, prior_t } => self.pieces[i as usize].t = prior_t,
            OpUndo::Captured { i, prior } => {
                let i = i as usize;
                if let Loc::Hand(s) = self.pieces[i].loc {
                    self.hands[s as usize][self.pieces[i].t as usize] -= 1;
                }
                self.pieces[i] = prior;
                let Loc::Board(sq) = prior.loc else { unreachable!("captured off board") };
                self.place(i, sq);
            }
            OpUndo::Hp { i, prior } => self.hp[i as usize] = prior,
            OpUndo::Terrain { sq, prior } => self.set_terrain(sq, prior),
            OpUndo::PlacedFromHand { i, prior_moved } => {
                let i = i as usize;
                let Loc::Board(sq) = self.pieces[i].loc else {
                    unreachable!("dropped piece is off-board at revert")
                };
                self.lift(i, sq);
                let s = self.pieces[i].side;
                self.pieces[i].loc = Loc::Hand(s);
                self.hands[s as usize][self.pieces[i].t as usize] += 1;
                self.pieces[i].moved = prior_moved;
            }
            OpUndo::PlacedFromDead { i, prior_moved } => {
                let i = i as usize;
                let Loc::Board(sq) = self.pieces[i].loc else {
                    unreachable!("revived piece is off-board at revert")
                };
                self.lift(i, sq);
                self.pieces[i].loc = Loc::Dead;
                self.pieces[i].moved = prior_moved;
            }
            OpUndo::Flipped { i, prior_side } => {
                let i = i as usize;
                let Loc::Board(sq) = self.pieces[i].loc else {
                    unreachable!("hacked piece is off-board at revert")
                };
                self.lift(i, sq);
                self.pieces[i].side = prior_side;
                self.place(i, sq);
            }
            OpUndo::Swapped { a, asq, am, b, bsq, bm } => {
                let (a, b) = (a as usize, b as usize);
                let Loc::Board(ca) = self.pieces[a].loc else { unreachable!() };
                let Loc::Board(cb) = self.pieces[b].loc else { unreachable!() };
                self.lift(a, ca);
                self.lift(b, cb);
                self.place(a, asq);
                self.place(b, bsq);
                self.pieces[a].moved = am;
                self.pieces[b].moved = bm;
            }
            OpUndo::HazardDeath { i, sq } => self.place(i as usize, sq),
            OpUndo::Ephemeral { prior } => self.ep = prior,
        }
    }

    // -- the ops themselves --------------------------------------------

    /// Relocate piece `i` from wherever it stands to `to`, marking it
    /// moved; `promo` fuses a type transform into the same hash bracket
    /// (the dominant-script fast path: one bracket per plain move).
    #[inline]
    pub(crate) fn op_relocate_promo(
        &mut self,
        g: &GameDef,
        i: usize,
        to: u16,
        promo: Option<TypeId>,
        log: &mut OpLog,
    ) {
        let Loc::Board(from) = self.pieces[i].loc else {
            unreachable!("relocating an off-board piece")
        };
        log.push(OpUndo::Moved { i: i as u32, from, prior_moved: self.pieces[i].moved });
        self.xor_piece(g, i);
        self.lift(i, from);
        self.place(i, to);
        self.pieces[i].moved = true;
        if let Some(pt) = promo {
            log.push(OpUndo::Transformed { i: i as u32, prior_t: self.pieces[i].t });
            self.pieces[i].t = pt;
        }
        self.xor_piece(g, i);
    }

    #[inline]
    pub(crate) fn op_relocate(&mut self, g: &GameDef, i: usize, to: u16, log: &mut OpLog) {
        self.op_relocate_promo(g, i, to, None, log);
    }

    /// Strike-or-kill the occupant of `sq` (§3.2). Returns whether the
    /// square was emptied (kill); a strike or an empty square is `false`.
    pub(crate) fn op_capture_at(&mut self, g: &GameDef, sq: u16, log: &mut OpLog) -> bool {
        let ci = self.board[sq as usize];
        if ci < 0 {
            return false;
        }
        let ci = ci as usize;
        if self.hp[ci] > 1 {
            // Armor strike (§3.2): decrement HP, victim stays.
            log.push(OpUndo::Hp { i: ci as u32, prior: self.hp[ci] });
            self.xor_piece(g, ci);
            self.hp[ci] -= 1;
            self.xor_piece(g, ci);
            return false;
        }
        let prior = self.pieces[ci];
        log.push(OpUndo::Captured { i: ci as u32, prior });
        self.xor_piece(g, ci);
        self.lift(ci, sq);
        match g.policy.capture_fate {
            crate::game::CaptureFate::Destroy => self.pieces[ci].loc = Loc::Dead,
            crate::game::CaptureFate::ToHand => {
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
        true
    }

    /// Enter from a hand at full HP for the type (shogi drop).
    pub(crate) fn op_place_from_hand(&mut self, g: &GameDef, i: usize, to: u16, log: &mut OpLog) {
        let (t, s) = (self.pieces[i].t, self.pieces[i].side);
        log.push(OpUndo::PlacedFromHand { i: i as u32, prior_moved: self.pieces[i].moved });
        self.xor_hand(g, s as usize, t as usize);
        self.hands[s as usize][t as usize] -= 1;
        self.xor_hand(g, s as usize, t as usize);
        // From hand (no key) to board (keyed): single trailing XOR.
        self.place(i, to);
        self.pieces[i].moved = true;
        if self.hp[i] != g.types[t as usize].max_hp {
            log.push(OpUndo::Hp { i: i as u32, prior: self.hp[i] });
            self.hp[i] = g.types[t as usize].max_hp;
        }
        self.xor_piece(g, i);
    }

    /// Revive from the dead pool at 1 HP (§7 resurrect).
    pub(crate) fn op_place_from_dead(&mut self, g: &GameDef, i: usize, to: u16, log: &mut OpLog) {
        log.push(OpUndo::PlacedFromDead { i: i as u32, prior_moved: self.pieces[i].moved });
        log.push(OpUndo::Hp { i: i as u32, prior: self.hp[i] });
        // Dead pieces are unkeyed: mutate fully, then one trailing XOR
        // keys the revived state.
        self.hp[i] = 1;
        self.place(i, to);
        self.pieces[i].moved = true;
        self.xor_piece(g, i);
    }

    /// Add HP (heal `+n` capped at type max, or raw damage like the
    /// overclock's self −1). Callers guarantee the result stays ≥ 1.
    pub(crate) fn op_hp_add(&mut self, g: &GameDef, i: usize, n: i16, cap: bool, log: &mut OpLog) {
        log.push(OpUndo::Hp { i: i as u32, prior: self.hp[i] });
        self.xor_piece(g, i);
        let mut v = self.hp[i] + n;
        if cap {
            v = v.min(g.types[self.pieces[i].t as usize].max_hp);
        }
        self.hp[i] = v;
        self.xor_piece(g, i);
    }

    /// Lay or clear terrain at `sq` inside a terrain hash bracket.
    pub(crate) fn op_set_terrain(&mut self, g: &GameDef, sq: u16, t: u8, log: &mut OpLog) {
        log.push(OpUndo::Terrain { sq, prior: self.terrain[sq as usize] });
        self.xor_terrain(g, sq);
        self.set_terrain(sq, t);
        self.xor_terrain(g, sq);
    }

    /// Block erosion: an unoccupied destructible block (registry
    /// `tiers > 0` — stdlib band or a Stage-4 custom row) drops one
    /// tier (tier 1 clears to bare floor); anything else is a no-op —
    /// a piece standing inside the block (a driller) shields it.
    pub(crate) fn op_terrain_step(&mut self, g: &GameDef, sq: u16, log: &mut OpLog) {
        let t = self.terrain[sq as usize];
        if self.board[sq as usize] >= 0 || !g.terrain_is_block(t) {
            return;
        }
        self.op_set_terrain(g, sq, g.terrain_erode(t), log);
    }

    /// Hack: flip the occupant of `sq` to `side`. Masks track side, so
    /// lift under the old side and re-place under the new; the hash
    /// brackets the whole flip.
    pub(crate) fn op_flip_side(&mut self, g: &GameDef, sq: u16, side: Side, log: &mut OpLog) {
        let ti = self.board[sq as usize] as usize;
        log.push(OpUndo::Flipped { i: ti as u32, prior_side: self.pieces[ti].side });
        self.xor_piece(g, ti);
        self.lift(ti, sq);
        self.pieces[ti].side = side;
        self.place(ti, sq);
        self.xor_piece(g, ti);
    }

    /// Atomic two-piece exchange: `a` and `b` trade squares, both marked
    /// moved, one undo record (a sequential decomposition would clobber
    /// the mailbox mid-transaction).
    pub(crate) fn op_swap(&mut self, g: &GameDef, a: usize, b: usize, log: &mut OpLog) {
        let Loc::Board(asq) = self.pieces[a].loc else { unreachable!() };
        let Loc::Board(bsq) = self.pieces[b].loc else { unreachable!() };
        log.push(OpUndo::Swapped {
            a: a as u32,
            asq,
            am: self.pieces[a].moved,
            b: b as u32,
            bsq,
            bm: self.pieces[b].moved,
        });
        self.xor_piece(g, a);
        self.xor_piece(g, b);
        self.lift(a, asq);
        self.lift(b, bsq);
        self.place(a, bsq);
        self.place(b, asq);
        self.pieces[a].moved = true;
        self.pieces[b].moved = true;
        self.xor_piece(g, a);
        self.xor_piece(g, b);
    }

    /// Make-prologue half of the ephemeral-marker discipline: an expiring
    /// marker (set exactly one ply ago) is cleared before the move's
    /// script runs. No-op when no marker is live — the dominant script's
    /// op log stays at its Stage-1 length on the hot path.
    #[inline]
    pub(crate) fn op_clear_ephemeral(&mut self, g: &GameDef, log: &mut OpLog) {
        if self.ep == NO_SQ {
            return;
        }
        log.push(OpUndo::Ephemeral { prior: self.ep });
        self.hash ^= g.zobrist.ep_key(self.ep);
        self.ep = NO_SQ;
    }

    /// Producer half (`SetEphemeral`): lay the one-ply marker at `sq`
    /// inside its own hash bracket. `ep_key(NO_SQ)` is 0, so the bracket
    /// pairs correctly whatever the prior state.
    #[inline]
    pub(crate) fn op_set_ephemeral(&mut self, g: &GameDef, sq: u16, log: &mut OpLog) {
        log.push(OpUndo::Ephemeral { prior: self.ep });
        self.hash ^= g.zobrist.ep_key(self.ep);
        self.ep = sq;
        self.hash ^= g.zobrist.ep_key(self.ep);
    }

    /// Standalone `TransformType`: re-type piece `i` inside its own hash
    /// bracket. The fast path prefers the fused `op_relocate_promo` (one
    /// bracket for relocate + transform — identical final hash).
    pub(crate) fn op_transform_type(&mut self, g: &GameDef, i: usize, t: TypeId, log: &mut OpLog) {
        log.push(OpUndo::Transformed { i: i as u32, prior_t: self.pieces[i].t });
        self.xor_piece(g, i);
        self.pieces[i].t = t;
        self.xor_piece(g, i);
    }

    /// Landing hazards (SRW Appendix B): the post-op hook, now a
    /// registry-driven interpreter over the terrain row's `on_land`
    /// script (Bits 2.0 Stage 3). The call sites — and their quirks:
    /// castle rooks and resurrected pieces do not trigger — are
    /// unchanged. The guard inlines to the derived range compares
    /// (`has_on_land`); the interpreted body stays cold and out of the
    /// dominant path.
    #[inline]
    pub(crate) fn hazard_landing(&mut self, g: &GameDef, mi: usize, log: &mut OpLog) {
        let Loc::Board(sq) = self.pieces[mi].loc else { return };
        let t = self.terrain[sq as usize];
        if terrain_defs::has_on_land(t) {
            self.land_hazard_interpret(g, mi, sq, t, log);
        } else if t >= terrain_defs::NT as u8 {
            // Stage-4 custom band: stdlib codes never take this branch,
            // so the added guard cost is one predictable compare.
            self.land_hazard_custom(g, mi, sq, t, log);
        }
    }

    /// The on-land interpreter: gate check → consumed handling → the
    /// row's ops against the lander, all through the shared op log. HP
    /// damage here carries the hazard floor-death rule (lethal at 0) —
    /// unlike the ability interpreter's generation-guarded `HpAdd`.
    #[cold]
    #[inline(never)]
    fn land_hazard_interpret(
        &mut self,
        g: &GameDef,
        mi: usize,
        sq: u16,
        t: u8,
        log: &mut OpLog,
    ) {
        let Some(land) = &terrain_defs::row(t).on_land else { return };
        match land.gate {
            LandGate::EnemyOfOwner => {
                // The banded owner's own pieces pass over unharmed (and
                // the hazard is NOT spent).
                if mine_owner(t) == Some(self.pieces[mi].side) {
                    return;
                }
            }
            LandGate::Anyone => {}
        }
        if land.consumed {
            self.op_set_terrain(g, sq, T_NONE, log);
        }
        for op in land.ops {
            match *op {
                MicroOp::HpAdd { n: HpAmt::Lit(v), .. } => self.on_land_hp(g, mi, sq, v, log),
                _ => unreachable!("on_land op outside the stdlib on_land vocabulary"),
            }
        }
    }

    /// One on-land HP tick against the lander: negative amounts carry
    /// the hazard floor-death rule (lethal at 0), positive amounts cap
    /// at the lander's type max (a custom healing spring cannot
    /// overfill; stdlib on-land amounts are all negative).
    fn on_land_hp(&mut self, g: &GameDef, mi: usize, sq: u16, v: i16, log: &mut OpLog) {
        if v >= 0 {
            self.op_hp_add(g, mi, v, true, log);
        } else if self.hp[mi] + v >= 1 {
            self.op_hp_add(g, mi, v, false, log);
        } else {
            self.xor_piece(g, mi);
            self.lift(mi, sq);
            self.pieces[mi].loc = Loc::Dead;
            log.push(OpUndo::HazardDeath { i: mi as u32, sq });
        }
    }

    /// The Stage-4 custom on-land interpreter: same shape as the stdlib
    /// interpreter — gate → consumed → ops through the op log — but the
    /// row lives on the GameDef and `EnemyOfOwner` resolves against the
    /// row's fixed authored owner (customs are not code-banded).
    #[cold]
    #[inline(never)]
    fn land_hazard_custom(&mut self, g: &GameDef, mi: usize, sq: u16, t: u8, log: &mut OpLog) {
        let Some(ct) = g.custom_terrain(t) else { return };
        let Some(land) = &ct.on_land else { return };
        match land.gate {
            LandGate::EnemyOfOwner => {
                if ct.owner == Some(self.pieces[mi].side) {
                    return;
                }
            }
            LandGate::Anyone => {}
        }
        if land.consumed {
            self.op_set_terrain(g, sq, T_NONE, log);
        }
        // The ops are cloned out of the row before interpreting: the
        // ops themselves take &mut self, and the validated vocabulary
        // (≤ 8 HpAdd literals) keeps the copy trivial.
        for i in 0..land.ops.len() {
            let op = g.custom_terrain(t).unwrap().on_land.as_ref().unwrap().ops[i];
            match op {
                MicroOp::HpAdd { n: HpAmt::Lit(v), .. } => {
                    // The lander may already have fallen to an earlier op.
                    if matches!(self.pieces[mi].loc, Loc::Board(_)) {
                        self.on_land_hp(g, mi, sq, v, log);
                    }
                }
                _ => unreachable!("custom on_land op outside the validated vocabulary"),
            }
        }
    }
}
