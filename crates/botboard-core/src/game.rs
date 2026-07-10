//! Game definition: board, zones, piece types, policy layer, and the
//! compile step from authoring Bits to runtime kernels (spec §5, §7.3).

use crate::bits::{AbilityBit, Geometry, Mode, MoveBit, PathRule, SpecialBit, TargetPred};
use crate::geometry::{symmetric_deltas, Delta};

pub type Side = u8;
pub type TypeId = u8;

/// Rectangular board shell. Arbitrary/graph boards are a later board class
/// behind the same occupancy abstraction (§7.1).
#[derive(Clone, Copy, Debug)]
pub struct BoardDef {
    pub w: u8,
    pub h: u8,
}

impl BoardDef {
    pub fn ncells(&self) -> usize {
        self.w as usize * self.h as usize
    }
    pub fn sq(&self, x: u8, y: u8) -> u16 {
        y as u16 * self.w as u16 + x as u16
    }
    pub fn xy(&self, sq: u16) -> (i16, i16) {
        ((sq % self.w as u16) as i16, (sq / self.w as u16) as i16)
    }
    /// Apply a delta with bounds check; `None` if off-board.
    pub fn offset(&self, sq: u16, dx: i8, dy: i8) -> Option<u16> {
        let (x, y) = self.xy(sq);
        let nx = x + dx as i16;
        let ny = y + dy as i16;
        if nx < 0 || ny < 0 || nx >= self.w as i16 || ny >= self.h as i16 {
            None
        } else {
            Some(ny as u16 * self.w as u16 + nx as u16)
        }
    }
}

/// A named board region, per side (palace, river halves, promotion ranks…).
/// Boards here are ≤ 128 cells, so one u128 mask per side suffices; larger
/// board classes swap the mask type behind this interface.
#[derive(Clone, Debug)]
pub struct Zone {
    masks: Vec<u128>,
}

impl Zone {
    pub fn new(sides: u8) -> Self {
        Zone { masks: vec![0; sides as usize] }
    }
    pub fn add(&mut self, side: Side, sq: u16) {
        self.masks[side as usize] |= 1u128 << sq;
    }
    pub fn add_rect(&mut self, side: Side, board: &BoardDef, x0: u8, y0: u8, x1: u8, y1: u8) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.add(side, board.sq(x, y));
            }
        }
    }
    pub fn contains(&self, side: Side, sq: u16) -> bool {
        (self.masks[side as usize] >> sq) & 1 == 1
    }
}

// ---------------------------------------------------------------------------
// Policy layer (§5): where composition stops.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StalematePolicy {
    /// Chess: no legal moves and not in check is a draw.
    Draw,
    /// Xiangqi/shogi: the stuck player loses.
    Loss,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureFate {
    Destroy,
    /// Shogi: captured piece banked to the capturer's hand as its base type.
    ToHand,
}

/// Turn policy (§3.4): one turn = one action, strict rotation. Multi-ply
/// turns are the calibration-excluded escape hatch and are not implemented
/// in Phase 0.
#[derive(Clone, Copy, Debug)]
pub enum TurnPolicy {
    Alternate,
}

/// Pass policy (§3.4): passing is illegal by default.
#[derive(Clone, Copy, Debug)]
pub enum PassPolicy {
    Forbidden,
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub stalemate: StalematePolicy,
    pub capture_fate: CaptureFate,
    pub turn: TurnPolicy,
    pub pass: PassPolicy,
}

// ---------------------------------------------------------------------------
// Piece types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromoTrigger {
    /// Chess: promotion triggers when the destination is in the zone.
    Dest,
    /// Shogi: triggers when origin or destination is in the zone.
    FromOrDest,
}

/// Transformation rule (§3.2). `optional` promotions still become forced when
/// the unpromoted piece could never act from its destination (shogi's
/// forced-if-immobile tier); that reachability is precomputed per square.
#[derive(Clone, Debug)]
pub struct PromoRule {
    pub zone: usize,
    pub choices: Vec<TypeId>,
    pub trigger: PromoTrigger,
    pub optional: bool,
}

#[derive(Clone, Debug)]
pub struct PieceTypeDef {
    pub name: &'static str,
    pub glyph: char,
    pub bits: Vec<MoveBit>,
    pub specials: Vec<SpecialBit>,
    pub abilities: Vec<AbilityBit>,
    pub royal: bool,
    pub promo: Option<PromoRule>,
    /// May be the rook half of a castling compound.
    pub castle_partner: bool,
    pub droppable: bool,
    /// Named legality predicate *nifu*: no drop where the file already holds
    /// an unpromoted piece of this type and side (§5).
    pub drop_no_dup_file: bool,
    /// Named legality predicate *uchifuzume*: no drop that delivers immediate
    /// checkmate (§5).
    pub drop_no_mate: bool,
    /// Hit-count armor (§3.2): captures strike for 1 HP; the piece falls at
    /// 0. HP lives in per-instance SoA state (§7.3), hashed via the state
    /// bucket (§7.5).
    pub max_hp: i16,
    /// Multi-action Bit (§3.4): compiles into ⟨move, move, self −1 HP⟩
    /// compound moves — piece-local, atomic, priced ×1.8 (§4.1).
    pub overclock: bool,
    /// Decoy (SRW Appendix B hologram): moves but cannot capture or give
    /// check; its bluff is carried by the identity-hidden belief substrate.
    pub hologram: bool,
    /// Concealed presence (SRW Appendix B stealth): hidden from opponents'
    /// search worlds until revealed — masking lives at the battle layer.
    pub stealth: bool,
}

impl PieceTypeDef {
    pub fn new(name: &'static str, glyph: char, bits: Vec<MoveBit>) -> Self {
        PieceTypeDef {
            name,
            glyph,
            bits,
            specials: Vec::new(),
            abilities: Vec::new(),
            royal: false,
            promo: None,
            castle_partner: false,
            droppable: false,
            drop_no_dup_file: false,
            drop_no_mate: false,
            max_hp: 1,
            overclock: false,
            hologram: false,
            stealth: false,
        }
    }
    pub fn specials(mut self, s: Vec<SpecialBit>) -> Self {
        self.specials = s;
        self
    }
    pub fn abilities(mut self, a: Vec<AbilityBit>) -> Self {
        self.abilities = a;
        self
    }
    pub fn hp(mut self, hp: i16) -> Self {
        self.max_hp = hp;
        self
    }
    pub fn overclock(mut self) -> Self {
        self.overclock = true;
        self
    }
    pub fn hologram(mut self) -> Self {
        self.hologram = true;
        self
    }
    pub fn stealth(mut self) -> Self {
        self.stealth = true;
        self
    }
    pub fn royal(mut self) -> Self {
        self.royal = true;
        self
    }
    pub fn promo(mut self, p: PromoRule) -> Self {
        self.promo = Some(p);
        self
    }
    pub fn castle_partner(mut self) -> Self {
        self.castle_partner = true;
        self
    }
    pub fn droppable(mut self) -> Self {
        self.droppable = true;
        self
    }
    pub fn nifu(mut self) -> Self {
        self.drop_no_dup_file = true;
        self
    }
    pub fn no_drop_mate(mut self) -> Self {
        self.drop_no_mate = true;
        self
    }
}

// ---------------------------------------------------------------------------
// Compiled runtime kernels (§7.3): Bits are the authoring format, not the
// runtime format.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LeapK {
    pub d: Delta,
    pub mode: Mode,
    /// Cells (relative to origin) that must be empty — lame leapers.
    pub blockers: Vec<Delta>,
    pub from_zone: Option<usize>,
    pub to_zone: Option<usize>,
    pub target: TargetPred,
}

#[derive(Clone, Debug)]
pub struct RideK {
    pub d: Delta,
    pub mode: Mode,
    pub from_zone: Option<usize>,
    pub to_zone: Option<usize>,
    pub target: TargetPred,
}

/// One-screen capture hopper (xiangqi cannon).
#[derive(Clone, Debug)]
pub struct HopK {
    pub d: Delta,
    pub from_zone: Option<usize>,
    pub to_zone: Option<usize>,
    pub target: TargetPred,
}

/// Wide-bitboard kernels (§7.1's bounded-mid-size class, portable u128
/// form): one register holds any board up to 128 cells, so chess (64),
/// shogi (81), and xiangqi (90) all ride the same fast path. Only *plain*
/// kernels (no zones, no target predicate) compile here; the rest stay on
/// the mailbox path — representation per kernel behind one abstraction.
#[derive(Clone, Debug)]
pub struct LeapBB {
    pub to: u16,
    /// Cells that must be empty (lame-leaper legs), as a mask.
    pub blockers: u128,
    pub mode: Mode,
}

#[derive(Clone, Debug)]
pub struct RideBB {
    pub mode: Mode,
    /// True when the per-step bit offset is positive (blocker = trailing
    /// one), false for negative (blocker = leading one).
    pub positive: bool,
    /// rays[sq] = cells reachable from sq along this delta, excluding sq.
    pub rays: Vec<u128>,
}

#[derive(Clone, Debug, Default)]
pub struct CompiledBB {
    /// leaps[sq] = precomputed landings from sq (plain kernels only).
    pub leaps: Vec<Vec<LeapBB>>,
    pub rides: Vec<RideBB>,
}

#[derive(Clone, Debug, Default)]
pub struct Compiled {
    pub leaps: Vec<LeapK>,
    pub rides: Vec<RideK>,
    pub hops: Vec<HopK>,
    /// Parallel to `leaps`/`rides`: kernel is covered by the bitboard path.
    pub leap_plain: Vec<bool>,
    pub ride_plain: Vec<bool>,
    pub bb: Option<CompiledBB>,
    /// Per square: could this type ever act from here on an empty board?
    /// Drives drop legality tier 2 and forced-if-immobile promotion.
    pub can_act_from: Vec<bool>,
}

// ---------------------------------------------------------------------------
// GameDef
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct GameDef {
    pub board: BoardDef,
    pub sides: u8,
    pub types: Vec<PieceTypeDef>,
    pub zones: Vec<Zone>,
    pub policy: Policy,
    /// Initial setup: (type, side, x, y).
    pub start: Vec<(TypeId, Side, u8, u8)>,
    compiled: Vec<Compiled>,
    /// Ground-truth hashing tables (§7.5), deterministic per game shape.
    pub zobrist: crate::zobrist::Zobrist,
    /// Dispatch to the wide-bitboard movegen path (auto for ≤128 cells;
    /// switchable for the Prototype-1 crossover measurement, §12).
    pub use_bitboards: bool,
}

impl GameDef {
    pub fn new(
        board: BoardDef,
        sides: u8,
        types: Vec<PieceTypeDef>,
        zones: Vec<Zone>,
        policy: Policy,
        start: Vec<(TypeId, Side, u8, u8)>,
    ) -> Self {
        let zobrist = crate::zobrist::Zobrist::for_shape(board.ncells(), types.len(), sides);
        // Prototype 1's measured answer (§7.1, §12): the tuned mailbox beats
        // the portable u128 bitboard class at every tested size (0.80–0.93x
        // for bitboards), confirming the expert position the spec cites.
        // Mailbox is therefore the default; the bitboard class stays
        // selectable (and perft-equivalence-tested) for re-measurement on
        // hardware with native wide-SIMD.
        let use_bitboards = false;
        let mut g = GameDef {
            board,
            sides,
            types,
            zones,
            policy,
            start,
            compiled: Vec::new(),
            zobrist,
            use_bitboards,
        };
        g.compile();
        g
    }

    pub fn compiled(&self, t: TypeId, s: Side) -> &Compiled {
        &self.compiled[t as usize * self.sides as usize + s as usize]
    }

    /// The compile step (§7.3): expand each Bit's symmetric deltas, apply the
    /// direction mask in the mover's frame, then flip `dy` into each side's
    /// frame; lame-leaper blockers are derived per flipped delta.
    fn compile(&mut self) {
        let mut all = Vec::with_capacity(self.types.len() * self.sides as usize);
        for t in 0..self.types.len() {
            for s in 0..self.sides {
                all.push(self.compile_one(t as TypeId, s));
            }
        }
        self.compiled = all;
    }

    fn compile_one(&self, t: TypeId, s: Side) -> Compiled {
        let mut ck = Compiled::default();
        for bit in &self.types[t as usize].bits {
            let (m, n) = match bit.geom {
                Geometry::Leaper(m, n) | Geometry::Rider(m, n) | Geometry::Hopper(m, n) => (m, n),
            };
            for d0 in symmetric_deltas(m, n) {
                if !bit.dirs.keep(d0) {
                    continue;
                }
                let d = if s == 0 { d0 } else { Delta::new(d0.dx, -d0.dy) };
                match bit.geom {
                    Geometry::Leaper(..) => {
                        let blockers = match bit.path {
                            PathRule::None => Vec::new(),
                            PathRule::Midpoint => vec![Delta::new(d.dx / 2, d.dy / 2)],
                            PathRule::Leg => {
                                if d.dx.abs() == 2 {
                                    vec![Delta::new(d.dx.signum(), 0)]
                                } else {
                                    vec![Delta::new(0, d.dy.signum())]
                                }
                            }
                        };
                        ck.leaps.push(LeapK {
                            d,
                            mode: bit.mode,
                            blockers,
                            from_zone: bit.from_zone,
                            to_zone: bit.to_zone,
                            target: bit.target,
                        });
                    }
                    Geometry::Rider(..) => ck.rides.push(RideK {
                        d,
                        mode: bit.mode,
                        from_zone: bit.from_zone,
                        to_zone: bit.to_zone,
                        target: bit.target,
                    }),
                    Geometry::Hopper(..) => ck.hops.push(HopK {
                        d,
                        from_zone: bit.from_zone,
                        to_zone: bit.to_zone,
                        target: bit.target,
                    }),
                }
            }
        }
        // Wide-bitboard tables for plain kernels (§7.1): no zones, no
        // target predicate — everything else stays on the mailbox path.
        let n = self.board.ncells();
        if n <= 128 {
            let plain_leap = |k: &LeapK| {
                k.from_zone.is_none() && k.to_zone.is_none() && k.target == TargetPred::Any
            };
            let plain_ride = |k: &RideK| {
                k.from_zone.is_none() && k.to_zone.is_none() && k.target == TargetPred::Any
            };
            ck.leap_plain = ck.leaps.iter().map(plain_leap).collect();
            ck.ride_plain = ck.rides.iter().map(plain_ride).collect();
            let mut bb = CompiledBB { leaps: vec![Vec::new(); n], rides: Vec::new() };
            for (ki, k) in ck.leaps.iter().enumerate() {
                if !ck.leap_plain[ki] {
                    continue;
                }
                for sq in 0..n as u16 {
                    let Some(to) = self.board.offset(sq, k.d.dx, k.d.dy) else { continue };
                    let mut blockers = 0u128;
                    let mut off_board = false;
                    for b in &k.blockers {
                        match self.board.offset(sq, b.dx, b.dy) {
                            Some(bsq) => blockers |= 1u128 << bsq,
                            None => off_board = true,
                        }
                    }
                    if !off_board {
                        bb.leaps[sq as usize].push(LeapBB { to, blockers, mode: k.mode });
                    }
                }
            }
            for (ki, k) in ck.rides.iter().enumerate() {
                if !ck.ride_plain[ki] {
                    continue;
                }
                let mut rays = vec![0u128; n];
                for sq in 0..n as u16 {
                    let mut cur = sq;
                    while let Some(nx) = self.board.offset(cur, k.d.dx, k.d.dy) {
                        rays[sq as usize] |= 1u128 << nx;
                        cur = nx;
                    }
                }
                let positive = (k.d.dy as i32 * self.board.w as i32 + k.d.dx as i32) > 0;
                bb.rides.push(RideBB { mode: k.mode, positive, rays });
            }
            ck.bb = Some(bb);
        } else {
            ck.leap_plain = vec![false; ck.leaps.len()];
            ck.ride_plain = vec![false; ck.rides.len()];
        }

        // Reachability on an empty board. Hoppers need a screen, so they
        // cannot contribute here.
        ck.can_act_from = (0..n as u16)
            .map(|sq| {
                let zone_ok = |z: Option<usize>, at: u16| {
                    z.map_or(true, |zi| self.zones[zi].contains(s, at))
                };
                ck.leaps.iter().any(|k| {
                    zone_ok(k.from_zone, sq)
                        && self
                            .board
                            .offset(sq, k.d.dx, k.d.dy)
                            .is_some_and(|to| zone_ok(k.to_zone, to))
                }) || ck.rides.iter().any(|k| {
                    zone_ok(k.from_zone, sq)
                        && self
                            .board
                            .offset(sq, k.d.dx, k.d.dy)
                            .is_some_and(|to| zone_ok(k.to_zone, to))
                })
            })
            .collect();
        ck
    }
}
