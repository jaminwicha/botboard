//! Zobrist hashing (§7.5). The key covers ⟨type, square, state-bucket⟩,
//! side-to-move, and hands. Phase-0 instance state is the `moved` flag
//! (castling rights), which is the state bucket; HP/ammo/cooldown buckets
//! and per-cell terrain keys extend the same tables when those Bits land.
//! Repetition is equality of this full ground-truth key (§7.5).

use crate::game::GameDef;
use crate::position::{Loc, Position};
use crate::rng::Rng;

const MAX_HAND: usize = 20;
/// State buckets in Phase 0: bit 0 = moved flag.
const BUCKETS: usize = 2;

pub struct Zobrist {
    /// [type][side][bucket][square]
    piece: Vec<u64>,
    /// [side][type][count 1..=MAX_HAND]
    hand: Vec<u64>,
    stm: Vec<u64>,
    /// [square] en-passant target file marker
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
            hand: fill(nsides * ntypes * MAX_HAND),
            stm: fill(nsides),
            ep: fill(ncells),
            ncells,
            ntypes,
            nsides,
        }
    }

    #[inline]
    pub fn piece_key(&self, t: usize, side: usize, moved: bool, sq: usize) -> u64 {
        let b = moved as usize;
        self.piece[((t * self.nsides + side) * BUCKETS + b) * self.ncells + sq]
    }

    /// Full ground-truth hash of a position (§7.5). Recomputed O(pieces);
    /// incremental update is a later optimization, not a semantic change.
    pub fn hash(&self, g: &GameDef, pos: &Position) -> u64 {
        let mut h = self.stm[pos.stm as usize];
        for p in &pos.pieces {
            if let Loc::Board(sq) = p.loc {
                h ^= self.piece_key(p.t as usize, p.side as usize, p.moved, sq as usize);
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
