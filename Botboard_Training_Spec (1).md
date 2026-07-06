# Botboard — Training & Self-Play Specification

**The self-play subsystem that evolves both pieces and opponents — serving the engine first, and the game later.**

`Version 0.1 · June 2026`

> **The third spec.** Companion to the Botboard Technical Specification v0.3 (cited *Botboard Spec §N*) and the Subterranean Robot Wars Spec v0.3 (cited *SRW Spec §N*), completing the engine / game / **training** triad. It resolves the training gap flagged in *Botboard Spec §9*, and it deliberately ties back to the project's founding vision: *millions of self-play games that evolve both the AI opponents (with varied personalities) and the pieces and armies themselves, saved into libraries and profiles* — and to the requirement that training is **part of the engine, and a feature of the game later as well**.

---

## Table of Contents

1. [The Vision and What Training Must Deliver](#1-the-vision-and-what-training-must-deliver)
2. [The Shared Network](#2-the-shared-network)
3. [The Reconciliation: One Regret-Minimization Family](#3-the-reconciliation-one-regret-minimization-family)
4. [Training Across the Information Continuum](#4-training-across-the-information-continuum)
5. [The Population: Diverse-Personality Opponents](#5-the-population-diverse-personality-opponents)
6. [The Gate's Training](#6-the-gates-training)
7. [The Co-Evolution Loop: Pieces *and* Opponents](#7-the-co-evolution-loop-pieces-and-opponents)
8. [Libraries & Profiles](#8-libraries--profiles)
9. [Two Deployments: Training in the Engine *and* the Game](#9-two-deployments-training-in-the-engine-and-the-game)
10. [Infrastructure](#10-infrastructure)
11. [Evaluation](#11-evaluation)
12. [Roadmap & Phasing](#12-roadmap--phasing)
13. [Risks & Open Questions](#13-risks--open-questions)
- [References](#references)
- [Appendix — Provenance & Changelog](#appendix--provenance--changelog)

---

## 1. The Vision and What Training Must Deliver

Training in Botboard is not a one-off step that builds an AI and then stops. It is a **continuous subsystem** and, ultimately, a **product feature**. The founding vision is explicit: create lots of random armies and random AI opponents with different personalities, evolve and train them across millions of games — *evolving both the opponents and the pieces/armies themselves* — and save them into libraries and profiles; and let players, in maker mode, train and simulate their own creations.

From that, the training subsystem owes **five deliverables**:

1. **The shared network** that plays across the whole information continuum (*Botboard Spec §8*).
2. **Measured piece values** that close the cost-model loop (*Botboard Spec §4*).
3. **A population of diverse-personality opponents** — the vision's "different personalities in strategy/tactics."
4. **Validated, balanced procedural content** (armies, dungeons) before it reaches a player (*SRW Spec §13*).
5. **Persistent libraries** of networks, armies/pieces, and opponent profiles — reusable assets.

And it serves **one pipeline, two deployments**: the engine side (the offline distributed farm that does the heavy lifting) and the game side (maker-mode self-training, roguelike content balancing, personalized opponents). The same C ABI and determinism principle make a single training engine serve both (*Botboard Spec §10.1–10.2*).

---

## 2. The Shared Network

The committed brain (*Botboard Spec §8.3*) is a single **counterfactual value-and-policy network (CVPN)**:

- **Inputs:** the observed board + terrain; the belief/mask state (revealed-rule masks, concealment, belief-sharpness features) (*Botboard Spec §8.6*); and, per piece, a **learned embedding of its Bit-set** rather than a one-hot type id (*Botboard Spec §7.4*).
- **Outputs:** a **policy prior**; **counterfactual / expected values** (the quantity both CFR-family search and model-free Nash consume); a **per-player value head** (multiplayer is not zero-sum); and the **cost-prior head** (the same Bit-set encoder that emits piece cost, *Botboard Spec §4*).

Why a CVPN superset: GT-CFR search needs counterfactual values, R-NaD/NeuRD operates on counterfactual values/advantages, and AlphaZero needs value + policy — one network with these heads serves all three (§3).

> **No information leakage.** Each agent is fed only its **information set** — the observed state plus belief features, never another player's ground truth. This is enforced on the mask substrate (*Botboard Spec §8.6*); training-time serialization carries the same discipline, so the learner never sees hidden information it would not have at play time.

The same network is shared across every rung of the search ladder and across the population (§5), which expresses personalities either as distinct population members or via a **personality latent** the network is conditioned on.

---

## 3. The Reconciliation: One Regret-Minimization Family

The load-bearing question from *Botboard Spec §9* — can one network serve the search-based CFR/GT-CFR trainer *and* the model-free R-NaD trainer? — resolves **yes**, on a theoretical result rather than hope:

> **NeuRD, the policy engine inside R-NaD/DeepNash, is formally equivalent to softmax counterfactual regret minimization.** It reduces to multiplicative-weights/Hedge in the single-state case, and in partially-observable games it can directly replace regret-matching inside CFR while keeping CFR's convergence guarantees. R-NaD's regularization (Follow-the-Regularized-Leader) sits in the same family.

So GT-CFR (search-based, sharp belief) and R-NaD/NeuRD (search-free, intractable belief) are **the same regret-minimization framework on the same counterfactual values**, differing only in whether a search tree is present. The shared net's counterfactual values + policy are the common substrate that makes one network coherent.

**The committed training backbone:**

| Regime | Trainer | Role |
|---|---|---|
| Rungs 0–2 (revealed → partial) | **GT-CFR sound self-play** (Student-of-Games style) | Primary trainer. Trains the CVPN from outcomes **plus recursive sub-searches**; natively spans perfect↔imperfect via expand-1 ↔ expand-top-k. One procedure → one net good across most of the ladder. |
| Rung 0 cap (point mass) | **AlphaZero-style targets** | Sharpens perfect-info play (value + policy from PUCT visit counts); a special case of GT-CFR (expand-1). |
| Rung 3 cap (intractable) | **R-NaD / NeuRD** with FoReL regularization | Refines the **same net's policy** model-free toward ε-Nash where search is too expensive. Because NeuRD = softmax-CFR, this refines the shared net rather than fighting it. |

**The rung-3 decision, resolved:** rung 3 uses the **same network's policy head**, refined by R-NaD regularization — *not* a separate network. This is justified by the NeuRD↔CFR equivalence, and is flagged as the single most important assumption to confirm in a prototype (§13).

**Honest caveat (carried from the research):** model-free Nash policy-gradient methods (NeuRD, F-FoReL) lack high-probability convergence proofs when sampling trajectories from the policy and can underperform search on small/tractable games — but scale to huge trees, which is why DeepNash used them for Stratego. Hence the design: **search where affordable, R-NaD only at the intractable cap.** The unifying signal throughout is the counterfactual value/advantage; the unifying objective is regret minimization with FoReL-style regularization.

---

## 4. Training Across the Information Continuum

The net must be calibrated at **every belief sharpness**, so belief sharpness is a first-class *training variable* — the curriculum controls it, rather than letting self-play sample it incidentally.

**Generating positions at controlled belief sharpness:**

- **Rematch / replay sampling** — pair armies that have "met" before (sharp belief) against fresh pairings (broad belief), by **persisting belief/codex across self-play episodes** so rematches occur on purpose.
- **Revelation injection** — spy/reveal events and partial-recon scenarios populate the *middle* of the axis.
- **Point-mass episodes** — fully-revealed games (all movesets known) train rung 0.
- **Cold-open episodes** — fully-hidden, Stratego-like starts train rung 3.

**Curriculum weighting** biases toward the revealed / common-campaign regime — where the engine spends most of its time, and where GT-CFR is otherwise weakest relative to specialized AlphaZero — while maintaining enough hidden-end coverage to keep the unexploitable policy sound. This directly manages the perfect-vs-imperfect **interference** risk (§13).

---

## 5. The Population: Diverse-Personality Opponents

Plain self-play (always training against the latest/self copy) **fails on non-transitive games**, and heterogeneous fairy armies are almost certainly non-transitive (army A beats B beats C beats A). A **population/league is therefore necessary, not a nicety** — both for robust play and for meaningful balance measurement.

```
            ┌──────────────────────────── LEAGUE / POPULATION ────────────────────────────┐
            │  main agents ──┐                                                              │
            │  exploiters ───┼─► co-trained; each a best response to the meta-strategy      │
            │  past members ─┘    diversity objective in the oracle → distinct STYLES        │
            └──────────────┬───────────────────────────────────────────────────────────────┘
                           ▼  solve restricted game on the meta-payoff matrix
                  Nash averaging  ──►  principled agent / army value (handles non-transitivity)
                           ▼
                  personality profiles  ──►  library (§8)  ──►  the game's clans & opponents
```

- **Method:** **PSRO / ongoing league training** — iteratively add approximate best responses to the meta-strategy, co-train a league of main agents and exploiters (AlphaStar-style), and solve a restricted Nash over the meta-game.
- **Personality = behavioral diversity.** Explicit diversity objectives (behavioral and response diversity) in the best-response/oracle step yield agents with distinct strategic and tactical styles — the vision's "different personalities." Styles can be **conditioned** (a personality latent the shared net consumes, so one network expresses many styles) or held as **distinct members**; both are supported.
- **Nash averaging** on the meta-game gives agent strength and army value beyond raw win rate — the principled valuation the cost model needs (§7), precisely because the matchup graph is non-transitive.
- **Output:** a **library of opponent profiles** (personality latent / member + signature army + competence tier), persisted for the game (§8, §9).

---

## 6. The Gate's Training

The belief-sharpness **gate** (*Botboard Spec §8.5*) that selects which rung to run is a small **learned meta-controller**.

- **Label generation:** during self-play, for sampled positions, record the **cheapest rung whose decision matches the soundest (most expensive) rung's decision within a tolerance** — that is the cheapest *sufficient* rung. The label is that rung; the features are the belief signals (entropy, an estimate of the disambiguation factor, pivotality of the residual uncertainty).
- **Objective:** predict the cheapest-sufficient rung from belief features, i.e. best value-per-compute.
- **Safe default:** when uncertain, bias toward the **next-sounder** rung. The asymmetry is context-dependent — unsoundness is the worse error in ranked/competitive play; the reverse can hold for throughput-critical self-play, where the gate may bias cheaper.

---

## 7. The Co-Evolution Loop: Pieces *and* Opponents

The founding vision's core — evolving **both** opponents and pieces — is a single coupled loop, not two:

```
        ┌─────────────────────────────────────────────────────────────────────┐
        │                         CO-EVOLUTION LOOP                            │
        │                                                                       │
        │   generate armies/pieces ──► self-play (shared net + population)      │
        │        ▲                              │                               │
        │        │                              ▼                               │
        │   costed prior  ◄── cost model   measured value (Nash-averaged,       │
        │   + anchors         correction       perfect-info rung, anchored)     │
        │        │                              │                               │
        │        └──────────────────────────────┘                               │
        │                                                                       │
        │   league/PSRO evolves the OPPONENTS in parallel, on the same games    │
        └─────────────────────────────────────────────────────────────────────┘
```

- **Opponents evolve** via the league/PSRO (§5).
- **Pieces/armies evolve** by: *generate* (from the costed prior) → *measure* (Nash-averaged value on the meta-game) → *select/mutate* → *correct* the cost prior (*Botboard Spec §4*) → *regenerate*.
- The shared net + population are what *play and measure*; the cost model + anchors turn measured value into a costed prior; army generation uses that prior. One loop, two evolving things.

> **Piece-value measurement protocol (resolves "which rung measures values").** Values are measured at the **point-mass / perfect-information rung** for clean attribution without belief noise, **Nash-averaged over the population** to respect non-transitivity, and **anchored** (*Botboard Spec §4.2*) to fix the scale. The acceptance test — recovering classical piece values — is run on this protocol.

**Output:** evolving **libraries of armies and pieces with measured costs**, plus the meta-game that valued them.

---

## 8. Libraries & Profiles

Persistence is what turns trained results into **reusable assets** (the vision's "saved into libraries/profiles"). Persisted artifacts:

| Artifact | Contents |
|---|---|
| Network checkpoints | the shared CVPN, versioned |
| Population | personality latents / league members and their parameters |
| Opponent profiles | personality + signature army + competence tier (the game's clans draw from these) |
| Army/piece libraries | constructed Bots with measured costs and provenance |
| Meta-game tables | the empirical payoff matrix + Nash-averaging results |
| Cost model | learned weights + anchors |
| Belief/codex priors | per-army priors for rematch warm-starts (*Botboard Spec §8.8*) |

Ownership and format: the **Rust core owns serialization, RNG, and determinism** (*Botboard Spec §10.1*); profiles are **versioned**; the **C ABI exposes load/save** so both the offline farm and the game read/write the same libraries. These libraries are the substrate the roguelike's clans, the maker's creations, and the marketplace's tradeable pieces all draw from.

---

## 9. Two Deployments: Training in the Engine *and* the Game

This is the explicit requirement that training is **part of the engine but also the game later**. One subsystem, two deployments, bridged by the engine's C ABI serving both (*Botboard Spec §10.2*).

```
        ENGINE SIDE (offline R&D)                 GAME SIDE (in-product)
  ┌───────────────────────────────┐      ┌────────────────────────────────────┐
  │ distributed self-play farm     │      │ 1. Maker mode self-training/sim     │
  │  · the millions of games       │      │ 2. Roguelike content balancing      │
  │  · ships: net, costs,          │ ───► │ 3. Adaptive/personalized opponents  │
  │    content, opponent library   │ libs │    (optional, later)                │
  └───────────────────────────────┘      └────────────────────────────────────┘
        same Rust engine + same C ABI + same determinism principle
```

**Engine side (offline R&D).** The heavy distributed farm runs the co-evolution loop at scale — the "millions of games" — and produces the shipped network, the measured cost model, the balanced content, and the opponent library. Run by developers, not players.

**Game side (in-product), three modes:**

1. **Maker-mode self-training / simulation.** A player designs pieces, armies, boards, or dungeons and **trains/simulates them locally** through the same C-ABI engine: a scaled-down self-play (fewer games, the shipped network as a warm start, optionally cloud-assisted) that scores their creations, surfaces degenerate combinations, and yields an AI that plays their army. This *is* the founding maker-mode vision, realized by reusing the training engine at small scale.
2. **Roguelike content balancing.** The roguelike ships with the offline-balanced cost model and opponent library; procedurally-generated content is validated against the shipped network — **cheap inference, not full training** — before a player sees it (*SRW Spec §13*). Telemetry-driven re-tuning happens offline and ships as updates.
3. **Adaptive / personalized opponents (optional, later).** In-game opponents draw personality profiles from the library to vary play; optionally, **belief-priming from the player's history** (the codex / rematch warm-start, *SRW Spec §10*) makes veteran enemies "know" the player — bounded, since heavy training stays offline.

The determinism principle is what keeps these honest: whether the training engine is driven by the dev farm or by maker mode, it is the *same* deterministic Rust core (*Botboard Spec §10.1*), so results are reproducible and portable between the two.

---

## 10. Infrastructure

A distributed self-play farm with separable roles:

- **Actors** generate self-play games (GT-CFR sound self-play, R-NaD episodes, AlphaZero-target games) across the belief-sharpness curriculum.
- **Learners** update the shared CVPN from a replay buffer.
- **Population/league manager** maintains members, runs the PSRO/league loop, computes the meta-game and Nash averaging.
- **Army-evolution manager** runs the generate→measure→select→regenerate loop (§7).
- **Cost-model fitter** corrects the prior's weights against measured, anchored values.
- **Evaluation arena** (statistically rigorous) for agent/army comparison.

Cross-cutting: per-procedure target pipelines (recursive sub-searches for GT-CFR; NeuRD updates + reward transformation + regularization schedule for R-NaD; visit-count targets for AlphaZero) all feed the one network; **GPU-batched inference** via a lock-free queue feeding a dedicated GPU worker (*Botboard Spec §7.4*); **deterministic seeding** owned by the Rust core; checkpointing and versioning. The compute envelope is designed to flex between the known poles — AlphaZero (large), Pluribus (cheap), DeepNash/R-NaD (at-scale but search-free), and Student-of-Games (search-heavy).

---

## 11. Evaluation

Evaluation must cover the **whole continuum**, because a single metric hides regime-specific failure:

- **Hidden end:** exploitability / NashConv via a local best-response (OOS-style) — lower means closer to unexploitable.
- **Revealed end:** Elo against a strong perfect-information baseline (and AlphaZero-style comparison where applicable).
- **Population health:** behavioral/response diversity metrics, Nash averaging, explicit non-transitivity detection.
- **Content QA:** win-rate bands, dominance/degeneracy detection, unwinnable/trivial-encounter flags (*SRW Spec §13*).
- **Cost-model gate:** recovering classical piece values (queen ≈ 9, rook ≈ 5, …) on the §7 measurement protocol is a training-validation gate.
- **Rung consistency:** the *one shared net* must yield values usable by alpha-beta, determinized search, GT-CFR, and as an R-NaD policy; verifying this consistency is an explicit, recurring test (*Botboard Spec §13*).

---

## 12. Roadmap & Phasing

Aligned to *Botboard Spec §12*:

1. **Phase 0–1 — value-measurement self-play first.** The perfect-information oracle and a small self-play loop come online to **measure piece values** → Prototype 2 (cost recovery). Cheapest, highest-information first step.
2. **Phase 2 — the backbone, then the population, then the cap.** GT-CFR sound self-play (the CVPN backbone); then the league/PSRO population with diversity objectives; then the R-NaD cap for intractable hidden play; then the gate once two adjacent rungs exist.
3. **Phase 2–3 — the continuum curriculum and multiplayer.** Belief-sharpness-controlled data generation; 1–4 player self-play (blueprint + league).
4. **Phase 4+ — the game-side deployments.** Maker-mode self-training and roguelike balancing reuse the matured pipeline through the C ABI.

The earliest prototype (§13) — confirming the rung-3 net-sharing assumption — should run as soon as the GT-CFR backbone and an R-NaD policy both exist, before heavy investment in the unified curriculum.

---

## 13. Risks & Open Questions

- **The rung-3 net-sharing assumption** — that one network's policy head, refined by R-NaD, can serve the intractable cap (vs a separate Nash network). Justified theoretically by NeuRD = CFR, but **the load-bearing prototype**: train the GT-CFR backbone and an R-NaD policy on a shared net on a small hidden-identity game and confirm they don't destructively interfere.
- **Perfect-vs-imperfect interference** — one net good at deterministic optimal *and* stochastic equilibrium play pulls in two directions (SoG's perfect-info gap is the warning). Mitigation: curriculum weighting toward the revealed regime, optional perfect-info fine-tuning.
- **Model-free Nash convergence caveats** — no high-probability proof when sampling from the policy; can underperform on tractable games. Mitigation: confine R-NaD to the intractable cap.
- **Population size vs compute**, and the **diversity-vs-strength** tradeoff — bigger leagues cost more and can dilute strength; truncation/diversity-estimation methods help.
- **Co-evolution stability** — army evolution + agent evolution + cost correction can oscillate; anchors and careful schedules are the brake.
- **Game-side training budget** — how much maker-mode self-play is feasible client-side vs cloud-assisted.
- **Overall compute** — DeepNash and SoG were expensive; the throughput engine (*Botboard Spec §7*) and the cheap-compute precedent (Pluribus) are the mitigations.

---

## References

1. NeuRD ↔ CFR — Hennes et al. (2020), *Neural Replicator Dynamics* (AAMAS); formal equivalence to softmax CFR, reduction to Hedge; OpenSpiel (Lanctot et al., arXiv:1908.09453) on NeuRD replacing regret-matching in CFR.
2. R-NaD / DeepNash — Perolat, Tuyls et al. (2022), *Mastering Stratego* (Science, arXiv:2206.15378); F-FoReL regularization (Perolat et al., 2021).
3. Student of Games / Player of Games — Schmid et al. (2021/2023), arXiv:2112.03178, Science Advances (GT-CFR + counterfactual value-and-policy network; sound self-play).
4. ReBeL — Brown et al. (2020), arXiv:2007.13544.
5. AlphaZero — Silver et al. (2017), arXiv:1712.01815; Leela Chess Zero.
6. Population-based / league training & diversity — Lanctot et al. (2017), *PSRO*; Vinyals et al. (2019), AlphaStar; McAleer et al., Pipeline PSRO / SP-PSRO; Liu et al. and Perez-Nieves et al. (behavioral/response diversity); Balduzzi et al., Nash averaging.
7. Automated balancing — wargame point-cost estimation via regression + MCTS; CCG evolutionary play-testing (Hearthstone, Dominion); *Metagame Autobalancing*; RaidEnv (play-tester generalization).
8. Determinization & information-set search (for the gate/ladder context) — Long, Sturtevant et al. (2010); Cowling et al. (2012), ISMCTS; Lisý et al., OOS.

---

## Appendix — Provenance & Changelog

### New document (v0.1)
Resolves the training gap flagged in *Botboard Spec v0.3 §9*. Committed decisions:

- **One shared CVPN** with Bit-derived embeddings, per-player value head, and a cost-prior head (§2).
- **Reconciliation:** GT-CFR sound self-play backbone + AlphaZero-style sharpening at the revealed cap + R-NaD/NeuRD at the intractable cap, unified because **NeuRD is formally softmax-CFR** (§3) — so all three are one regret-minimization family on shared counterfactual values; rung 3 shares the net's policy head.
- **Population/league** (PSRO / league training) with explicit behavioral-diversity objectives for **diverse-personality opponents**, made necessary by army non-transitivity, with Nash-averaging valuation (§5).
- **The co-evolution loop** evolving pieces *and* opponents on the same games, with a perfect-info, Nash-averaged, anchored **piece-value measurement protocol** (§7).
- **Libraries/profiles** persistence owned by the Rust core and exposed over the C ABI (§8).
- **Dual deployment:** the offline farm and the in-product modes (maker self-training, roguelike balancing, adaptive opponents) are one subsystem bridged by the C ABI and determinism principle (§9).

### Research grounding (this document)
The NeuRD↔softmax-CFR equivalence (resolving the reconciliation); PSRO / AlphaStar league and behavioral-diversity methods (the personality population); Nash averaging and the non-transitivity motivation; the model-free-Nash convergence caveat.

### Frontier decisions flagged as prototypes
The rung-3 net-sharing assumption (the earliest, load-bearing experiment); interference management across the continuum; co-evolution stability. See §13.

---

*Botboard — Training & Self-Play Specification, Version 0.1. One self-play subsystem evolves both the pieces and the opponents, persists them as reusable libraries, and serves the engine first and the game later — unified because the search-based and model-free trainers are the same regret-minimization family.*
