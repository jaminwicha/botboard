//! Bits: atomic, typed, parameterized rule fragments (spec §3).
//!
//! Bits are the *authoring* format; `game::Compiled` is the runtime format
//! produced by the compile step (§7.3).

use crate::geometry::DirFilter;

/// Axis-A geometry primitives (§3.1). The hopper is the one-screen family
/// member, parameterized by landing mode (`HopMode`).
#[derive(Clone, Copy, Debug)]
pub enum Geometry {
    Leaper(i8, i8),
    Rider(i8, i8),
    Hopper(i8, i8),
}

/// Hopper landing modes (§3.1's "hopper family parameterized by screen
/// count and landing mode"; screen count is fixed at one in this batch).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HopMode {
    /// Xiangqi cannon: after exactly one screen, capture at any distance
    /// beyond it. Capture-only by geometry (the paired rider Bit supplies
    /// quiet movement).
    CannonAtRange,
    /// Grasshopper: land on the square IMMEDIATELY beyond the screen —
    /// onto empty (if the Bit can move) or capturing an enemy there (if
    /// the Bit can capture).
    BeyondScreen,
    /// Locust (the checkers atom): capture the SCREEN itself — which must
    /// be an enemy — landing on the empty square immediately beyond it.
    Locust,
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
/// The HP gates are the §3.1 state condition: the kernel only activates
/// while the mover's *current* HP is inside `[min_hp, max_hp]`
/// (0 = unbounded on that end) — a `max_hp: 1` kernel is a desperation
/// Bit that wakes when the armor is gone.
#[derive(Clone, Debug)]
pub struct MoveBit {
    pub geom: Geometry,
    pub dirs: DirFilter,
    pub mode: Mode,
    pub path: PathRule,
    pub from_zone: Option<usize>,
    pub to_zone: Option<usize>,
    pub target: TargetPred,
    /// State gate (§3.1): kernel active only when hp ≥ min_hp (0 = no bound).
    pub min_hp: i16,
    /// State gate (§3.1): kernel active only when hp ≤ max_hp (0 = no bound).
    pub max_hp: i16,
    /// Hopper landing mode; ignored by leapers and riders.
    pub landing: HopMode,
    /// Range limit for riders in steps along the ray (0 = unlimited).
    /// The R4 "short rook" atom; ignored by leapers and hoppers.
    pub max_steps: u8,
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
            min_hp: 0,
            max_hp: 0,
            landing: HopMode::CannonAtRange,
            max_steps: 0,
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
    pub fn min_hp(mut self, hp: i16) -> Self {
        self.min_hp = hp;
        self
    }
    pub fn max_hp(mut self, hp: i16) -> Self {
        self.max_hp = hp;
        self
    }
    /// Set the hopper landing mode. `BeyondScreen` (the grasshopper) also
    /// widens the hopper's default capture-only mode to `Both` — override
    /// with `.mode(..)` after this call if a narrower Bit is wanted.
    pub fn landing(mut self, l: HopMode) -> Self {
        self.landing = l;
        if l == HopMode::BeyondScreen {
            self.mode = Mode::Both;
        }
        self
    }
    /// Cap a rider at `n` steps along each ray (0 = unlimited).
    pub fn max_steps(mut self, n: u8) -> Self {
        self.max_steps = n;
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

/// Axis-B ability Bits (§3.2, §3.4): effects invoked *as* the turn's single
/// action. Ranges are chebyshev for point-targeted effects; the laser
/// targets along rays like a bounded rider, capturing without vacating the
/// origin. `retreat` is the coupled nerf: a forced 1-step retreat away from
/// the target, and the whole compound is illegal if that square is blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbilityBit {
    Heal { amount: i16, range: u8 },
    CreateWall { range: u8 },
    DigPit { range: u8 },
    Laser { range: u8, retreat: bool, pierce: bool },
    /// Revive a dead friendly non-royal piece onto an empty in-range
    /// square at 1 HP (the SRW §7 resurrect controller spell).
    Resurrect { range: u8 },
    /// Flip an in-range enemy to the caster's side (the SRW §7 hack
    /// spell). The spec's "target predicate": non-royal at exactly 1 HP —
    /// its defenses are down. Deterministic, so search stays sound.
    Hack { range: u8 },
    /// Lay an owner-tagged mine on an empty in-range square (SRW Appendix
    /// B): passable, non-blocking; an enemy landing takes 1 HP (lethal at
    /// 1) and the mine is spent. Concealment lives at the battle layer.
    MineLayer { range: u8 },
    /// Exchange the caster with a friendly piece within chebyshev range
    /// (both on board) — an atomic two-piece relocation script (§3.4).
    Swap { range: u8 },
    /// Shove an in-range enemy one square directly away from the caster
    /// (chebyshev direction sign); generated only when that square is
    /// open (for the pushed piece's terrain permissions) and empty.
    /// Landing hazards bite the shoved piece.
    Push { range: u8 },
    /// Bits 2.0 Stage 4: reference to a CUSTOM ability row
    /// (`GameDef::custom_effects[index]`). The row carries the whole
    /// definition — selector, range, ops, cost, descriptor slot — so
    /// this bit has no parameters of its own (per-piece parameters on a
    /// custom reference are ignored by design: the row IS the
    /// definition).
    Custom(u16),
}
