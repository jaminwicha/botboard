# Botboard — Technical Specification

**A compositional engine for heterogeneous fairy chess, and the performance foundation for *Subterranean Robot Wars*.**

`Version 0.4 · July 2026`

> **Version 0.4.** Supersedes v0.3. This revision closes the two Tier-1 model gaps surfaced by the second gap analysis: **turn structure & action economy** is now explicit (§3.4 — one turn = one action, with multi-action content compiled into compound moves), and the **determinism principle is reconciled with the learned evaluator** (§10.6 — two inference grades). Transposition hashing is extended to mutable terrain and per-instance state (§7.5). Everything else carries from v0.3. Changes are traced in [Appendix C — Provenance & Changelog](#appendix-c--provenance--changelog).

---

## Abstract

Botboard is a headless engine for turn-based games on a board, designed so that pieces are not hard-coded but **composed** from a small vocabulary of atomic, cost-bearing rules. This lets the engine represent 1–4 heterogeneous armies on variable and arbitrarily-shaped boards, score them for balance, and evolve both armies and opponents through large-scale self-play. All known chess variants are reconstructable from the model, which also extends to terrain, lasers, shields, hacking, and resurrection. A defining mechanic — a piece's rules are hidden until observed in play — makes the game one of **imperfect information**, structurally closest to Stratego. Crucially, the *amount* of hidden information is not fixed: rematches, dungeon-accumulated reconnaissance, and spying continually move a position along a continuum from fully hidden to fully revealed. The engine is therefore designed as **one shared brain with a belief-gated ladder of search modes**, so it is strong and cheap at the revealed end (where traditional chess-engine optimizations apply) and sound at the hidden end (where game-theoretic reasoning is required). This document specifies the core model — including an explicit turn/action model — the performance architecture, that AI design, the technology stack with its two-grade determinism discipline, and a phased roadmap grounded in prototypes.

---

## Table of Contents

1. [Introduction & Vision](#1-introduction--vision)
2. [Design Goals and Non-Goals](#2-design-goals-and-non-goals)
3. [Core Model: Bots, Bits, and the Two Axes](#3-core-model-bots-bits-and-the-two-axes)
4. [The Cost Model](#4-the-cost-model)
5. [The Policy Layer](#5-the-policy-layer)
6. [Validation: the Acceptance Suite](#6-validation-the-acceptance-suite)
7. [Engine Architecture](#7-engine-architecture)
8. [Artificial Intelligence and the Information Continuum](#8-artificial-intelligence-and-the-information-continuum)
9. [Self-Play and Training](#9-self-play-and-training)
10. [Technology Stack and System Architecture](#10-technology-stack-and-system-architecture)
11. [The Game Layers](#11-the-game-layers)
12. [Development Roadmap](#12-development-roadmap)
13. [Risks and Open Questions](#13-risks-and-open-questions)
- [References](#references)
- [Appendix A — Worked Bit-Mappings](#appendix-a--worked-bit-mappings)
- [Appendix B — Glossary](#appendix-b--glossary)
- [Appendix C — Provenance & Changelog](#appendix-c--provenance--changelog)

---

## 1. Introduction & Vision

Botboard is the engine; *Subterranean Robot Wars* is the game built on top of it (via an intermediate combat layer, *Robot Wars*). The layers are distinct modules: the engine is a reusable, headless core, and everything player-facing sits above a narrow interface.

The engine generalizes chess. In ordinary chess every piece type is a fixed bundle of rules. In Botboard a piece — a **Bot** — is assembled from atomic rule fragments called **Bits**. Bits cover movement geometry, capture behavior, and further effects; each carries a cost so the relative value of any constructed piece can be scored and heterogeneous armies balanced. The same model that reconstructs Western chess, xiangqi, shogi, and historical variants such as Tamerlane chess also expresses pieces those games never had.

**Position relative to prior art.** Botboard sits between two traditions. *Dedicated engines* (Stockfish, Fairy-Stockfish) are fast but hard-code a fixed family of pieces. *General Game Playing* systems (Ludii, Polygames, RBG) describe arbitrary games compositionally, but their rule-interpretation and learning components are computationally heavy and not built for self-play throughput. Botboard's niche is to be **compositional like a general game system, but compiled for speed like a dedicated engine** — and, beyond either, to treat imperfect information and large-scale self-play as first-class.

The product is a stack of layers, each depending on the one below: **Botboard** (the headless engine), **Robot Wars** (one battle = one Botboard game between robot armies), **Subterranean Robot Wars** (a card-collection roguelike/metroidvania wrapping those battles), and then **maker mode**, **network play**, and a **marketplace**.

---

## 2. Design Goals and Non-Goals

### Goals

- **Compositional generality.** Any classical chess variant is reconstructable from atomic Bits; novel pieces extend the same vocabulary.
- **Heterogeneous balance.** Costs derived from Bits, corrected by self-play, let armies with different pieces meet in fair games.
- **Throughput.** The engine is optimized so millions of self-play games across many random armies are feasible.
- **Strength across the whole information continuum.** The engine must be strong and cheap when information is revealed (the common campaign case) *and* sound when it is hidden — not one or the other.
- **A learned evaluation that generalizes** to never-before-seen procedural pieces and arbitrary boards.
- **Headless and embeddable**, with all rules/state/RNG owned by the core for determinism.

### Non-Goals

- Matching a top specialist engine at standard chess.
- Hand-crafted, variant-specific evaluation (pieces are procedural; evaluation must be learned, §7.4).
- The full universality of a GDL-based general game system (we trade some expressiveness for throughput).
- A bespoke renderer (delegated to a high-level 2D engine, §10.4).

---

## 3. Core Model: Bots, Bits, and the Two Axes

> A piece's behavior composes along two axes: how it *reaches* a square, and what *happens* as a result. A Bit is a typed, parameterized fragment on one of these axes; a piece is the set of its Bits.

```
                         ┌──────────────────────────┐
                         │     BOT = set of BITS     │
                         └─────────────┬────────────┘
            ┌──────────────────────────┼──────────────────────────┐
            ▼                          ▼                           ▼
 ┌──────────────────────┐  ┌───────────────────────┐  ┌──────────────────┐
 │ AXIS A               │  │ AXIS B                │  │ SPECIAL &        │
 │ reaching a square    │  │ consequence           │  │ COMPOUND MOVES   │
 ├──────────────────────┤  ├───────────────────────┤  │ drops · castling │
 │ Geometry             │  │ Capture semantics     │  │ en passant       │
 │   leaper/rider/hopper│  │   destroy/flip/to-hand│  │ double-step      │
 │ Direction            │  │ Transformation        │  │ multi-step       │
 │ Mode (move/capture)  │  │   promotion           │  │ effect scripts   │
 │ Path interaction     │  │ Spawn / utility       │  │ (§3.4)           │
 │   lame leapers,terrain│ │   wall/pit/resurrect  │  └──────────────────┘
 │ Condition            │  │ Ranged effect         │
 │ Target predicate     │  │   laser, coupled nerf │
 └──────────────────────┘  └───────────────────────┘
            └──────────────────────────┬──────────────────────────┘
                                       ▼
 ┌────────────────────────────────────────────────────────────────────┐
 │ POLICY LAYER  (where composition stops)                            │
 │ royalty · win/draw/stalemate · repetition + termination ·          │
 │ turn & pass policy · capture-fate defaults · named legality        │
 │ predicates (escape hatch)                                          │
 └────────────────────────────────────────────────────────────────────┘
```

### 3.1 Axis A — reaching a square

**Geometry** is the atom, in three primitives parameterized by an `(m,n)` step: `leaper(m,n)` jumps and ignores the path; `rider(m,n)` repeats the step until blocked; `hopper(m,n)` rides but must jump a screen (a family parameterized by screen count and landing mode). **Direction** masks a leaper/rider to a subset of its symmetric copies. **Mode** sets move-only / capture-only / both. **Path interaction** expresses lame leapers (cells that must be empty) and terrain permissions (drill, flight). **Condition** gates on the mover's state/position. **Target predicate** gates on the piece being captured — surfaced by xiangqi's flying-general, reused by hack, laser, and spy.

### 3.2 Axis B — consequence

**Capture semantics** specify a captured piece's fate (destroyed / flipped in place / banked to hand), plus hit-count armor and immunities. **Transformation** turns a piece into another under a trigger (offered / automatic / forced-if-immobile), recording base type for de-promotion. **Spawn/utility** effects create walls, dig pits, resurrect, or reveal a move-set (spy). **Ranged effects** include lasers that capture at range without vacating the origin, with optional coupled nerfs.

> **Stateful effects.** Armor, HP, ammunition, and cooldowns are *state attached to a piece instance*, kept out of the spatial representation (§7.3).

### 3.3 Special moves

Drops, castling, en passant, and the double-step depend on history or compound two pieces — first-class special-move generators, not geometry. Drop legality resolves in three tiers: baseline (empty target), a derivable rule (the dropped piece must have a legal move from its target), and bespoke constraints (*nifu*, *uchifuzume*) in the policy layer.

### 3.4 Turn structure, actions, and compound moves *(new in v0.4)*

> Earlier versions left the action economy implicit while their own content (act-twice effects, fire-and-retreat lasers) quietly strained it. This section makes the ruling explicit, because everything downstream — move generation, branching factor, search cost, evaluation, and the feel of combat — depends on it.

**The base rule: one turn = one action.** Play proceeds in strict rotation among the 1–4 players (§8.7); on a player's turn they perform exactly **one action** with exactly one piece:

- **a move** — an Axis-A move, carrying all of its Axis-B consequences;
- **an ability** — an Axis-B effect invoked *as* the turn's action (heal, spy, hack, wall/pit creation), targeted per its Bits;
- **a drop** — placing a piece from hand (§3.3).

Rationale, in order of force:

1. **Search viability.** The whole AI design (§8) inherits from lineages built on chess-scale branching (~35 average legal moves per turn). Action-point economies multiply per-turn choices combinatorially: **Arimaa** — just four steps per turn on an 8×8 — averages **~17,000 legal turn-moves** (individual piece moves *simpler* than chess), which is why a computer that searches eight turns deep in chess manages roughly three in Arimaa, and why the Arimaa Challenge stood for over a decade until 2015, with the eventual winner requiring dedicated innovations aimed squarely at the branching-factor obstacle. A global action-point economy would put every rung of the ladder — and the self-play budget — on the wrong side of that curve.
2. **Legibility under hidden information.** The reveal mechanic meters knowledge by *observed actions*; one action per turn is what keeps the metering readable and the belief update well-defined, for humans and for the belief substrate (§8.6) alike.
3. **The acceptance suite demands it.** Chess, xiangqi, and shogi are strict-alternation games; the base rule must be theirs.

**Multi-action is piece-local, explicit, and compiled into compound moves.** Effects that let a piece do more than one thing do **not** change the turn structure; they are **compound moves** — a single generated move whose *effect script* has multiple steps. The generator enumerates the legal compounds; search, hashing, and the belief substrate see *one move*; the cost model prices the Bits that create them (multi-action is a strong multiplicative modifier, §4). Mapping the current catalog (*SRW Spec, Appendix B*):

| Content | As a compound move |
|---|---|
| Overclock ("acts twice, 1 self-damage") | one generated move = ⟨action₁, action₂, self-HP −1⟩; the generator enumerates legal pairs, bounded by the piece's own move set |
| Laser with coupled retreat | ⟨fire at target, forced 1-step retreat⟩ — one move; **illegal if the retreat square is blocked** (the nerf has teeth) |
| Mine-layer | not multi-action at all: a **move modifier** (leaves a trap on the vacated square as a side effect of an ordinary move) |
| Welding torch (heal) | an **ability action** — the turn's single action |

Two properties keep compounds safe: they are **bounded locally** — only pieces carrying such Bits generate them, so branching inflation is paid for where it occurs and priced by the cost model, never globally — and they are **atomic**: a compound either fully applies or is illegal, with no mid-compound interaction window, which keeps make/unmake, hashing (§7.5), and the belief update simple.

**Plies, turns, and the turn policy.** Formally: a **ply** is one action; a **turn** is one player's consecutive plies; under the base rule they coincide. Rotation order, elimination handling, and any variant turn structure are owned by the **turn policy** in the policy layer (§5). Multi-ply turns (Marseillais-style double-move, progressive chess) are *representable* — in-tree, consecutive plies with the side-to-move unchanged — but are a deliberate **escape hatch**, not the base game: they are flagged as search-cost-explosive (see Arimaa, above) and are **excluded from all cost/balance calibration**.

**Passing.** Illegal by default — zugzwang is a real and desirable part of the chess-like game. A policy flag may allow passing (some encounter designs want it), with a termination guard: N consecutive passes by all live players ends the game per the victory policy (otherwise pass + repetition can loop forever, §5).

**No simultaneity.** Turns are strictly sequential. Hidden information does not imply simultaneous moves — Stratego and Kriegspiel are both sequential — and simultaneous resolution would move the game into a different class entirely (matrix-game stages at every tick), invalidating the search ladder. Out of scope at the model level.

---

## 4. The Cost Model

> Piece value is emergent and contextual. The analytic cost is a *prior*; self-play measures realized value and corrects it. v0.2 made the prior concrete (explicit formulas, a synergy term, calibration anchors) and grounded the loop in the automated-balancing literature; v0.4 adds one rule: multi-action (compound-move) Bits are priced as strong multiplicative modifiers (§3.4).

A static table can never perfectly recover empirical values, because those values are outputs of a particular game. So the compositional cost is a fast prior, and self-play folds realized value back — the same strategy the literature uses to balance asymmetric games (point costs estimated from self-play via regression with varied playstyles; CCGs balanced by evolutionary play-testing).

### 4.1 The prior

```
  C_prior = max(1, ( C_base × M_utility ) − S_nerfs )   +   Σ_{i<j} ( S_ij · x_i · x_j )
```

| Term | Meaning |
|---|---|
| `C_base` | additive — mobility integral: `(avg_moved · w_move) + (avg_attacked · w_attack)` |
| `M_utility` | multiplicative — force multipliers (armor `1 + 0.5(HP−1)`, piercing `×1.3`, stealth `×1.4`, **multi-action/compound `×~1.8` initial prior**); product over effects |
| `S_nerfs` | subtractive — coupled constraints (directional lock, recoil) |
| `max(1, ·)` | a floor of 1 preventing free/negative-cost spam |
| `Σ S_ij x_i x_j` | the synergy term — pairwise Bit-interaction weights (a structured prior, O(n²), pairwise-only) |

### 4.2 Calibration anchors

A set of **anchor pieces** with externally-fixed reference values (low-value historical leapers, mid-value classical compounds, exotic constrained pieces) is held constant during weight-fitting, regularizing the scale so learned costs stay interpretable and don't inflate. They double as the acceptance test: recover queen ≈ 9, rook ≈ 5, bishop ≈ knight ≈ 3, pawn ≈ 1 emergently.

### 4.3 The correction loop

Self-play tracks realized win-contribution per Bit and Bit-pair and adjusts the prior's weights by gradient descent, anchored as above. Effects that cannot be valued analytically are learned this way. Caveat from the literature: the play-testing agents that measure value must **generalize across procedurally-generated content** — a motivation for the generalizing evaluation in §7.4.

---

## 5. The Policy Layer

> Not everything is a composable Bit, and the model is healthier once it admits that.

A thin policy layer holds each variant's rules of the world: the **royalty policy** (royalty is a per-piece attribute — per-army, possibly multiple opposed controllers, possibly none for stray pieces), the **turn policy** and **pass policy** (rotation order, elimination handling, the multi-ply escape hatch, pass legality and its termination guard — §3.4), the **stalemate policy**, **repetition and termination**, the **default capture-fate**, and a deliberately small set of **named legality predicates** for genuinely irreducible interactions (*uchifuzume*, flying-general exposure). The named-predicate set is kept small on purpose — it is where composition stops, not a junk drawer. Termination rules are not optional: without a repetition rule and a ply cap, self-play games can loop forever; passing, where enabled, needs its own guard (§3.4).

---

## 6. Validation: the Acceptance Suite

The atomic system is judged expressive enough if it reconstructs Western chess, then xiangqi, then shogi, purely from Bits — validated move-for-move by **perft** differential-tested against a reference engine (Fairy-Stockfish as golden oracle, in non-shipped test tooling only, §10.5). Between them these three exercise nearly every primitive — including strict alternation, the base turn policy (§3.4). Each also sharpened the model:

| Variant | What it exercised / added |
|---|---|
| Western chess | the baseline; pawns prove direction + mode + special moves. |
| Xiangqi | lame leaper, mode-split cannon, palace/river zones — *added* target predicates, royalty-as-attribute, policy layer. |
| Shogi | forward-biased pieces and promotion — *added* capture-fate enum, three drop-legality tiers, transformation optionality + base-type, named-predicate escape hatch. |

Worked mappings: [Appendix A](#appendix-a--worked-bit-mappings).

---

## 7. Engine Architecture

> Stockfish-grade move generation/infrastructure, AlphaZero-style learned evaluation and self-play, and the imperfect-information lineage (§8), with two departures: representation per board class (§7.1), and evaluation built to generalize (§7.4).

### 7.1 Board representation

The strong, widely-held expert position is that a **well-tuned mailbox has always been at least as fast as bitboards for raw move generation**, and on large boards bitboards are likely not superior (tables scale O(N²) in board side, looping O(N)). Two facts reshape the choice: **wide SIMD registers extend the single-register regime** (a 256-bit AVX2 register holds a 16×16 board; AVX-512 up to ~512 cells), and **with NNUE-grade evaluation raw move-gen speed is largely moot** because the network update dominates — which lowers the stakes and argues for choosing representation by *flexibility*.

> **Departure A — representation per board class, behind an occupancy abstraction:**

| Board class | Cells | Representation |
|---|---|---|
| ≤ 8×8 | ≤ 64 | one 64-bit bitboard |
| bounded mid-size | ≤ 256 / ≤ 512 | one **wide SIMD bitboard** (AVX2 / AVX-512) |
| large / sparse / arbitrary | larger | **mailbox array + piece lists** |

The crossover is to be set empirically (Prototype 1, §12). An arbitrary board *is a graph of cells* — a natural fit for the GNN evaluation option (§7.4) and a reason mailbox/graph is right at the top of the size range.

### 7.2 Sliding attacks

Per board class: **magic bitboards** (fastest small, but ~200 MB tables on large boards); **PEXT bitboards** (faster where supported, but ~250× slower on AMD before Zen 3 — requires runtime dispatch); **parallel-prefix ray-scan** (table-free, portable, scales to large boards — the default for large/arbitrary boards and the safe fallback). Terrain edits (created walls, dug pits) mutate the blocker/permission masks these kernels query at runtime; the geometry tables themselves are static per board shape.

### 7.3 Move generation and stateful data

Bits are the authoring format, not the runtime format. At init each piece type **compiles** its Axis-A movement into fast structures; the hot loop consumes compiled tables and queries occupancy. Axis-B effects, special moves, and **compound moves** (§3.4) run on a separate interpreted path — compounds are enumerated by the generator and applied atomically.

> **Structure-of-Arrays for stateful pieces.** Per-instance state (armor, HP, ammo, cooldown) lives in **parallel flat arrays indexed by piece ID**, not in the spatial representation. When an attack mask intersects enemy occupancy, extract the hit index with a hardware bit-scan and update its state array — branchless until a piece dies.

### 7.4 Evaluation

Evaluation must be learned by self-play (procedural pieces). Two architectures, traded off:

- **NNUE-style** — sparse features, an incrementally-updated accumulator, int8/int16 quantization, SIMD; optional batched GPU inference via a lock-free queue feeding a dedicated GPU worker. Fastest on fixed-topology boards.

> **Departure B — Bit-derived piece embeddings.** A piece's evaluation feature is a **learned embedding of its Bit-set**, not a one-hot type id, so never-before-seen procedural pieces (and no-king / multi-royal armies) get sensible evaluations; the same encoder emits the cost prior (§4); the value head outputs **one expected outcome per player**.

- **Graph neural network over the board-graph** — node embeddings as evaluation, generalizing across positions and handling arbitrary shapes (walls/pits = absent nodes). Heavier than an incremental NNUE update, so the likely split is **NNUE+embeddings for fixed-topology high-throughput self-play, GNN for arbitrary-topology rooms and maximum generalization** — to be measured. The determinism ruling (§10.6) adds a second, decisive constraint: the *shipped* deterministic path is NNUE+embeddings; the GNN is confined to the performance grade unless a quantized variant proves out.

This network is shared across every search mode in §8 — it is the single "brain."

### 7.5 Infrastructure

A transposition table with Zobrist hashing is generalized so the key covers piece-square, side-to-move, hands, terrain, repetition/ply state, and player count. Imperfect-information play additionally needs a *separate* cache keyed on public states / information sets. **As belief sharpens mid-game (§8), caching promotes from infoset-keyed to full-state keys** — recovering the full power of the ordinary transposition table at the revealed end. Move-ordering, pruning, and Lazy-SMP parallelism are adopted, gated where unsound. *(The Gemini spec omits a transposition table / Zobrist entirely; retained here.)*

> **Hashing mutable state (new in v0.4).** Per-instance state and mutable terrain must be first-class in the key, or the table and the repetition rule silently break — two board-identical positions differing only in a piece's HP or in a created wall are *not* the same node. Concretely: (i) piece keys extend from ⟨type, square⟩ to **⟨type, square, state-bucket⟩**, where the bucket quantizes the instance state that affects play (HP/armor level, ammunition, cooldown phase); (ii) each cell's **terrain type carries its own Zobrist key**, so wall creation, pit digging, and drilling XOR into the hash exactly like piece movement; (iii) hands, side-to-move, and turn-policy state are keyed as before; compound moves (§3.4) hash as their net state change, applied atomically. **Repetition** is then defined as equality of this *full ground-truth state* — never a player's masked view; the arbiter core judges it. Two useful consequences: monotone counters (ammunition) make true repetition impossible while they tick, strengthening termination (§5); cyclic state (cooldown phase) must match for a repetition to count, which is exactly right.

---

## 8. Artificial Intelligence and the Information Continuum

> A piece's rules are hidden until observed, which makes Botboard a game of imperfect information whose structure is closest to **Stratego**. But the *amount* of hidden information is a state variable, not a constant: rematches, dungeon recon, and spying continually move a position along a continuum from fully hidden to fully revealed. The engine is built as **one shared brain with a belief-gated ladder of search modes** that spans that continuum — strong and cheap when revealed, sound when hidden.

### 8.1 The dial: belief sharpness

The degree of "imperfectness" is measurable as the **sharpness of the belief** — the entropy of the distribution over each hidden piece's still-consistent Bit-set hypotheses, read directly from the knowledge masks (§8.6). Perfect information is the limiting case: a belief that has collapsed to a point mass. Three further signals (after Long et al.) tell us *whether cheap determinized search is safe in a given position*: **leaf correlation**, **bias**, and especially the **disambiguation factor** — how quickly hidden information is revealed by play. Botboard's rematch/recon regime is high-disambiguation by construction, which is exactly where the cheap methods are sound.

### 8.2 The search ladder (cheapest-sound-first)

```
 belief sharpness ───────────────────────────────────────────────────►  hidden
 REVEALED (point mass)        SHARP (mostly known)      BROAD (pivotal)      INTRACTABLE
 ┌───────────────────┐  ┌────────────────────────┐  ┌──────────────────┐  ┌───────────────┐
 │ Rung 0            │  │ Rung 1                 │  │ Rung 2           │  │ Rung 3        │
 │ perfect-info      │  │ determinization /      │  │ sound GT search  │  │ model-free    │
 │ αβ + TT + SIMD,   │  │ ISMCTS                 │  │ OOS / GT-CFR     │  │ Nash policy   │
 │ or MCTS (expand-1)│  │ fast, near-sound here  │  │ sound, costlier  │  │ R-NaD (no     │
 │ STRONGEST/CHEAPEST│  │ (high disambiguation)  │  │ (decomposable)   │  │ search)       │
 └─────────┬─────────┘  └───────────┬────────────┘  └────────┬─────────┘  └───────┬───────┘
   discrete cap          ┌──── one parameterized GT-CFR core ────┐          discrete cap
   (max speed)            expand-1 ◄──── tied to belief ────► expand-top-k   (intractable)
```

- **Rung 0 — point-mass belief** (rematch, spied, fully reconned): the *perfect-information engine* — alpha-beta with the full classical toolkit (transposition table, magic/PEXT/SIMD move-gen, null-move, LMR, deep search) or AlphaZero MCTS. Strongest and cheapest; the **common case in a campaign**.
- **Rung 1 — sharp belief, high disambiguation** (mostly revealed): **determinization (PIMC) / ISMCTS** — sample the few consistent worlds, solve each with the perfect-info engine, aggregate. Sound and fast *precisely* in this regime; ISMCTS reduces strategy fusion by reasoning over information sets.
- **Rung 2 — broad belief, decomposable, strategically pivotal** (hidden): **sound game-theoretic search** — Online Outcome Sampling (equilibrium-convergent, exploitability shrinking with compute) or growing-tree CFR. Sound under genuine uncertainty; the only rungs that play bluffs and information-gathering correctly.
- **Rung 3 — intractable tree** (Stratego-scale cold-open against an unknown clan): the **model-free Nash policy** — R-NaD/DeepNash, evaluated directly with no search.

### 8.3 Why this is one engine, not four (the unifying core)

Rungs 0–2 are not bolted-together algorithms. Growing-tree CFR already *interpolates* across them: in the perfect-information limit it expands a single best action (AlphaZero-like, deterministic), and under hidden information it expands the top-k actions ranked by the prior (CFR-like, stochastic). So rungs 0–2 are largely **one parameterized GT-CFR searcher** whose expansion width (`expand-1 ↔ expand-top-k`) and tree-growth budget are driven by belief sharpness — sliding continuously from AlphaZero-like to CFR-like. Only the two **caps** are discrete: plain alpha-beta at the point-mass end (for maximum speed in the common revealed case) and the search-free R-NaD policy at the intractable end. One shared value-and-policy network (§7.4, with Bit-derived embeddings and a per-player value head) underlies every rung.

### 8.4 Why not a single unified algorithm everywhere, and why not two brains

Two candidate architectures were rejected:

- **A single unified algorithm everywhere** (run growing-tree CFR / Student-of-Games for all positions, letting it degrade toward AlphaZero-like search when revealed) is elegant and provably sound for both regimes. It was rejected because Student of Games is measurably *weaker than specialized AlphaZero in the perfect-information regime* — and that regime is our **common case** (campaigns, rematches, accumulated recon). Paying the game-theoretic tax on the majority of positions is the wrong trade.
- **Two separate brains** (an independent perfect-info engine and imperfect-info engine, with a coordinator) was rejected because the perfect-info engine still needs the *same* learned evaluation for procedural pieces, so the two networks converge anyway — doubling training and maintenance and introducing handoff seams for no genuine separation.

The committed design — **one shared network + a belief-gated search ladder** — is "one engine" from the substrate (one network, one move generator, one transposition/infrastructure, one training loop) and "multiple logic paths" from the search wrapper. Both descriptions are correct depending on perspective; that duality is the point.

### 8.5 The gate

Dispatch among rungs is governed by: (a) **belief sharpness** (entropy); (b) the **determinization-safety signals** (leaf correlation, disambiguation factor); and (c) **pivotality** — whether the residual uncertainty is strategically loaded (could a hidden piece be the enemy royal or a game-swinging laser?). Minor, fast-disambiguating unknowns → cheap rungs; pivotal or slow-disambiguating uncertainty → escalate. The escalation triggers are exactly the determinization failure modes: **strategy fusion** (escalate when information-gathering or deception carries value, since determinization can never choose to gather information) and **non-locality** (escalate when the hidden-state distribution is strongly history-dependent). The gate may be a small **learned meta-controller** predicting best value-per-compute from belief features.

### 8.6 The belief substrate (carried from v0.2, adopted from the Gemini spec)

Beneath the ladder sits a concrete bookkeeping substrate: a strict separation of **Ground Truth** from each agent's **observed view**, implemented with bitmasks; each enemy piece carries a **knowledge mask** of which rules have been revealed; stealth/fog hide pieces via an AND-NOT with a concealment mask; a spy / True-Sight vision mask overlays to reveal. This layer represents *what is seen*; the belief distribution and the ladder reason about *what is uncertain*. It also enforces that self-play never leaks hidden information to a player. One observed action per turn (§3.4) is what keeps each belief update well-posed: a compound move reveals its whole effect script as a single observation.

### 8.7 Multiplayer (1–4 players)

A free-for-all has no single equilibrium. Two compatible recipes apply: the **Pluribus** blueprint-plus-depth-limited-search approach (hard-to-exploit rather than optimal) and **R-NaD**'s inherently multi-agent dynamics. The value head is per-player throughout. The ladder still applies — the discrete caps especially. Rotation order and elimination handling are owned by the turn policy (§3.4, §5); *victory, draw, and alliance rules for N > 2 remain the one Tier-1 model gap still open* (§13).

### 8.8 Coherence with the campaign

The SRW codex/recon mechanic **is** belief collapse: cumulative reconnaissance moves the belief toward a point mass, which automatically slides the engine down to its cheap, strong perfect-information rungs — so it plays *harder and faster against armies the player has scouted*, with no special-casing. Scouting the enemy (the game mechanic) and dispatching to chess-engine mode (the engine optimization) are the same variable seen from two layers.

---

## 9. Self-Play and Training

The loop follows AlphaZero in outline — generate self-play games, train the network, update, repeat — but training must **span the information continuum**, so the shared network is calibrated at every belief sharpness and the gate learns where to switch. Concretely: **sound self-play** in the manner of Student of Games (training value/policy targets from both game outcomes and recursive sub-searches) for the imperfect-info competence; AlphaZero-style targets for perfect-info sharpness; and **R-NaD self-play** for the unexploitable hidden-end policy. A curriculum runs positions from cold-opens (high entropy) through rematches (collapsing) to fully-revealed. Critically, the same self-play measures empirical piece values, correcting the cost prior (§4) and driving the generation and evolution of armies — the ML core serves as opponent *and* content-balancer, exactly the automated-balancing pattern validated in the literature, with the inherited caveat that play-testers must generalize across procedural content (§7.4). Caching promotes from infoset-keyed to full-state transposition keys as belief collapses (§7.5), and persisted codex priors warm-start the next rematch's belief. The full training subsystem — the shared CVPN, the three-trainer reconciliation, the population/league, the co-evolution loop, libraries, and the dual engine/game deployment — is specified in the companion **Training & Self-Play Specification**.

---

## 10. Technology Stack and System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ C# APPLICATION LAYERS               (MonoGame / Godot)       │
│   Robot Wars  ·  Maker mode  ·  Network  ·  Marketplace      │
└──────────────────────────────┬──────────────────────────────┘
                               │  coarse commands
┌──────────────────────────────▼──────────────────────────────┐
│ C ABI BOUNDARY    opaque handle · repr(C) structs            │
└──────────────────────────────┬──────────────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────┐   ┌───────────────────┐
│ RUST ENGINE CORE  (cdylib)                                  │◄─►│ PYTHON TRAINING   │
│  Bit system · Move-gen · Shared net · Search ladder · Self-play│PyO3 NNUE/GNN · GT-CFR │
└─────────────────────────────────────────────────────────────┘   │ · R-NaD (PyTorch) │
                                                                   └───────────────────┘

  Determinism: the Rust core owns all rules, state, and RNG.
              C# is presentation, input, and orchestration only.
```

### 10.1 Languages and the determinism principle

The engine is **Rust**; application layers are **C#**; training is **Python**. The Rust core owns everything that decides outcomes — rules, state, RNG, the search ladder, self-play — and C# owns only presentation, input, and orchestration. No game logic lives in C#. This protects determinism (reproducible self-play, authoritative network play, replays). *(The Gemini spec does not articulate this boundary; retained as a load-bearing invariant.)* How this principle coexists with a floating-point learned evaluator is resolved in §10.6.

### 10.2 The boundary

The engine compiles to a `cdylib` with a flat `extern "C"` surface; C# bindings generated by csbindgen/interoptopus, with source-generated `LibraryImport`. The **opaque-handle pattern**: Rust owns the engine-state lifetime and frees it. The API is **coarse, not chatty** — high-level commands, batched results. Long-running search/self-play run on Rust threads; C# polls. The same C ABI serves both the C# game and the Python (PyO3) training harness from one surface.

### 10.3 Why Rust, and the SIMD question

Rust and C++ reach the same machine code through the same back-end. Rust is chosen for memory safety and fearless concurrency across the self-play farm and a lockless shared transposition table, cargo tooling, native 128-bit integers, and the fact that evaluation is learned. The Gemini spec's SIMD lean (wide bitboards, `tzcnt`/popcnt, runtime CPU dispatch, compile-time constraint resolution) maps cleanly onto Rust's `core::arch`, runtime feature detection, and const-generics/traits; strong Rust engines already ship runtime-dispatched SIMD NNUE, and the most widely-adopted NNUE trainer in computer chess is itself Rust. SIMD is not a sufficient reason for C++.

### 10.4 Rendering

A 2D pixel-art turn-based board does not stress any modern GPU; low-level APIs (Vulkan/Metal) would not improve frame rate and cost boilerplate. The path is a high-level 2D engine (MonoGame or Godot), which also resolves what matters for feel — consistent frame pacing and pixel-perfect rendering. Headless, so reversible.

### 10.5 Licensing posture

Fairy-Stockfish (GPLv3) is used as a *specification and behavioral oracle* — learning algorithms, verifying perft — with original idiomatic Rust written, not copied/transliterated. Algorithms (magic bitboards, PEXT, perft) are reimplementable freely; specific GPLv3 source is not copied. Any direct reference-code use is isolated in non-shipped test tooling. *Engineering note, not legal advice; formal review warranted before commercial release.* (The Gemini spec, leaning heavily on Stockfish-style techniques, does not flag this.)

### 10.6 Determinism grades and the learned evaluator *(new in v0.4)*

> v0.3 asserted the determinism principle (§10.1) and, separately, a floating-point neural evaluator with optional GPU batching (§7.4). Those collide: floating-point inference is a classic source of **cross-platform nondeterminism** — compiler instruction selection and reordering (FMA contraction, auto-vectorization), per-system math-library differences, GPU reduction order — the same network can produce bit-different outputs on different machines, even though float math is perfectly repeatable on one executable and one machine. Lockstep-style architectures solve this with fixed-point/integer math. This section resolves the collision by scoping where bit-exactness is actually load-bearing and splitting inference into **two grades**.

**Where bit-exactness is required, and how each path meets it:**

| Path | Requirement | How it's met |
|---|---|---|
| Rules, state, RNG | bit-exact, always | integer-only logic; core-owned seeded RNG (§10.1) — unchanged |
| Replays | bit-exact playback | replays are **move lists + the recorded RNG event stream**; playback re-applies decisions, never re-runs inference |
| Network play | consistency across clients | **server-authoritative** (§11): the server's core computes truth; client float drift cannot desync a game (and server authority is the anti-cheat posture hidden information demands anyway) |
| Shipped AI: in-game opponents, daily seeds, maker-mode certification, content-validation verdicts | **same inputs → same move on every machine** | **deterministic-grade inference** (below) |
| Training farm | reproducible *enough* to debug | **performance-grade inference** + logging (seeds, versioned nets, sampled actions) |

**Deterministic grade (the product default).** Integer-quantized NNUE-style inference — int8/int16 weights and accumulators, fixed-point activations, integer SIMD — executed by the Rust core. Integer arithmetic with defined overflow semantics is bit-exact across platforms and register widths, so any result the product records, compares, or certifies (an AI's chosen move, a validation verdict, a certified maker score) is reproducible everywhere. **This is the only grade permitted on paths where the determinism principle applies.**

**Performance grade (the farm).** Float inference, GPU batching, mixed precision — used by the offline training pipeline (§9; *Training Spec §10*), where cross-machine bit-exactness is not claimed and would cost real throughput. Reproducibility there is achieved by construction of the logs (seeds, network versions, sampled actions), not bit-replay.

**Consequences.**
- The **GNN evaluation option** (§7.4) is float-heavy and hard to quantize incrementally → confined to the performance grade (training/analysis) unless a quantized variant proves out. The shipped deterministic path is **NNUE + Bit-embeddings** — which settles §7.4's split in favor of NNUE-first for the product.
- **Quantization parity** becomes a named test obligation: the deterministic grade must match the float net's play strength within a stated tolerance (quantization-aware training, or post-training calibration against the anchor suite, §4.2) — else the shipped AI silently diverges from the trained one.
- **Cross-grade agreement is CI'd:** positions where the two grades' *chosen moves* differ are logged and bounded; drift beyond tolerance blocks a release.

---

## 11. The Game Layers

The combat game (**Robot Wars**) and the roguelike campaign (**Subterranean Robot Wars**) are specified in the companion document. **Maker mode** exposes the same authoring vocabulary; **network play** uses the Rust engine as the authoritative server; the **marketplace** scaffolds piece trading on Ethereum. These are downstream of a working engine and game.

---

## 12. Development Roadmap

```
 Phase 0          Phase 1            Phase 2                      Phase 3           Phase 4+
 engine core  →   cost prior +   →   AI ladder: perfect-info  →   fog / stealth →   roguelike,
 + acceptance     embeddings/GNN     → determinize/ISMCTS →        + abilities       maker, net,
 + 2 prototypes   + anchors          GT-CFR/OOS → R-NaD → FFA      + terrain         marketplace
```

1. **Phase 0 — engine core + two prototypes.** Board, Bits, the compile step, move generation, win conditions, the occupancy abstraction — with the turn policy plumbed and the compound-move representation reserved in the core from the start (§3.4). *Acceptance:* reconstruct chess/xiangqi/shogi, perft-validated. **Prototype 1 (representation):** perft + microbenchmark, wide-SIMD bitboard vs mailbox across sizes, to *find* the crossover. **Prototype 2 (cost):** a small self-play run testing whether the mobility prior + anchors recover classical piece values.
2. **Phase 1 — cost prior and the Bit encoder** (emitting embeddings and the cost prior), with anchors and the synergy term.
3. **Phase 2 — AI and self-play, built up the ladder.** The perfect-information rung and self-play harness first (measuring piece values); then the parameterized GT-CFR core with determinization/ISMCTS and OOS; then the R-NaD cap for intractable hidden play; then multiplayer. The belief-sharpness gate is introduced once two adjacent rungs exist. The deterministic inference grade (§10.6) ships with the first product-facing rung.
4. **Phase 3 — fog, stealth, line-of-sight** as the mask substrate plus the ladder, then abilities and terrain — including the **compound-move generator** and the ability catalog (§3.4).
5. **Phase 4+ —** *Subterranean Robot Wars*, then maker mode, network play, and the marketplace.

---

## 13. Risks and Open Questions

- **N-player victory, draw, and alliance rules — the remaining Tier-1 model gap.** Free-for-all raises kingmaking, collusion, elimination handling, and draw definitions that the model does not yet specify; the turn policy (§3.4) now owns rotation and elimination mechanics, but *what winning means* for N > 2 is unresolved. Next in the queue.
- **The gate is the central AI risk.** Mis-estimating belief sharpness or pivotality could route a position to too-cheap a rung (unsound) or too-expensive a rung (slow). The gate's signals (entropy, disambiguation, pivotality) are principled but their thresholds — or the learned meta-controller — must be tuned and validated; a safe default is to bias toward the next sounder rung when uncertain.
- **Rung interoperability.** One shared network must produce consistent values usable by alpha-beta, determinized search, GT-CFR, and as an R-NaD policy; verifying this consistency is an explicit test obligation.
- **Quantization parity (new in v0.4).** The deterministic grade must track the trained float net's strength within tolerance (§10.6); if quantization costs too much strength on procedural pieces, the shipped AI diverges from the trained one — a named, recurring test.
- **Compound-move branching in practice.** Compounds are bounded locally and priced (§3.4), but pathological Bit-sets (a high-mobility piece with act-twice) can still spike branching; the cost model and content vetting are the brakes, and the generator should expose a per-piece compound-count metric so vetting can catch spikes.
- **Representation crossover** — resolved in principle; exact crossover is Prototype 1.
- **Eval generalization, NNUE vs GNN** — whether embeddings/GNN generalize across the procedural space is the load-bearing empirical question.
- **Synergy-matrix scaling** — O(n²), pairwise-only; self-play remains the authority.
- **Scope** — several multi-year projects stacked; the layering and phase gates are the mitigation.

---

## References

1. Stockfish & NNUE — nnue-pytorch docs; Chess Programming Wiki, *Stockfish NNUE*.
2. Fairy-Stockfish — repository/wiki; maintainer notes on large-board representation and magic-table cost.
3. Board representation — Chess Programming Wiki, *Mailbox*/*Bitboard*; TalkChess, "Bitboard Tricks for Large Chess Variants" and "Bitboards vs. Mailboxes in the Era of NNUE" (mailbox parity; NNUE makes representation speed moot).
4. PEXT/PDEP — cozy-chess (Rust) docs; pre-Zen-3 AMD microcode (~18-cycle, ~250× slower); TalkChess parallel ray-scan.
5. AlphaZero — Silver et al. (2017), arXiv:1712.01815; Leela Chess Zero.
6. Poker / imperfect-info lineage — Zinkevich et al. (2007), CFR; Moravčík et al. (2017), DeepStack; Brown & Sandholm, Libratus (2017), Pluribus (2019).
7. Determinization & information-set search — Frank & Basin (strategy fusion, non-locality); Long, Sturtevant et al. (2010), *Understanding the Success of PIMC* (leaf correlation, bias, disambiguation factor); Cowling, Powley & Whitehouse (2012), *Information Set MCTS*; Lisý, Lanctot & Bowling, *Online Outcome Sampling* (equilibrium-convergent); Cazenave, α-μ; EPIMC.
8. Search + belief — N. Brown et al. (2020), ReBeL, arXiv:2007.13544.
9. Unified perfect/imperfect — M. Schmid et al. (2021/2023), *Player of Games / Student of Games*, arXiv:2112.03178, Science Advances (GT-CFR + counterfactual value-and-policy network; sound for both; expand-1 vs expand-top-k; weaker than AlphaZero in some perfect-info cases).
10. Hidden-identity / model-free Nash — Perolat, Tuyls et al. (2022), *Mastering Stratego* (DeepNash / R-NaD), Science, arXiv:2206.15378.
11. Automated balancing — asymmetric-game balancing (microRTS/SHAP); De Mesentier Silva (CCG meta-balancing); Volz et al. and CCG playtesting (Hearthstone); wargame point-cost estimation via regression + MCTS; *Metagame Autobalancing*; RaidEnv (play-tester generalization).
12. Generalizing evaluation & GGP — GNN/GCN board-game evaluators; *GNN Reasoner for GDL*; Ludii, Polygames, RBG, Simplified Boardgames, GBG.
13. R. Betza — fairy-piece movement notation.
14. Action economy & branching — O. Syed & A. Syed (2003), *Arimaa — a New Game Designed to be Difficult for Computers*, ICGA Journal; D. Fotland (2004), *Building a World-Champion Arimaa Program* (~300,000 four-step sequences; 20–30k distinct moves); D. J. Wu (2015), *Designing a Winning Arimaa Program*, ICGA Journal 38(1) (avg branching ~16.5k measured; Sharp wins the 2015 Challenge via branching-factor-directed search innovations); Wikipedia, *Computer Arimaa* (chess ≈ 35 vs Arimaa ≈ 17,000; 8-turn chess depth ≈ 3-turn Arimaa depth); Janzert, *Arimaa branching factor study*.
15. Floating-point determinism & lockstep practice — G. Fiedler, *Floating Point Determinism* (Gaffer on Games); SnapNet, *Netcode Architectures: Lockstep* (bitwise determinism requirement; cross-platform float hazards: instruction selection, reordering, vectorization, transcendental libraries); Gamedeveloper.com, *Minimizing the Pain of Lockstep Multiplayer* (same executable + hardware repeatable; cross-hardware/compiler is the hazard); fixed-point/integer simulation as the standard remedy; Terrano & Bettner (2001), *1500 Archers on a 28.8* (lockstep at production scale).

---

## Appendix A — Worked Bit-Mappings

### Western chess
| Piece | Bits |
|---|---|
| King | `leaper(0,1)` + `leaper(1,1)`; royal; castling |
| Queen | `rider(0,1)` + `rider(1,1)` |
| Rook | `rider(0,1)`; castling |
| Bishop | `rider(1,1)` (color-bound) |
| Knight | `leaper(1,2)` |
| Pawn | fwd move-only `leaper(0,1)` + fwd capture-only `leaper(1,1)` + double-step + en passant + last-rank transformation |

### Xiangqi
| Piece | Bits |
|---|---|
| Chariot | `rider(0,1)` |
| Horse | `leaper(1,2)` + path: adjacent orthogonal cell empty |
| Elephant | `leaper(2,2)` + path: `(1,1)` midpoint empty + condition: own half |
| Advisor | `leaper(1,1)` + condition: palace |
| Cannon | move `rider(0,1)` move-only; capture `hopper(0,1)` capture-only, one screen |
| General | `leaper(0,1)` + condition: palace + flying-general (`rider(0,1)` capture, target = enemy general only) |
| Soldier | `leaper(0,1)` fwd; past river add `leaper(0,1)` sideways; no promotion |

### Shogi
| Piece | Bits | Promotes to |
|---|---|---|
| King | `leaper(0,1)` + `leaper(1,1)`; royal | — |
| Rook | `rider(0,1)` | `rider(0,1)` + `leaper(1,1)` |
| Bishop | `rider(1,1)` | `rider(1,1)` + `leaper(0,1)` |
| Gold | `leaper(0,1)` + `leaper(1,1)` fwd-only | — |
| Silver | `leaper(1,1)` + `leaper(0,1)` fwd-only | gold |
| Knight | `leaper(1,2)` fwd-only (true jumper) | gold |
| Lance | `rider(0,1)` fwd-only | gold |
| Pawn | `leaper(0,1)` fwd | gold |

*Notes:* default capture-fate = send-to-hand (de-promoted); stalemate is a loss; drops obey the three tiers; *uchifuzume* forbids pawn-drop checkmate. All three variants run the base turn policy: one action, strict alternation (§3.4).

---

## Appendix B — Glossary

| Term | Meaning |
|---|---|
| Bot / Bit | a piece / an atomic cost-bearing rule fragment (Axis A or B). |
| Action / ply / turn | one piece's single act (move, ability, or drop) / one action / one player's consecutive plies; under the base rule all three coincide (§3.4). |
| Compound move | one generated move whose effect script has multiple steps (move+fire, act-twice); atomic, piece-local, priced by the cost model (§3.4). |
| Turn policy / pass policy | policy-layer ownership of rotation order, elimination handling, the multi-ply escape hatch, and pass legality with its termination guard (§3.4, §5). |
| Policy layer | per-variant rules of the world (royalty, turn/pass, win/draw, repetition, capture-fate, named predicates). |
| Mobility integral | average attacked-cell count over board positions; the cost prior's base term. |
| Anchor | a piece pinned to a fixed value to regularize the learned cost scale. |
| Bit-derived embedding | a learned vector encoding a piece's Bit-set, enabling evaluation of unseen pieces. |
| Belief sharpness | the entropy of the distribution over a hidden piece's consistent Bit-sets; the continuum's dial. |
| Disambiguation factor | how quickly hidden information is revealed by play; high → cheap determinized search is sound. |
| Determinization / PIMC | sample worlds consistent with the belief, solve each as perfect-info, aggregate. |
| Strategy fusion / non-locality | determinization's failure modes; the gate's escalation triggers. |
| ISMCTS | Information Set MCTS; search over information sets, reducing strategy fusion. |
| OOS | Online Outcome Sampling; equilibrium-convergent online imperfect-info search. |
| GT-CFR / SoG | growing-tree CFR (Student of Games); interpolates expand-1 (AlphaZero-like) ↔ expand-top-k (CFR-like). |
| R-NaD / DeepNash | Regularized Nash Dynamics; model-free, search-free RL converging to ε-Nash; mastered Stratego. |
| PBS | public belief state — a distribution over hidden information consistent with the public state. |
| The ladder / the gate | the belief-gated sequence of search modes / the dispatcher that selects among them. |
| Deterministic grade / performance grade | int-quantized bit-exact inference for all product-canonical paths / float-GPU inference for the training farm (§10.6). |
| State-bucket | the quantized per-instance state (HP/armor/ammo/cooldown) folded into the Zobrist key (§7.5). |
| Perft | leaf-node count at fixed depth, for differential move-gen validation. |

---

## Appendix C — Provenance & Changelog

### New in v0.4 (this revision)
Closes the two Tier-1 model gaps surfaced by the second gap analysis (July 2026), plus one directly-downstream infrastructure fold-in:

- **§3.4 Turn structure & action economy** — committed: **one turn = one action**, strict rotation; multi-action content compiles into **compound moves** (piece-local, atomic, priced); ply/turn formalized; the policy layer gains a **turn policy** and **pass policy**; multi-ply turns demoted to a calibration-excluded escape hatch; no simultaneity. Grounded in the Arimaa evidence (chess ≈ 35 vs Arimaa ≈ 17,000 branching from four steps per turn; the Challenge stood 2004–2015) — Reference 14.
- **§10.6 Determinism grades** — resolved the collision between the determinism principle (§10.1) and float neural inference (§7.4): **deterministic grade** (int-quantized NNUE, bit-exact, mandatory on all product-canonical paths) vs **performance grade** (float/GPU, training farm); replays defined as move-lists + RNG stream; server authority already covers netplay; GNN confined to performance grade; **quantization parity** named a recurring test obligation. Grounded in lockstep/fixed-point industry practice — Reference 15.
- **§7.5 hashing fold-in** — Zobrist keys extended to **mutable terrain** (per-cell terrain-type keys) and **per-instance state buckets**; repetition defined as full ground-truth state equality, judged by the arbiter core; monotone counters strengthen termination.
- Consequential touches: §4.1 gains a multi-action multiplier prior; §5 lists the new policies; §7.4 records the NNUE-first product ruling; §8.6/§8.7 cross-reference the turn model; §12 phases the compound generator and deterministic grade; §13 re-ranks risks — **the remaining Tier-1 gap is N-player victory/draw/alliance rules**, now first in the queue.

### v0.3 — architecture decision recorded
Three options were weighed (§8.4): **two separate brains** (rejected — the perfect-info engine needs the same learned eval anyway), **one unified algorithm everywhere** (rejected — Student of Games is weaker than specialized AlphaZero in the perfect-info regime, which is our common case), and **one shared brain with a belief-gated ladder** (committed). The decision is reversible; the rejected options remain documented so it can be revisited if prototyping warrants. v0.3 also added the information continuum (§8), the gate (§8.5), spectrum-spanning training (§9), caching promotion (§7.5), and the codex-as-belief-collapse coherence (§8.8), grounded in References 7–10.

### Carried from v0.2 (unchanged)
The Bit taxonomy and policy layer; the anchored cost model with synergy term; representation-per-class with the wide-SIMD option and the NNUE-makes-it-moot reframing; the PEXT caveat and ray-scan slider; SoA stateful design; NNUE+embeddings and the GNN option; the belief substrate; the Rust/C-ABI/C#/Python stack and determinism principle; licensing posture; and the prior provenance (Gemini adoptions, v0.2 research). See v0.2 Appendix C for that history.

---

*Botboard — Technical Specification, Version 0.4. One shared brain with a belief-gated ladder of search modes; one turn, one action, with multi-action compiled into priced compound moves; and two determinism grades so the learned evaluator never breaks the core's bit-exact guarantees.*
