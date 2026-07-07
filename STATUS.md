# Botboard — Implementation Status

Traceability from **Botboard_Spec_v4.md** and **Botboard_Training_Spec.md** to code.
Everything listed as *implemented* is exercised by `cargo test --release`
(deep perft and statistical matches behind `-- --ignored`).

## Engine spec (Botboard_Spec_v4.md)

| Spec | Status | Where |
|---|---|---|
| §3.1 Axis A — leaper/rider/hopper, direction, mode, path (lame leapers), zone conditions, target predicates | ✅ | `bits.rs`, `geometry.rs`, compiled in `game.rs` |
| §3.2 Axis B — capture fates (destroy / to-hand), transformation with base-type de-promotion, forced-if-immobile | ✅ | `game.rs` (PromoRule), `position.rs` (make/unmake) |
| §3.2 stateful effects — hit-count armor (strikes), heal, spawn (walls/pits), laser-at-range with coupled retreat | ✅ SoA HP arrays; ammo/cooldown extend the same bucket | `position.rs`, `bits.rs::AbilityBit`, `tests/abilities_suite.rs` |
| §3.3 Special moves — castling, en passant, double-step; drops with 3 legality tiers (*nifu*, *uchifuzume*) | ✅ | `movegen.rs` |
| §3.4 Turn model — one turn = one action, strict rotation, pass forbidden by default; compound moves piece-local, atomic, priced | ✅ incl. overclock ⟨move,move,−1HP⟩ compounds and ability actions; laser retreat "nerf has teeth" | `moves.rs`, `movegen.rs::gen_overclock/gen_abilities`, `tests/abilities_suite.rs` |
| §4 Cost model — C_prior formula, mobility integral, floor, utility/nerf hooks, synergy hook | ✅ (synergy weights ship 0 until self-play fits them) | `cost.rs` |
| §4.2 Anchors — recover Q≈9 R≈5 B≈N≈3 P≈1 | ✅ Q 8.71 / R 5.04 / B 3.51 / N 3.23 / P 1.0 | `cost.rs`, test `cost_prior_recovers_classical_ordering` |
| §4.3 Correction loop — self-play folds realized value back, anchored | ✅ miniature (logistic regression on material diffs) | `selfplay.rs::correct_values` |
| §5 Policy layer — royalty attr, stalemate, capture-fate, turn/pass, named predicates | ✅ | `game.rs`, predicates as flags on types |
| §6 Acceptance — chess/xiangqi/shogi from Bits, perft-validated | ✅ chess d6=119,060,324 (+ Kiwipete, CPW 3–5), xiangqi d5=133,312,995, shogi d5=19,861,490 | `variants/`, `tests/perft_acceptance.rs` |
| §7.1 Representation per board class | ✅ mailbox+piece-list class (Phase-0 per roadmap); SIMD-bitboard class & crossover = Prototype 1, pending | `position.rs` |
| §7.2 Sliding attacks | ✅ ray-scan (the portable default); magic/PEXT are small-board optimizations, pending | `position.rs::is_attacked` |
| §7.3 Compile step, SoA stateful data | ✅ compile; SoA arrives with stateful Bits | `game.rs::compile` |
| §7.4 Learned evaluation, per-player value head | ✅ seed: integer material(=cost)+mobility linear eval, per-player vector; NNUE+Bit-embeddings is the scale-up | `eval.rs` |
| §7.5 Zobrist w/ state buckets (moved+HP), per-cell terrain keys, hands, stm; repetition = full-state equality | ✅ tested: HP and terrain change the key; unmake restores it | `zobrist.rs`, `search.rs`, `tests/abilities_suite.rs` |
| §8.1 Belief sharpness (entropy dial) | ✅ | `belief.rs::entropy` |
| §8.2 Ladder — rung 0 αβ+TT; rung 1 PIMC; rung 2 ISMCTS; rung 3 search-free policy | ✅ all four dispatchable; GT-CFR interpolation core = Phase-2 growth | `search.rs`, `ladder.rs` |
| §8.5 Gate — entropy + pivotality, bias-to-sounder | ✅ threshold gate; learned meta-controller pending | `ladder.rs::gate` |
| §8.6 Belief substrate — ground truth vs observed view, per-piece knowledge masks, no leakage | ✅ hidden-identity regime; per-Bit masks arrive with Phase-3 stealth | `belief.rs` |
| §8.7 Multiplayer N>2 | 🔲 open Tier-1 gap per spec §13 | model supports N sides; victory rules unresolved |
| §8.8 Recon = belief collapse → cheaper rungs | ✅ observed in test `hidden_game_runs_the_ladder_and_reveals` | `selfplay.rs::play_hidden_game` |
| §10.1 Determinism — core owns rules/state/RNG | ✅ integer-only rules, seeded SplitMix64, same-seed ⇒ identical game (tested) | `rng.rs` |
| §10.2 C ABI — opaque handle, coarse commands | ✅ `cdylib` + smoke-tested via ctypes | `crates/botboard-ffi` |
| §10.6 Determinism grades | ✅ deterministic grade (integer eval) is the only shipped path; float exists only in offline fitting math | `eval.rs`, `cost.rs` |
| §12 Prototype 1 (representation crossover benchmark) | ◐ harness shipped (`botboard bench` measures kn/s per board class); the wide-SIMD bitboard second class plugs into it to find the crossover | `botboard-cli` |

## Training spec (Botboard_Training_Spec.md)

| Spec | Status | Where |
|---|---|---|
| §2 Shared network (CVPN) | ✅ seed: one shared evaluator w/ per-player head + cost head (same table) serving every rung; neural CVPN is the scale-up | `eval.rs` |
| §3 Reconciliation (GT-CFR / AlphaZero / R-NaD family) | ✅ miniature: the rung-3 net-sharing prototype is implemented — a NeuRD-style regularized policy head (FoReL pull, R-NaD outer iteration) refining the *shared* evaluator's move scores, trained by hidden self-play; regret matching in `league.rs`; full neural GT-CFR is the farm-scale growth | `training.rs::PolicyHead/train_policy_selfplay` |
| §4 Continuum curriculum | ✅ miniature: cold-open ↔ revealed self-play both exercised; rematch belief persistence pending | `selfplay.rs` |
| §5 Population / league, diversity, Nash averaging | ✅ personalities → round-robin meta-payoff → regret-matching Nash mixture + ratings | `league.rs` |
| §6 Gate training | ✅ miniature: cheapest-sufficient-rung labels from rung agreement on sampled hidden positions; conservative threshold fit (escalate when uncertain) | `training.rs::collect_gate_samples/fit_gate_thresholds` |
| §7 Co-evolution — generate → measure → correct | ✅ miniature: `random_army` (generate, priced, budget-packed) + `correct_values` (measure/correct, anchored) | `selfplay.rs` |
| §8 Libraries & profiles — versioned, core-owned serialization | ✅ JSON profiles + meta-payoff (`league_profiles.json`) | `league.rs::profiles_json` |
| §9 Two deployments over one C ABI | ✅ same surface serves CLI (game side) and ctypes harness (training side) | `botboard-ffi` |
| §10 Infrastructure (distributed farm, GPU batching) | 🔲 offline scale-up | single-process loops shipped |
| §11 Evaluation — cost-model gate, rung consistency, population health | ◐ cost gate + RPS-cycle Nash test shipped; exploitability metrics pending | tests |

## UI

`botboard` CLI (simple UI per current direction): `play` (interactive vs AI,
`--hidden` for the imperfect-info ladder with masked view), `selfplay`,
`cost`, `league`, `armies`, `show`, `perft`, `divide`.
