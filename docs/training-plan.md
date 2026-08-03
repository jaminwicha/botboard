# Botboard Training — Medium/Long-Term Plan

The strategy document for the learned-evaluator program. The companion
`docs/training-log.md` is the **record** (every experiment, config →
result); this file is the **plan** (phases, exit criteria, decision
gates). STATUS.md carries the one-line state. Update discipline: when a
phase's runs land, the log gets the numbers, this file gets the gate
decision, STATUS gets the summary line.

## The constraint chain (why this order)

Measured, not assumed (see training-log lessons 1–6):

```
corpus signal  →  corpus scale  →  net capacity  →  trainer throughput  →  inference speed
   (M1)             (M3)             (M4)              (M5a)                 (M5b)
```

The Aug 2026 H sweep proved capacity is not the front constraint: every
loss curve pins at ≈ln 2 because one-teacher mirrored-army corpora carry
near-zero label information. Fixing signal is cheap and unblocks
everything behind it; nothing downstream is worth compute until the loss
floor breaks.

---

## Phase M0 — Tooling prerequisites (low-hanging; do before everything)

Small tools that make every later phase cheaper to run and cleaner to
interpret. Each is hours of work, not days.

1. **Standalone probe command** (`botboard probe-srw --net A [--vs B]
   --pairs K --seed S`). Today the probe is welded into `train-net
   srw`, so re-probing an existing checkpoint forces a full retrain,
   and A/B-ing two checkpoints on SRW armies is impossible. Extracting
   it gives: M2 gate-stability tests on a FIXED net (the clean test —
   vary only probe armies), M4 sweeps probed after the fact at any
   power, and checkpoint-vs-checkpoint promotion gates (net-vs-net,
   not just net-vs-linear) for M3's retrain cadence.
2. **Corpus label-distribution stat.** `train-net srw` reports the
   win/draw/loss fraction of its own labels. M1's exit criterion
   (decisive fraction ≥ 40%) needs this number; today it is invisible.
   One print line plus a `TrainReport` field.
3. **Replay-based corpus storage principle (M3 design note, no code
   yet).** Do NOT serialize positions/GameDefs for the farm store —
   everything is deterministic per seed, so a stored game is just
   `(seed, params, move-log)` and a shard is a list of those; replay
   regenerates positions exactly. This keeps the store tiny, diffable,
   and version-proof (a replay on new engine code either matches
   hashes or loudly flags a behavior change — which doubles as a
   regression canary). At today's scale (65 s/corpus), "the corpus" is
   simply the `(seed, flags)` tuple in the log — regeneration IS
   persistence until M3.

**Status: ✅ SHIPPED (Aug 2026).** `probe-srw` (net-vs-linear and
net-vs-net), the decisive-label stat, and adjudicated probe scoring
all landed together; every later phase's runs use them.

## Phase M1 — Corpus signal (code + short runs; days)

**Goal:** training labels that carry information; loss curves that
descend.

Actionable steps:
1. **Style-diverse teachers** — `train-net srw` samples teacher evals
   from the league lattice (`population(n)` styles: material scale ×
   mobility), different styles per side. Decisive games rise; the
   student sees more than one value function's play. (Code: thread a
   style pair through `train_srw_from_selfplay`.)
2. **Adjudicated labels** — undecided games at the horizon label by
   material verdict (teacher eval sign at the final position, dead-zone
   → 0.5) instead of flat 0.5. (Code: small change in the outcome
   labeling.)
3. **Asymmetric budgets** — a fraction of games sample one side at
   ~120% budget so the label honestly says "favored side won".
   (Code: budget jitter in the corpus loop.)

**Exit criteria:** last-epoch loss ≤ 0.66 on a 800-game corpus (vs the
0.687–0.689 floor today) AND decisive-label fraction ≥ 40% (vs
draw-dominated today). **Gate:** if loss still pins near ln 2 with all
three, the descriptor featurization is the suspect — escalate to
descriptor v3 before spending any farm compute.

## Phase M2 — Probe & gate hardening (config + small code; days)

**Goal:** a promotion gate that can actually rank nets.

Actionable steps:
1. **Probe power now (config only):** `--probe-pairs 32` minimum on all
   future comparisons (64 games; ±~11% two-sigma on a 50% score —
   still coarse but halves today's noise).
2. **Probe adjudication (small code):** score drawn probe games by
   material verdict at the end position instead of 0.5, so the draw
   mass stops hiding differences.
3. Keep the ≥55% PROMOTE bar; require it at ≥32 pairs.

**Exit criteria:** probing a FIXED checkpoint (M0.1) under two disjoint
probe seeds yields scores within 6 points. **Status: ✅ MET (Aug
2026)** — raw Δ1.6, adjudicated Δ5.5 on srw_net_m2_s23 (log lesson 7).
Two rules follow, binding on all later phases: (a) recipe comparisons
run at a FIXED training seed (or averaged over 2–3 seeds — same-config
retrains spread ~8 points); (b) the ADJUDICATED score is the score —
adjudication revealed the current nets sit at 12–18% vs the teacher,
not the draw-flattered 40–45%.

## Phase M3 — The corpus farm (ops; runs for days/weeks unattended)

**Goal:** 10⁵–10⁶ samples of signal-bearing SRW games; the first
genuinely long-term training loop. This is STATUS scale-rung 4.

Actionable steps:
1. **Farm script** (`scripts/` — new): N `botboard` processes, disjoint
   seed ranges, each appending games/samples to a shard file; a merge
   step concatenates shards deterministically (the per-seed actor pool
   makes merges reproducible). Nightly cadence.
2. **Retrain cadence:** retrain from the accumulated store at each 2×
   growth in corpus size; probe at ≥32 pairs; log every run in
   training-log.md.
3. **Sample-efficiency curve:** track probe score vs corpus size — the
   scaling signal that justifies (or kills) further farming.

**Exit criteria:** probe score improves monotonically with corpus
doublings across ≥3 points. **Gate:** flat scaling curve at ≥100k
samples → capacity or featurization is binding; go to M4.

## Phase M4 — Capacity re-sweep (config; hours per point)

**Goal:** re-ask the H question on a corpus that carries signal.
Runtime-H is shipped (`--hidden`); this phase is pure runs: H ∈ {32,
64, 128, 256} on the best M3 corpus, ≥32-pair adjudicated probes.

**Exit criteria:** either a PROMOTE (done — ship the checkpoint into
the SRW FFI default path and the league), or a monotone capacity curve
that names the next width, or a flat curve that rules capacity out
again (→ descriptor v3).

## Phase M5 — Throughput decisions (only when they bind)

- **M5a Trainer:** per-sample Rust SGD is fine at ~15k samples; at
  ~200k+ it binds. Decision then, not before: batched/parallel SGD in
  Rust (cheapest, likely 5–10×) vs the Training Spec §10.2 PyO3/PyTorch
  performance-grade path (heavier, buys GPU training + optimizer
  variety). Default: batch the Rust trainer first.
- **M5b Inference (rung 5, GPU):** only after promoted nets reach
  widths (H≥256) that starve the CPU int path, and only for the
  training farm — product-canonical play stays on the deterministic
  int grade.

## Continuous loops (no finish line; schedule once M1–M2 land)

- **League refresh:** re-run the 14-member lattice league at deeper
  `--games` periodically; Nash mix feeds teacher sampling (M1.1) and
  value targets.
- **Synergy refit** (training-report follow-up): refit S_ij from a
  corpus including the full Axis-B vocabulary; renormalize the logistic
  scale k before shipping any scaled-up material table (standing
  advisory).
- **Gate retraining:** rung-choice thresholds refit when rungs or nets
  change.
- **Campaign telemetry** (post-Godot-integration): encounter outcome
  streams feed difficulty dials and content vetting (SRW spec §13).

---

## Execution & monitoring protocol (how this plan runs day to day)

Roles: **planner** (this file — humans + the design session), **runner**
(background processes or a delegated subagent executing a phase's
runs), **monitor** (a delegated subagent that watches runs, applies the
EXIT CRITERIA above verbatim, and writes results to training-log.md).

Rules of engagement for a monitor/runner agent:
1. Everything it launches is deterministic-per-seed; record every seed.
2. Results land as a training-log.md table row + a lessons bullet when
   a gate decision follows; it never edits exit criteria (that's a
   planner change) — if a criterion seems wrong, it reports back
   instead.
3. HOLD/PROMOTE calls follow M2's bar (≥55% at ≥32 pairs); anything
   else is "evidence", not a verdict.
4. Long runs (M3 farm) run as background processes with a
   session-schedule (loop/cron) check-in cadence; short runs (M1/M2/M4)
   run synchronously inside the delegated agent.
5. Checkpoints land in `artifacts/` with descriptive names
   (`srw_net_<phase>_<config>.bin`); nothing overwrites a promoted
   checkpoint.

Current delegation state: see training-log.md's experiment table for
what has already run; M1 items 1–3 and M2 item 2 are the open
code-bearing steps.
