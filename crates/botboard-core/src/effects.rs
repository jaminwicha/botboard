//! Bits 2.0 Stage 1 — the ability registry (docs/bits2-effects-as-data.md).
//!
//! Every shipped Axis-B ability is a data row over the closed micro-op
//! set: `make_impl`'s ability arm is ONE interpreter loop over the row's
//! `ops` then `self_ops` — no per-kind match survives in make/unmake.
//!
//! Scope this stage:
//! - `AbilityBit` stays the AUTHORING surface (ranges, amounts, pierce,
//!   retreat) and each bit compiles to a registry row id via
//!   [`ability_row`]; FFI/JSON/authoring are unchanged.
//! - `Effect` on a generated move is the wire id of the row that applies
//!   it ([`row`] maps it 1:1); per-move parameters ride on the move
//!   (heal amount in `Effect::Heal(n)`, retreat square in `aux`,
//!   revived type in `drop_type`).
//! - `Selector` rows document each ability's target shape; movegen's
//!   per-kind ability arms remain hand-shaped but share the Stage-2 gate
//!   vocabulary's predicates where they overlap (HP bands via
//!   `hp_gate_ok`); move SPECIALS generate from `move_defs` script rows.
//! - `CostHint`/`descriptor_slot` mirror the numbers cost.rs and nnue.rs
//!   use today; those modules stay authoritative until Stage 2 reads the
//!   registry instead.

use crate::bits::AbilityBit;
use crate::moves::Effect;
use crate::ops::{HpAmt, MicroOp, Pool, SqRef, TerrainKind};

/// Who an ability may target (design doc `Selector.who`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    Friendly,
    Enemy,
    /// An empty, bare-floor cell (terrain construction).
    EmptyCell,
    /// An enemy piece OR an unoccupied destructible-block cell — the
    /// laser's beam damages cover when no robot blocks the shot.
    EnemyOrBlockCell,
    /// A dead friendly type (resurrect); the landing cell is `to`.
    DeadFriendly,
}

/// Target reach shape. Ranges/pierce are authoring parameters on
/// `AbilityBit` this stage; the row records only the shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reach {
    /// Chebyshev ball around the caster.
    Point,
    /// Queen-line ray from the caster (the laser; `pierce` on the bit).
    Ray,
}

/// Generation-time target predicates (the "teeth" each per-kind movegen
/// arm enforces today; Stage 2 walks these instead).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pred {
    /// Target HP below its type max (heal).
    Damaged,
    /// Target HP exactly 1 — defenses down (hack).
    HpEq1,
    /// Royals excluded (hack; resurrect: royals stay dead).
    NonRoyal,
    /// Bare floor: no terrain, no piece, not the caster's square.
    BareCell,
    /// Landing square empty and terrain-open (resurrect, push dest,
    /// swap's two landings under each piece's terrain permissions).
    OpenLanding,
    /// Retreat coupling: the whole compound is illegal when the retreat
    /// square is blocked (laser's `retreat` nerf has teeth).
    RetreatFree,
}

/// Cost-model hint (flat prior + utility multiplier), mirroring cost.rs
/// (§4.1/§4.3 flat priors; laser pierce ×1.3 and retreat −0.5 stay
/// conditional on the authoring bit's parameters there).
#[derive(Clone, Copy, Debug)]
pub struct CostHint {
    pub flat: f64,
    pub mult: f64,
}

/// Target selector row (design doc `Selector`).
#[derive(Clone, Copy, Debug)]
pub struct Selector {
    pub who: Who,
    pub reach: Reach,
    pub preds: &'static [Pred],
}

/// "No NNUE descriptor dim": the 21-dim v2 layout is frozen; swap/push
/// flow through flat priors only (nnue.rs documents the same).
pub const NO_SLOT: u8 = u8::MAX;

/// One ability as data: id, target selector, op script, self-op script
/// (the caster's coupled effects, e.g. forced retreat), cost hint, and
/// the NNUE descriptor dim it feeds.
#[derive(Clone, Copy, Debug)]
pub struct AbilityDef {
    pub id: &'static str,
    pub selector: Selector,
    /// Applied atomically, in order, against the target.
    pub ops: &'static [MicroOp],
    /// Applied after `ops`, against the caster.
    pub self_ops: &'static [MicroOp],
    pub cost: CostHint,
    pub descriptor_slot: u8,
}

// ---------------------------------------------------------------------------
// The stdlib: the nine shipped abilities as registry rows, bit-exact to
// the retired per-kind arms.
// ---------------------------------------------------------------------------

pub static HEAL: AbilityDef = AbilityDef {
    id: "heal",
    selector: Selector { who: Who::Friendly, reach: Reach::Point, preds: &[Pred::Damaged] },
    ops: &[MicroOp::HpAdd { n: HpAmt::FromMove, cap: true }],
    self_ops: &[],
    cost: CostHint { flat: 0.0, mult: 1.0 },
    descriptor_slot: 10,
};

pub static WALL: AbilityDef = AbilityDef {
    id: "wall",
    selector: Selector { who: Who::EmptyCell, reach: Reach::Point, preds: &[Pred::BareCell] },
    ops: &[MicroOp::SetTerrain { kind: TerrainKind::Wall }],
    self_ops: &[],
    cost: CostHint { flat: 0.0, mult: 1.0 },
    descriptor_slot: 12,
};

pub static PIT: AbilityDef = AbilityDef {
    id: "pit",
    selector: Selector { who: Who::EmptyCell, reach: Reach::Point, preds: &[Pred::BareCell] },
    ops: &[MicroOp::SetTerrain { kind: TerrainKind::Pit }],
    self_ops: &[],
    cost: CostHint { flat: 0.0, mult: 1.0 },
    descriptor_slot: 12,
};

/// Owner-tagged mine (SRW Appendix B): laid on bare floor, spent when an
/// enemy lands (the hazard hook), concealment at the battle layer.
pub static MINE: AbilityDef = AbilityDef {
    id: "mine",
    selector: Selector { who: Who::EmptyCell, reach: Reach::Point, preds: &[Pred::BareCell] },
    ops: &[MicroOp::SetTerrain { kind: TerrainKind::OwnerMine }],
    self_ops: &[],
    cost: CostHint { flat: 2.0, mult: 1.0 },
    descriptor_slot: 13,
};

/// Capture-at-range without vacating the origin. `TerrainStep` before
/// `CaptureAt` sequences the beam-vs-cover rule without a conditional:
/// an unoccupied destructible block erodes one tier (and `CaptureAt`
/// then no-ops on the empty cell); any occupant shields the block and
/// takes the strike-or-kill instead. The self-op is the coupled retreat
/// (`aux`; no-op when the row was generated without the nerf).
pub static LASER: AbilityDef = AbilityDef {
    id: "laser",
    selector: Selector {
        who: Who::EnemyOrBlockCell,
        reach: Reach::Ray,
        preds: &[Pred::RetreatFree],
    },
    ops: &[MicroOp::TerrainStep, MicroOp::CaptureAt { at: SqRef::To }],
    self_ops: &[MicroOp::Relocate { who: SqRef::From, dest: SqRef::Aux, hazard: true }],
    cost: CostHint { flat: 0.0, mult: 1.0 },
    descriptor_slot: 11,
};

/// Revive the lowest-index dead friendly of the move's named type at
/// 1 HP on an open cell. Royals never resurrect — their loss decided
/// something (generation excludes them; the selector documents it).
pub static REZ: AbilityDef = AbilityDef {
    id: "rez",
    selector: Selector {
        who: Who::DeadFriendly,
        reach: Reach::Point,
        preds: &[Pred::NonRoyal, Pred::OpenLanding],
    },
    ops: &[MicroOp::PlaceFrom { pool: Pool::Dead, hazard: false }],
    self_ops: &[],
    cost: CostHint { flat: 4.0, mult: 1.0 },
    descriptor_slot: 14,
};

/// Flip a non-royal enemy at exactly 1 HP (defenses down) to the
/// caster's side — deterministic, so search stays sound.
pub static HACK: AbilityDef = AbilityDef {
    id: "hack",
    selector: Selector {
        who: Who::Enemy,
        reach: Reach::Point,
        preds: &[Pred::HpEq1, Pred::NonRoyal],
    },
    ops: &[MicroOp::FlipSide],
    self_ops: &[],
    cost: CostHint { flat: 5.0, mult: 1.0 },
    descriptor_slot: 15,
};

/// Atomic exchange with a friendly in range; both landings must be
/// terrain-open for the piece arriving there; hazards bite both.
pub static SWAP: AbilityDef = AbilityDef {
    id: "swap",
    selector: Selector { who: Who::Friendly, reach: Reach::Point, preds: &[Pred::OpenLanding] },
    ops: &[MicroOp::SwapWithCaster],
    self_ops: &[],
    cost: CostHint { flat: 1.5, mult: 1.0 },
    descriptor_slot: NO_SLOT,
};

/// Shove an in-range enemy one square directly away from the caster
/// (the away-vector rides on `aux`); generated only when that square is
/// open for the pushed piece and empty. Landing hazards bite the victim.
pub static PUSH: AbilityDef = AbilityDef {
    id: "push",
    selector: Selector { who: Who::Enemy, reach: Reach::Point, preds: &[Pred::OpenLanding] },
    ops: &[MicroOp::Relocate { who: SqRef::To, dest: SqRef::Aux, hazard: true }],
    self_ops: &[],
    cost: CostHint { flat: 2.0, mult: 1.0 },
    descriptor_slot: NO_SLOT,
};

/// Reveal an enemy in range (SRW §7/§10): the ops are EMPTY — the board
/// effect is nothing, the turn is the price. The information effect
/// (belief collapse for the caster's side) is applied by the
/// imperfect-info layer keying on `Effect::Spy` after the move applies;
/// the core stays belief-free.
pub static SPY: AbilityDef = AbilityDef {
    id: "spy",
    selector: Selector { who: Who::Enemy, reach: Reach::Point, preds: &[] },
    ops: &[],
    self_ops: &[],
    cost: CostHint { flat: 0.5, mult: 1.0 },
    descriptor_slot: NO_SLOT,
};

/// The empty row: `Effect::None`/`Effect::Twice` never reach the ability
/// interpreter (Twice applies as `MoveKind::Compound`), but the mapping
/// stays total.
pub static EMPTY: AbilityDef = AbilityDef {
    id: "none",
    selector: Selector { who: Who::EmptyCell, reach: Reach::Point, preds: &[] },
    ops: &[],
    self_ops: &[],
    cost: CostHint { flat: 0.0, mult: 1.0 },
    descriptor_slot: NO_SLOT,
};

/// Registry lookup for a move's effect id: which STDLIB row applies it.
/// (`Effect` keeps its enum shape as the stable wire/notation surface;
/// its variants ARE the stdlib ids.) `Effect::Custom` never reaches this
/// lookup — custom rows are GameDef-owned (`GameDef::custom_effects`)
/// and every consumer branches to them first.
#[inline]
pub fn row(e: Effect) -> &'static AbilityDef {
    match e {
        Effect::Heal(_) => &HEAL,
        Effect::Wall => &WALL,
        Effect::Pit => &PIT,
        Effect::Laser => &LASER,
        Effect::Resurrect => &REZ,
        Effect::Hack => &HACK,
        Effect::Mine => &MINE,
        Effect::Swap => &SWAP,
        Effect::Push => &PUSH,
        Effect::Spy => &SPY,
        Effect::None | Effect::Twice => &EMPTY,
        Effect::Custom(_) => unreachable!("custom effects resolve via GameDef::custom_effects"),
    }
}

/// Compile step for the authoring surface: which registry row an
/// `AbilityBit` generates moves for (ranges/amounts/pierce/retreat stay
/// parameters of the bit; the row is the application script). Custom
/// bits have no static row — their GameDef row IS the definition.
pub fn ability_row(a: &AbilityBit) -> &'static AbilityDef {
    match a {
        AbilityBit::Heal { .. } => &HEAL,
        AbilityBit::CreateWall { .. } => &WALL,
        AbilityBit::DigPit { .. } => &PIT,
        AbilityBit::Laser { .. } => &LASER,
        AbilityBit::Resurrect { .. } => &REZ,
        AbilityBit::Hack { .. } => &HACK,
        AbilityBit::MineLayer { .. } => &MINE,
        AbilityBit::Swap { .. } => &SWAP,
        AbilityBit::Push { .. } => &PUSH,
        AbilityBit::Spy { .. } => &SPY,
        AbilityBit::Custom(_) => {
            unreachable!("custom ability bits resolve via GameDef::custom_effects")
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 4: custom, GameDef-owned ability rows (setup-JSON authored).
// ---------------------------------------------------------------------------

/// The stdlib ability ids (registry row ids plus the FFI parser's kind
/// names) — custom ids must not collide with any of these.
pub const STDLIB_ABILITY_IDS: &[&str] = &[
    "heal", "wall", "pit", "laser", "rez", "resurrect", "hack", "mine", "swap", "push", "spy",
    "none",
];

/// Target reach of a custom ability. Unlike stdlib rows (whose
/// range/pierce parameters ride the authoring `AbilityBit`), a custom
/// row carries its own parameters — the row IS the definition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CustomReach {
    /// Chebyshev ball of `range` around the caster.
    Point { range: u8 },
    /// Queen-line beam: first enemy on each of the 8 rays within `max`
    /// steps (all enemies on the ray when `pierce`); blocking terrain
    /// stops the beam; a friendly stops a non-piercing beam.
    Ray { max: u8, pierce: bool },
}

/// One custom ability as data (Bits 2.0 Stage 4): validated at the FFI
/// boundary, owned by the `GameDef`, applied by the (cold) custom-effect
/// interpreter. Ops draw from the same closed micro-op set; the target
/// vocabulary is the piece/empty subset of the stdlib selector shapes
/// (`Who::Friendly` / `Who::Enemy` / `Who::EmptyCell`; preds `Damaged` /
/// `HpEq1` / `NonRoyal`). Relocation-coupled shapes (swap/push/retreat/
/// resurrect) stay stdlib-only — they need aux-square generation the
/// custom walker does not do.
#[derive(Clone, Debug)]
pub struct CustomAbility {
    pub id: String,
    pub who: Who,
    pub reach: CustomReach,
    pub preds: Vec<Pred>,
    /// Applied in order against the target square (`to`). `HpAdd` here
    /// is lethal at 0 (strike-to-kill through the capture fate), unlike
    /// the stdlib ability interpreter's generation-guarded `HpAdd`.
    pub ops: Vec<MicroOp>,
    /// Applied after `ops`, against the caster (`from`). `HpAdd` only.
    pub self_ops: Vec<MicroOp>,
    pub cost: CostHint,
    /// NNUE descriptor dim (a stdlib kin's slot via [`kin_slot`], or
    /// [`NO_SLOT`]): the frozen 21-dim layout, documented approximation.
    pub descriptor_slot: u8,
}

/// Map a stdlib kin NAME to its v2 descriptor slot (the frozen 21-dim
/// layout: dims 10–15 are heal/laser/wall+pit/mine/rez/hack). Kins
/// without a dedicated dim ("swap"/"push"/"none") map to [`NO_SLOT`] —
/// their value flows through the cost hint's flat prior only.
pub fn kin_slot(name: &str) -> Option<u8> {
    Some(match name {
        "heal" => 10,
        "laser" => 11,
        "wall" | "pit" => 12,
        "mine" => 13,
        "resurrect" | "rez" => 14,
        "hack" => 15,
        "swap" | "push" | "none" => NO_SLOT,
        _ => return None,
    })
}
