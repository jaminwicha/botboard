# Botboard ML Training Log

The **living record** of the learned-evaluator work: the approach, every
promotion experiment (config → result), and the lessons each one bought.
Newest entries at the bottom of each section. Companions:
`docs/training-report.md` (the July 2026 training campaign, narrative),
`STATUS.md` (scale ladder + one-line outcomes), `Botboard_Training_Spec.md`
(the design the experiments serve).

## The system in one paragraph

One shared evaluator sits under every search rung (Training Spec §2): an
NNUE-style two-perspective accumulator over **Bit-derived descriptors**
(21 dims/type — movement/armor block, per-ability-kind signals, Appendix-B
flags, bias — no type ids, so unseen procedural pieces evaluate sensibly),
one ReLU hidden layer (width **runtime-sized** since Aug 2026; shipped
checkpoints before that are H=32), scalar value head. Two grades (§10.6):
f32 SGD training, bit-exact i16/i32 inference, with tested quantization
parity (≥90% chosen-move agreement, ≤20cp drift). Teachers are the
anchored linear cost eval; corpora are self-play. Promotion is empirical:
paired-army probes vs the teacher, ≥55% to PROMOTE.

## Recipe as of Aug 2026

`train-net srw [--games N --epochs E --budget B --depth D --plies P
--hidden H --probe-pairs K --out F]`

- fresh sampled robot armies per game (budget-priced, mirrored, 8×8)
- teacher: linear cost eval, both sides, `--depth` search (default 3)
- value targets: outcome from side 0, snapshots every 3 plies after
  ply 6, horizon `--plies` (default 240; undecided = 0.5)
- probe: quantized net vs linear teacher, paired armies + paired
  openings, colors swapped, depth 3 / 20k nodes

## Promotion experiments (SRW content nets)

| date | id | config | corpus | loss | probe vs linear | verdict |
|---|---|---|---|---|---|---|
| Jul 2026 | srw_net_v1 | H=32, depth 2, 160 plies, 400 g / 8 ep | 8k samples | — | 3–18–3 = 50.0% | HOLD |
| Jul 2026 | (unnamed) | H=32, depth 2, 160 plies, 2000 g / 12 ep | ~56k | — | 37.5% | HOLD — more corpus alone is WORSE |
| Aug 2026 | srw_net_v2_candidate | H=32, **depth 3, 240 plies**, 800 g / 10 ep | 15.6k | 0.692→0.689 (flat) | 0–26–6 = 40.6% | HOLD — deeper teacher strengthens the baseline too |
| Aug 2026 | srw_net_h64 | **H=64**, depth 3, 240 plies, 800 g / 10 ep | 15.6k | 0.6915→0.6873 | 4–21–7 = 45.3% | HOLD |
| Aug 2026 | srw_net_h128 | **H=128**, depth 3, 240 plies, 800 g / 10 ep | 15.6k | 0.6914→0.6866 | 0–23–9 = 35.9% | HOLD — non-monotonic in H |

## Lessons bought so far

1. **The linear teacher is a strong baseline on armies its own cost
   prior priced.** The probe is materially self-referential: the armies
   are budget-balanced BY the teacher's value function, so the teacher
   starts on home turf. A net must find non-material structure to beat
   it there.
2. **More corpus alone regresses** (2000-game run probed worse than
   400): with capacity fixed, extra samples of the same teacher's play
   don't add signal, they average it.
3. **Deeper teacher games don't promote either** (40.6% < 50%): the
   probe plays at the same depth, so teacher-strength gains cancel; and
   the near-flat loss curve (0.692→0.689 over 10 epochs at H=32) says
   the model, not the data, is the constraint.
4. **Draw-heavy probes blunt the signal**: 26/32 draws in the depth-3
   run. If wider nets keep drawing, consider probe adjudication or
   longer probe games before reading the percentage as truth.
5. **Chess promotion worked at H=32** (v3 beat v2 72.5% in the paired
   netmatch gate): one variant, one fixed GameDef, dense positional
   signal. The SRW setting is harder — fresh armies per game spread the
   descriptor space thin. Capacity was the natural suspect, hence the
   H=64/128 experiments.
6. **Capacity is NOT the silver bullet at this corpus** (H sweep,
   identical recipe: 32 → 40.6%, 64 → 45.3%, 128 → 35.9%). Two reads,
   both actionable:
   - **Every loss curve is pinned at ≈0.69 ≈ ln 2.** The corpus labels
     are near-uninformative: mirrored budget-balanced armies with the
     SAME teacher on both sides produce mostly draws and coin-flip
     outcomes. No width can learn signal that isn't there. The corpus,
     not the model, is the front constraint — capacity only becomes
     testable after the labels carry information.
   - **A 32-game probe cannot rank near-tied nets** (the three scores
     differ by a handful of games). Widen `--probe-pairs` or adjudicate
     before trusting single-digit deltas.

## Infrastructure changes

- **Aug 2026 — runtime-sized H.** `FloatNet`/`QuantNet` carry their
  width; `FloatNet::with_h(h, seed)` (fan-in-tempered init);
  BBNET002 headers always stored H, so every shipped checkpoint loads
  unchanged and old readers reject wide checkpoints loudly. `--hidden`
  on both `train-net` paths. Parity obligations re-proven at H=64
  (`nnue_suite::runtime_width_roundtrips_and_keeps_parity`).
- **Aug 2026 — recipe knobs.** `--depth` (teacher depth, was fixed 2)
  and `--plies` (value horizon, was fixed 160) on `train-net srw`.
- **Aug 2026 — league lattice.** `league --members N` over
  `population(n)`: 4 seeds + a deterministic 2-D style lattice
  (material scale × mobility). 14-member chess run produced Nash
  support on 5 members — spread achieved for future value targets.

## Next levers, in order of current belief (revised after the H sweep)

1. **Corpus signal first**: the ln-2 loss floor says the labels are the
   constraint. Options, cheapest first: (a) style-diverse teachers per
   side (the league lattice — different material scales/mobility make
   games decisive), (b) adjudicate undecided training games to a
   material verdict instead of labeling 0.5, (c) asymmetric army
   budgets so one side is honestly favored and the label says so.
2. **Probe power**: more pairs (K=32+) and/or draw adjudication before
   comparing recipes; current 32-game probes are noise-dominated.
3. **Capacity re-test** — AFTER (1): re-run the H sweep on a corpus
   whose loss actually descends; runtime-H is shipped and waiting.
4. **Cross-palette synergy refit** (training-report follow-up).
5. **Descriptor v3**: per-custom-ability dims — matters once campaign
   content leans on custom effects.
