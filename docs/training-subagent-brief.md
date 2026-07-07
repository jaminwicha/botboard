# Training & Balance Subagent — Mission Brief

Owner: background agent (worktree-isolated on botboard). The main session is
concurrently doing SRW graphics work — do not touch the SubterraneanRobotWars
repo except to READ it or run its tests.

## Goal

Strengthen the shipped evaluator and produce a balance report for SRW,
using the existing loops — no new architecture.

## Tasks (in order, commit each on your worktree branch)

1. **Net training scale-up** (botboard): use `botboard train-net` (or the
   `nnue::train_from_selfplay` API) to train a chess net with materially
   more games/epochs than the shipped default (target ≥200 games, ≥12
   epochs, H stays as-is). Save checkpoint `chess_net_v2.bin`. Verify the
   nnue parity suite still passes (`cargo test -p botboard-core nnue`).
2. **Value correction + synergy refit**: run `selfplay::correct_values` and
   `SynergyModel::fit` at larger sample counts via the existing test-style
   entry points; record before/after per-type values in the report.
3. **League health**: `botboard league chess --games 4` (or API), capture
   `league_profiles.json`, note Nash mixture drift vs the committed one.
4. **SRW balance sweep** (read-only on the SRW repo): run
   `dotnet test tests/SRW.Core.Tests --filter Vetting` and then write a
   small standalone harness in YOUR worktree (or a dotnet script) that
   calls `Vetting.VetClan` per built-in clan at budgets {8, 14, 20},
   6 games each, seeds {31, 32}. Tabulate win rates / mean plies / flags.
5. **Report**: `docs/training-report.md` in your worktree — what ran, all
   numbers, flags raised, and 3–5 concrete tuning suggestions for SRW's
   `EncounterFactory.BudgetFor/TierFor` curves.

## Constraints

- Determinism: fixed seeds everywhere; note every seed in the report.
- Do not change engine semantics or public APIs; training/config only.
- Keep total wall-clock sane (< ~40 min); scale counts down if needed and
  say so in the report.
- `cargo test --workspace` must be green at the end.
