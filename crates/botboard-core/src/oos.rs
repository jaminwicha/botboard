//! Rung 2 — sound game-theoretic search (§8.2): a depth-limited
//! **external-sampling MCCFR / OOS** over information sets.
//!
//! The information set is the public action history (piece locations and
//! actions are public in the hidden-identity regime; identities are not),
//! so both players' infosets are well-defined across sampled worlds. Each
//! iteration samples a belief-consistent world (chance), regret-matches at
//! every visited infoset, explores all of the updating player's actions
//! (external sampling), samples the opponent's, and scores depth-capped
//! leaves with the shared evaluator squashed to [0, 1]. The average
//! strategy at the root converges toward equilibrium in the depth-capped
//! game — this is the rung that can bluff and value information correctly.

use std::collections::HashMap;

use crate::belief::{determinize, Belief};
use crate::game::GameDef;
use crate::movegen::legal_moves;
use crate::moves::{move_str, Move};
use crate::position::Position;
use crate::rng::Rng;
use crate::search::Searcher;

struct InfoNode {
    moves: Vec<Move>,
    regret: Vec<f64>,
    strat_sum: Vec<f64>,
}

impl InfoNode {
    fn strategy(&self) -> Vec<f64> {
        let pos_sum: f64 = self.regret.iter().map(|r| r.max(0.0)).sum();
        if pos_sum > 1e-12 {
            self.regret.iter().map(|r| r.max(0.0) / pos_sum).collect()
        } else {
            vec![1.0 / self.moves.len() as f64; self.moves.len()]
        }
    }
}

pub struct OosConfig {
    pub iterations: u32,
    /// Ply cap per sampled episode (the depth-limited game).
    pub depth: u32,
    /// Eval squash scale (centipawns → [0,1]).
    pub scale: f64,
}

impl Default for OosConfig {
    fn default() -> Self {
        OosConfig { iterations: 256, depth: 4, scale: 300.0 }
    }
}

/// Infoset key: rolling hash of the public action history + actor.
fn child_key(key: u64, g: &GameDef, mv: &Move) -> u64 {
    let mut h = key ^ 0x9E3779B97F4A7C15;
    for b in move_str(g, mv).bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001B3);
    }
    h
}

pub struct Oos {
    nodes: HashMap<u64, InfoNode>,
}

impl Oos {
    pub fn new() -> Self {
        Oos { nodes: HashMap::new() }
    }

    /// Run OOS from `truth` under `belief` and return the average-strategy
    /// best move at the root, with the root strategy for inspection.
    pub fn choose(
        &mut self,
        g: &GameDef,
        truth: &mut Position,
        belief: &Belief,
        searcher: &mut Searcher,
        cfg: &OosConfig,
        rng: &mut Rng,
    ) -> Option<(Move, Vec<f64>)> {
        let root_moves = legal_moves(g, truth);
        if root_moves.is_empty() {
            return None;
        }
        // Seed the root infoset with the arbiter's legal set, so no root
        // move is invisible just because the first sampled world forbade it.
        self.nodes.entry(1).or_insert_with(|| InfoNode {
            regret: vec![0.0; root_moves.len()],
            strat_sum: vec![0.0; root_moves.len()],
            moves: root_moves.clone(),
        });
        let updating_root = truth.stm;
        for it in 0..cfg.iterations {
            let mut world = determinize(g, truth, belief, rng, 32);
            // Alternate the updating player so both sides' infosets learn.
            let update_for = (updating_root + (it % 2) as u8) % g.sides;
            self.walk(g, &mut world, searcher, cfg, rng, 1, cfg.depth, update_for);
        }
        // Average strategy at the root infoset.
        let root = self.nodes.get(&1)?;
        let total: f64 = root.strat_sum.iter().sum();
        let strat: Vec<f64> = if total > 1e-12 {
            root.strat_sum.iter().map(|s| s / total).collect()
        } else {
            root.strategy()
        };
        let best = (0..root.moves.len())
            .max_by(|&a, &b| strat[a].partial_cmp(&strat[b]).unwrap())?;
        Some((root.moves[best], strat))
    }

    /// Returns utility in [0,1] from the *updating player's* perspective.
    fn walk(
        &mut self,
        g: &GameDef,
        world: &mut Position,
        searcher: &mut Searcher,
        cfg: &OosConfig,
        rng: &mut Rng,
        key: u64,
        depth: u32,
        update_for: u8,
    ) -> f64 {
        let stm = world.stm;
        let moves = legal_moves(g, world);
        if moves.is_empty() {
            // Terminal: mate/stalemate — utility for the updating player.
            let in_check = world.royal_attacked(g, stm);
            let loser_is_stm = in_check
                || g.policy.stalemate == crate::game::StalematePolicy::Loss;
            if !loser_is_stm {
                return 0.5;
            }
            return if stm == update_for { 0.0 } else { 1.0 };
        }
        if depth == 0 {
            // Depth-capped leaf: shared evaluator, squashed to [0,1].
            let v = searcher.eval.stm(g, world) as f64 / cfg.scale;
            let p = 1.0 / (1.0 + (-v).exp());
            return if stm == update_for { p } else { 1.0 - p };
        }

        let node = self.nodes.entry(key).or_insert_with(|| InfoNode {
            regret: vec![0.0; moves.len()],
            strat_sum: vec![0.0; moves.len()],
            moves: moves.clone(),
        });
        // The infoset's move list is fixed by the public history; a world
        // where a listed move is illegal contributes only over its legal
        // subset.
        let strategy = node.strategy();
        let node_moves = node.moves.clone();

        if stm == update_for {
            // External sampling: explore every action of the updating player.
            let mut util = vec![0.0f64; node_moves.len()];
            let mut node_util = 0.0;
            let mut legal_mask = vec![false; node_moves.len()];
            for (i, mv) in node_moves.iter().enumerate() {
                if !moves.contains(mv) {
                    continue;
                }
                legal_mask[i] = true;
                let u = world.make(g, mv);
                util[i] = self.walk(
                    g,
                    world,
                    searcher,
                    cfg,
                    rng,
                    child_key(key, g, mv),
                    depth - 1,
                    update_for,
                );
                world.unmake(g, &u);
                node_util += strategy[i] * util[i];
            }
            let node = self.nodes.get_mut(&key).unwrap();
            for i in 0..node_moves.len() {
                if legal_mask[i] {
                    node.regret[i] += util[i] - node_util;
                    node.strat_sum[i] += strategy[i];
                }
            }
            node_util
        } else {
            // Sample the opponent per its current strategy (restricted to
            // moves legal in this world).
            let legal_idx: Vec<usize> = (0..node_moves.len())
                .filter(|&i| moves.contains(&node_moves[i]))
                .collect();
            if legal_idx.is_empty() {
                return 0.5;
            }
            let mass: f64 = legal_idx.iter().map(|&i| strategy[i]).sum();
            let mut r = rng.unit_f64() * mass.max(1e-12);
            let mut pick = legal_idx[legal_idx.len() - 1];
            for &i in &legal_idx {
                r -= strategy[i];
                if r <= 0.0 {
                    pick = i;
                    break;
                }
            }
            {
                let node = self.nodes.get_mut(&key).unwrap();
                for &i in &legal_idx {
                    node.strat_sum[i] += strategy[i] / mass.max(1e-12);
                }
            }
            let mv = node_moves[pick];
            let u = world.make(g, &mv);
            let v = self.walk(
                g,
                world,
                searcher,
                cfg,
                rng,
                child_key(key, g, &mv),
                depth - 1,
                update_for,
            );
            world.unmake(g, &u);
            v
        }
    }
}

impl Default for Oos {
    fn default() -> Self {
        Self::new()
    }
}
