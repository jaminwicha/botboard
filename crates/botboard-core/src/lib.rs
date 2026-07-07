//! Botboard core engine — Phase 0 (spec v0.4 §12).
//!
//! Headless, deterministic, integer-only rules core. Pieces ("Bots") are
//! composed from atomic rule fragments ("Bits", §3); at init each piece type
//! compiles its Axis-A movement into fast kernels (§7.3); the policy layer
//! (§5) holds per-variant world rules. Acceptance: chess, xiangqi, and shogi
//! reconstructed purely from Bits and validated by perft (§6).
//!
//! Phase-0 board representation is the mailbox + piece-list path (§7.1);
//! the bitboard/SIMD classes and the empirical crossover are Prototype 1.

pub mod belief;
pub mod bits;
pub mod cost;
pub mod eval;
pub mod fen;
pub mod game;
pub mod geometry;
pub mod ladder;
pub mod league;
pub mod movegen;
pub mod moves;
pub mod perft;
pub mod position;
pub mod rng;
pub mod search;
pub mod selfplay;
pub mod training;
pub mod variants;
pub mod zobrist;
