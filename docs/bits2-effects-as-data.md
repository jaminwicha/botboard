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
