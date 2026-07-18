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
4. **Specials stay named.** Castling, en-passant, double-step, drops,
   overclock compounds remain the spec's §3.3 *special generators* —
   they are history-dependent state machines, not stateless effect
   scripts, and are already atomic components in the spec's taxonomy.
   (Documented decision; revisit only with a concrete need.)

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

| op | semantics | undo record |
|---|---|---|
| `HpAdd(n, cap_max)` | heal/damage, floor 0 = death via standard path | hp_changes / mine_deaths |
| `SetTerrain(kind_or_owner)` | lay/clear terrain | terrain_changes |
| `TerrainStep(-1)` | block erosion (tiered kinds) | terrain_changes |
| `FlipSide` | hack | hacked |
| `RelocateAway(d)` | push (away-vector from caster) | mover-relocation record |
| `SwapWithCaster` | swap | partner |
| `ReviveHere(type_choice)` | resurrect | revived |
| `SelfStep(away)` | retreat (legality-gated at generation) | standard mover path |

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

- **Stage 1**: ability registry + micro-op interpreter; stdlib rows;
  enum retired; parity gates green. (Engine-internal; no new features.)
- **Stage 2**: terrain registry the same way.
- **Stage 3**: JSON-defined custom abilities/terrains over the FFI +
  SRW `data/abilities.json`, `data/terrains.json`; Maker Mode gains
  custom-effect composition (game-side follow-up).

Each stage is behavior-preserving until Stage 3 adds authoring.
