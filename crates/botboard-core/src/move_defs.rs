//! Bits 2.0 Stage 2 — the move-script registry (docs/bits2-effects-as-data.md).
//!
//! The GENERATION side of moves becomes data: every special move is a
//! stdlib row of generation-time **gates** (predicates), a **binding**
//! (second-piece selection), a **source** pool, and an op script over the
//! Stage-1 micro-op set. `MoveKind` survives only as the stable display
//! code recording which stdlib row generated a move (FFI kind codes and
//! notation are frozen); make/unmake and search consult the row — the one
//! `MoveKind` branch left is [`script`], the kind→row lookup.
//!
//! Generation stays hand-optimized per *shape* (the castle's
//! between-squares walk, the en-passant kernel match), but each walker is
//! driven by the row's data — step counts, gate lists, binding fields —
//! not hard-coded constants. The shape itself ([`GenShape`]) is DERIVED
//! from the row's gates/binding/ops at compile time, not authored.
//!
//! Promotion is not a row of its own: it is a `TransformType` op appended
//! to the generating mover script by the type's zone-gated `PromoRule`
//! (movegen's `push_with_promo`), applied fused into the mover's relocate
//! bracket on the fast path.

use crate::bits::SpecialBit;
use crate::game::GameDef;
use crate::moves::{Move, MoveKind, NO_SQ};
use crate::ops::{HpAmt, MicroOp, OpLog, Pool, SqRef};
use crate::position::{hp_gate_ok, Position};

/// A row's application entry point: apply the move's script against the
/// position, logging exact undo. Each stdlib row's function destructures
/// its OWN static row (so the row stays the single source of truth) and
/// the compiler constant-folds it into a specialized body — the indirect
/// call through the row replaces any per-kind branching in make.
pub type ApplyFn = fn(&mut Position, &GameDef, &Move, &mut OpLog);

/// Generation-time predicate rows — the gate vocabulary. Zone and HP-band
/// mirror the same gates the compiled movement kernels already carry
/// (`from_zone`/`min_hp`/`max_hp` on `LeapK`/`RideK`/`HopK`); the rest are
/// the history/path/state predicates the special generators enforce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gate {
    /// Origin lies in the row's zone parameter (per side). The zone id is
    /// an authoring parameter (`SpecialBit::DoubleStep.start_zone`),
    /// resolved once into the compiled [`SpecialRef::zone`].
    Zone,
    /// Mover HP within [min, max] (0 = unbounded) — `hp_gate_ok`, the
    /// same predicate the HP-gated kernels use.
    HpBand { min: i16, max: i16 },
    /// The acting piece's `moved` history flag (castle king).
    MovedFlag { must_be_unmoved: bool },
    /// The destination must equal the live ephemeral one-ply marker (the
    /// en-passant square a `SetEphemeral` producer laid last ply).
    EphemeralMatch,
    /// The squares strictly between the compound's endpoints must be
    /// unobstructed: actor→partner when the row has a partner binding
    /// (castle), origin→destination otherwise (double-step).
    PathClear,
    /// The actor's origin, crossing, and landing squares must not be
    /// attacked by the enemy (castle's not-through-check rule).
    PathSafe,
    /// The hopper screen rule (exactly one screen on the ray, land per
    /// the landing mode) — realized by the compiled hop kernels; listed
    /// as the locust row's generation gate for vocabulary completeness.
    ScreenRule,
    /// NAMED predicate *nifu* (§5): no drop onto a file already holding
    /// an unpromoted piece of the same type and side. Stays a named row
    /// in the vocabulary — its board walk is engine-owned; per-type
    /// participation is the `drop_no_dup_file` flag ([`DropRef`]).
    NoDupFile,
    /// NAMED predicate *uchifuzume* (§5): no drop delivering immediate
    /// checkmate. Needs make/unmake, so it is interpreted in the
    /// legality filter (`is_legal`), not at generation; per-type
    /// participation is the `drop_no_mate` flag.
    NoDropMate,
}

/// Which piece-type flag a partner binding selects on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PartnerFlag {
    /// `PieceTypeDef::castle_partner` (the rook half of the compound).
    CastlePartner,
}

/// Second-piece selection for two-piece compound scripts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Binding {
    None,
    /// Select a friendly piece carrying `flag`, optionally required to be
    /// on the actor's rank and unmoved (the castle partner). The selected
    /// piece's square rides the move's `aux` field.
    Partner { flag: PartnerFlag, same_rank: bool, unmoved: bool },
}

/// Where the acting piece comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Board,
    /// Shogi-style hand (drops). Hand moves have no board origin, so
    /// search's quiet/capture classes treat them specially.
    Hand,
    /// The dead pool (resurrect — an ability row, listed for the shared
    /// source vocabulary).
    DeadPool,
}

/// How make/unmake applies the row. The interpreter loop over `ops`
/// covers simple rows; the two specialized shapes preserve the Stage-1
/// fast path (dominant script, one relocate bracket) and the overclock's
/// conditional sequencing — both parameterized by row data.
#[derive(Clone, Copy, Debug)]
pub enum ApplyShape {
    /// The dominant mover shape: `CaptureAt(victim)` (strike-or-kill,
    /// §3.2 armor) then `Relocate(From→To)` with any promotion fused
    /// into the same bracket, then the landing-hazard hook, then the
    /// row's trailing ops. `strike_stops_mover`: a strike (armored
    /// victim survives) leaves the mover in place — true for the
    /// dominant and locust rows, false for en passant (the pawn passes
    /// regardless).
    Mover {
        victim: Option<SqRef>,
        strike_stops_mover: bool,
        trailing: &'static [MicroOp],
    },
    /// Interpret `ops` in order (drop).
    Ops,
    /// The move's `Effect` names an ability registry row
    /// (`effects::row`); that row's `ops` then `self_ops` apply.
    EffectRow,
    /// Overclock ⟨move, move, self −1 HP⟩ (§3.4): step 1 quiet
    /// relocate, `CaptureAt(Aux)`, relocate onto `aux` only if it fell
    /// or was empty (a strike leaves the mover on `to`), `HpAdd(−1)`,
    /// hazard. The kill-conditional second relocate keeps this a
    /// specialized shape rather than a flat op list.
    Sequenced,
}

/// One stdlib move script: the generation row (source, gates, binding)
/// plus the application script and the search-facing derived properties.
#[derive(Clone, Copy, Debug)]
pub struct MoveScript {
    pub id: &'static str,
    /// The display code stamped on generated moves — the stable
    /// wire/notation/FFI surface (kind codes 0–7 frozen).
    pub kind: MoveKind,
    pub source: Source,
    pub binding: Binding,
    pub gates: &'static [Gate],
    /// The application script over the micro-op set. A move carrying a
    /// promotion appends `TransformType` (zone-gated at generation).
    pub ops: &'static [MicroOp],
    pub apply: ApplyShape,
    /// Specialized entry point for `apply` (constant-folded from this
    /// row — see [`ApplyFn`]).
    pub apply_fn: ApplyFn,
    /// Actor travel distance for stepped generators (double-step pawn,
    /// castle king): squares moved along the generation axis.
    pub gen_steps: u8,
    /// Search treats this row's moves as captures even though `to` is
    /// empty (locust: the victim stands on `aux`): qsearch inclusion,
    /// MVV ordering via `aux`, never LMR-quiet. En passant deliberately
    /// does NOT set this — the Stage-1 search treated ep as quiet
    /// (dest-empty), a preserved, documented quirk.
    pub capture_class: bool,
    /// Ability/compound rows: the policy head's "effectful" feature
    /// class (training.rs).
    pub effectful: bool,
}

impl MoveScript {
    #[inline]
    pub fn has_gate(&self, gate: Gate) -> bool {
        self.gates.contains(&gate)
    }

    /// Quiet for search purposes (LMR, killer/history credit): lands on
    /// an empty square, is not from a hand, and carries no aux-victim
    /// capture class.
    #[inline]
    pub fn is_quiet(&self, pos: &Position, mv: &Move) -> bool {
        !self.capture_class
            && !matches!(self.source, Source::Hand)
            && pos.piece_at(mv.to).is_none()
    }

    /// Counts as a capture for quiescence: aux-victim capture class, or
    /// an enemy on the destination (and not a hand move). Derived from
    /// row data — replaces the old kind allowlist.
    #[inline]
    pub fn counts_as_capture(&self, pos: &Position, mv: &Move) -> bool {
        self.capture_class
            || (!matches!(self.source, Source::Hand)
                && pos.piece_at(mv.to).is_some_and(|p| p.side != pos.stm))
    }
}

// ---------------------------------------------------------------------------
// The stdlib rows. Shared op constants keep each op literal single-sourced
// between a row's documented `ops` and its apply-shape parameters.
// ---------------------------------------------------------------------------

/// The mover's relocation: from → to, landing hazards bite.
const RELOC_MOVER: MicroOp =
    MicroOp::Relocate { who: SqRef::From, dest: SqRef::To, hazard: true };
/// The partner's relocation: aux (its origin) → midpoint of the actor's
/// from/to (castle rook). Partners do not trigger landing hazards
/// (Stage-1 behavior quirk, preserved).
const RELOC_PARTNER: MicroOp =
    MicroOp::Relocate { who: SqRef::Aux, dest: SqRef::Mid, hazard: false };
/// The double-step's marker: lay the ephemeral (en-passant) square on
/// the passed-over midpoint.
const SET_EP_MID: MicroOp = MicroOp::SetEphemeral { at: SqRef::Mid };

/// The dominant script: `[CaptureAt(To), Relocate(From→To)]` — every
/// kernel-generated move (leaper/rider/hopper landings). Promotions
/// append `TransformType`, fused into the relocate bracket.
pub static NORMAL: MoveScript = MoveScript {
    id: "normal",
    kind: MoveKind::Normal,
    source: Source::Board,
    binding: Binding::None,
    gates: &[],
    ops: &[MicroOp::CaptureAt { at: SqRef::To }, RELOC_MOVER],
    apply: ApplyShape::Mover {
        victim: Some(SqRef::To),
        strike_stops_mover: true,
        trailing: &[],
    },
    apply_fn: apply_normal,
    gen_steps: 0,
    capture_class: false,
    effectful: false,
};

/// Pawn double-step: zone-gated two-step advance producing the ephemeral
/// marker en passant consumes. `PathClear` covers the passed-over square;
/// the landing must be empty (the Relocate lands quietly).
pub static DOUBLE_STEP: MoveScript = MoveScript {
    id: "double_step",
    kind: MoveKind::DoubleStep,
    source: Source::Board,
    binding: Binding::None,
    gates: &[Gate::Zone, Gate::PathClear],
    ops: &[RELOC_MOVER, SET_EP_MID],
    apply: ApplyShape::Mover {
        victim: Some(SqRef::To),
        strike_stops_mover: true,
        trailing: &[SET_EP_MID],
    },
    apply_fn: apply_double_step,
    gen_steps: 2,
    capture_class: false,
    effectful: false,
};

/// En passant: capture onto the live ephemeral marker; the victim (the
/// double-stepped pawn on the passed-over square) rides `aux`. The pawn
/// relocates regardless of strike-vs-kill (`strike_stops_mover: false`).
pub static EN_PASSANT: MoveScript = MoveScript {
    id: "en_passant",
    kind: MoveKind::EnPassant,
    source: Source::Board,
    binding: Binding::None,
    gates: &[Gate::EphemeralMatch],
    ops: &[MicroOp::CaptureAt { at: SqRef::Aux }, RELOC_MOVER],
    apply: ApplyShape::Mover {
        victim: Some(SqRef::Aux),
        strike_stops_mover: false,
        trailing: &[],
    },
    apply_fn: apply_en_passant,
    gen_steps: 0,
    capture_class: false,
    effectful: false,
};

/// Castle: partner binding (castle_partner flag, same rank, unmoved) ·
/// gates (king unmoved, path clear to the partner, king's path safe) ·
/// ops `[Relocate(self, 2 toward partner), Relocate(partner, mid)]`.
pub static CASTLE: MoveScript = MoveScript {
    id: "castle",
    kind: MoveKind::Castle,
    source: Source::Board,
    binding: Binding::Partner {
        flag: PartnerFlag::CastlePartner,
        same_rank: true,
        unmoved: true,
    },
    gates: &[
        Gate::MovedFlag { must_be_unmoved: true },
        Gate::PathClear,
        Gate::PathSafe,
    ],
    ops: &[RELOC_MOVER, RELOC_PARTNER],
    apply: ApplyShape::Mover {
        victim: None,
        strike_stops_mover: true,
        trailing: &[RELOC_PARTNER],
    },
    apply_fn: apply_castle,
    gen_steps: 2,
    capture_class: false,
    effectful: false,
};

/// Shogi drop: enter from the hand onto an empty square the type could
/// act from. The two NAMED predicates stay named gate rows — *nifu*
/// (generation-time file walk) and *uchifuzume* (legality filter);
/// per-type participation flags live on the compiled [`DropRef`].
pub static DROP: MoveScript = MoveScript {
    id: "drop",
    kind: MoveKind::Drop,
    source: Source::Hand,
    binding: Binding::None,
    gates: &[Gate::NoDupFile, Gate::NoDropMate],
    ops: &[MicroOp::PlaceFrom { pool: Pool::Hand, hazard: true }],
    apply: ApplyShape::Ops,
    apply_fn: apply_drop,
    gen_steps: 0,
    capture_class: false,
    effectful: false,
};

/// Ability action (§3.4): the move's `Effect` selects the ability
/// registry row (`effects::row`), which carries the ops.
pub static ABILITY: MoveScript = MoveScript {
    id: "ability",
    kind: MoveKind::Ability,
    source: Source::Board,
    binding: Binding::None,
    gates: &[],
    ops: &[],
    apply: ApplyShape::EffectRow,
    apply_fn: apply_ability,
    gen_steps: 0,
    capture_class: false,
    effectful: true,
};

/// Overclock compound (§3.4): two sequenced kernel steps plus the self
/// op `HpAdd(−1)`. Generated by enumerate-by-applying (movegen's
/// `gen_overclock`), gated by the HP band (the self-damage must not be
/// lethal). The second relocate is kill-conditional — see
/// [`ApplyShape::Sequenced`].
pub static OVERCLOCK: MoveScript = MoveScript {
    id: "overclock",
    kind: MoveKind::Compound,
    source: Source::Board,
    binding: Binding::None,
    gates: &[Gate::HpBand { min: 2, max: 0 }],
    ops: &[
        MicroOp::Relocate { who: SqRef::From, dest: SqRef::To, hazard: false },
        MicroOp::CaptureAt { at: SqRef::Aux },
        MicroOp::Relocate { who: SqRef::To, dest: SqRef::Aux, hazard: false },
        MicroOp::HpAdd { n: HpAmt::Lit(-1), cap: false },
    ],
    apply: ApplyShape::Sequenced,
    apply_fn: apply_overclock,
    gen_steps: 0,
    capture_class: false,
    effectful: true,
};

/// Locust hop (§3.1 landing mode): capture the screen (`aux`), land on
/// the empty square beyond — the en-passant op script, hop-targeted.
/// Generation lives in the compiled hop-kernel walk (the `ScreenRule`
/// gate); a strike on an armored screen chips it at range without
/// leaping (`strike_stops_mover: true`).
pub static LOCUST: MoveScript = MoveScript {
    id: "locust",
    kind: MoveKind::Locust,
    source: Source::Board,
    binding: Binding::None,
    gates: &[Gate::ScreenRule],
    ops: &[MicroOp::CaptureAt { at: SqRef::Aux }, RELOC_MOVER],
    apply: ApplyShape::Mover {
        victim: Some(SqRef::Aux),
        strike_stops_mover: true,
        trailing: &[],
    },
    apply_fn: apply_locust,
    gen_steps: 0,
    capture_class: true,
    effectful: false,
};

// ---------------------------------------------------------------------------
// Row application entry points. Each destructures its own static row —
// the row stays the single source of the script — and the compiler
// constant-folds the row's parameters into a specialized body, so the
// dominant script keeps the Stage-1 fast-path codegen.
// ---------------------------------------------------------------------------

/// Shared Mover-shape driver, monomorphized per row by the wrappers.
#[inline(always)]
fn apply_mover(row: &'static MoveScript, pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    let ApplyShape::Mover { victim, strike_stops_mover, trailing } = row.apply else {
        unreachable!("apply_mover on a non-Mover row")
    };
    pos.mover_apply(g, mv, victim, strike_stops_mover, trailing, log);
}

#[inline(always)]
fn apply_normal(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    apply_mover(&NORMAL, pos, g, mv, log);
}
#[inline(always)]
fn apply_double_step(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    apply_mover(&DOUBLE_STEP, pos, g, mv, log);
}
#[inline(always)]
fn apply_en_passant(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    apply_mover(&EN_PASSANT, pos, g, mv, log);
}
#[inline(always)]
fn apply_castle(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    apply_mover(&CASTLE, pos, g, mv, log);
}
#[inline(always)]
fn apply_locust(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    apply_mover(&LOCUST, pos, g, mv, log);
}
#[inline(always)]
fn apply_drop(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    pos.apply_move_ops(g, mv, DROP.ops, log);
}
#[inline(always)]
fn apply_ability(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    pos.apply_effect_row(g, mv, log);
}
#[inline(always)]
fn apply_overclock(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    pos.apply_sequenced(g, mv, log);
}

/// The kind→row table, in wire-code order (the FFI kind codes 0–7).
static SCRIPTS: [&MoveScript; 8] = [
    &NORMAL,      // 0
    &DOUBLE_STEP, // 1
    &EN_PASSANT,  // 2
    &CASTLE,      // 3
    &DROP,        // 4
    &ABILITY,     // 5
    &OVERCLOCK,   // 6
    &LOCUST,      // 7
];

/// The kind→row lookup — the ONE place `MoveKind` selects behavior.
/// Everything downstream (make/unmake shape, quiet/capture classes,
/// legality gates) derives from the returned row's data.
#[inline]
pub fn script(k: MoveKind) -> &'static MoveScript {
    SCRIPTS[k as usize]
}

/// Apply the generating script row of `mv` — the jump-threaded form of
/// `(script(mv.kind).apply_fn)(..)`: the same single MoveKind branch,
/// but each arm's row is a known constant, so its `apply_fn` folds to a
/// direct call and each row's body inlines into make.
#[inline(always)]
pub fn apply_script(pos: &mut Position, g: &GameDef, mv: &Move, log: &mut OpLog) {
    match mv.kind {
        MoveKind::Normal => (NORMAL.apply_fn)(pos, g, mv, log),
        MoveKind::DoubleStep => (DOUBLE_STEP.apply_fn)(pos, g, mv, log),
        MoveKind::EnPassant => (EN_PASSANT.apply_fn)(pos, g, mv, log),
        MoveKind::Castle => (CASTLE.apply_fn)(pos, g, mv, log),
        MoveKind::Drop => (DROP.apply_fn)(pos, g, mv, log),
        MoveKind::Ability => (ABILITY.apply_fn)(pos, g, mv, log),
        MoveKind::Compound => (OVERCLOCK.apply_fn)(pos, g, mv, log),
        MoveKind::Locust => (LOCUST.apply_fn)(pos, g, mv, log),
    }
}

// ---------------------------------------------------------------------------
// Compiled references (the Stage-2 compile step): SpecialBits and drop /
// overclock flags resolve to script references once, like kernels — no
// allocation on the generation hot path.
// ---------------------------------------------------------------------------

/// Generation shape, DERIVED from a row's data (binding → partner
/// compound; `EphemeralMatch` gate → marker capture; `SetEphemeral` op →
/// marker producer). Each shape has one hand-optimized walker in movegen,
/// parameterized by the row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GenShape {
    /// Stepped quiet advance laying the ephemeral marker (double-step).
    TwoStepEphemeral,
    /// Kernel-shaped capture onto the live marker (en passant).
    EphemeralCapture,
    /// Two-piece compound over a partner binding (castle).
    PartnerCompound,
}

/// A script row's origin-checkable gates folded into straight-line
/// fields — the compile step's fast runtime form of `Gate::Zone` /
/// `HpBand` / `MovedFlag` / `EphemeralMatch` (like kernels, interpreted
/// once at compile, not per node). Path/screen gates and the named drop
/// predicates are interpreted where they have the context they need:
/// the shape walkers and the legality filter.
#[derive(Clone, Copy, Debug)]
pub struct OriginGates {
    /// `Gate::Zone` with its resolved zone parameter (`NO_ZONE` = none).
    zone: u32,
    /// `Gate::HpBand` (0 = unbounded, `hp_gate_ok`).
    hp_min: i16,
    hp_max: i16,
    /// `Gate::MovedFlag { must_be_unmoved: true }`.
    must_be_unmoved: bool,
    /// `Gate::EphemeralMatch` precondition: a marker must be live (the
    /// destination match itself lives in the walker).
    needs_ephemeral: bool,
}

const NO_ZONE: u32 = u32::MAX;

impl OriginGates {
    /// Fold a row's gate list (plus its resolved zone parameter) into
    /// the compiled form.
    pub fn compile(gates: &[Gate], zone: Option<usize>) -> Self {
        let mut og = OriginGates {
            zone: NO_ZONE,
            hp_min: 0,
            hp_max: 0,
            must_be_unmoved: false,
            needs_ephemeral: false,
        };
        for gt in gates {
            match *gt {
                Gate::Zone => og.zone = zone.map_or(NO_ZONE, |z| z as u32),
                Gate::HpBand { min, max } => {
                    og.hp_min = min;
                    og.hp_max = max;
                }
                Gate::MovedFlag { must_be_unmoved } => og.must_be_unmoved = must_be_unmoved,
                Gate::EphemeralMatch => og.needs_ephemeral = true,
                Gate::PathClear
                | Gate::PathSafe
                | Gate::ScreenRule
                | Gate::NoDupFile
                | Gate::NoDropMate => {}
            }
        }
        og
    }

    /// Do the acting piece and position pass this row's origin gates?
    #[inline]
    pub fn pass(&self, g: &GameDef, pos: &Position, from: u16, moved: bool, hp: i16) -> bool {
        if self.must_be_unmoved && moved {
            return false;
        }
        if self.needs_ephemeral && pos.ep == NO_SQ {
            return false;
        }
        if self.zone != NO_ZONE && !g.zones[self.zone as usize].contains(pos.stm, from) {
            return false;
        }
        hp_gate_ok(self.hp_min, self.hp_max, hp)
    }
}

/// One compiled special: the stdlib row, its compiled origin gates, and
/// the derived generation shape. The walker-hot row fields (`kind`,
/// `gen_steps`) are copied in at compile so the generation loop reads
/// only this contiguous compiled row.
#[derive(Clone, Copy, Debug)]
pub struct SpecialRef {
    pub script: &'static MoveScript,
    pub gates: OriginGates,
    pub shape: GenShape,
    /// Copy of `script.kind` (the display code stamped on moves).
    pub kind: MoveKind,
    /// Copy of `script.gen_steps` (stepped-walker travel distance).
    pub gen_steps: u8,
}

/// A compiled script reference with origin gates but no generation shape
/// (the overclock compound — generated by enumerate-by-applying).
#[derive(Clone, Copy, Debug)]
pub struct ScriptRef {
    pub script: &'static MoveScript,
    pub gates: OriginGates,
}

/// Compiled drop reference: the drop row plus which of its NAMED
/// predicate gates the piece type activates.
#[derive(Clone, Copy, Debug)]
pub struct DropRef {
    pub script: &'static MoveScript,
    /// *nifu* participation (`Gate::NoDupFile`).
    pub no_dup_file: bool,
    /// *uchifuzume* participation (`Gate::NoDropMate`).
    pub no_mate: bool,
}

fn shape_of(sc: &'static MoveScript) -> GenShape {
    if matches!(sc.binding, Binding::Partner { .. }) {
        GenShape::PartnerCompound
    } else if sc.has_gate(Gate::EphemeralMatch) {
        GenShape::EphemeralCapture
    } else if sc.ops.iter().any(|o| matches!(o, MicroOp::SetEphemeral { .. })) {
        GenShape::TwoStepEphemeral
    } else {
        unreachable!("special script {} has no derivable generation shape", sc.id)
    }
}

/// Compile one authoring `SpecialBit` into its stdlib script reference.
pub fn compile_special(s: &SpecialBit) -> SpecialRef {
    let (script, zone): (&'static MoveScript, Option<usize>) = match s {
        SpecialBit::DoubleStep { start_zone } => (&DOUBLE_STEP, Some(*start_zone)),
        SpecialBit::EnPassant => (&EN_PASSANT, None),
        SpecialBit::Castling => (&CASTLE, None),
    };
    SpecialRef {
        script,
        gates: OriginGates::compile(script.gates, zone),
        shape: shape_of(script),
        kind: script.kind,
        gen_steps: script.gen_steps,
    }
}

/// Compile the overclock script reference (HP-band gate folded).
pub fn compile_overclock() -> ScriptRef {
    ScriptRef {
        script: &OVERCLOCK,
        gates: OriginGates::compile(OVERCLOCK.gates, None),
    }
}
