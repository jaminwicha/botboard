//! Codex persistence (§8.8, §9; Training Spec §8): accumulated
//! reconnaissance is belief collapse made durable. A `Belief`'s knowledge
//! masks serialize to versioned JSON, keyed by piece index; a rematch
//! warm-starts from the intersection of the cold-open belief with the
//! persisted codex — so the engine plays *harder and faster against armies
//! the player has scouted*, with no special-casing.

use crate::belief::Belief;
use crate::game::{GameDef, Side, TypeId};
use crate::position::Position;

/// Serialize the belief's hypothesis sets (core-owned format, §10.1).
pub fn belief_to_json(b: &Belief) -> String {
    let mut s = String::from("{\n  \"version\": 1,\n  \"observer\": ");
    s.push_str(&b.observer.to_string());
    s.push_str(",\n  \"candidates\": [\n");
    for (i, c) in b.candidates.iter().enumerate() {
        let inner: Vec<String> = c.iter().map(|t| t.to_string()).collect();
        s.push_str(&format!(
            "    [{}]{}\n",
            inner.join(","),
            if i + 1 < b.candidates.len() { "," } else { "" }
        ));
    }
    s.push_str("  ]\n}\n");
    s
}

/// Parse a persisted codex. Tolerant, hand-rolled (no dependencies).
pub fn belief_from_json(json: &str) -> Result<Belief, String> {
    let observer: Side = json
        .split("\"observer\":")
        .nth(1)
        .and_then(|s| s.trim().split([',', '\n']).next())
        .and_then(|s| s.trim().parse().ok())
        .ok_or("missing observer")?;
    let cands_part = json.split("\"candidates\":").nth(1).ok_or("missing candidates")?;
    let mut candidates = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for ch in cands_part.chars() {
        match ch {
            '[' => {
                depth += 1;
                if depth == 2 {
                    cur.clear();
                }
            }
            ']' => {
                if depth == 2 {
                    let v: Vec<TypeId> = cur
                        .split(',')
                        .filter_map(|x| x.trim().parse().ok())
                        .collect();
                    candidates.push(v);
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ if depth == 2 => cur.push(ch),
            _ => {}
        }
    }
    Ok(Belief { observer, candidates })
}

/// Warm-start a rematch (§8.8): intersect the fresh cold-open belief with
/// the codex learned in earlier encounters. Empty intersections fall back
/// to the cold-open set (the enemy may have changed that piece).
pub fn warm_start(g: &GameDef, pos: &Position, observer: Side, codex: &Belief) -> Belief {
    let mut b = Belief::cold_open(g, pos, observer);
    for i in 0..b.candidates.len().min(codex.candidates.len()) {
        if b.candidates[i].len() > 1 && !codex.candidates[i].is_empty() {
            let inter: Vec<TypeId> = b.candidates[i]
                .iter()
                .copied()
                .filter(|t| codex.candidates[i].contains(t))
                .collect();
            if !inter.is_empty() {
                b.candidates[i] = inter;
            }
        }
    }
    b
}
