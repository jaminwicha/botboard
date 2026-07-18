//! Bits 2.0 Stage 3 — the terrain registry (docs/bits2-effects-as-data.md).
//!
//! Every shipped terrain kind is a data row over six properties: blocking
//! (per movement-permission class), an on-land hazard script over the
//! Stage-1 micro-op set, a carry rule (destination rewriting), a
//! concealment class (the SRW-layer masks), destructible tiers, and the
//! owner-band flag. The per-kind `match` arms in position/movegen/ops and
//! the FFI's hard-coded kind checks are all registry lookups now — the
//! rows are the single source of terrain behavior, bit-exact to the
//! retired constants.
//!
//! Hot-path discipline: the blocking predicates run inside kernel walks
//! and mask maintenance, so the row table is folded at compile time into
//! per-class u32 code-masks and, from those, into CONTIGUOUS CODE RANGES
//! (`ranges_of`). The range form compiles to the same two or three
//! register compares the pre-registry comparisons produced — measured:
//! every "cleverer" form (bit-mask probe, byte table, cold-path split)
//! costs 5–9% of chess perft against the compare chain, so the compare
//! chain IS the cached form, derived from the rows and exhaustively
//! const-asserted equal to them (edit a row and the build re-derives; a
//! row layout the range extractor cannot express fails the build).
//!
//! Internal codes vs wire codes: the `T_*` internal codes (the values in
//! `Position::terrain` and the Zobrist terrain keys) are frozen — mines
//! keep their owner-band encoding `T_MINE0 + side` (SRW Appendix B), so
//! four registry rows share the id "mine". The `code` field is the row's
//! stable FFI wire id (`srw_terrain` codes, where all four mine rows
//! surface as 3).

use crate::game::Side;
use crate::ops::{HpAmt, MicroOp};

// -- internal terrain codes (frozen; the registry row index) ---------------

pub const T_NONE: u8 = 0;
pub const T_WALL: u8 = 1;
pub const T_PIT: u8 = 2;
/// Owner-tagged mines (SRW Appendix B): T_MINE0 + side, codes 3..=6.
pub const T_MINE0: u8 = 3;
pub const T_ICE: u8 = 7;
pub const T_GRASS: u8 = 8;
pub const T_ACID: u8 = 9;
pub const T_BLOCK1: u8 = 10;
pub const T_BLOCK2: u8 = 11;
pub const T_BLOCK3: u8 = 12;
/// Conveyors: N is +y (side 0's forward), E is +x, S is −y, W is −x.
pub const T_CONV_N: u8 = 13;
pub const T_CONV_E: u8 = 14;
pub const T_CONV_S: u8 = 15;
pub const T_CONV_W: u8 = 16;

/// Number of internal terrain codes (rows). Also sizes the Zobrist
/// terrain key table.
pub const NT: usize = 17;

// -- the row vocabulary ----------------------------------------------------

/// Per-permission-class blocking (§3.1 path interaction): does this
/// terrain stop entry, rays, screens, and lame-leaper legs for a piece
/// moving in that class? A piece passes if ANY class it holds passes
/// (every piece is ground; flight and drill are additive permissions).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Blocks {
    pub ground: bool,
    pub flight: bool,
    pub drill: bool,
}

/// Who a landing-hazard script bites.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LandGate {
    /// Every lander (acid: the pool has no allegiance).
    Anyone,
    /// Only landers hostile to the terrain's banded owner (mines).
    EnemyOfOwner,
}

/// The on-land hazard script: ops applied to the LANDING piece when it
/// comes to rest on this terrain (via the post-op hazard hook — the call
/// sites and their quirks are unchanged: castle rooks and resurrected
/// pieces do not land "actively" and never trigger).
#[derive(Clone, Copy, Debug)]
pub struct OnLand {
    /// Micro-ops against the lander. On-land HP damage is lethal at 0
    /// (the hazard floor-death), unlike the ability interpreter's
    /// generation-guarded `HpAdd`.
    pub ops: &'static [MicroOp],
    pub gate: LandGate,
    /// The hazard is spent on trigger (cleared before its ops run).
    pub consumed: bool,
}

/// Carry: quiet landings on this terrain are rewritten at GENERATION time
/// (make/unmake and hashing never see carry terrain move anyone).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Carry {
    None,
    /// Ice: keep sliding along the entry vector until a blocker or the
    /// far edge of the sheet. `riders_only`: only a piece RIDING in
    /// exactly the entry direction skids (leapers land normally) — the
    /// Appendix B rule, carried as row data.
    Slide { riders_only: bool },
    /// Conveyor: carried one square per belt in the belt's fixed
    /// direction (chains, bounded). Belts carry everyone — no rider
    /// restriction.
    Belt { dx: i8, dy: i8 },
}

/// Concealment class — drives the SRW battle-layer presence masks; the
/// engine only stores the tile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Conceal {
    None,
    /// Tall grass: a piece STANDING here is hidden from observers with
    /// nobody adjacent (positional, memoryless).
    Standing,
    /// The tile itself is secret to everyone but its banded owner until
    /// it triggers (mines).
    OwnerSecret,
}

/// One terrain kind as data (design doc `TerrainDef`).
#[derive(Clone, Copy, Debug)]
pub struct TerrainDef {
    pub id: &'static str,
    /// Stable FFI wire code (`srw_terrain`); all owner-band rows share
    /// their kind's one code.
    pub code: u8,
    pub blocks: Blocks,
    pub on_land: Option<OnLand>,
    pub carry: Carry,
    pub conceal: Conceal,
    /// Destructible-block tier (0 = not destructible). Erosion steps
    /// down the contiguous block band; tier 1 clears to floor.
    pub tiers: u8,
    /// This row lives in an owner code band (internal code = band base +
    /// side); its `LandGate::EnemyOfOwner` resolves against that owner.
    pub owner_banded: bool,
}

// -- the stdlib rows -------------------------------------------------------

const OPEN: Blocks = Blocks { ground: false, flight: false, drill: false };
/// Wall-like: solid to ground and flight; a driller chews through.
const SOLID: Blocks = Blocks { ground: true, flight: true, drill: false };
/// Pit-like: impassable on the ground, hovered over by flight; a
/// driller still falls in unless it also flies.
const CHASM: Blocks = Blocks { ground: true, flight: false, drill: true };

/// The hazard bite: 1 HP off the lander, lethal at 1 (on-land HP damage
/// carries the floor-death rule).
const BITE: &[MicroOp] = &[MicroOp::HpAdd { n: HpAmt::Lit(-1), cap: false }];

/// Bare floor — the absence row (row 0 of the registry).
const FLOOR: TerrainDef = TerrainDef {
    id: "none",
    code: 0,
    blocks: OPEN,
    on_land: None,
    carry: Carry::None,
    conceal: Conceal::None,
    tiers: 0,
    owner_banded: false,
};

/// Owner-banded mine: passable, secret to non-owners, spent when an
/// enemy of the owner lands (1 HP, lethal at 1).
const MINE: TerrainDef = TerrainDef {
    id: "mine",
    code: 3,
    on_land: Some(OnLand { ops: BITE, gate: LandGate::EnemyOfOwner, consumed: true }),
    conceal: Conceal::OwnerSecret,
    owner_banded: true,
    ..FLOOR
};

const fn block(id: &'static str, code: u8, tiers: u8) -> TerrainDef {
    TerrainDef { id, code, blocks: SOLID, tiers, ..FLOOR }
}

const fn conveyor(id: &'static str, code: u8, dx: i8, dy: i8) -> TerrainDef {
    TerrainDef { id, code, carry: Carry::Belt { dx, dy }, ..FLOOR }
}

/// The registry: one row per internal terrain code, bit-exact to the
/// retired per-kind constants. `const` so the class masks and ranges
/// below fold at compile time; runtime lookups go through the [`ROWS`]
/// static (one materialization).
const ROWS_C: [TerrainDef; NT] = [
    FLOOR,
    TerrainDef { id: "wall", code: 1, blocks: SOLID, ..FLOOR },
    TerrainDef { id: "pit", code: 2, blocks: CHASM, ..FLOOR },
    MINE, // T_MINE0 + 0
    MINE, // T_MINE0 + 1
    MINE, // T_MINE0 + 2
    MINE, // T_MINE0 + 3
    TerrainDef { id: "ice", code: 4, carry: Carry::Slide { riders_only: true }, ..FLOOR },
    TerrainDef { id: "grass", code: 5, conceal: Conceal::Standing, ..FLOOR },
    TerrainDef {
        id: "acid",
        code: 6,
        on_land: Some(OnLand { ops: BITE, gate: LandGate::Anyone, consumed: false }),
        ..FLOOR
    },
    block("block1", 7, 1),
    block("block2", 8, 2),
    block("block3", 9, 3),
    conveyor("conv_n", 10, 0, 1),
    conveyor("conv_e", 11, 1, 0),
    conveyor("conv_s", 12, 0, -1),
    conveyor("conv_w", 13, -1, 0),
];

/// The runtime registry table (the `const` twin feeds compile-time
/// derivation; this static is the one runtime materialization).
pub static ROWS: [TerrainDef; NT] = ROWS_C;

/// The registry row for an internal terrain code.
#[inline]
pub fn row(t: u8) -> &'static TerrainDef {
    &ROWS[t as usize]
}

/// Look up an authorable row by id (setup-JSON terrain kinds). The
/// absence row and the owner-band rows are not authorable this stage
/// (mines are laid by the ability, terrain 0 by omission), preserving
/// the FFI parser's exact acceptance set.
pub fn by_id(id: &str) -> Option<u8> {
    ROWS.iter()
        .position(|r| r.code != 0 && !r.owner_banded && r.id == id)
        .map(|i| i as u8)
}

// -- compile-time per-class code masks and ranges (the hot-path form) ------
//
// One bit per internal code, folded from the rows; each mask is then
// re-expressed as up to `NR` contiguous code ranges. The range form is
// what the predicates use: it compiles to the same register-compare
// chains the pre-registry hard-coded comparisons produced (measured
// fastest — see the module header), while staying entirely row-derived.

const fn mask_ground() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if ROWS_C[i].blocks.ground {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

const fn mask_flight() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if ROWS_C[i].blocks.flight {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

const fn mask_drill() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if ROWS_C[i].blocks.drill {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

const fn mask_on_land() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if ROWS_C[i].on_land.is_some() {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

const fn mask_slide() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if matches!(ROWS_C[i].carry, Carry::Slide { .. }) {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

const fn mask_belt() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if matches!(ROWS_C[i].carry, Carry::Belt { .. }) {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

const fn mask_tiers() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if ROWS_C[i].tiers > 0 {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

const fn mask_owner_band() -> u32 {
    let mut m = 0u32;
    let mut i = 0;
    while i < NT {
        if ROWS_C[i].owner_banded {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}

/// Codes blocking the ground class (== `terrain_blocks`).
pub const STOP_GROUND: u32 = mask_ground();
/// Codes blocking the flight class.
pub const STOP_FLIGHT: u32 = mask_flight();
/// Codes blocking the drill class.
pub const STOP_DRILL: u32 = mask_drill();
/// Ground-blocking codes a flyer is EXEMPT from (pits) — feeds the
/// wide-bitboard fast-path gate (`Position::has_pit`).
pub const FLIGHT_EXEMPT: u32 = STOP_GROUND & !STOP_FLIGHT;
/// Ground-blocking codes a driller is EXEMPT from (walls + blocks) —
/// feeds `Position::has_wall`.
pub const DRILL_EXEMPT: u32 = STOP_GROUND & !STOP_DRILL;
/// Ground-blocking codes NO stdlib permission is exempt from.
pub const STOP_ALWAYS: u32 = STOP_GROUND & STOP_FLIGHT & STOP_DRILL;
/// Ground-blocking codes BOTH permissions are exempt from.
pub const EXEMPT_BOTH: u32 = FLIGHT_EXEMPT & DRILL_EXEMPT;
/// Codes with an on-land hazard script (the hook's fast test).
pub const ON_LAND: u32 = mask_on_land();
/// Codes with a Slide carry row (ice).
pub const SLIDE: u32 = mask_slide();
/// Codes with a Belt carry row (conveyors).
pub const BELT: u32 = mask_belt();
/// Destructible codes (tiers > 0).
pub const DESTRUCTIBLE: u32 = mask_tiers();
/// Owner-band codes (mines).
pub const OWNER_BAND: u32 = mask_owner_band();

/// Maximum contiguous ranges a class mask may occupy. A future row
/// layout that fragments a mask past this fails the build (`ranges_of`
/// const-panics) — widen NR and the predicates keep working.
const NR: usize = 3;

/// Never-matching filler range (lo > hi): folds away at compile time.
const NO_RANGE: (u8, u8) = (1, 0);

/// Extract a mask's contiguous code ranges as inclusive (lo, hi) pairs.
const fn ranges_of(mask: u32) -> [(u8, u8); NR] {
    let mut out = [NO_RANGE; NR];
    let mut n = 0;
    let mut i = 0u32;
    while i < 32 {
        if mask & (1 << i) != 0 {
            let lo = i;
            while i < 32 && mask & (1 << i) != 0 {
                i += 1;
            }
            assert!(n < NR, "class mask fragments past NR contiguous ranges");
            out[n] = (lo as u8, (i - 1) as u8);
            n += 1;
        } else {
            i += 1;
        }
    }
    out
}

const STOP_GROUND_R: [(u8, u8); NR] = ranges_of(STOP_GROUND);
const STOP_ALWAYS_R: [(u8, u8); NR] = ranges_of(STOP_ALWAYS);
const FLIGHT_EXEMPT_R: [(u8, u8); NR] = ranges_of(FLIGHT_EXEMPT & !EXEMPT_BOTH);
const DRILL_EXEMPT_R: [(u8, u8); NR] = ranges_of(DRILL_EXEMPT & !EXEMPT_BOTH);
const EXEMPT_BOTH_R: [(u8, u8); NR] = ranges_of(EXEMPT_BOTH);
const ON_LAND_R: [(u8, u8); NR] = ranges_of(ON_LAND);
const SLIDE_R: [(u8, u8); NR] = ranges_of(SLIDE);
const BELT_R: [(u8, u8); NR] = ranges_of(BELT);
const DESTRUCTIBLE_R: [(u8, u8); NR] = ranges_of(DESTRUCTIBLE);
const OWNER_BAND_R: [(u8, u8); NR] = ranges_of(OWNER_BAND);

/// Range-membership test: with the ranges compile-time constant this
/// folds to a two- or three-compare chain (unused filler ranges fold to
/// `false`), reproducing the pre-registry predicates' codegen exactly.
#[inline]
const fn in_ranges(r: &[(u8, u8); NR], t: u8) -> bool {
    (t >= r[0].0 && t <= r[0].1)
        || (t >= r[1].0 && t <= r[1].1)
        || (t >= r[2].0 && t <= r[2].1)
}

// -- the registry-backed predicates ---------------------------------------

/// Terrain that blocks entry, rays, screens, and legs for the bare
/// ground class (mines and the other passables do not).
#[inline]
pub const fn terrain_blocks(t: u8) -> bool {
    in_ranges(&STOP_GROUND_R, t)
}

/// `terrain_blocks` under the terrain permissions (§3.1): flight ignores
/// pits; drill ignores walls and destructible blocks. Pits still stop a
/// driller unless it also flies. Decomposed over the derived exemption
/// classes so the common open cell short-circuits on `t` alone — with
/// the stdlib rows the always/both classes are empty and fold away,
/// leaving the exact pre-registry compare chain.
#[inline]
pub const fn terrain_stops(t: u8, flies: bool, drills: bool) -> bool {
    in_ranges(&STOP_ALWAYS_R, t)
        || (in_ranges(&DRILL_EXEMPT_R, t) && !drills)
        || (in_ranges(&FLIGHT_EXEMPT_R, t) && !flies)
        || (in_ranges(&EXEMPT_BOTH_R, t) && !flies && !drills)
}

/// Is this a destructible block tier (`tiers > 0`)?
#[inline]
pub const fn is_block(t: u8) -> bool {
    in_ranges(&DESTRUCTIBLE_R, t)
}

/// One erosion step down the block band: tier 1 clears to floor, higher
/// tiers step to the next code down (the band is code-contiguous by
/// construction — debug-asserted here).
#[inline]
pub fn erode(t: u8) -> u8 {
    debug_assert!(is_block(t));
    if row(t).tiers <= 1 {
        T_NONE
    } else {
        debug_assert_eq!(row(t - 1).tiers + 1, row(t).tiers, "block band must be contiguous");
        t - 1
    }
}

/// The belt direction, if this terrain carries `Belt`.
#[inline]
pub fn conv_dir(t: u8) -> Option<(i8, i8)> {
    match row(t).carry {
        Carry::Belt { dx, dy } => Some((dx, dy)),
        _ => None,
    }
}

/// Does this terrain carry `Slide` (the ice pass's sheet test)?
#[inline]
pub const fn slides(t: u8) -> bool {
    in_ranges(&SLIDE_R, t)
}

/// Does this terrain carry `Belt` (the belt pass's board scan)?
#[inline]
pub const fn belts(t: u8) -> bool {
    in_ranges(&BELT_R, t)
}

/// Does this terrain have an on-land hazard script? The hazard hook's
/// per-landing fast test.
#[inline]
pub const fn has_on_land(t: u8) -> bool {
    in_ranges(&ON_LAND_R, t)
}

/// Is this code in the flight-exempt blocking class (`has_pit`
/// count maintenance)?
#[inline]
pub const fn flight_exempt(t: u8) -> bool {
    in_ranges(&FLIGHT_EXEMPT_R, t) || in_ranges(&EXEMPT_BOTH_R, t)
}

/// Is this code in the drill-exempt blocking class (`has_wall`
/// count maintenance)?
#[inline]
pub const fn drill_exempt(t: u8) -> bool {
    in_ranges(&DRILL_EXEMPT_R, t) || in_ranges(&EXEMPT_BOTH_R, t)
}

/// Bare floor (the absence row) — ability targeting's "no stacking
/// terrain on terrain" check.
#[inline]
pub const fn is_bare(t: u8) -> bool {
    t == T_NONE
}

/// The banded owner, if this code sits in an owner band (mines occupy
/// `T_MINE0 + side`, codes 3..=6; the band base is derived from the
/// owner-band range).
#[inline]
pub fn mine_owner(t: u8) -> Option<Side> {
    if in_ranges(&OWNER_BAND_R, t) {
        Some((t - OWNER_BAND_R[0].0) as Side)
    } else {
        None
    }
}

// -- compile-time equivalence proof ----------------------------------------
//
// The range-form predicates are exhaustively checked against the rows
// for every representable code and permission combo: a row edit that the
// derivation no longer captures is a build error, never a silent drift.

const _: () = {
    let mut t = 0usize;
    while t < 256 {
        let b = if t < NT { ROWS_C[t].blocks } else { OPEN };
        assert!(terrain_blocks(t as u8) == b.ground);
        let mut c = 0;
        while c < 4 {
            let flies = c & 1 != 0;
            let drills = c & 2 != 0;
            // A piece stops iff every permission class it holds is blocked.
            let want = b.ground && (!flies || b.flight) && (!drills || b.drill);
            assert!(terrain_stops(t as u8, flies, drills) == want);
            c += 1;
        }
        if t < NT {
            assert!(is_block(t as u8) == (ROWS_C[t].tiers > 0));
            assert!(slides(t as u8) == matches!(ROWS_C[t].carry, Carry::Slide { .. }));
            assert!(belts(t as u8) == matches!(ROWS_C[t].carry, Carry::Belt { .. }));
            assert!(has_on_land(t as u8) == ROWS_C[t].on_land.is_some());
            assert!(flight_exempt(t as u8) == (b.ground && !b.flight));
            assert!(drill_exempt(t as u8) == (b.ground && !b.drill));
        }
        t += 1;
    }
};
