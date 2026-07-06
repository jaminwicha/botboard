//! Position state, make/unmake, and attack detection.
//!
//! Mailbox array + piece list (§7.1, Phase-0 board class). Pieces never
//! leave the list: capture sends them to `Dead` or to a hand (§3.2 capture
//! fate); shogi drops re-activate hand pieces, so unmake is a pure reversal.

use crate::game::{CaptureFate, GameDef, Side, TypeId};
use crate::moves::{Move, MoveKind, NO_SQ};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loc {
    Board(u16),
    Hand(Side),
    Dead,
}

#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub t: TypeId,
    /// De-promotion target: the type this piece reverts to in hand (§3.2).
    pub base: TypeId,
    pub side: Side,
    pub loc: Loc,
    pub moved: bool,
}

#[derive(Clone, Debug)]
pub struct Position {
    /// Mailbox: piece index or -1.
    pub board: Vec<i32>,
    pub pieces: Vec<Piece>,
    pub stm: Side,
    /// En-passant target square, or NO_SQ.
    pub ep: u16,
    /// hands[side][type] = count of that type held.
    pub hands: Vec<Vec<u8>>,
}

pub struct Undo {
    mv: Move,
    moving: usize,
    prior_t: TypeId,
    prior_moved: bool,
    captured: Option<(usize, Piece)>,
    /// Castle partner: (index, prior square, prior moved).
    partner: Option<(usize, u16, bool)>,
    prior_ep: u16,
}

impl Position {
    pub fn from_pieces(g: &GameDef, list: &[(TypeId, Side, u16, bool)], stm: Side) -> Self {
        let mut pos = Position {
            board: vec![-1; g.board.ncells()],
            pieces: Vec::with_capacity(list.len()),
            stm,
            ep: NO_SQ,
            hands: vec![vec![0; g.types.len()]; g.sides as usize],
        };
        for &(t, side, sq, moved) in list {
            pos.board[sq as usize] = pos.pieces.len() as i32;
            pos.pieces.push(Piece { t, base: t, side, loc: Loc::Board(sq), moved });
        }
        pos
    }

    pub fn startpos(g: &GameDef) -> Self {
        let list: Vec<_> = g
            .start
            .iter()
            .map(|&(t, side, x, y)| (t, side, g.board.sq(x, y), false))
            .collect();
        Self::from_pieces(g, &list, 0)
    }

    pub fn piece_at(&self, sq: u16) -> Option<&Piece> {
        let i = self.board[sq as usize];
        if i < 0 {
            None
        } else {
            Some(&self.pieces[i as usize])
        }
    }

    fn fwd(side: Side) -> i8 {
        if side == 0 {
            1
        } else {
            -1
        }
    }

    pub fn make(&mut self, g: &GameDef, mv: &Move) -> Undo {
        let prior_ep = self.ep;
        self.ep = NO_SQ;

        if mv.kind == MoveKind::Drop {
            let idx = self
                .pieces
                .iter()
                .position(|p| p.loc == Loc::Hand(self.stm) && p.t == mv.drop_type)
                .expect("drop with empty hand");
            let u = Undo {
                mv: *mv,
                moving: idx,
                prior_t: self.pieces[idx].t,
                prior_moved: self.pieces[idx].moved,
                captured: None,
                partner: None,
                prior_ep,
            };
            self.hands[self.stm as usize][mv.drop_type as usize] -= 1;
            self.pieces[idx].loc = Loc::Board(mv.to);
            self.pieces[idx].moved = true;
            self.board[mv.to as usize] = idx as i32;
            self.stm = (self.stm + 1) % g.sides;
            return u;
        }

        let mi = self.board[mv.from as usize] as usize;
        let victim_sq = if mv.kind == MoveKind::EnPassant { mv.aux } else { mv.to };
        let cap_idx = if mv.kind == MoveKind::Castle {
            -1
        } else {
            self.board[victim_sq as usize]
        };

        let mut u = Undo {
            mv: *mv,
            moving: mi,
            prior_t: self.pieces[mi].t,
            prior_moved: self.pieces[mi].moved,
            captured: None,
            partner: None,
            prior_ep,
        };

        if cap_idx >= 0 && cap_idx as usize != mi {
            let ci = cap_idx as usize;
            u.captured = Some((ci, self.pieces[ci]));
            self.board[victim_sq as usize] = -1;
            match g.policy.capture_fate {
                CaptureFate::Destroy => self.pieces[ci].loc = Loc::Dead,
                CaptureFate::ToHand => {
                    let base = self.pieces[ci].base;
                    self.pieces[ci].t = base;
                    self.pieces[ci].side = self.stm;
                    self.pieces[ci].loc = Loc::Hand(self.stm);
                    self.hands[self.stm as usize][base as usize] += 1;
                }
            }
        }

        self.board[mv.from as usize] = -1;
        self.board[mv.to as usize] = mi as i32;
        self.pieces[mi].loc = Loc::Board(mv.to);
        self.pieces[mi].moved = true;
        if let Some(pt) = mv.promo {
            self.pieces[mi].t = pt;
        }

        match mv.kind {
            MoveKind::DoubleStep => {
                let (x, _) = g.board.xy(mv.from);
                let (_, y2) = g.board.xy(mv.to);
                let (_, y1) = g.board.xy(mv.from);
                let mid_y = (y1 + y2) / 2;
                self.ep = g.board.sq(x as u8, mid_y as u8);
            }
            MoveKind::Castle => {
                let ri = self.board[mv.aux as usize] as usize;
                let rook_to = (mv.from + mv.to) / 2;
                u.partner = Some((ri, mv.aux, self.pieces[ri].moved));
                self.board[mv.aux as usize] = -1;
                self.board[rook_to as usize] = ri as i32;
                self.pieces[ri].loc = Loc::Board(rook_to);
                self.pieces[ri].moved = true;
            }
            _ => {}
        }

        self.stm = (self.stm + 1) % g.sides;
        u
    }

    pub fn unmake(&mut self, g: &GameDef, u: &Undo) {
        self.stm = (self.stm + g.sides - 1) % g.sides;
        self.ep = u.prior_ep;
        let mv = &u.mv;

        if mv.kind == MoveKind::Drop {
            self.board[mv.to as usize] = -1;
            self.pieces[u.moving].loc = Loc::Hand(self.stm);
            self.pieces[u.moving].moved = u.prior_moved;
            self.hands[self.stm as usize][mv.drop_type as usize] += 1;
            return;
        }

        if let Some((ri, rsq, rmoved)) = u.partner {
            let rook_to = (mv.from + mv.to) / 2;
            self.board[rook_to as usize] = -1;
            self.board[rsq as usize] = ri as i32;
            self.pieces[ri].loc = Loc::Board(rsq);
            self.pieces[ri].moved = rmoved;
        }

        self.board[mv.to as usize] = -1;
        self.board[mv.from as usize] = u.moving as i32;
        self.pieces[u.moving].loc = Loc::Board(mv.from);
        self.pieces[u.moving].moved = u.prior_moved;
        self.pieces[u.moving].t = u.prior_t;

        if let Some((ci, prior)) = u.captured {
            if let Loc::Hand(s) = self.pieces[ci].loc {
                self.hands[s as usize][self.pieces[ci].t as usize] -= 1;
            }
            self.pieces[ci] = prior;
            let victim_sq = if mv.kind == MoveKind::EnPassant { mv.aux } else { mv.to };
            self.board[victim_sq as usize] = ci as i32;
        }
    }

    /// Is `sq` attacked by any piece of `by`? Target predicates see the
    /// occupant of `sq` (empty ⇒ predicates like EnemyRoyal fail).
    pub fn is_attacked(&self, g: &GameDef, sq: u16, by: Side) -> bool {
        use crate::bits::TargetPred;
        let (tx, ty) = g.board.xy(sq);
        let occ_royal = self.piece_at(sq).map_or(false, |p| g.types[p.t as usize].royal);
        let pred_ok = |t: TargetPred| match t {
            TargetPred::Any => true,
            TargetPred::EnemyRoyal => occ_royal,
        };
        for p in &self.pieces {
            let Loc::Board(psq) = p.loc else { continue };
            if p.side != by || psq == sq {
                continue;
            }
            let (px, py) = g.board.xy(psq);
            let (dx, dy) = ((tx - px) as i32, (ty - py) as i32);
            let ck = g.compiled(p.t, p.side);
            let zone_ok = |z: Option<usize>, s: Side, at: u16| {
                z.map_or(true, |zi| g.zones[zi].contains(s, at))
            };
            for k in &ck.leaps {
                if !k.mode.can_capture() || k.d.dx as i32 != dx || k.d.dy as i32 != dy {
                    continue;
                }
                if !pred_ok(k.target)
                    || !zone_ok(k.from_zone, p.side, psq)
                    || !zone_ok(k.to_zone, p.side, sq)
                {
                    continue;
                }
                let blocked = k.blockers.iter().any(|b| {
                    g.board
                        .offset(psq, b.dx, b.dy)
                        .map_or(true, |bsq| self.board[bsq as usize] >= 0)
                });
                if !blocked {
                    return true;
                }
            }
            'ride: for k in &ck.rides {
                if !k.mode.can_capture() || !pred_ok(k.target) {
                    continue;
                }
                let Some(steps) = ray_steps(dx, dy, k.d.dx as i32, k.d.dy as i32) else {
                    continue;
                };
                if !zone_ok(k.from_zone, p.side, psq) || !zone_ok(k.to_zone, p.side, sq) {
                    continue;
                }
                let mut cur = psq;
                for _ in 1..steps {
                    cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                    if self.board[cur as usize] >= 0 {
                        continue 'ride;
                    }
                }
                return true;
            }
            'hop: for k in &ck.hops {
                if !pred_ok(k.target) {
                    continue;
                }
                let Some(steps) = ray_steps(dx, dy, k.d.dx as i32, k.d.dy as i32) else {
                    continue;
                };
                if !zone_ok(k.from_zone, p.side, psq) || !zone_ok(k.to_zone, p.side, sq) {
                    continue;
                }
                let mut screens = 0;
                let mut cur = psq;
                for _ in 1..steps {
                    cur = g.board.offset(cur, k.d.dx, k.d.dy).unwrap();
                    if self.board[cur as usize] >= 0 {
                        screens += 1;
                        if screens > 1 {
                            continue 'hop;
                        }
                    }
                }
                if screens == 1 {
                    return true;
                }
            }
        }
        false
    }

    /// True if any royal piece of `side` is attacked (includes xiangqi's
    /// facing-generals rule via the flying-general target-predicate Bit).
    pub fn royal_attacked(&self, g: &GameDef, side: Side) -> bool {
        let enemy = (side + 1) % g.sides;
        self.pieces.iter().any(|p| {
            g.types[p.t as usize].royal
                && p.side == side
                && matches!(p.loc, Loc::Board(sq) if self.is_attacked(g, sq, enemy))
        })
    }

    pub fn forward(side: Side) -> i8 {
        Self::fwd(side)
    }
}

/// If (dx,dy) = k·(sx,sy) for integer k ≥ 1, return k.
fn ray_steps(dx: i32, dy: i32, sx: i32, sy: i32) -> Option<i32> {
    let k = if sx != 0 {
        if dx % sx != 0 {
            return None;
        }
        dx / sx
    } else if dx != 0 {
        return None;
    } else if sy != 0 {
        if dy % sy != 0 {
            return None;
        }
        dy / sy
    } else {
        return None;
    };
    if k >= 1 && dx == k * sx && dy == k * sy {
        Some(k)
    } else {
        None
    }
}
