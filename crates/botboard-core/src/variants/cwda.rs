//! Betza's Chess with Different Armies (CwDA), composed from Bits — the
//! batch-3 fairness measurement bench. Each army fills the four FIDE
//! slots (queen/rook/bishop/knight) with pieces built from the atoms this
//! engine already speaks: leapers, riders (now range-limited), direction
//! masks, and compounds thereof. Every piece documents its Betza atoms.
//!
//! All four canonical armies used here are fully expressible with the
//! current vocabulary — no substitutions were needed (the Rookies'
//! Half-Duck's `H` is the (0,3) threeleaper, an ordinary leaper atom).
//!
//! Pawns and kings are standard FIDE on all sides (as in CwDA); pawns
//! promote to their own army's four slot pieces. The R-slot piece of each
//! army is its castle partner, per Betza's rules.

use crate::cost::{cost_prior, CostWeights};
use crate::game::*;
use crate::geometry::{Delta, DirFilter};
use crate::bits::{Mode, MoveBit, SpecialBit};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Army {
    /// The FIDE Fabulous Kingdom: Q, R, B, N.
    Fide,
    /// Colorbound Clobberers: Archbishop, Bede, FAD, Waffle.
    Clobberers,
    /// Remarkable Rookies: Chancellor, Short Rook (R4), Half-Duck, Woody Rook.
    Rookies,
    /// Nutty Knights: Colonel, Charging Rook, Charging Knight, Fibnif.
    Nutters,
}

impl Army {
    pub fn name(self) -> &'static str {
        match self {
            Army::Fide => "FIDE",
            Army::Clobberers => "Colorbound Clobberers",
            Army::Rookies => "Remarkable Rookies",
            Army::Nutters => "Nutty Knights",
        }
    }
}

pub const ALL: [Army; 4] = [Army::Fide, Army::Clobberers, Army::Rookies, Army::Nutters];

/// Forward-half knight (Betza fhN): the four forward-most (1,2) copies,
/// in the mover's frame (the compile step flips per side).
fn fh_n() -> DirFilter {
    DirFilter::Custom(vec![
        Delta::new(1, 2),
        Delta::new(-1, 2),
        Delta::new(2, 1),
        Delta::new(-2, 1),
    ])
}

/// Narrow (vertical-most) knight (Betza ffbbN as used by fbN in Fibnif):
/// the four (±1, ±2) copies.
fn narrow_n() -> DirFilter {
    DirFilter::Custom(vec![
        Delta::new(1, 2),
        Delta::new(-1, 2),
        Delta::new(1, -2),
        Delta::new(-1, -2),
    ])
}

/// Rook rays forward and sideways only (Betza fsR).
fn fs_r() -> DirFilter {
    DirFilter::Custom(vec![Delta::new(0, 1), Delta::new(1, 0), Delta::new(-1, 0)])
}

/// The four slot pieces of an army, in [Q-slot, R-slot, B-slot, N-slot]
/// order, each with its Betza decomposition noted.
fn slot_pieces(a: Army) -> [PieceTypeDef; 4] {
    match a {
        Army::Fide => [
            // Q = RB: orthogonal rider + diagonal rider.
            PieceTypeDef::new("queen", 'Q', vec![MoveBit::rider(0, 1), MoveBit::rider(1, 1)]),
            // R: orthogonal rider.
            PieceTypeDef::new("rook", 'R', vec![MoveBit::rider(0, 1)]).castle_partner(),
            // B: diagonal rider.
            PieceTypeDef::new("bishop", 'B', vec![MoveBit::rider(1, 1)]),
            // N: (1,2) leaper.
            PieceTypeDef::new("knight", 'N', vec![MoveBit::leaper(1, 2)]),
        ],
        Army::Clobberers => [
            // Archbishop = BN: bishop rider + knight leaper.
            PieceTypeDef::new(
                "archbishop",
                'A',
                vec![MoveBit::rider(1, 1), MoveBit::leaper(1, 2)],
            ),
            // Bede = BD: bishop rider + dabbaba (0,2) leaper.
            PieceTypeDef::new("bede", 'E', vec![MoveBit::rider(1, 1), MoveBit::leaper(0, 2)])
                .castle_partner(),
            // FAD = F + A + D: ferz (1,1) + alfil (2,2) + dabbaba (0,2).
            PieceTypeDef::new(
                "fad",
                'F',
                vec![MoveBit::leaper(1, 1), MoveBit::leaper(2, 2), MoveBit::leaper(0, 2)],
            ),
            // Waffle = WA: wazir (0,1) + alfil (2,2).
            PieceTypeDef::new("waffle", 'W', vec![MoveBit::leaper(0, 1), MoveBit::leaper(2, 2)]),
        ],
        Army::Rookies => [
            // Chancellor = RN: rook rider + knight leaper.
            PieceTypeDef::new(
                "chancellor",
                'C',
                vec![MoveBit::rider(0, 1), MoveBit::leaper(1, 2)],
            ),
            // Short Rook = R4: orthogonal rider capped at 4 steps — the
            // batch-3 range-limited-rider atom, priced by the mobility
            // integral with no hand adjustment.
            PieceTypeDef::new("shortrook", 'S', vec![MoveBit::rider(0, 1).max_steps(4)])
                .castle_partner(),
            // Half-Duck = HFD: threeleaper (0,3) + ferz (1,1) + dabbaba (0,2).
            PieceTypeDef::new(
                "halfduck",
                'H',
                vec![MoveBit::leaper(0, 3), MoveBit::leaper(1, 1), MoveBit::leaper(0, 2)],
            ),
            // Woody Rook = WD: wazir (0,1) + dabbaba (0,2).
            PieceTypeDef::new("woody", 'D', vec![MoveBit::leaper(0, 1), MoveBit::leaper(0, 2)]),
        ],
        Army::Nutters => [
            // Colonel = fhNfsRK: forward-half knight + forward/sideways
            // rook + full king step.
            PieceTypeDef::new(
                "colonel",
                'O',
                vec![
                    MoveBit::leaper(1, 2).dirs(fh_n()),
                    MoveBit::rider(0, 1).dirs(fs_r()),
                    MoveBit::leaper(0, 1),
                    MoveBit::leaper(1, 1),
                ],
            ),
            // Charging Rook = fsRbK: forward/sideways rook + king steps
            // backward (straight back + back diagonals).
            PieceTypeDef::new(
                "chargingrook",
                'G',
                vec![
                    MoveBit::rider(0, 1).dirs(fs_r()),
                    MoveBit::leaper(0, 1).dirs(DirFilter::Backward),
                    MoveBit::leaper(1, 1).dirs(DirFilter::Backward),
                ],
            )
            .castle_partner(),
            // Charging Knight = fhNrlbK: forward-half knight + king steps
            // sideways and backward.
            PieceTypeDef::new(
                "chargingknight",
                'J',
                vec![
                    MoveBit::leaper(1, 2).dirs(fh_n()),
                    MoveBit::leaper(0, 1).dirs(DirFilter::Sideways),
                    MoveBit::leaper(0, 1).dirs(DirFilter::Backward),
                    MoveBit::leaper(1, 1).dirs(DirFilter::Backward),
                ],
            ),
            // Fibnif = fbNF: narrow (vertical) knight + ferz.
            PieceTypeDef::new(
                "fibnif",
                'I',
                vec![MoveBit::leaper(1, 2).dirs(narrow_n()), MoveBit::leaper(1, 1)],
            ),
        ],
    }
}

const Z_PAWN_START: usize = 0;
const Z_LAST_RANK: usize = 1;

/// Type-id layout of a two-army game: 0 = king; 1..=5 = side 0's army
/// (Q-slot, R-slot, B-slot, N-slot, pawn); 6..=10 = side 1's.
pub const KING: TypeId = 0;

fn pawn_for(base: TypeId) -> PieceTypeDef {
    // Standard FIDE pawn (CwDA keeps pawns and kings across armies),
    // promoting to its own army's four slot pieces.
    PieceTypeDef::new(
        "pawn",
        'P',
        vec![
            MoveBit::leaper(0, 1).dirs(DirFilter::Forward).mode(Mode::Move),
            MoveBit::leaper(1, 1).dirs(DirFilter::Forward).mode(Mode::Capture),
        ],
    )
    .specials(vec![
        SpecialBit::DoubleStep { start_zone: Z_PAWN_START },
        SpecialBit::EnPassant,
    ])
    .promo(PromoRule {
        zone: Z_LAST_RANK,
        choices: vec![base, base + 1, base + 2, base + 3],
        trigger: PromoTrigger::Dest,
        optional: false,
    })
}

/// Compose a CwDA game: side 0 fields `a0`, side 1 fields `a1`.
pub fn game(a0: Army, a1: Army) -> GameDef {
    let board = BoardDef { w: 8, h: 8 };

    let mut pawn_start = Zone::new(2);
    pawn_start.add_rect(0, &board, 0, 1, 7, 1);
    pawn_start.add_rect(1, &board, 0, 6, 7, 6);
    let mut last_rank = Zone::new(2);
    last_rank.add_rect(0, &board, 0, 7, 7, 7);
    last_rank.add_rect(1, &board, 0, 0, 7, 0);

    let mut types = vec![PieceTypeDef::new(
        "king",
        'K',
        vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)],
    )
    .royal()
    .specials(vec![SpecialBit::Castling])];
    for (army, base) in [(a0, 1 as TypeId), (a1, 6)] {
        types.extend(slot_pieces(army));
        types.push(pawn_for(base));
    }

    let policy = Policy {
        stalemate: StalematePolicy::Draw,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };

    // Back rank per side, FIDE layout by slot: R N B Q K B N R.
    let mut start = Vec::new();
    for (side, base, back_y, pawn_y) in [(0 as Side, 1 as TypeId, 0u8, 1u8), (1, 6, 7, 6)] {
        let (q, r, b, n, p) = (base, base + 1, base + 2, base + 3, base + 4);
        for (x, t) in [r, n, b, q, KING, b, n, r].into_iter().enumerate() {
            start.push((t, side, x as u8, back_y));
        }
        for x in 0..8 {
            start.push((p, side, x, pawn_y));
        }
    }

    GameDef::new(board, 2, types, vec![pawn_start, last_rank], policy, start)
}

/// One army's cost sheet under the standard pricing weights: per-slot
/// prior and the fielded-army total (1 Q-slot + 2 R + 2 B + 2 N + 8 P;
/// the king prices 0 — royalty is priceless, not priced).
pub struct ArmyCosts {
    pub army: Army,
    /// (piece name, unit prior, fielded count).
    pub pieces: Vec<(&'static str, f64, u32)>,
    pub total: f64,
}

pub fn army_costs(a: Army, w: &CostWeights) -> ArmyCosts {
    let g = game(a, a);
    let counts = [1u32, 2, 2, 2, 8]; // Q-slot, R-slot, B-slot, N-slot, pawns
    let mut pieces = Vec::new();
    let mut total = 0.0;
    for (i, &count) in counts.iter().enumerate() {
        let t = 1 + i as TypeId;
        let c = cost_prior(&g, t, w);
        total += c * count as f64;
        pieces.push((g.types[t as usize].name, c, count));
    }
    ArmyCosts { army: a, pieces, total }
}

/// Paired-opening army match (the netmatch pairing idea across asymmetric
/// armies): each pair plays the same opening seed twice with the army
/// assignment swapped, both sides searched at equal strength on the
/// engine's own cost-prior material. Returns (a_wins, b_wins, draws).
pub fn paired_match(
    a: Army,
    b: Army,
    pairs: u32,
    depth: i32,
    nodes: u64,
    seed: u64,
    max_plies: u32,
) -> (u32, u32, u32) {
    use crate::eval::Eval;
    use crate::search::Searcher;
    use crate::selfplay::{play_game, Outcome};

    let (mut wa, mut wb, mut dr) = (0u32, 0u32, 0u32);
    for p in 0..pairs {
        let s = seed.wrapping_add(p as u64 * 7919);
        for a_first in [true, false] {
            let g = if a_first { game(a, b) } else { game(b, a) };
            let material = crate::cost::material_table(&g, &W);
            let mut s0 = Searcher::new(&g, Eval::new(material.clone()));
            let mut s1 = Searcher::new(&g, Eval::new(material));
            let rec = play_game(
                &g,
                &mut s0,
                &mut s1,
                (depth, depth),
                (nodes, nodes),
                s,
                6,
                max_plies,
                0,
            );
            match rec.outcome {
                Outcome::Win(0) => {
                    if a_first {
                        wa += 1
                    } else {
                        wb += 1
                    }
                }
                Outcome::Win(_) => {
                    if a_first {
                        wb += 1
                    } else {
                        wa += 1
                    }
                }
                Outcome::Draw => dr += 1,
            }
        }
    }
    (wa, wb, dr)
}

/// The standard pricing weights shared with the SRW surface and the army
/// generator: cross-battle comparable, deterministic.
pub const W: CostWeights = CostWeights { w_move: 0.35, w_attack: 0.35 };
