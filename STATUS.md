# Botboard — Implementation Status

Traceability from **Botboard_Spec_v4.md**, **Botboard_Training_Spec.md**,
and **docs/bits2-effects-as-data.md** to code.
Everything listed as *implemented* is exercised by `cargo test --release`
(deep perft and statistical matches behind `-- --ignored`; debug builds
additionally assert incremental hash/mirror consistency on every make).

## Engine spec (Botboard_Spec_v4.md)

| Spec | Status | Where |
|---|---|---|
| §3.1 Axis A — leaper/rider/hopper, direction, mode, path (lame leapers), zone conditions, target predicates | ✅ incl. both terrain permissions (flight over pits, drill through walls/blocks) and the state gate (per-kernel min/max-HP conditions, mailbox-checked, excluded from the bb tables) | `bits.rs`, `geometry.rs`, compiled in `game.rs`, `tests/vocab2_suite.rs` |
| §3.2 Axis B — capture fates, transformation w/ base-type de-promotion, forced-if-immobile, hit-count armor (strikes), heal, spawn (walls/pits), laser-at-range with coupled retreat | ✅ SoA HP arrays; ammo/cooldown extend the same state bucket; vocab batch 2 adds swap (atomic friendly exchange), push (shove with landing hazards), laser-cracked destructible blocks (3 tiers), and conveyor belts (destination rewriting like ice). Post-Bits-2.0: execution is the micro-op interpreter over registry rows; `AbilityBit` is the parameter-authoring surface | `ops.rs`, `effects.rs`, `position.rs`, `bits.rs::AbilityBit`, `tests/abilities_suite.rs`, `tests/vocab2_suite.rs` |
| §3.3 Special moves — castling, en passant, double-step; drops with 3 legality tiers (*nifu*, *uchifuzume*) | ✅ post-Bits-2.0 these are stdlib move-script rows (CASTLE/EN_PASSANT/DOUBLE_STEP/DROP) | `move_defs.rs`, `movegen.rs` |
| §3.4 Turn model — one turn = one action, strict rotation, pass forbidden; compound moves piece-local, atomic, priced | ✅ incl. overclock ⟨move,move,−1HP⟩ compounds, ability actions, laser-retreat "nerf has teeth". `MoveKind` is a display code; behavior is row data. Multi-ply turn policies and the optional pass-allowed flag (§3.4/§5 escape hatches) remain unbuilt — see Open goals | `move_defs.rs`, `moves.rs`, `movegen.rs`, `tests/abilities_suite.rs` |
| §4 Cost model — C_prior, mobility integral, utility multipliers (armor 1+0.5(HP−1), overclock ×1.8), nerfs, floor, **synergy term with fitting loop** | ✅ synergy S_ij over Bit categories, SGD-fitted from residuals, recovers planted interactions | `cost.rs::SynergyModel`, `tests/systems_suite.rs` |
| §4.2 Anchors — recover Q≈9 R≈5 B≈N≈3 P≈1 | ✅ Q 8.71 / R 5.04 / B 3.51 / N 3.23 / P 1.0 | `cost.rs`, `tests/engine_suite.rs` |
| §4.3 Correction loop — self-play folds realized value back, anchored | ✅ logistic regression on material diffs + synergy residual fitting | `selfplay.rs::correct_values` |
| §5 Policy layer | ✅ | `game.rs` |
| §6 Acceptance — chess/xiangqi/shogi from Bits, perft-validated | ✅ chess d6=119,060,324 (+Kiwipete, CPW 3–5), xiangqi d5=133,312,995, shogi d5=19,861,490 | `variants/`, `tests/perft_acceptance.rs` |
| §7.1 Representation per board class + **Prototype 1 crossover** | ✅ **measured**: two classes (mailbox+piece-list; wide-u128 bitboard kernels w/ ray tables) behind one abstraction, perft-equivalent; mailbox wins at all tested sizes (bitboards 0.80–0.93×), confirming the spec's cited expert position — mailbox default, bitboards selectable | `game.rs::CompiledBB`, `position.rs` mirrors, `tests/representation_suite.rs`, `botboard bench` |
| §7.2 Sliding attacks | ✅ ray-scan (portable default) + mask-intersection fast path for plain kernels | `position.rs::is_attacked` |
| §7.3 Compile step, SoA stateful data | ✅ | `game.rs`, `position.rs` |
| §7.4 Learned evaluation — **Bit-derived embeddings, per-player value head, generalizes to unseen pieces** | ✅ NNUE-style accumulator net: descriptors from compiled kernels (no type ids), two-perspective accumulator, hidden layer; trained by self-play; evaluates novel random-army pieces sanely (tested). Known limit: Stage-4 CUSTOM abilities alias their nearest stdlib kin's descriptor dim (no dim of their own) — descriptor v3 would widen | `nnue.rs`, `tests/nnue_suite.rs` |
| §7.5 Zobrist — **incremental**, state buckets (moved+HP), per-cell terrain keys, hands; repetition = full-state equality; TT + move ordering + pruning | ✅ incremental key w/ O(1) unmake restore, debug-asserted vs full recompute on every make; null-move, LMR, killers, history, aspiration | `zobrist.rs`, `position.rs`, `search.rs` |
| §8.1 Belief sharpness | ✅ | `belief.rs` |
| §8.2 Ladder — rung 0 αβ+TT; rung 1 PIMC; **rung 2 OOS** (the spec's named algorithm); rung 3 search-free policy | ✅ OOS = depth-limited external-sampling MCCFR over infosets, regret matching, average-strategy root; finds forced wins at point mass, sound + deterministic under uncertainty | `search.rs`, `ladder.rs`, `oos.rs`, `tests/oos_suite.rs` |
| §8.5 Gate — entropy + pivotality, bias-to-sounder, **trained thresholds** | ✅ cheapest-sufficient-rung labels → monotone threshold fit | `ladder.rs`, `training.rs` |
| §8.6 Belief substrate — ground truth vs observed view, knowledge masks, no leakage | ✅ | `belief.rs` |
| §8.7 Multiplayer 1–4 — **committed baseline ruling for the open Tier-1 gap** | 🟡 last-royal-standing FFA: strict rotation over live players, elimination removes the army, N-player-correct check, per-player value head chooser; 3-player game tested to verdict. Alliance/collusion/kingmaking rules (the other half of §13's Tier-1 gap) deliberately out of scope | `ffa.rs`, `tests/systems_suite.rs` |
| §8.8 Recon = belief collapse → cheaper rungs; **codex persistence + rematch warm-start** | ✅ belief JSON roundtrip; rematch starts strictly sharper | `codex.rs`, `tests/systems_suite.rs` |
| §10.1 Determinism — core owns rules/state/RNG | ✅ same-seed ⇒ identical games (tested, incl. FFA and OOS) | `rng.rs` |
| §10.2 C ABI | ✅ opaque handle, coarse commands, ctypes-smoke-tested | `crates/botboard-ffi` |
| §10.6 Determinism grades + **quantization parity (named obligation)** | ✅ deterministic grade = int16/i32 fixed-point net inference, bit-exact (tested); performance grade = f32 training; parity: ≥90% chosen-move agreement, ≤20cp drift (tested); checkpoints (BBNET002, legacy BBNET001 loads) | `nnue.rs`, `tests/nnue_suite.rs` |

## Training spec (Botboard_Training_Spec.md)

| Spec | Status | Where |
|---|---|---|
| §2 Shared network — Bit-set encoder inputs, policy/value heads, cost head | ✅ one evaluator (net or anchored-cost linear) under every rung; descriptors are the Bit-set encoder; cost prior shares the same kernel-derived features | `nnue.rs`, `eval.rs`, `cost.rs` |
| §3 Reconciliation — GT-CFR family + AlphaZero cap + R-NaD cap, NeuRD=softmax-CFR | ✅ OOS (CFR family, search-based) + alpha-beta cap + NeuRD policy head with **R-NaD reward transformation** r′=r−η·log(π/π_reg) and FoReL regularization, all on the shared substrate — the rung-3 net-sharing prototype | `oos.rs`, `training.rs` |
| §4 Continuum curriculum | ✅ cold-open ↔ revealed self-play; rematch sampling via codex warm-starts | `selfplay.rs`, `codex.rs` |
| §5 Population/league, diversity, Nash averaging | ✅ | `league.rs` |
| §6 Gate training | ✅ rung-agreement labels, monotone conservative fit | `training.rs` |
| §7 Co-evolution — generate → measure → select → correct | ✅ random-army generation priced under budget; value + synergy correction | `selfplay.rs`, `cost.rs` |
| §8 Libraries & profiles — versioned, core-owned | ✅ net checkpoints (BBNET002; BBNET001 loads via legacy remap), league profiles JSON, codex JSON | `nnue.rs`, `league.rs`, `codex.rs` |
| §9 Two deployments over one C ABI | ✅ CLI game side + ctypes training side | `botboard-ffi`, `botboard-cli` |
| §10 Infrastructure — actors/learners | ✅ in-process **parallel actor pool** (thread-scoped, deterministic per seed at any thread count — tested); distributed multi-machine deployment is an ops scale-out of the same loop | `selfplay.rs::parallel_selfplay` |
| §11 Evaluation — cost gate, rung consistency, parity, population health | ✅ all tested | test suites |

## Bits 2.0 — effects/terrain/moves as data (docs/bits2-effects-as-data.md)

| Stage | Status | Where |
|---|---|---|
| 1 — micro-op interpreter in make/unmake; ability registry; unified undo (op log) | ✅ behavior-preserving, perft trio flat, dominant-script fast path kept | `ops.rs`, `effects.rs` |
| 2 — generation as data: move scripts, gates/bindings/sources; MoveKind demoted to display code | ✅ behavior-preserving; perf within the 3% gate | `move_defs.rs`, `movegen.rs`, `game.rs` |
| 3 — terrain as registry rows (blocks/on-land/carry/conceal); hot predicates stay range-compares | ✅ behavior-preserving; compile-time proof rows ≡ derived forms | `terrain_defs.rs` |
| 4 — JSON authoring: custom ability + terrain rows over the SRW setup (`"abilities"`/`"terrains"`), FFI-validated | ✅ core + FFI + suites (`stage4_suite.rs`, srw_suite Stage-4 block) | `effects.rs::CustomAbility`, `terrain_defs.rs::CustomTerrain`, `game.rs::with_customs`, `srw.rs` |
| 4 — descoped remainder: custom MOVE scripts (`moves.json`); Maker-Mode composition UX | ⬜ moves stay engine-side scripts; Maker Mode is the SRW client's layer | see "as built" notes in the design doc |

## Scale notes (honest boundaries)

The architecture is complete and every committed decision has a tested
realization. Numbers scale with compute, not code: the shipped net is small
(H=32) and trained on thousands—not millions—of games; OOS runs depth-capped;
the league is 4 members. Growing those is configuration + hardware on the
same loops. Distributed multi-machine orchestration and GPU-batched inference
remain ops work outside the engine's semantics.

**Concrete scale ladder (July 2026 plan, in effort order):**

1. **Bigger corpus, same loop** — ✅ DONE (July 2026): v3 trained on a
   10× corpus (2000 games / 12 epochs, 56k samples) and promoted after
   beating v2 9–11–0 (72.5%) in the new paired-opening `netmatch` gate
   (`artifacts/chess_net_v3.bin`). H stays the compile-time 32; making
   H runtime-sized (Vec-backed nets) is the prerequisite for true
   width scaling and belongs with rung 5's inference work.
2. **Wider league** (config): grow to 12–16 members with the §5 pool so
   Nash averaging has spread; feeds better value targets for (1).
3. **SRW-content curriculum** — ✅ pipes DONE / net promotion OPEN
   (July 2026): descriptor v2
   (D 12→21) gives the net per-ability-kind signals (heal/laser+pierce/
   wall-pit/mine/resurrect/hack) plus the Appendix-B flags (stealth,
   flight, hologram, EMP radius); checkpoints bumped to BBNET002 with
   exact legacy loading of BBNET001 (surviving rows remap, new rows
   zero). `random_robot_army` samples the full vocabulary at clan-
   palette-ish rates under budget; `train-net srw` trains over freshly
   sampled defs per game and runs a paired-army promotion probe (net
   vs linear teacher, colors swapped, ≥55% to PROMOTE). The SRW FFI
   setup accepts `"net": <checkpoint>` — quantized per battle GameDef,
   bad paths fail the build. Honest numbers: `srw_net_v1.bin`
   (400 games / 8 epochs, 8k samples) probed 3–18–3 = 50.0% **HOLD**;
   a 2000/12 run probed 37.5%, worse — the linear teacher is a strong
   baseline on armies its own cost prior priced, so promotion waits on
   a better recipe (deeper teacher games, value targets past 160
   plies, or H growth), not more of the same corpus. The pipes are
   what this rung delivers. See `nnue.rs`, `selfplay.rs`,
   `botboard-ffi/src/srw.rs`, `artifacts/srw_net_v1.bin`.
4. **Process-parallel farm** (ops): N engine processes with disjoint
   seed ranges appending to a shared game store; the deterministic
   per-seed actor pool makes shard merges trivially reproducible.
   Multi-machine is the same recipe over rsync/NFS.
5. **GPU-batched inference** (last, biggest): only worth it after (1)
   makes nets big enough to starve the CPU int path.

Rungs 1–2 are pure configuration and can run unattended overnight;
rung 3 is the first one that touches code and is the highest-leverage
for SRW play quality.

## Open goals (audited August 2026)

Everything above marked ✅ is built and tested. This section is the
honest remainder: spec promises with no code yet. None are regressions —
each is a deliberate deferral, listed so it can't silently vanish.

**Engine spec (Botboard_Spec_v4.md):**

- **§8.3 unified GT-CFR searcher** — the spec's one parameterized core
  sliding expand-1 ↔ expand-top-k by belief sharpness. As built, the
  ladder is four separate algorithms behind the trained gate
  (`ladder.rs` calls the interpolation "the Phase-2+ growth path").
- **§7.5/§8.8 mid-game cache promotion** — infoset-keyed → full-state
  keys as belief sharpens. Both tables exist (`search.rs` TT,
  `oos.rs` infosets); the promotion bridge does not.
- **§7.4/§13 NNUE-vs-GNN comparison** — the "load-bearing empirical
  question" was never measured; NNUE shipped as the committed default.
- **§8.7/§13 alliance rules** — FFA baseline is committed;
  alliance/collusion/kingmaking (the other half of the Tier-1 gap)
  deliberately out of scope.
- **§3.4 multi-ply turn policies; §3.4/§5 pass-allowed + N-pass
  termination guard** — `TurnPolicy::Alternate` / `PassPolicy::Forbidden`
  are the only variants.
- **§7.1 AVX2/AVX-512 board classes (129–512 cells)** — only the
  portable u128 class and mailbox exist; the crossover was measured on
  u128 only, and mid-size boards fall to mailbox.
- **§10.2 PyTorch/PyO3 performance-grade trainer** — all training is
  in-process Rust; the C-ABI obligation is met via ctypes, the Python
  trainer is unrealized (and so far unneeded).

**SRW surface (Subterranean_Robot_Wars_Spec_v3.md):**

- **Spy ability (§7, §10)** — no active "spend an action to reveal"
  exists; belief collapse is setup-time `intel` or passive observation.
  (`bits.rs` reserves the hook: "hack/laser/spy reuse it later".)
- **Codex unwired (§10, §11)** — `codex.rs` (belief JSON roundtrip,
  rematch warm-start) is core-only; the SRW FFI has no belief
  export-after / import-before-battle calls, so cross-battle recon
  persistence can't reach the campaign layer yet.
- **Controller capture → army transfer (§7)** — elimination only today;
  no primitive supports reassigning a captured side's pieces.
- **N-player battles run the FFA baseline, not the belief ladder** —
  §10's information mechanics fully apply to 2-side battles only.
- **The spec doc trails the implementation** — custom
  abilities/terrains (Stage 4), vocab batches 2–3 (drill, blocks,
  conveyors, swap/push, HP gates, grasshopper/locust, short riders),
  and per-battle `"net"` checkpoints are all beyond-spec and
  undocumented there; needs a spec v4 catch-up pass.
- Campaign/run/meta loops (§4, §5, §11), room archetypes (§6), clans
  and alignment (§9) live in the C#/Godot layer
  (`~/Workspace/Projects/SubterraneanRobotWars`), not this repo; the
  engine primitives they need (dials, drops, terrain gates, pricing)
  are all built and tested.

**Training follow-ups (docs/training-report.md):**

- Renormalize the logistic scale k before ever shipping the scaled-up
  correction as material tables (report's standing advisory).
- Cross-palette synergy refit + SRW net promotion recipe — rung 3's
  open half: `srw_net_v1` probed 50% HOLD; the recipe (deeper teacher
  games, value targets past 160 plies, or H growth) is the work, not
  more corpus.

## Next phases (proposed order, August 2026)

1. **Wire the codex + add spy** — small, high-leverage for SRW: FFI
   belief export/import (the core already does the work) plus a spy
   ability row (active belief collapse). Together they unlock the
   campaign recon arc (§10–§11) end to end.
2. **SRW spec v4 catch-up** — document the authoring surface (Stage 4
   customs, batches 2–3, `"net"`), and rule on spy/army-transfer scope
   while in there.
3. **SRW net promotion recipe** (scale rung 3's open half) — highest
   leverage for play quality; try deeper teacher games and longer
   value targets before H growth.
4. **League widening** (scale rung 2) — pure config, overnight run.
5. **Bits 2.0 tail, on demand** — custom MOVE scripts and descriptor
   v3 only when Maker Mode / custom-content eval actually need them;
   army-transfer primitive when the campaign layer asks.
6. **Scale rungs 4–5** (process farm, GPU inference) — later, after
   nets are big enough to matter.

## UI

`botboard` CLI: `play` (interactive vs AI; `--hidden` for the imperfect-info
ladder; `--net` for the deterministic-grade net), `selfplay`, `train-net`,
`cost`, `league`, `armies`, `show`, `perft`, `divide`, `bench`.
