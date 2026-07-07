# Botboard — Implementation Status

Traceability from **Botboard_Spec_v4.md** and **Botboard_Training_Spec.md** to code.
Everything listed as *implemented* is exercised by `cargo test --release`
(deep perft and statistical matches behind `-- --ignored`; debug builds
additionally assert incremental hash/mirror consistency on every make).

## Engine spec (Botboard_Spec_v4.md)

| Spec | Status | Where |
|---|---|---|
| §3.1 Axis A — leaper/rider/hopper, direction, mode, path (lame leapers), zone conditions, target predicates | ✅ | `bits.rs`, `geometry.rs`, compiled in `game.rs` |
| §3.2 Axis B — capture fates, transformation w/ base-type de-promotion, forced-if-immobile, hit-count armor (strikes), heal, spawn (walls/pits), laser-at-range with coupled retreat | ✅ SoA HP arrays; ammo/cooldown extend the same state bucket | `position.rs`, `bits.rs::AbilityBit`, `tests/abilities_suite.rs` |
| §3.3 Special moves — castling, en passant, double-step; drops with 3 legality tiers (*nifu*, *uchifuzume*) | ✅ | `movegen.rs` |
| §3.4 Turn model — one turn = one action, strict rotation, pass forbidden; compound moves piece-local, atomic, priced | ✅ incl. overclock ⟨move,move,−1HP⟩ compounds, ability actions, laser-retreat "nerf has teeth" | `moves.rs`, `movegen.rs`, `tests/abilities_suite.rs` |
| §4 Cost model — C_prior, mobility integral, utility multipliers (armor 1+0.5(HP−1), overclock ×1.8), nerfs, floor, **synergy term with fitting loop** | ✅ synergy S_ij over Bit categories, SGD-fitted from residuals, recovers planted interactions | `cost.rs::SynergyModel`, `tests/systems_suite.rs` |
| §4.2 Anchors — recover Q≈9 R≈5 B≈N≈3 P≈1 | ✅ Q 8.71 / R 5.04 / B 3.51 / N 3.23 / P 1.0 | `cost.rs`, `tests/engine_suite.rs` |
| §4.3 Correction loop — self-play folds realized value back, anchored | ✅ logistic regression on material diffs + synergy residual fitting | `selfplay.rs::correct_values` |
| §5 Policy layer | ✅ | `game.rs` |
| §6 Acceptance — chess/xiangqi/shogi from Bits, perft-validated | ✅ chess d6=119,060,324 (+Kiwipete, CPW 3–5), xiangqi d5=133,312,995, shogi d5=19,861,490 | `variants/`, `tests/perft_acceptance.rs` |
| §7.1 Representation per board class + **Prototype 1 crossover** | ✅ **measured**: two classes (mailbox+piece-list; wide-u128 bitboard kernels w/ ray tables) behind one abstraction, perft-equivalent; mailbox wins at all tested sizes (bitboards 0.80–0.93×), confirming the spec's cited expert position — mailbox default, bitboards selectable | `game.rs::CompiledBB`, `position.rs` mirrors, `tests/representation_suite.rs`, `botboard bench` |
| §7.2 Sliding attacks | ✅ ray-scan (portable default) + mask-intersection fast path for plain kernels | `position.rs::is_attacked` |
| §7.3 Compile step, SoA stateful data | ✅ | `game.rs`, `position.rs` |
| §7.4 Learned evaluation — **Bit-derived embeddings, per-player value head, generalizes to unseen pieces** | ✅ NNUE-style accumulator net: descriptors from compiled kernels (no type ids), two-perspective accumulator, hidden layer; trained by self-play; evaluates novel random-army pieces sanely (tested) | `nnue.rs`, `tests/nnue_suite.rs` |
| §7.5 Zobrist — **incremental**, state buckets (moved+HP), per-cell terrain keys, hands; repetition = full-state equality; TT + move ordering + pruning | ✅ incremental key w/ O(1) unmake restore, debug-asserted vs full recompute on every make; null-move, LMR, killers, history, aspiration | `zobrist.rs`, `position.rs`, `search.rs` |
| §8.1 Belief sharpness | ✅ | `belief.rs` |
| §8.2 Ladder — rung 0 αβ+TT; rung 1 PIMC; **rung 2 OOS** (the spec's named algorithm); rung 3 search-free policy | ✅ OOS = depth-limited external-sampling MCCFR over infosets, regret matching, average-strategy root; finds forced wins at point mass, sound + deterministic under uncertainty | `search.rs`, `ladder.rs`, `oos.rs`, `tests/oos_suite.rs` |
| §8.5 Gate — entropy + pivotality, bias-to-sounder, **trained thresholds** | ✅ cheapest-sufficient-rung labels → monotone threshold fit | `ladder.rs`, `training.rs` |
| §8.6 Belief substrate — ground truth vs observed view, knowledge masks, no leakage | ✅ | `belief.rs` |
| §8.7 Multiplayer 1–4 — **committed baseline ruling for the open Tier-1 gap** | ✅ last-royal-standing FFA: strict rotation over live players, elimination removes the army, N-player-correct check, per-player value head chooser; 3-player game tested to verdict | `ffa.rs`, `tests/systems_suite.rs` |
| §8.8 Recon = belief collapse → cheaper rungs; **codex persistence + rematch warm-start** | ✅ belief JSON roundtrip; rematch starts strictly sharper | `codex.rs`, `tests/systems_suite.rs` |
| §10.1 Determinism — core owns rules/state/RNG | ✅ same-seed ⇒ identical games (tested, incl. FFA and OOS) | `rng.rs` |
| §10.2 C ABI | ✅ opaque handle, coarse commands, ctypes-smoke-tested | `crates/botboard-ffi` |
| §10.6 Determinism grades + **quantization parity (named obligation)** | ✅ deterministic grade = int16/i32 fixed-point net inference, bit-exact (tested); performance grade = f32 training; parity: ≥90% chosen-move agreement, ≤20cp drift (tested); checkpoints (BBNET001) | `nnue.rs`, `tests/nnue_suite.rs` |

## Training spec (Botboard_Training_Spec.md)

| Spec | Status | Where |
|---|---|---|
| §2 Shared network — Bit-set encoder inputs, policy/value heads, cost head | ✅ one evaluator (net or anchored-cost linear) under every rung; descriptors are the Bit-set encoder; cost prior shares the same kernel-derived features | `nnue.rs`, `eval.rs`, `cost.rs` |
| §3 Reconciliation — GT-CFR family + AlphaZero cap + R-NaD cap, NeuRD=softmax-CFR | ✅ OOS (CFR family, search-based) + alpha-beta cap + NeuRD policy head with **R-NaD reward transformation** r′=r−η·log(π/π_reg) and FoReL regularization, all on the shared substrate — the rung-3 net-sharing prototype | `oos.rs`, `training.rs` |
| §4 Continuum curriculum | ✅ cold-open ↔ revealed self-play; rematch sampling via codex warm-starts | `selfplay.rs`, `codex.rs` |
| §5 Population/league, diversity, Nash averaging | ✅ | `league.rs` |
| §6 Gate training | ✅ rung-agreement labels, monotone conservative fit | `training.rs` |
| §7 Co-evolution — generate → measure → select → correct | ✅ random-army generation priced under budget; value + synergy correction | `selfplay.rs`, `cost.rs` |
| §8 Libraries & profiles — versioned, core-owned | ✅ net checkpoints (BBNET001), league profiles JSON, codex JSON | `nnue.rs`, `league.rs`, `codex.rs` |
| §9 Two deployments over one C ABI | ✅ CLI game side + ctypes training side | `botboard-ffi`, `botboard-cli` |
| §10 Infrastructure — actors/learners | ✅ in-process **parallel actor pool** (thread-scoped, deterministic per seed at any thread count — tested); distributed multi-machine deployment is an ops scale-out of the same loop | `selfplay.rs::parallel_selfplay` |
| §11 Evaluation — cost gate, rung consistency, parity, population health | ✅ all tested | test suites |

## Scale notes (honest boundaries)

The architecture is complete and every committed decision has a tested
realization. Numbers scale with compute, not code: the shipped net is small
(H=32) and trained on thousands—not millions—of games; OOS runs depth-capped;
the league is 4 members. Growing those is configuration + hardware on the
same loops. Distributed multi-machine orchestration and GPU-batched inference
remain ops work outside the engine's semantics.

## UI

`botboard` CLI: `play` (interactive vs AI; `--hidden` for the imperfect-info
ladder; `--net` for the deterministic-grade net), `selfplay`, `train-net`,
`cost`, `league`, `armies`, `show`, `perft`, `divide`, `bench`.
