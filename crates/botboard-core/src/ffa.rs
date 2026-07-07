//! N-player free-for-all — the committed baseline ruling for the spec's
//! open Tier-1 gap (§8.7, §13).
//!
//! **The ruling (v1):** last-royal-standing FFA. Strict rotation over live
//! players (§3.4); a player is **eliminated** when, on their turn, they
//! have no legal move (checkmate/stalemate-as-loss alike) — and their army
//! is removed from the board (no inert-piece bookkeeping, no kingmaking
//! via corpses). The last live player wins; a global ply cap or full-state
//! repetition among the live set draws among survivors. Alliances are
//! deliberately out of scope for the baseline. Check/legality is
//! N-player-correct: a royal is unsafe if *any* other side attacks it.
//!
//! Move choice uses the per-player value head (§7.4): greedy
//! maximize-own-minus-best-rival over one ply — the Pluribus-flavored
//! "hard to exploit, not optimal" baseline; deeper N-player search plugs
//! in behind the same chooser signature.

use crate::eval::Eval;
use crate::game::{GameDef, Side};
use crate::movegen::legal_moves;
use crate::position::{Loc, Position};
use crate::rng::Rng;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfaOutcome {
    Win(Side),
    DrawAmong(u8),
}

/// Advance `pos.stm` to the next live player. Returns false if none.
fn next_live(g: &GameDef, pos: &mut Position, alive: &[bool]) -> bool {
    for _ in 0..g.sides {
        if alive[pos.stm as usize] {
            return true;
        }
        let next = (pos.stm + 1) % g.sides;
        pos.set_stm(g, next);
    }
    false
}

/// Remove every piece of `side` (elimination: the army collapses).
fn eliminate(g: &GameDef, pos: &mut Position, side: Side) {
    for i in 0..pos.pieces.len() {
        if pos.pieces[i].side == side {
            if let Loc::Board(_) = pos.pieces[i].loc {
                pos.pieces[i].loc = Loc::Dead;
            }
        }
    }
    // Hands of an eliminated player are void as well.
    for t in 0..g.types.len() {
        pos.hands[side as usize][t] = 0;
    }
    // Out-of-band bulk edit: re-derive mailbox, key, and mirrors.
    for sq in 0..pos.board.len() {
        if pos.board[sq] >= 0 && !matches!(pos.pieces[pos.board[sq] as usize].loc, Loc::Board(s) if s as usize == sq)
        {
            pos.board[sq] = -1;
        }
    }
    pos.rehash(g);
}

/// Greedy per-player-value chooser: maximize own value minus the best
/// rival's, with seeded tie-breaking.
pub fn choose_ffa_move(
    g: &GameDef,
    pos: &mut Position,
    eval: &Eval,
    rng: &mut Rng,
) -> Option<crate::moves::Move> {
    let me = pos.stm as usize;
    let moves = legal_moves(g, pos);
    if moves.is_empty() {
        return None;
    }
    let mut best: (i64, usize) = (i64::MIN, 0);
    for (i, mv) in moves.iter().enumerate() {
        let u = pos.make(g, mv);
        let v = eval.players(g, pos);
        pos.unmake(g, &u);
        let rival = (0..g.sides as usize)
            .filter(|&s| s != me)
            .map(|s| v[s])
            .max()
            .unwrap_or(0);
        let score = (v[me] - rival) as i64 * 4 + (rng.below(4)) as i64;
        if score > best.0 {
            best = (score, i);
        }
    }
    Some(moves[best.1])
}

/// Run a seeded N-player FFA to termination under the baseline ruling.
pub fn play_ffa(
    g: &GameDef,
    eval: &Eval,
    seed: u64,
    max_plies: u32,
) -> (FfaOutcome, u32) {
    let mut rng = Rng::new(seed);
    let mut pos = Position::startpos(g);
    let mut alive = vec![true; g.sides as usize];
    let mut plies = 0;
    let mut history: Vec<u64> = Vec::new();

    loop {
        if !next_live(g, &mut pos, &alive) {
            return (FfaOutcome::DrawAmong(0), plies);
        }
        let live_count = alive.iter().filter(|&&a| a).count();
        if live_count == 1 {
            let w = alive.iter().position(|&a| a).unwrap() as Side;
            return (FfaOutcome::Win(w), plies);
        }
        let stm = pos.stm;
        match choose_ffa_move(g, &mut pos, eval, &mut rng) {
            None => {
                // No legal move on your turn ⇒ elimination (the ruling).
                alive[stm as usize] = false;
                eliminate(g, &mut pos, stm);
                let next = (pos.stm + 1) % g.sides;
                pos.set_stm(g, next);
            }
            Some(mv) => {
                pos.make(g, &mv);
                plies += 1;
                // A capture that took a royal eliminates its owner too.
                for s in 0..g.sides {
                    if alive[s as usize]
                        && !pos.pieces.iter().any(|p| {
                            p.side == s
                                && g.types[p.t as usize].royal
                                && matches!(p.loc, Loc::Board(_))
                        })
                    {
                        alive[s as usize] = false;
                        eliminate(g, &mut pos, s);
                    }
                }
                history.push(pos.hash);
                if history.iter().filter(|&&h| h == pos.hash).count() >= 3
                    || plies >= max_plies
                {
                    let n = alive.iter().filter(|&&a| a).count() as u8;
                    return (FfaOutcome::DrawAmong(n), plies);
                }
            }
        }
    }
}
