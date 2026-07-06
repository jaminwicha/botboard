//! The population/league (Training Spec §5): diverse-personality members,
//! an empirical meta-payoff matrix from round-robin self-play, and Nash
//! averaging on the meta-game (the principled valuation under
//! non-transitivity). Profiles persist as JSON (Training Spec §8) —
//! serialization owned by this core, exposed to any front end.

use crate::eval::Eval;
use crate::game::GameDef;
use crate::selfplay::play_match;

/// A population member: a personality expressed as evaluator weights
/// (behavioral diversity in miniature — the personality latent of the
/// Training Spec conditions a network the same way).
#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    /// Multiplier on material (aggression < 1.0 trades material for activity).
    pub material_scale: f64,
    pub mobility_cp: i32,
}

impl Member {
    pub fn eval(&self, base_material: &[i32]) -> Eval {
        let material = base_material
            .iter()
            .map(|&m| (m as f64 * self.material_scale).round() as i32)
            .collect();
        let mut e = Eval::new(material);
        e.mobility_cp = self.mobility_cp;
        e
    }
}

/// Seed personalities spanning material-vs-activity styles.
pub fn default_population() -> Vec<Member> {
    vec![
        Member { name: "solid".into(), material_scale: 1.15, mobility_cp: 2 },
        Member { name: "balanced".into(), material_scale: 1.0, mobility_cp: 4 },
        Member { name: "active".into(), material_scale: 0.9, mobility_cp: 8 },
        Member { name: "berserk".into(), material_scale: 0.75, mobility_cp: 12 },
    ]
}

/// Empirical meta-payoff: payoff[i][j] = score of i vs j in [0,1].
pub fn round_robin(
    g: &GameDef,
    members: &[Member],
    base_material: &[i32],
    games_per_pair: u32,
    depth: i32,
    node_budget: u64,
    seed: u64,
) -> Vec<Vec<f64>> {
    let n = members.len();
    let mut payoff = vec![vec![0.5f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let (w, l, d) = play_match(
                g,
                &members[i].eval(base_material),
                &members[j].eval(base_material),
                depth,
                node_budget,
                games_per_pair,
                seed + (i * n + j) as u64 * 10_000,
            );
            let total = (w + l + d) as f64;
            let s = (w as f64 + 0.5 * d as f64) / total;
            payoff[i][j] = s;
            payoff[j][i] = 1.0 - s;
        }
    }
    payoff
}

/// Nash averaging (Training Spec §5): regret matching on the symmetric
/// zero-sum meta-game A[i][j] = payoff[i][j] − 0.5. Returns the maxent-ish
/// equilibrium mixture and each member's rating (expected payoff against
/// the mixture) — robust to non-transitive cycles where raw win rate lies.
pub fn nash_averaging(payoff: &[Vec<f64>], iters: u32) -> (Vec<f64>, Vec<f64>) {
    let n = payoff.len();
    let a = |i: usize, j: usize| payoff[i][j] - 0.5;
    let mut regret = vec![0f64; n];
    let mut strat_sum = vec![0f64; n];

    for _ in 0..iters {
        let pos_sum: f64 = regret.iter().map(|r| r.max(0.0)).sum();
        let strat: Vec<f64> = if pos_sum > 1e-12 {
            regret.iter().map(|r| r.max(0.0) / pos_sum).collect()
        } else {
            vec![1.0 / n as f64; n]
        };
        for (i, s) in strat.iter().enumerate() {
            strat_sum[i] += s;
        }
        // Expected payoff of each pure strategy vs the current mixture.
        let u: Vec<f64> = (0..n)
            .map(|i| (0..n).map(|j| a(i, j) * strat[j]).sum())
            .collect();
        let ev: f64 = (0..n).map(|i| strat[i] * u[i]).sum();
        for i in 0..n {
            regret[i] += u[i] - ev;
        }
    }
    let total: f64 = strat_sum.iter().sum();
    let mixture: Vec<f64> = strat_sum.iter().map(|s| s / total).collect();
    let rating: Vec<f64> = (0..n)
        .map(|i| (0..n).map(|j| payoff[i][j] * mixture[j]).sum())
        .collect();
    (mixture, rating)
}

/// Profile persistence (Training Spec §8): versioned JSON, core-owned.
pub fn profiles_json(
    members: &[Member],
    mixture: &[f64],
    rating: &[f64],
    payoff: &[Vec<f64>],
) -> String {
    let mut s = String::from("{\n  \"version\": 1,\n  \"profiles\": [\n");
    for (i, m) in members.iter().enumerate() {
        s.push_str(&format!(
            "    {{\"name\": \"{}\", \"material_scale\": {}, \"mobility_cp\": {}, \
             \"nash_weight\": {:.4}, \"rating\": {:.4}}}{}\n",
            m.name,
            m.material_scale,
            m.mobility_cp,
            mixture[i],
            rating[i],
            if i + 1 < members.len() { "," } else { "" }
        ));
    }
    s.push_str("  ],\n  \"meta_payoff\": [\n");
    for (i, row) in payoff.iter().enumerate() {
        let cells: Vec<String> = row.iter().map(|v| format!("{v:.3}")).collect();
        s.push_str(&format!(
            "    [{}]{}\n",
            cells.join(", "),
            if i + 1 < payoff.len() { "," } else { "" }
        ));
    }
    s.push_str("  ]\n}\n");
    s
}
