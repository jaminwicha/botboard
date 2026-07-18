# Bits 2.0 — Effects as Data (design)

## Problem

Axis-A movement is genuinely atomic and parametric (geometry × mode ×
dirs × path × gates × landing × range), but Axis-B is a closed enum:
every ability (Heal/Wall/Pit/Laser/Resurrect/Hack/Mine/Swap/Push) and
every terrain kind (wall/pit/mine/ice/grass/acid/block/conveyor) is a
bespoke `match` arm in movegen and `make_impl`. Adding an effect means
an engine release. The goal: effects and terrain become **data rows
over a small closed set of atomic micro-operations**, so new content is
authored, validated, and shipped as JSON — same as armies.

## Design principles

1. **Micro-ops are the new enum.** The closed set moves DOWN a level:
   instead of "Heal is a variant," *"add HP, capped"* is a variant. The
   micro-op set is small, engine-owned, and each op carries exact undo
   and its own hash bracket. Effects compose ops; ops never compose ops.
2. **Behavior-preserving migration.** Every shipped ability/terrain is
   re-expressed as a data row in a built-in registry; the existing test
   suites (abilities, vocab2, fairy, srw, perft) are the parity gate.
   Bit-identical hashes, notations stable.
3. **Determinism unchanged.** No op may consult a clock or unseeded
   randomness. Target selection is exhaustive generation (movegen), not
   sampling.
4. **Moves unify too — kinds become scripts.** §3.4 already states a
   move is "a single Move whose application script has several steps";
   Bits 2.0 makes that literal. The specials decompose fully:
   - Castle = partner-binding (type+unmoved+rank) · gates (unmoved ×2,
     path-clear, path-not-attacked) · ops [Relocate(self,+2),
     Relocate(partner, mid)]
   - En passant = ephemeral-state gate · ops [Relocate, CaptureAt(aux)]
   - Locust = the same op script as en passant, hop-targeted
   - Double-step = zone gate · ops [Relocate(2), SetEphemeral(mid, 1)]
     — the producer of the state en passant consumes
   - Drop / Resurrect = the same PlaceFrom op over different source
     pools (hand vs dead)
   - Promotion = zone gate · ops [Relocate, TransformType]
   - Overclock = sequenced kernel steps · self op HpAdd(−1)
   `MoveKind` survives only as a stable FFI display code recording
   which stdlib script generated a move. A specialized fast path for
   the dominant script `[Relocate(+CaptureAt(to))]` keeps perft flat.

## Ability model

```
AbilityDef {
  id: string,                       // "heal", or any custom id
  target: Selector {
    who:   Friendly|Enemy|AnyPiece|EmptyCell|TerrainCell(kinds)|Dead(friendly),
    range: u8 (chebyshev)  |  Ray { max, pierce },   // point vs beam
    pred:  [] of Predicate { Damaged, HpEq(n), HpLe(n), NonRoyal, BareCell, ... }
  },
  ops: [ MicroOp, ... ],            // applied atomically, in order
  self_ops: [ MicroOp, ... ],       // e.g. forced retreat (legality-gated)
  cost: CostHint { flat: f64, mult: f64 },
  descriptor_slot: u8,              // which NNUE dim this contributes to
}
```

### Micro-op set (closed, engine-owned, each with exact undo)

Shared by ability scripts AND move-application scripts:

| op | semantics | undo record |
|---|---|---|
| `Relocate(piece_ref, dest_rule)` | mover/partner/target relocation | mover / partner records |
| `CaptureAt(sq)` | strike-or-kill at a square (≠ dest ok: ep, locust) | captured / hp_changes |
| `PlaceFrom(pool, sq)` | enter from hand or dead pool | drop / revived records |
| `TransformType(t)` | promotion / demotion | prior_t |
| `SetEphemeral(sq, ttl)` | ep-style one-ply markers | prior_ep (generalized) |
| `HpAdd(n, cap_max)` | heal/damage, floor 0 = death | hp_changes / mine_deaths |
| `SetTerrain(kind_or_owner)` | lay/clear terrain | terrain_changes |
| `TerrainStep(-1)` | block erosion | terrain_changes |
| `FlipSide` | hack | hacked |
| `SwapWithCaster` | swap | partner |

Gates (generation-time predicates): zone, hp-band (exist today),
moved-flag, ephemeral-state match, path-clear, path-not-attacked.
Bindings: second-piece selection (castle partner). Sources: board |
hand | dead. Sequencing: ordered op list; compounds nest move-steps.

Shipped abilities become registry rows (e.g. Heal = target Friendly,
range r, pred [Damaged], ops [HpAdd(+n, cap)]). Laser = target Enemy
via Ray{max, pierce}, ops [HpAdd(-1)], self_ops [SelfStep(away)] when
retreat. TerrainCell targeting expresses laser-vs-block.

## Terrain model

```
TerrainDef {
  id, code: u8,
  blocks: { ground: bool, flight: bool, drill: bool },   // permission classes
  on_land: { ops: [MicroOp], gate: Anyone|EnemyOfOwner, consumed: bool },
  carry:   None | Slide(entry_vector) | Belt(fixed_dir),  // dest rewriting
  conceal: None | Standing(unless_adjacent) | OwnerSecret, // presence masks
  tiers:   u8,                                            // block erosion
}
```

Wall/pit/ice/grass/acid/mine/blocks/conveyors are eight rows over six
properties. Owner-tagged kinds (mines) keep the code-band encoding.

## Registry & compile

`GameDef` gains an `EffectRegistry` + `TerrainRegistry`, populated from
(1) the built-in stdlib rows (exact parity with today), then (2) any
`"abilities"` / `"terrains"` definitions in the setup JSON (validated:
unknown ops rejected, ranges bounded, cost hints required). Piece types
reference abilities by id. `Effect` enum → `EffectId(u16)` index;
`move_str` uses the registry id ("e2!heal:e3" unchanged for stdlib).

## Consumers to keep honest

- **movegen**: one generator walks Selector shapes (point/ray/dead-set)
  instead of per-kind arms.
- **make/unmake**: one interpreter loop over ops; Undo already carries
  Vec-based records (vocab2 generalization) — extend, don't rewrite.
- **cost model**: flat/mult from CostHint; mobility integral unchanged.
- **NNUE**: descriptor_slot maps registry rows into the frozen 21-dim
  layout (custom effects share the slot of their nearest stdlib kin —
  documented approximation; a descriptor v3 can widen later).
- **qsearch**: "counts as capture" = derived property (any op with
  HpAdd(negative) targeting Enemy) — replaces the kind allowlist.
- **belief/masks**: conceal properties drive the SRW-layer masks that
  currently special-case mines/grass/stealth terrain.
- **FFI**: effect ids surface as strings; `srw_legal_info` keeps stable
  numeric codes for stdlib, allocates upward for customs.

## Staging

- **Stage 1**: the micro-op interpreter inside make/unmake — ability
  effects become stdlib op-scripts; the aux-victim (ep/locust) and
  partner (castle/swap) machinery unify onto shared ops; Effect enum
  retired to registry ids. Fast path for `[Relocate(+CaptureAt(to))]`.
  Parity gates: every suite bit-identical, perft trio flat.
- **Stage 2**: generation side — gates/bindings/sources as data;
  MoveKind reduced to a display code; specials become stdlib move
  scripts (castle/ep/double-step/drop rows).
- **Stage 3**: terrain registry rows (blocks/on-land/carry/conceal).
- **Stage 4**: JSON authoring over the FFI + SRW data files
  (`abilities.json`, `terrains.json`, `moves.json`) with validation;
  Maker Mode gains custom-effect composition.

Each stage is behavior-preserving until Stage 4 adds authoring.

## Stage 1 as built (July 2026) — deviations and notes

Landed in `ops.rs` (micro-op layer + op log) and `effects.rs` (registry
stdlib); `make_impl`'s ability arm is one interpreter loop over the
row's `ops` then `self_ops`; `unmake` is a single backwards replay of
the move's op log (the bespoke per-kind rewind ordering is gone).
Deviations from the design table above:

- **`SwapWithCaster` stays one op** (not two `Relocate`s): an atomic
  two-piece exchange cannot decompose into sequential relocations
  without a temp square mid-transaction; it carries a single `Swapped`
  undo record.
- **`TransformType` is an undo record, not yet an interpreted op**:
  promotion fuses into the fast path's relocate bracket
  (`op_relocate_promo`, one hash bracket per plain move — the
  `[Relocate(+CaptureAt(to))]` fast path). It becomes a standalone op
  when move kinds turn into stdlib scripts (Stage 2).
- **`SetEphemeral` remains the make-prologue `prior_ep` snapshot** plus
  the DoubleStep arm; same Stage-2 promotion path as `TransformType`.
- **`TerrainStep` carries an inherent emptiness guard**: it only erodes
  an *unoccupied* destructible block. This lets the laser row sequence
  `[TerrainStep, CaptureAt(To)]` with no conditional — an occupant
  (e.g. a driller inside the block) shields the block and takes the
  strike instead, bit-exact to the old arm's `if empty && is_block`.
- **One laser row serves both retreat variants**: `Relocate` to `Aux`
  no-ops when the move carries no aux square, so
  `self_ops: [Relocate(From→Aux, hazard)]` covers `retreat: false` too.
- **`HpAdd` has no floor-death**: Stage-1 users (heal +n capped,
  overclock self −1) never reach 0 by generation guarantee; kill paths
  are `CaptureAt` and the landing-hazard hook.
- **Landing hazards stay a post-op hook** (mine/acid; unchanged
  behavior quirks preserved: castle rooks and resurrected pieces do NOT
  trigger hazards; swap bites caster then partner). Their mutations
  flow through the shared op undo records. Terrain rows are Stage 3.
- **`Effect` keeps its enum shape** as the stable wire/notation/FFI
  surface; its variants are the stdlib row ids (`effects::row` maps 1:1,
  `effects::ability_row` maps the `AbilityBit` authoring surface).
  Ability parameters stay on `AbilityBit` (ranges, pierce) and on the
  move itself (heal amount, retreat square, revived type) this stage,
  so the stdlib rows are static.
- **Undo shrank to `{op log, prior_ep, prior_hash}`**: the old
  special-purpose records (captured/captured2/partner/revived/hacked/
  hp_changes/terrain_changes/mine_deaths) folded 1:1 into `OpUndo`
  variants. The log has 4 inline slots (heap spill only for long
  scripts), so the dominant script never allocates; measured perft is
  slightly *faster* than pre-refactor (interleaved A/B, chess/xiangqi/
  shogi all within +2–3%).

## Stage 2 as built (July 2026) — deviations and notes

Landed in `move_defs.rs` (the move-script registry) plus restructures of
`movegen.rs` (generation walks compiled script references), `game.rs`
(the compile step resolves them), and `position.rs`/`ops.rs` (make is
one script selection; `SetEphemeral`/`TransformType` are first-class
ops). Strictly behavior-preserving: every suite bit-identical, deep
perft trio exact, selfplay traces identical.

**The vocabulary as built.** `Gate` = Zone (id resolved at compile from
the authoring bit), HpBand{min,max} (shared `hp_gate_ok`, the kernels'
HP-gate predicate), MovedFlag{must_be_unmoved}, EphemeralMatch,
PathClear, PathSafe, ScreenRule (realized by the compiled hop kernels;
listed on the locust row for vocabulary completeness), and the two
NAMED predicates NoDupFile (*nifu*, generation-time file walk) and
NoDropMate (*uchifuzume*, interpreted in the legality filter — it needs
make/unmake). `Binding` = Partner{flag: CastlePartner, same_rank,
unmoved}; the selected partner's square rides `aux`. `Source` = Board |
Hand | DeadPool (resurrect's ability row shares the vocabulary).

**Script rows.** NORMAL `[CaptureAt(To), Relocate(From→To)]`;
DOUBLE_STEP {gates [Zone, PathClear], ops [Relocate, SetEphemeral(Mid)],
gen_steps 2}; EN_PASSANT {gates [EphemeralMatch], ops [CaptureAt(Aux),
Relocate]}; CASTLE {binding Partner, gates [MovedFlag, PathClear,
PathSafe], ops [Relocate(self, 2 toward partner),
Relocate(partner, Mid)], gen_steps 2}; DROP {source Hand, gates
[NoDupFile, NoDropMate], ops [PlaceFrom(Hand)]}; LOCUST (the ep op
script, hop-targeted); ABILITY (delegates to the effect registry row);
OVERCLOCK {gates [HpBand min 2], sequenced two kernel steps +
HpAdd(−1) self}. `SqRef::Mid` (midpoint of from/to) covers both the
castle rook's landing and the double-step's marker square exactly.

Deviations and notes:

- **Promotion is not a row**: it stays the type's zone-gated
  `PromoRule`, applied as a `TransformType` op APPENDED to the
  generating mover script (movegen's promo expansion), and fused into
  the relocate bracket on the fast path — identical final hash, one
  bracket. `TransformType` and `SetEphemeral` are now first-class
  interpreted ops (the Stage-1 deviations closed); the make-prologue ep
  snapshot folded into the op log as `OpUndo::Ephemeral` records
  (`Undo` is now `{op log, prior_hash}` — no move-level state snapshot
  remains).
- **Generation shape is DERIVED, not authored**: `GenShape` (partner
  compound / ephemeral capture / two-step producer) is computed at
  compile time from the row's binding/gates/ops. Each shape keeps one
  hand-optimized walker (the castle between-squares walk, the ep
  kernel match), parameterized by the row — step counts, gate lists,
  binding fields — never hard-coded constants.
- **Compile step**: `Compiled` gains `specials: Vec<SpecialRef>`,
  `drop: Option<DropRef>`, `overclock: Option<ScriptRef>`. Origin
  gates fold into `OriginGates` — a compact straight-line-checkable
  form (like kernels: interpret the gate list once at compile, not per
  node). No allocation on the generation or apply hot paths.
- **MoveKind demoted to a display code**: the enum and its wire values
  (FFI kind codes 0–7, notation) are frozen; its only behavioral role
  is `move_defs::script(kind)` / `apply_script`, the single kind→row
  selection. Make/unmake interpret the row (`ApplyShape`); search's
  quiet/capture classes are row-derived (`is_quiet`,
  `counts_as_capture`, `capture_class`, `effectful`); is_legal
  interprets the row's NoDropMate gate; belief's drop check reads the
  row's `source`.
- **En passant deliberately keeps `capture_class: false`**: Stage-1
  search treated ep as quiet (dest-empty) for LMR/qsearch — a
  preserved, documented quirk, now explicit row data instead of an
  accident of the kind allowlist. Locust sets it true.
- **The fast path survives as row data**: `ApplyShape::Mover{victim,
  strike_stops_mover, trailing}` parameterizes the dominant
  non-interpreted path (strike-or-kill at the row's victim square,
  relocate+promo in one bracket, hazards, trailing ops). Each row
  carries an `apply_fn` whose body destructures its OWN static row, so
  the compiler constant-folds the parameters into a specialized body —
  `apply_script` jump-threads the kind branch to those entries and the
  dominant script keeps Stage-1 codegen (a plain indirect
  `apply_fn` call measurably stalled the pipeline; so did letting the
  out-of-line `revert_op` call survive in unmake — both are
  inline-always now).
- **Perf**: interleaved balanced A/B perft medians (release, 16 runs
  each side): chess d5 0.974×, xiangqi d4 1.010×, shogi d4 1.002× —
  within the 3% gate. Hardware counters on chess d5: instructions
  +0.11%, cycles +2.7% (the residual is the mandated ephemeral-marker
  op-log records ≈0.6% plus dispatch/layout stalls; xiangqi and shogi
  are at parity or faster).

## Stage 3 as built (July 2026) — deviations and notes

Landed in `terrain_defs.rs` (the terrain registry); `position.rs` keeps
only re-exports of the codes and predicates, `ops.rs`'s hazard hook is a
registry interpreter, `movegen.rs`'s carry/targeting checks read rows,
and `srw.rs`'s parsing/codes/concealment read rows. Strictly
behavior-preserving: every suite bit-identical (zero expectation
changes), deep perft trio exact, selfplay traces identical.

**The rows as built.** One row per INTERNAL code (17 rows): the
`T_*` codes stay frozen as the registry index and the Zobrist terrain
key space (`zobrist::TERRAINS` = `terrain_defs::NT`). Mines occupy the
owner band `T_MINE0 + side` (codes 3..=6): four rows sharing id
`"mine"`, `owner_banded: true`, and wire code 3 — the code-band
encoding survives as row data. `blocks` uses three named profiles:
SOLID (ground+flight; drill passes — wall, block1-3), CHASM
(ground+drill; flight passes — pit), OPEN (everything else). `on_land`
is `Some` on mines {BITE, gate EnemyOfOwner, consumed} and acid
{BITE, Anyone, persistent}; `carry` is `Slide{riders_only: true}` on
ice and `Belt{dx,dy}` on the four conveyors; `conceal` is `Standing`
on grass and `OwnerSecret` on mines; `tiers` 1/2/3 on the block band.
The `code` field is the stable FFI wire id (`srw_terrain` codes 0–13).

Deviations and notes:

- **The hot predicates compile to derived RANGE COMPARES, not mask
  probes.** The planned per-class u32 code-mask probe
  (`mask & (1 << t)`) measured ~9% slower on chess perft than the
  pre-registry compare chains — and a 256-byte table ~6% — because the
  hottest sites (`cell_obstructed_for` inside `is_attacked`, kernel
  walks) live and die by two fused register compares on an
  already-loaded byte. As built, the masks ARE still folded from the
  rows at compile time, then re-expressed as contiguous code ranges
  (`ranges_of`, up to `NR = 3` per class; unused ranges fold to
  `false`), so `terrain_stops` compiles to exactly the old comparison
  chain. `terrain_stops` decomposes over derived exemption classes
  (STOP_ALWAYS / DRILL_EXEMPT / FLIGHT_EXEMPT / EXEMPT_BOTH) so the
  open cell short-circuits on `t` alone; the stdlib's empty classes
  fold away. A compile-time exhaustive proof (all 256 codes × 4
  permission combos) asserts the range forms equal the rows — a row
  edit the derivation cannot express fails the build, never drifts.
- **`pit_count`/`wall_count` generalized to exemption-class counts**:
  maintained via the derived `flight_exempt`/`drill_exempt` predicates
  (ground-blocking codes the class ignores). `has_pit`/`has_wall` keep
  their names and their EXACT bb fast-path gating semantics: flyers
  yield the wide-bitboard path when any flight-exempt terrain exists,
  drillers when any drill-exempt terrain exists.
- **The hazard hook is the on-land interpreter**: same call sites, same
  quirks (castle rooks and resurrected pieces do not trigger; swap
  bites caster then partner). The guard (`has_on_land`, derived
  ranges) is now INLINE at the call sites — cheaper than Stage 2's
  unconditional call — and the interpreter body (`land_hazard_interpret`,
  gate → consumed → ops through the op log) is `#[cold]`/out-of-line.
  On-land `HpAdd` carries the hazard floor-death rule (lethal at 0),
  unlike the ability interpreter's generation-guarded `HpAdd`; the
  gate check precedes `consumed`, so an owner crossing its own mine
  spends nothing.
- **Carry rules live in the rows**: the ice pass dispatches on
  `Carry::Slide { riders_only }` (the rider-in-entry-direction rule is
  row data, not a kind check) and slides across any `slides()` cell;
  the belt pass on `Carry::Belt { dx, dy }` via `conv_dir`/`belts`.
  The Stage-2 interleave rule (ice pass, then belt pass, no re-entry)
  and all guards are unchanged.
- **Concealment is row data at the SRW layer**: `masked_world` blanks
  `OwnerSecret` terrain for non-owners, `srw_terrain_for` surfaces its
  wire code only to the banded owner, and `concealed_by_grass` keys on
  `Conceal::Standing` — no kind constants left in the FFI's masking.
  `srw.rs` terrain parsing is `by_id` over the AUTHORABLE rows (the
  absence row and owner-band rows excluded — mines are laid by the
  ability), preserving the parser's exact acceptance set (unknown
  kinds still default to wall).
- **Ability targeting reads row properties**: bare-floor checks are
  `is_bare` (the absence row), laser-vs-cover and `TerrainStep`
  erosion are `is_block` (`tiers > 0`) with `erode` stepping down the
  code-contiguous block band (debug-asserted contiguous).
- **Perf**: interleaved A/B perft medians (release, user-CPU time,
  12–16 pairs, base-vs-base control 0.997): chess d5 1.017×, xiangqi
  d4 1.019×, shogi d4 1.000× — within the 3% gate. The residual is
  layout-lottery (LTO + one codegen unit: ±2% swings were observed
  from semantically identical formulations); the inline hazard guard
  claws most of it back.
