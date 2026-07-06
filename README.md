# Botboard

A compositional engine for heterogeneous fairy chess — pieces (**Bots**) are
assembled from atomic, cost-bearing rule fragments (**Bits**) — and the
performance foundation for *Subterranean Robot Wars*.

Specs: [engine](Botboard_Spec_v4.md) · [training & self-play](Botboard_Training_Spec.md) ·
implementation traceability: [STATUS.md](STATUS.md).

## Layout

- `crates/botboard-core` — the headless Rust core: Bit system, compile step,
  move generation, policy layer, Zobrist/TT, alpha-beta search, cost model,
  belief substrate, the belief-gated search ladder, self-play + league.
- `crates/botboard-cli` — the simple UI: play/selfplay/cost/league/perft.
- `crates/botboard-ffi` — the C ABI boundary (`cdylib`, opaque handle).

## Quick start

```sh
cargo test --release                 # acceptance + engine + self-play suites
cargo test --release -- --ignored    # deep perft & statistical matches

cargo run --release -p botboard-cli -- play chess --depth 4
cargo run --release -p botboard-cli -- play shogi --hidden   # imperfect info
cargo run --release -p botboard-cli -- cost chess            # anchored prior
cargo run --release -p botboard-cli -- league chess          # population + Nash
cargo run --release -p botboard-cli -- armies --budget 20    # generated pieces
cargo run --release -p botboard-cli -- perft xiangqi 5
```

## Acceptance status

Chess, xiangqi, and shogi are reconstructed purely from Bits and validated
move-for-move by perft (chess d6 = 119,060,324 incl. Kiwipete/CPW positions;
xiangqi d5 = 133,312,995; shogi d5 = 19,861,490 with drops, *nifu*,
*uchifuzume*). The anchored cost prior recovers classical piece values
(Q 8.7 / R 5.0 / B 3.5 / N 3.2 / P 1.0).
