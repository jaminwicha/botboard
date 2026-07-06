//! Movement geometry: `(m,n)` step deltas and direction masks (spec §3.1).

/// A relative board step. `dy > 0` is "forward" for side 0; the compile step
/// (§7.3) flips `dy` for the opposing side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Delta {
    pub dx: i8,
    pub dy: i8,
}

impl Delta {
    pub const fn new(dx: i8, dy: i8) -> Self {
        Delta { dx, dy }
    }
}

/// All distinct symmetric copies of an `(m,n)` step: (±m,±n) and (±n,±m).
pub fn symmetric_deltas(m: i8, n: i8) -> Vec<Delta> {
    let mut out = Vec::new();
    for (a, b) in [(m, n), (n, m)] {
        for sx in [1i8, -1] {
            for sy in [1i8, -1] {
                let d = Delta::new(a * sx, b * sy);
                if d.dx == 0 && d.dy == 0 {
                    continue;
                }
                if !out.contains(&d) {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// Direction mask (§3.1): restricts a geometry to a subset of its symmetric
/// copies. Evaluated in the mover's frame (before the per-side flip).
/// `Custom` covers asymmetric sets like the shogi knight's narrow-forward pair.
#[derive(Clone, Debug)]
pub enum DirFilter {
    All,
    Forward,
    Backward,
    Sideways,
    Vertical,
    Custom(Vec<Delta>),
}

impl DirFilter {
    pub fn keep(&self, d: Delta) -> bool {
        match self {
            DirFilter::All => true,
            DirFilter::Forward => d.dy > 0,
            DirFilter::Backward => d.dy < 0,
            DirFilter::Sideways => d.dy == 0,
            DirFilter::Vertical => d.dx == 0,
            DirFilter::Custom(v) => v.contains(&d),
        }
    }
}
