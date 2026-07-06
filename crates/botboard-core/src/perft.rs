//! Perft: leaf counts for differential move-gen validation (§6).

use crate::game::GameDef;
use crate::movegen::{is_legal, legal_moves, pseudo_moves};
use crate::moves::move_str;
use crate::position::Position;

pub fn perft(g: &GameDef, pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    if depth == 1 {
        return pseudo_moves(g, pos)
            .iter()
            .filter(|mv| is_legal(g, pos, mv))
            .count() as u64;
    }
    let mut nodes = 0;
    for mv in legal_moves(g, pos) {
        let u = pos.make(g, &mv);
        nodes += perft(g, pos, depth - 1);
        pos.unmake(g, &u);
    }
    nodes
}

/// Per-root-move breakdown, for diffing against a reference engine.
pub fn divide(g: &GameDef, pos: &mut Position, depth: u32) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for mv in legal_moves(g, pos) {
        let u = pos.make(g, &mv);
        let n = if depth <= 1 { 1 } else { perft(g, pos, depth - 1) };
        pos.unmake(g, &u);
        out.push((move_str(g, &mv), n));
    }
    out.sort();
    out
}
