//! Bits: atomic, typed, parameterized rule fragments (spec §3).
//!
//! Bits are the *authoring* format; `game::Compiled` is the runtime format
//! produced by the compile step (§7.3).

use crate::geometry::DirFilter;

/// Axis-A geometry primitives (§3.1). The hopper is the one-screen,
/// capture-landing family member (xiangqi cannon); further landing modes are
/// later-phase content.
#[derive(Clone, Copy, Debug)]
pub enum Geometry {
    Leaper(i8, i8),
    Rider(i8, i8),
    Hopper(i8, i8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Move,
    Capture,
    Both,
}

impl Mode {
    pub fn can_move(self) -> bool {
        matches!(self, Mode::Move | Mode::Both)
    }
    pub fn can_capture(self) -> bool {
        matches!(self, Mode::Capture | Mode::Both)
    }
}

/// Path interaction for lame leapers (§3.1): cells that must be empty.
/// `Midpoint` = xiangqi elephant (blocker at d/2); `Leg` = xiangqi horse
/// (blocker one orthogonal step along the dominant axis).
#[derive(Clone, Copy, Debug)]
pub enum PathRule {
    None,
    Midpoint,
    Leg,
}

/// Target predicate (§3.1): gates on the piece being captured.
/// `EnemyRoyal` is xiangqi's flying general; hack/laser/spy reuse it later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetPred {
    Any,
    EnemyRoyal,
}

/// One Axis-A movement Bit. Zone conditions gate the origin and/or the
/// destination square (palace confinement is a destination constraint).
#[derive(Clone, Debug)]
pub struct MoveBit {
    pub geom: Geometry,
    pub dirs: DirFilter,
    pub mode: Mode,
    pub path: PathRule,
    pub from_zone: Option<usize>,
    pub to_zone: Option<usize>,
    pub target: TargetPred,
}

impl MoveBit {
    fn new(geom: Geometry) -> Self {
        MoveBit {
            geom,
            dirs: DirFilter::All,
            mode: Mode::Both,
            path: PathRule::None,
            from_zone: None,
            to_zone: None,
            target: TargetPred::Any,
        }
    }
    pub fn leaper(m: i8, n: i8) -> Self {
        Self::new(Geometry::Leaper(m, n))
    }
    pub fn rider(m: i8, n: i8) -> Self {
        Self::new(Geometry::Rider(m, n))
    }
    pub fn hopper(m: i8, n: i8) -> Self {
        let mut b = Self::new(Geometry::Hopper(m, n));
        b.mode = Mode::Capture;
        b
    }
    pub fn dirs(mut self, d: DirFilter) -> Self {
        self.dirs = d;
        self
    }
    pub fn mode(mut self, m: Mode) -> Self {
        self.mode = m;
        self
    }
    pub fn path(mut self, p: PathRule) -> Self {
        self.path = p;
        self
    }
    pub fn from_zone(mut self, z: usize) -> Self {
        self.from_zone = Some(z);
        self
    }
    pub fn to_zone(mut self, z: usize) -> Self {
        self.to_zone = Some(z);
        self
    }
    pub fn target(mut self, t: TargetPred) -> Self {
        self.target = t;
        self
    }
}

/// Special-move generators (§3.3): history-dependent or two-piece compounds
/// that are first-class generators, not geometry.
#[derive(Clone, Debug)]
pub enum SpecialBit {
    /// Pawn double-step from its start zone; sets the en-passant square.
    DoubleStep { start_zone: usize },
    /// Capture onto the en-passant square (victim is on the passed-over file).
    EnPassant,
    /// Standard castling with an unmoved `castle_partner` rook on the same rank.
    Castling,
}
