# Training & Balance Report — 2026-07-07

Run by the training/balance subagent per `docs/training-subagent-brief.md`
(botboard repo). Provenance and reproducibility:

- **botboard** @ `75d9bc5` — treated as READ-ONLY (all cargo runs used a
  redirected `CARGO_TARGET_DIR`; no file in that repo was created or
  modified). Binaries built `--release` (lto, codegen-units=1).
- **SRW** — this worktree, branch `training-balance-pass` (parent
  `f13b15b`). SRW `src/` untouched; harnesses live in `tools/`.
- Wall clock for everything below: ~50 minutes (the brief budgeted ~40;
  the overage is entirely the veilworks vetting shard, which turned out
  to be a real perf finding rather than a tuning run). The only scaled-
  down cell is veilworks (tier 0, budget 8, 3 games/seed instead of
  tier 1 × {8,14,20} × 6 games — see §4); every other target was met or
  exceeded.

## 1. Net training scale-up (`botboard train-net chess`)

Trainer internals (fixed by the CLI, unchanged): self-play depth 2,
lr 0.01, selfplay seed base **11** (game *i* uses seed 11+*i*), H=7
(FloatNet::new(7)).

| run | games | epochs | samples | first loss | last loss | Δloss | time |
|---|---|---|---|---|---|---|---|
| shipped default | 24 | 6 | 561 | 0.6950 | 0.6852 | −0.0098 | ~2 s |
| **chess_net_v2** | **200** | **12** | **5801** | **0.6759** | **0.6587** | −0.0172 | 21.4 s |
| probe (diminishing-returns check) | 400 | 16 | 11244 | 0.6722 | 0.6534 | −0.0188 | 47.8 s |

- Checkpoint shipped: `training-artifacts/chess_net_v2.bin` (12,564 bytes,
  FloatNet serialization; deterministic QuantNet grade derives at load).
- The 400-game probe buys only −0.0053 more final loss for 2.2× compute —
  200/12 is a sensible new default; kept as v2.
- **Checkpoint sanity**: v2 parses via `FloatNet::from_bytes`, quantizes via
  `QuantNet::from_float`, and a depth-3 search on the loaded net completes
  (best `b2b3`, score 114 cp from startpos).
- **NNUE parity suite**: `cargo test -p botboard-core --test nnue_suite` —
  **4/4 pass** (`quantization_parity`,
  `training_learns_and_quantized_grade_is_deterministic`,
  `generalizes_to_unseen_procedural_pieces`,
  `searcher_runs_on_the_deterministic_net`), 1.58 s.

## 2. Value correction + synergy refit (larger samples)

Harness: `tools/botboard-tuning` (path-dep on botboard-core; exact same
entry points and hyperparameters as the committed suites, only sample
counts raised). Self-play: `play_game`, depth (2,2), nodes (10k,10k),
opening_random 6, max 100 plies, sample_every 6, **seeds 0..N** (the
committed test's scheme). Correction: `correct_values`, anchor **P=1.0**,
lr 0.02.

Prior material (centipawns): K 0, Q 871, R 504, B 351, N 323, P 100.

| type | prior | test-scale (4 games, 31 samples, 3 ep) | scaled-up (64 games, 630 samples, 8 ep) | Δ scaled |
|---|---|---|---|---|
| queen | 871 | 867 | **1492** | +621 |
| rook | 504 | 502 | **845** | +341 |
| bishop | 351 | 351 | **551** | +200 |
| knight | 323 | 319 | **512** | +189 |
| pawn (anchor) | 100 | 100 | 100 | 0 |

Scaled-up outcome split: side0 +22 / side1 +15 / =27 over 64 games.

Reading: with real signal (630 samples vs 31) the logistic fit *stretches*
every piece value relative to the pawn anchor (Q/P 8.7 → 14.9) while
preserving the ordering Q > R > B > N. That is the fit compensating for
the fixed k=0.7 logistic scale, not a claim that a queen is worth 15
pawns — treat the deltas as directional (pieces are under-valued relative
to pawns at depth-2 outcomes), and renormalize k before ever shipping
these as material tables.

**Synergy refit** (`SynergyModel::fit`, planted rider×forward-only = +1.5,
lr 0.05):

| run | samples | epochs | planted(1,3) | err | control(0,1) |
|---|---|---|---|---|---|
| test-scale | 250 | 200 | 1.4851 | −0.0149 | 0.0000 |
| scaled-up | 2500 | 400 | 1.4851 | −0.0149 | 0.0000 |

The fitter is already at its fixed point at test scale — the −0.015
residual is the L2 regularizer's (0.01) shrinkage, not sample noise.
Applied to real chess `bit_categories`, the planted (1,3) pair co-occurs
in **no** chess piece (Q/R/B are cat[1], N/K cat[0], P cats[0,3,7]), so
`cost_prior_with_synergy` == `cost_prior` for every type — a clean
negative control.

## 3. League health (`botboard league chess --games 4`)

Fixed by the CLI: round-robin depth 2, node budget 20k, base seed 7,
`nash_averaging` 5000 iterations. Committed profile was produced at
2 games/pair; this run doubles it. Artifact:
`training-artifacts/league_profiles_v2.json`.

| profile | nash (committed, 2 g/pair) | nash (this run, 4 g/pair) | rating (old → new) |
|---|---|---|---|
| solid | 0.0001 | 0.0001 | 0.375 → **0.125** |
| balanced | 0.0001 | 0.0001 | 0.375 → 0.500 |
| active | 0.5000 | **0.9999** | 0.500 → 0.500 |
| berserk | 0.5000 | **0.0001** | 0.500 → 0.500 |

**Drift**: the 50/50 active–berserk mixture collapses to essentially pure
**active** (material_scale 0.9, mobility 2→8 cp). At 4 games/pair berserk
loses its edge over solid (payoff 0.500 → 0.250) — its Nash share at
2 games/pair was sampling noise. "Solid" (over-weighted material, low
mobility) craters to 0.125 rating. Two takeaways: (a) the committed
profile file is under-sampled and should be regenerated at ≥4 games/pair;
(b) mobility-forward styles dominate at these depths — relevant to SRW
tier tuning below.

## 4. SRW balance sweep (read-only)

`dotnet test tests/SRW.Core.Tests --filter Vetting` (this worktree, real
engine dylib built from botboard @75d9bc5): **2/2 pass**, ~1 s.

Per-clan sweep (`tools/VetSweep`): every built-in clan × budgets
{8, 14, 20} × seeds {31, 32}, 6 games each, **tier 1** (VetClan default:
depth 3 / 20k nodes / 4 determinizations / 128 OOS iters), 8×8 board,
mirror armies (same clan both sides), cold-open intel both ways.

| clan | budget | seed | side0 win | draws | mean plies | flags |
|---|---|---|---|---|---|---|
| scrapline | 8 | 31 | 0% | 67% | 134.2 | — |
| scrapline | 8 | 32 | 0% | 83% | 185.2 | — |
| scrapline | 14 | 31 | 0% | 67% | 129.5 | — |
| scrapline | 14 | 32 | 17% | 50% | 139.8 | — |
| scrapline | 20 | 31 | 0% | 67% | 129.5 | — |
| scrapline | 20 | 32 | 33% | 33% | 133.5 | — |
| foundry | 8 | 31 | 17% | 83% | 42.2 | — |
| foundry | 8 | 32 | 0% | 50% | 20.8 | — |
| foundry | 14 | 31 | 0% | 50% | 27.8 | — |
| foundry | 14 | 32 | 17% | 67% | 45.7 | — |
| foundry | 20 | 31 | 33% | 17% | 29.7 | — |
| foundry | 20 | 32 | 33% | 67% | 59.3 | — |
| deepcore | 8 | 31 | 0% | **100%** | 36.7 | **inert armies: 100% draws** |
| deepcore | 8 | 32 | 17% | 67% | 31.0 | — |
| deepcore | 14 | 31 | **83%** | 0% | 17.8 | **lopsided mirror: side0 83%** |
| deepcore | 14 | 32 | 50% | 17% | 15.8 | — |
| deepcore | 20 | 31 | 67% | 0% | 18.3 | — |
| deepcore | 20 | 32 | 33% | 17% | 15.7 | — |
| veilworks | — | — | — | — | — | **DNF at tier 1 — see below** |

Veilworks scaled-down probe (honest fallback after the tier-1 DNF:
**tier 0**, budget 8 only, **3 games**/seed):

| clan | budget | seed | side0 win | draws | mean plies | flags |
|---|---|---|---|---|---|---|
| veilworks | 8 | 31 | 0% | **100%** | 50.7 | **inert armies: 100% draws** |
| veilworks | 8 | 32 | 0% | **100%** | 50.7 | **inert armies: 100% draws** |

Notes:

- **Determinism cross-check**: the foundry 20/seed-31 cell was executed in
  two separate processes and reproduced bit-identically (33% / 17% / 29.7).
- **Veilworks DNF (perf pathology, raised as a flag)**: the veilworks
  shard ran > 12 min wall (~95 CPU-min, the engine's parallel pool busy
  throughout) without completing even the first cell (budget 8, seed 31,
  tier 1), while every other clan's full 6-cell row finished in minutes.
  Veilworks is the only clan whose ability palette combines `Pit(2)` and
  `Heal(1,1)` — the prime suspect is heal-loop plies × tier-1 OOS
  (128 iters, 4 determinizations) blowing up per-move cost. This
  reproduces with
  `tools/VetSweep/bin/Release/net8.0/VetSweep veilworks 8` and deserves
  an engine-side profile before veilworks rooms ship at tier ≥ 1.
- Aggregates over completed cells (36 games/clan): scrapline s0 8%,
  draws 61%, ~142 mean plies; foundry s0 17%, draws 56%, ~38 plies;
  deepcore s0 42%, draws 34%, ~23 plies. Second player wins ~3.7× more
  often than the first for scrapline (31% vs 8%) and ~1.7× for foundry;
  deepcore flips it (s0 42% vs s1 25%).

## 5. Tuning suggestions for `EncounterFactory`

Grounded in the sweep, five concrete changes to
`EncounterFactory.BudgetFor` / `TierFor` (and the two knobs immediately
adjacent to them):

1. **Clamp budgets to a per-clan floor in `BudgetFor`.** Deepcore is
   inert at budget 8 (100% draws — two or three expensive riders can't
   force a win) but sharp and decisive at 14+. Its `MinDepth = 2` means
   the *skirmish* curve already yields 13 there, but any future path that
   hands a clan a sub-floor budget (events, dismantle rewards, ambushes
   with modifiers) degenerates. Concretely:
   `return Math.Max(clan.BudgetFloor, kindCurve)` with floors ≈
   scrapline 6, foundry 8, veilworks 8, deepcore 12.

2. **Flatten the early skirmish slope for the scrapline band.** Scrapline
   mirrors are 130–185-ply slogs with 33–83% draws *at every budget* —
   more budget buys more chaff, not more resolution. At depths 0–2 (the
   only place scrapline spawns) prefer `7.0 + 2.0 * depth` over
   `8.0 + 2.5 * depth`: smaller early armies shorten the grind without
   changing difficulty, and the freed pacing matters more than the ~2
   points of chaff. (If slogs persist, cap army size via the generator's
   `maxPieces` for swarm clans rather than budget.)

3. **Do not raise `TierFor` to fix pacing — it does nothing for draws
   and it is dangerous for veilworks.** All the drawish rows above were
   *already* tier 1 (depth 3 / 20k nodes / 4 determinizations / 128 OOS
   iters), and veilworks could not finish a single tier-1 mirror game in
   12 minutes. Until the heal-loop cost is profiled engine-side, gate
   veilworks rooms to tier 0 explicitly in `TierFor`
   (`room.ClanId == "veilworks" ? 0 : …`) — at 8 plies/s it is the only
   clan where dial 2 has a wall-clock failure mode, not just a strength
   one.

4. **Compensate the mover asymmetry per clan, not globally.** Mirror
   games are systematically second-player-favored for scrapline
   (8% vs 31%) and foundry (17% vs 28%), but *first*-player-favored for
   deepcore (42% vs 25%). Since the enemy army is generated at exactly
   `BudgetFor(room)` and the player moves first in battles, a flat
   `* 1.05` enemy premium would overshoot half the clans. Cheapest fix
   at the factory: scale enemy budget by a small per-clan factor
   (scrapline ×1.00, foundry ×1.00, deepcore ×0.95 if the player moves
   second in that room, inverse if first) once the mover convention is
   pinned down in `Battle`.

5. **Vet at build time using the room's own (clan, budget) pair.** The
   two hard flags (deepcore@8 inert, deepcore@14/31 lopsided) and the
   veilworks heal-stall would all be caught by running
   `Vetting.VetClan(gen, clan, BudgetFor(room), 4, runSeed, TierFor(room))`
   for each *new* (clan, budget-bracket) combination per run and
   regenerating on a flag — 4 games at tier 0–1 costs well under a
   second for every clan except veilworks (which is precisely the point:
   the vet would have refused to ship those rooms). Cache verdicts per
   bracket so it is a one-time cost per run.

One engine-side note that is not an `EncounterFactory` knob but blocks
suggestion 3's removal: profile tier-1+ move selection for armies with
`Heal` — ~95 CPU-minutes without completing six budget-8 games is a
pathology, not a tuning issue.

## Test gates

- `cargo test --workspace` (botboard @75d9bc5, redirected target dir):
  **44 passed, 0 failed** across 15 suites (4 ignored = the `#[ignore]`d
  heavy statistical runs, as committed).
- `dotnet test tests/SRW.Core.Tests --filter Vetting`: 2/2 pass.

## Artifacts (this worktree, branch `training-balance-pass`)

- `training-artifacts/chess_net_v2.bin` — the new checkpoint.
- `training-artifacts/league_profiles_v2.json` — 4-games/pair league rerun.
- `tools/botboard-tuning/` — Rust harness for §2 (path-dep, read-only).
- `tools/VetSweep/` — C# harness for §4 (references SRW.Core.csproj).
- `native/libbotboard.dylib` — engine cdylib built for the test runs
  (untracked; rebuild with `cargo build --release -p botboard-ffi`).

## Addendum (2026-07-10): veilworks DNF root-caused and fixed

The §4 veilworks tier-1 DNF was profiled engine-side (`crates/botboard-ffi/
examples/srw_profile.rs` over exact sweep-game fixtures dumped with
`VetSweep dump`). The report's hypothesis — heal-loop plies × tier-1 OOS
cost — was wrong on both counts:

- **OOS was innocent.** Rung 2 runs ~50–60 ms/move at tier 1. Games 0–2
  and 4–5 of the DNF cell complete in 0.04–3.4 s each.
- **The stall was one `qsearch` call.** Game 3's very first `ai_move`
  (rung 0/1 negamax) never returned: a 15 s `sample` put 100 % of stacks
  in quiescence recursion. `qsearch`'s capture filter admitted any move
  onto an *occupied* square — which includes friendly-target `Heal`
  abilities. Captures terminate quiescence because material/HP strictly
  falls; heal *undoes* armor-strike damage, so damage/heal lines had no
  monotone bound and the ply-64 cap left a ~b^64 tree. The node budget
  was also never checked inside `qsearch`.

Fix (botboard `search.rs`): quiescence now recurses only on
**enemy-occupied targets** (every line strictly reduces opponent HP) and
stands pat once the node budget is exhausted. Deterministic; chess/
xiangqi/shogi are ability-free and unaffected; all suites green plus a
new regression (`heal_army_tier1_battle_terminates_deterministically`,
fixture = the exact stalling game).

Post-fix: the full veilworks tier-1 cell (6 games) completes in seconds;
tier-2 informed play dispatches rung 0 as assumed. The `EncounterFactory`
veilworks tier cap (suggestion 3) is therefore lifted. The "inert armies:
100 % draws" flag on veilworks mirrors persists at tier 1 (mean ~51
plies, threefold repetition) — a balance item, not a perf one.
