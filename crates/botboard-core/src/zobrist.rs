//! Zobrist hashing (§7.5). The key covers ⟨type, square, state-bucket⟩,
//! per-cell terrain type, side-to-move, hands, and the en-passant square.
//! The state bucket quantizes the instance state that affects play — the
//! `moved` flag (castling rights) and HP (hit-count armor); ammunition and
//! cooldown phases extend the same bucket when those Bits land.
//! Repetition is equality of this full ground-truth key: monotone counters
//! make true repetition impossible while they tick, and cyclic state must
//! match for a repeat to count.
//!
//! The tables live in `GameDef`; `Position.hash` is maintained
//! *incrementally* by make/unmake (unmake restores the recorded prior key),
//! with a debug assertion against the full recompute.

use crate::position::{Loc, Position, T_NONE};
use crate::rng::Rng;

const MAX_HAND: usize = 20;
/// State buckets: bit 0 = moved flag, bits 1..4 = HP capped at 7.
const BUCKETS: usize = 16;
/// Terrain types keyed per cell (T_NONE hashes to nothing).
const TERRAINS: usize = 4;

#[derive(Clone, Debug)]
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
    pub fn for_shape(ncells: usize, ntypes: usize, nsides: u8) -> Self {
        let mut rng = Rng::new(0xB07B0A2D_5EED_0001);
        let nsides = nsides as usize;
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

    #[inline]
    pub fn terrain_key(&self, terrain: u8, sq: usize) -> u64 {
        if terrain == T_NONE {
            0
        } else {
            self.terrain[terrain as usize * self.ncells + sq]
        }
    }

    #[inline]
    pub fn hand_key(&self, side: usize, t: usize, count: usize) -> u64 {
        if count == 0 {
            0
        } else {
            self.hand[(side * self.ntypes + t) * MAX_HAND + (count - 1).min(MAX_HAND - 1)]
        }
    }

    #[inline]
    pub fn stm_key(&self, side: usize) -> u64 {
        self.stm[side]
    }

    #[inline]
    pub fn ep_key(&self, ep: u16) -> u64 {
        if ep == crate::moves::NO_SQ {
            0
        } else {
            self.ep[ep as usize]
        }
    }

    /// Full ground-truth hash — the incremental key's reference definition.
    pub fn full_hash(&self, pos: &Position) -> u64 {
        let mut h = self.stm_key(pos.stm as usize);
        for (i, p) in pos.pieces.iter().enumerate() {
            if let Loc::Board(sq) = p.loc {
                h ^= self.piece_key(p.t as usize, p.side as usize, p.moved, pos.hp[i], sq as usize);
            }
        }
        for sq in 0..self.ncells {
            h ^= self.terrain_key(pos.terrain[sq], sq);
        }
        for s in 0..self.nsides {
            for t in 0..self.ntypes {
                h ^= self.hand_key(s, t, pos.hands[s][t] as usize);
            }
        }
        h ^= self.ep_key(pos.ep);
        h
    }
}
