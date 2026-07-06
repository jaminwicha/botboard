//! Zobrist hashing (§7.5). The key covers ⟨type, square, state-bucket⟩,
//! per-cell terrain type, side-to-move, hands, and the en-passant square.
//! The state bucket quantizes the instance state that affects play — here
//! the `moved` flag (castling rights) and HP (hit-count armor); ammunition
//! and cooldown phases extend the same bucket when those Bits land.
//! Repetition is equality of this full ground-truth key (§7.5): monotone
//! counters make true repetition impossible while they tick, and cyclic
//! state must match for a repeat to count.

use crate::game::GameDef;
use crate::position::{Loc, Position, T_NONE};
use crate::rng::Rng;

const MAX_HAND: usize = 20;
/// State buckets: bit 0 = moved flag, bits 1..3 = HP capped at 7.
const BUCKETS: usize = 16;
/// Terrain types keyed per cell (T_NONE hashes to nothing).
const TERRAINS: usize = 4;

pub struct Zobrist {
    /// [type][side][bucket][square]
    piece: Vec<u64>,
    /// [terrain-type][square]
    terrain: Vec<u64>,
    /// [side][type][count 1..=MAX_HAND]
    hand: Vec<u64>,
    stm: Vec<u64>,
    /// [square] en-passant target marker
    ep: Vec<u64>,
    ncells: usize,
    ntypes: usize,
    nsides: usize,
}

impl Zobrist {
    pub fn new(g: &GameDef) -> Self {
        let mut rng = Rng::new(0xB07B0A2D_5EED_0001);
        let ncells = g.board.ncells();
        let ntypes = g.types.len();
        let nsides = g.sides as usize;
        let mut fill = |n: usize| (0..n).map(|_| rng.next_u64()).collect::<Vec<_>>();
        Zobrist {
            piece: fill(ntypes * nsides * BUCKETS * ncells),
            terrain: fill(TERRAINS * ncells),
            hand: fill(nsides * ntypes * MAX_HAND),
            stm: fill(nsides),
            ep: fill(ncells),
            ncells,
            ntypes,
            nsides,
        }
    }

    #[inline]
    pub fn piece_key(&self, t: usize, side: usize, moved: bool, hp: i16, sq: usize) -> u64 {
        let b = (moved as usize) | ((hp.clamp(0, 7) as usize) << 1);
        self.piece[((t * self.nsides + side) * BUCKETS + b) * self.ncells + sq]
    }

    /// Full ground-truth hash of a position (§7.5). Recomputed O(pieces);
    /// incremental update is a later optimization, not a semantic change.
    pub fn hash(&self, g: &GameDef, pos: &Position) -> u64 {
        let mut h = self.stm[pos.stm as usize];
        for (i, p) in pos.pieces.iter().enumerate() {
            if let Loc::Board(sq) = p.loc {
                h ^= self.piece_key(
                    p.t as usize,
                    p.side as usize,
                    p.moved,
                    pos.hp[i],
                    sq as usize,
                );
            }
        }
        for sq in 0..self.ncells {
            let t = pos.terrain[sq];
            if t != T_NONE {
                h ^= self.terrain[t as usize * self.ncells + sq];
            }
        }
        for s in 0..self.nsides {
            for t in 0..self.ntypes {
                let c = pos.hands[s][t] as usize;
                if c > 0 {
                    h ^= self.hand[(s * self.ntypes + t) * MAX_HAND + (c - 1).min(MAX_HAND - 1)];
                }
            }
        }
        if pos.ep != crate::moves::NO_SQ {
            h ^= self.ep[pos.ep as usize];
        }
        let _ = g;
        h
    }
}
