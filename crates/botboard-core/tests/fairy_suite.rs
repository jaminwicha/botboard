//! Vocabulary batch 3 — the fairy-chess atoms, validated by composing
//! known variants and armies from the compendium: Shatranj, Capablanca,
//! Grasshopper chess, Nightrider chess, Berolina pawns, locust capture,
//! and Betza's Chess with Different Armies. No external perft numbers are
//! asserted (none invented); the validation spine is mathematical
//! invariants (exact move sets derived by construction) plus
//! cross-representation perft equivalence (mailbox vs wide-bitboard) and
//! apply/undo hash parity (debug builds also assert the incremental key
//! inside every make).

use botboard_core::bits::{HopMode, Mode, MoveBit, SpecialBit};
use botboard_core::game::*;
use botboard_core::geometry::DirFilter;
use botboard_core::movegen::legal_moves;
use botboard_core::moves::{move_str, MoveKind};
use botboard_core::perft::perft;
use botboard_core::position::{Loc, Position};
use botboard_core::variants::cwda::{self, Army};

fn chess_policy() -> Policy {
    Policy {
        stalemate: StalematePolicy::Draw,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    }
}

/// Cross-representation spine: perft at 1..=maxd must agree between the
/// mailbox and wide-bitboard board classes, and be nonzero.
fn assert_reps_agree(mut build: impl FnMut() -> (GameDef, Position), maxd: u32) {
    for d in 1..=maxd {
        let mut counts = Vec::new();
        for use_bb in [false, true] {
            let (mut g, pos) = build();
            g.use_bitboards = use_bb;
            // Positions were built against a mailbox-only def; re-derive
            // the occupancy mirrors under the bitboard dispatch.
            let list: Vec<_> = pos
                .pieces
                .iter()
                .filter_map(|p| match p.loc {
                    Loc::Board(sq) => Some((p.t, p.side, sq, p.moved)),
                    _ => None,
                })
                .collect();
            let terrain = pos.terrain.clone();
            let stm = pos.stm;
            let mut pos2 = Position::from_pieces(&g, &list, stm);
            pos2.terrain = terrain;
            pos2.rehash(&g);
            counts.push(perft(&g, &mut pos2, d));
        }
        assert_eq!(counts[0], counts[1], "mailbox vs bitboard diverge at depth {d}");
        assert!(counts[0] > 0, "no nodes at depth {d}");
    }
}

// ---------------------------------------------------------------------------
// 1. Shatranj: ferz + alfil (the medieval slow pieces) from bare leapers
// ---------------------------------------------------------------------------

fn shatranj() -> (GameDef, Position) {
    const FERS: TypeId = 4;
    let board = BoardDef { w: 8, h: 8 };
    let mut last_rank = Zone::new(2);
    last_rank.add_rect(0, &board, 0, 7, 7, 7);
    last_rank.add_rect(1, &board, 0, 0, 7, 0);
    let types = vec![
        // Shah: king steps, no castling in shatranj.
        PieceTypeDef::new("shah", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        // Rukh: the rook, unchanged through the centuries.
        PieceTypeDef::new("rukh", 'R', vec![MoveBit::rider(0, 1)]),
        // Faras: the knight.
        PieceTypeDef::new("faras", 'N', vec![MoveBit::leaper(1, 2)]),
        // Alfil: (2,2) leaper — jumps the midpoint (a true leaper here,
        // unlike xiangqi's blockable elephant).
        PieceTypeDef::new("alfil", 'A', vec![MoveBit::leaper(2, 2)]),
        // Fers: (1,1) leaper, the medieval queen.
        PieceTypeDef::new("fers", 'F', vec![MoveBit::leaper(1, 1)]),
        // Baidaq: pawn with no double step, promoting only to fers.
        PieceTypeDef::new(
            "baidaq",
            'P',
            vec![
                MoveBit::leaper(0, 1).dirs(DirFilter::Forward).mode(Mode::Move),
                MoveBit::leaper(1, 1).dirs(DirFilter::Forward).mode(Mode::Capture),
            ],
        )
        .promo(PromoRule {
            zone: 0,
            choices: vec![FERS],
            trigger: PromoTrigger::Dest,
            optional: false,
        }),
    ];
    let policy = Policy {
        // Shatranj: a stalemated player loses.
        stalemate: StalematePolicy::Loss,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };
    let mut start = Vec::new();
    let back = [1u8, 2, 3, 4, 0, 3, 2, 1]; // R N A F K A N R
    for (x, &t) in back.iter().enumerate() {
        start.push((t, 0, x as u8, 0));
        start.push((t, 1, x as u8, 7));
    }
    for x in 0..8 {
        start.push((5, 0, x, 1));
        start.push((5, 1, x, 6));
    }
    let g = GameDef::new(board, 2, types, vec![last_rank], policy, start);
    let pos = Position::startpos(&g);
    (g, pos)
}

#[test]
fn shatranj_builds_and_representations_agree() {
    let (g, mut pos) = shatranj();
    // The slow pieces exist and the game runs: structural spine only.
    assert_eq!(g.types.len(), 6);
    assert!(!legal_moves(&g, &mut pos).is_empty());
    assert_reps_agree(shatranj, 3);
}

// ---------------------------------------------------------------------------
// 2. Capablanca chess 10×8: archbishop (B+N) and chancellor (R+N)
// ---------------------------------------------------------------------------

fn capablanca() -> (GameDef, Position) {
    const K: TypeId = 0;
    const Q: TypeId = 1;
    const R: TypeId = 2;
    const B: TypeId = 3;
    const N: TypeId = 4;
    const A: TypeId = 5; // archbishop
    const C: TypeId = 6; // chancellor
    const P: TypeId = 7;
    let board = BoardDef { w: 10, h: 8 };
    let mut pawn_start = Zone::new(2);
    pawn_start.add_rect(0, &board, 0, 1, 9, 1);
    pawn_start.add_rect(1, &board, 0, 6, 9, 6);
    let mut last_rank = Zone::new(2);
    last_rank.add_rect(0, &board, 0, 7, 9, 7);
    last_rank.add_rect(1, &board, 0, 0, 9, 0);
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal()
            .specials(vec![SpecialBit::Castling]),
        PieceTypeDef::new("queen", 'Q', vec![MoveBit::rider(0, 1), MoveBit::rider(1, 1)]),
        PieceTypeDef::new("rook", 'R', vec![MoveBit::rider(0, 1)]).castle_partner(),
        PieceTypeDef::new("bishop", 'B', vec![MoveBit::rider(1, 1)]),
        PieceTypeDef::new("knight", 'N', vec![MoveBit::leaper(1, 2)]),
        // Archbishop = B + N: compound of two existing atoms.
        PieceTypeDef::new(
            "archbishop",
            'A',
            vec![MoveBit::rider(1, 1), MoveBit::leaper(1, 2)],
        ),
        // Chancellor = R + N.
        PieceTypeDef::new(
            "chancellor",
            'C',
            vec![MoveBit::rider(0, 1), MoveBit::leaper(1, 2)],
        ),
        PieceTypeDef::new(
            "pawn",
            'P',
            vec![
                MoveBit::leaper(0, 1).dirs(DirFilter::Forward).mode(Mode::Move),
                MoveBit::leaper(1, 1).dirs(DirFilter::Forward).mode(Mode::Capture),
            ],
        )
        .specials(vec![
            SpecialBit::DoubleStep { start_zone: 0 },
            SpecialBit::EnPassant,
        ])
        .promo(PromoRule {
            zone: 1,
            choices: vec![Q, A, C, R, B, N],
            trigger: PromoTrigger::Dest,
            optional: false,
        }),
    ];
    // Capablanca's array: R N A B Q K B C N R.
    let back = [R, N, A, B, Q, K, B, C, N, R];
    let mut start = Vec::new();
    for (x, &t) in back.iter().enumerate() {
        start.push((t, 0, x as u8, 0));
        start.push((t, 1, x as u8, 7));
    }
    for x in 0..10 {
        start.push((P, 0, x, 1));
        start.push((P, 1, x, 6));
    }
    let g = GameDef::new(
        board,
        2,
        types,
        vec![pawn_start, last_rank],
        chess_policy(),
        start,
    );
    let pos = Position::startpos(&g);
    (g, pos)
}

#[test]
fn capablanca_builds_and_representations_agree() {
    let (g, mut pos) = capablanca();
    assert_eq!(g.board.ncells(), 80, "10×8 rides the ≤128-cell fast path too");
    assert!(!legal_moves(&g, &mut pos).is_empty());
    assert_reps_agree(capablanca, 3);
}

// ---------------------------------------------------------------------------
// 3. Grasshopper chess: the BeyondScreen landing mode
// ---------------------------------------------------------------------------

/// A minimal grasshopper world: kings tucked off every queen-line of the
/// grasshopper's post at d4, plus optional extra pieces per test.
fn grasshopper_game(extra: Vec<(TypeId, Side, u8, u8)>) -> (GameDef, Position) {
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        // Grasshopper: queen-line hopper landing immediately beyond the
        // screen (Betza gQ); `.landing(BeyondScreen)` widens the hopper
        // default to move-or-capture on that square.
        PieceTypeDef::new(
            "grasshopper",
            'G',
            vec![
                MoveBit::hopper(0, 1).landing(HopMode::BeyondScreen),
                MoveBit::hopper(1, 1).landing(HopMode::BeyondScreen),
            ],
        ),
        // Inert screen fodder.
        PieceTypeDef::new("stone", 'S', vec![MoveBit::leaper(0, 1).mode(Mode::Move)]),
    ];
    let mut start = vec![(0, 0, 0, 7), (0, 1, 7, 0), (1, 0, 3, 3)];
    start.extend(extra);
    let g = GameDef::new(BoardDef { w: 8, h: 8 }, 2, types, vec![], chess_policy(), start);
    let pos = Position::startpos(&g);
    (g, pos)
}

#[test]
fn grasshopper_needs_a_screen_and_hops_all_eight_lines() {
    // On an otherwise empty board a grasshopper has ZERO moves: every hop
    // needs a screen. (Kings at a8/h1 sit off all eight queen-lines of d4.)
    let (g, mut pos) = grasshopper_game(vec![]);
    let gsq = g.board.sq(3, 3);
    let hops: Vec<_> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == gsq)
        .collect();
    assert!(hops.is_empty(), "no screens ⇒ no grasshopper moves, got {hops:?}");
    // And the attack map agrees: nothing on the board is threatened by the
    // grasshopper (side 0's own king at a8 covers its chebyshev-1 ring, so
    // skip those three squares).
    let ksq = g.board.sq(0, 7);
    let (kx, ky) = g.board.xy(ksq);
    for sq in 0..g.board.ncells() as u16 {
        let (x, y) = g.board.xy(sq);
        if (x - kx).abs().max((y - ky).abs()) <= 1 {
            continue;
        }
        assert!(
            sq == gsq || !pos.is_attacked(&g, sq, 0),
            "empty-board grasshopper must threaten nothing (sq {sq})"
        );
    }

    // A full ring of screens on the 8 neighbors: the known geometry is a
    // hop to the square immediately beyond each screen — the distance-2
    // ring, all eight directions, and nothing else.
    let ring: Vec<(TypeId, Side, u8, u8)> = [(2u8, 2u8), (2, 3), (2, 4), (3, 2), (3, 4), (4, 2), (4, 3), (4, 4)]
        .iter()
        .map(|&(x, y)| (2 as TypeId, 0 as Side, x, y))
        .collect();
    let (g, mut pos) = grasshopper_game(ring);
    let gsq = g.board.sq(3, 3);
    let mut dests: Vec<u16> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == gsq)
        .map(|m| m.to)
        .collect();
    dests.sort_unstable();
    let mut expected: Vec<u16> = [
        (1u8, 1u8), (1, 3), (1, 5), (3, 1), (3, 5), (5, 1), (5, 3), (5, 5),
    ]
    .iter()
    .map(|&(x, y)| g.board.sq(x, y))
    .collect();
    expected.sort_unstable();
    assert_eq!(dests, expected, "8-direction hop ring, distance exactly 2");
}

#[test]
fn grasshopper_captures_only_on_the_landing_square() {
    // Screen at d6, enemy stone at d7: the hop lands exactly beyond the
    // screen and captures there. The enemy at d8-line beyond is untouchable.
    let (g, mut pos) = grasshopper_game(vec![
        (2, 0, 3, 5), // friendly screen d6
        (2, 1, 3, 6), // enemy on the landing square d7
    ]);
    let gsq = g.board.sq(3, 3);
    let hops: Vec<_> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == gsq)
        .collect();
    assert_eq!(hops.len(), 1);
    assert_eq!(hops[0].to, g.board.sq(3, 6), "capture exactly beyond the screen");
    // Attack map parity: the landing square is threatened, the screen is
    // not (BeyondScreen never captures the screen), and past-landing isn't.
    assert!(pos.is_attacked(&g, g.board.sq(3, 6), 0));
    assert!(!pos.is_attacked(&g, g.board.sq(3, 5), 0), "the screen itself is safe");
    assert!(!pos.is_attacked(&g, g.board.sq(3, 7), 0), "no cannon-style reach beyond");

    // A friendly on the landing square kills the hop.
    let (g2, mut pos2) = grasshopper_game(vec![(2, 0, 3, 5), (2, 0, 3, 6)]);
    let gsq2 = g2.board.sq(3, 3);
    assert!(
        !legal_moves(&g2, &mut pos2).iter().any(|m| m.from == gsq2),
        "friendly landing square blocks the hop"
    );

    // Hash parity through a grasshopper capture.
    let mv = hops[0];
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert!(pos.piece_at(g.board.sq(3, 6)).map_or(false, |p| p.side == 0));
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0);
}

// ---------------------------------------------------------------------------
// 4. Nightrider chess: rider(1,2) proven as the Nightrider
// ---------------------------------------------------------------------------

fn nightrider_game(extra: Vec<(TypeId, Side, u8, u8)>) -> (GameDef, Position) {
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        // Nightrider: repeated (1,2) steps along a straight knight-line.
        PieceTypeDef::new("nightrider", 'M', vec![MoveBit::rider(1, 2)]),
        PieceTypeDef::new("stone", 'S', vec![MoveBit::leaper(0, 1).mode(Mode::Move)]),
    ];
    // Kings at a8/h1: off every knight-line of both test posts (d4, a1).
    let mut start = vec![(0, 0, 0, 7), (0, 1, 7, 0)];
    start.extend(extra);
    let g = GameDef::new(BoardDef { w: 8, h: 8 }, 2, types, vec![], chess_policy(), start);
    let pos = Position::startpos(&g);
    (g, pos)
}

#[test]
fn nightrider_geometry_from_the_center() {
    // From d4 = (3,3) on an empty 8×8, walking each of the 8 knight deltas
    // to the edge gives, by construction:
    //   (1,2):  e6, f8      (2,1):  f5, h6
    //   (-1,2): c6, b8      (-2,1): b5
    //   (1,-2): e2          (2,-1): f3, h2
    //   (-1,-2): c2         (-2,-1): b3
    // — 12 destinations in all.
    let (g, mut pos) = nightrider_game(vec![(1, 0, 3, 3)]);
    let nsq = g.board.sq(3, 3);
    let mut dests: Vec<u16> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == nsq)
        .map(|m| m.to)
        .collect();
    dests.sort_unstable();
    let mut expected: Vec<u16> = [
        (4u8, 5u8), (5, 7), (5, 4), (7, 5), (2, 5), (1, 7), (1, 4),
        (4, 1), (5, 2), (7, 1), (2, 1), (1, 2),
    ]
    .iter()
    .map(|&(x, y)| g.board.sq(x, y))
    .collect();
    expected.sort_unstable();
    assert_eq!(dests.len(), 12, "centered nightrider on empty 8×8 = 12 destinations");
    assert_eq!(dests, expected);
}

#[test]
fn nightrider_is_blocked_mid_path() {
    // Nightrider at a1 = (0,0); its (1,2) ray runs b3, c5, d7. An enemy
    // stone on c5 = (2,4) is capturable, but d7 beyond it is not; a
    // FRIENDLY stone on b3 cuts the whole ray.
    let (g, mut pos) = nightrider_game(vec![(1, 0, 0, 0), (2, 1, 2, 4)]);
    let nsq = g.board.sq(0, 0);
    let dests: Vec<u16> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == nsq)
        .map(|m| m.to)
        .collect();
    assert!(dests.contains(&g.board.sq(1, 2)), "b3 open");
    assert!(dests.contains(&g.board.sq(2, 4)), "captures the blocker at c5");
    assert!(!dests.contains(&g.board.sq(3, 6)), "may not pass through c5 to d7");
    assert!(pos.is_attacked(&g, g.board.sq(2, 4), 0), "attack map sees the c5 hit");
    assert!(!pos.is_attacked(&g, g.board.sq(3, 6), 0), "and not past it");

    let (g2, mut pos2) = nightrider_game(vec![(1, 0, 0, 0), (2, 0, 1, 2)]);
    let n2 = g2.board.sq(0, 0);
    let dests2: Vec<u16> = legal_moves(&g2, &mut pos2)
        .into_iter()
        .filter(|m| m.from == n2)
        .map(|m| m.to)
        .collect();
    assert!(
        !dests2.iter().any(|&d| d == g2.board.sq(2, 4) || d == g2.board.sq(3, 6)),
        "friendly at b3 cuts the (1,2) ray entirely"
    );
}

#[test]
fn nightrider_representations_agree() {
    assert_reps_agree(
        || {
            nightrider_game(vec![
                (1, 0, 3, 3),
                (2, 1, 2, 4),
                (2, 1, 5, 4),
                (2, 0, 1, 2),
                (1, 1, 4, 4),
            ])
        },
        4,
    );
}

// ---------------------------------------------------------------------------
// 5. Berolina pawns: the mode-split filters, mirrored
// ---------------------------------------------------------------------------

#[test]
fn berolina_pawns_move_diagonally_and_capture_straight() {
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        // Berolina pawn: FIDE pawn with move and capture swapped —
        // diagonal-forward move-only, straight-forward capture-only.
        PieceTypeDef::new(
            "berolina",
            'B',
            vec![
                MoveBit::leaper(1, 1).dirs(DirFilter::Forward).mode(Mode::Move),
                MoveBit::leaper(0, 1).dirs(DirFilter::Forward).mode(Mode::Capture),
            ],
        ),
        PieceTypeDef::new("stone", 'S', vec![MoveBit::leaper(0, 1).mode(Mode::Move)]),
    ];
    // Berolina at d4; enemy stones straight ahead at d5 and diagonally at e5.
    let g = GameDef::new(
        BoardDef { w: 8, h: 8 },
        2,
        types,
        vec![],
        chess_policy(),
        vec![(0, 0, 0, 0), (0, 1, 7, 7), (1, 0, 3, 3), (2, 1, 3, 4), (2, 1, 4, 4)],
    );
    let mut pos = Position::startpos(&g);
    let bsq = g.board.sq(3, 3);
    let mut moves: Vec<(u16, bool)> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == bsq)
        .map(|m| (m.to, pos.piece_at(m.to).is_some()))
        .collect();
    moves.sort_unstable();
    // Exactly: quiet diagonal to c5 (e5 is occupied — the diagonal cannot
    // capture) and a straight capture on d5. Nothing else.
    let mut expected = vec![(g.board.sq(2, 4), false), (g.board.sq(3, 4), true)];
    expected.sort_unstable();
    assert_eq!(moves, expected, "berolina exact move set at d4");
    // Attack map: the straight square is threatened, the diagonals are not.
    assert!(pos.is_attacked(&g, g.board.sq(3, 4), 0));
    assert!(!pos.is_attacked(&g, g.board.sq(4, 4), 0));
    assert!(!pos.is_attacked(&g, g.board.sq(2, 4), 0));
}

// ---------------------------------------------------------------------------
// 6. Locust capture: screen removed, hash parity on undo
// ---------------------------------------------------------------------------

fn locust_game(extra: Vec<(TypeId, Side, u8, u8)>) -> (GameDef, Position) {
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        // Locust: queen-line hopper that captures its screen (checkers
        // atom) and lands just beyond it.
        PieceTypeDef::new(
            "locust",
            'L',
            vec![
                MoveBit::hopper(0, 1).landing(HopMode::Locust),
                MoveBit::hopper(1, 1).landing(HopMode::Locust),
            ],
        ),
        PieceTypeDef::new("stone", 'S', vec![MoveBit::leaper(0, 1).mode(Mode::Move)]),
    ];
    let mut start = vec![(0, 0, 0, 7), (0, 1, 7, 0)];
    start.extend(extra);
    let g = GameDef::new(BoardDef { w: 8, h: 8 }, 2, types, vec![], chess_policy(), start);
    let pos = Position::startpos(&g);
    (g, pos)
}

#[test]
fn locust_captures_the_screen_and_lands_beyond() {
    // Locust d4; enemy stone d6 (the screen); d7 empty (the landing).
    let (g, mut pos) = locust_game(vec![(1, 0, 3, 3), (2, 1, 3, 5)]);
    let lsq = g.board.sq(3, 3);
    let screen = g.board.sq(3, 5);
    let landing = g.board.sq(3, 6);
    let moves: Vec<_> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == lsq)
        .collect();
    assert_eq!(moves.len(), 1, "exactly the one locust hop: {moves:?}");
    let mv = moves[0];
    assert_eq!(mv.kind, MoveKind::Locust);
    assert_eq!(mv.to, landing, "lands immediately beyond the screen");
    assert_eq!(mv.aux, screen, "the aux victim is the screen itself");
    assert_eq!(move_str(&g, &mv), "d4d7xd6", "locust notation never aliases a quiet move");

    // The capture geometry threatens the SCREEN square, not the landing.
    assert!(pos.is_attacked(&g, screen, 0), "locust threatens its screen");
    assert!(!pos.is_attacked(&g, landing, 0), "the empty landing is not a target");

    let si = pos.board[screen as usize] as usize;
    let h0 = pos.hash;
    let u = pos.make(&g, &mv);
    assert_eq!(pos.pieces[si].loc, Loc::Dead, "the screen is captured");
    assert!(pos.piece_at(screen).is_none(), "…and its square is empty");
    assert!(
        pos.piece_at(landing).map_or(false, |p| p.side == 0),
        "the locust stands beyond the crater"
    );
    pos.unmake(&g, &u);
    assert_eq!(pos.hash, h0, "hash parity on undo (screen restored)");
    assert_eq!(pos.pieces[si].loc, Loc::Board(screen));
    assert!(pos.piece_at(lsq).map_or(false, |p| p.side == 0), "locust back home");
}

#[test]
fn locust_needs_an_enemy_screen_and_an_empty_landing() {
    // Friendly screen: no locust move on that ray.
    let (g, mut pos) = locust_game(vec![(1, 0, 3, 3), (2, 0, 3, 5)]);
    let lsq = g.board.sq(3, 3);
    assert!(
        !legal_moves(&g, &mut pos).iter().any(|m| m.from == lsq),
        "a friendly screen is not a victim"
    );
    // Occupied landing: the hop dies even over an enemy screen…
    let (g2, mut pos2) = locust_game(vec![(1, 0, 3, 3), (2, 1, 3, 5), (2, 1, 3, 6)]);
    let l2 = g2.board.sq(3, 3);
    assert!(
        !legal_moves(&g2, &mut pos2).iter().any(|m| m.from == l2),
        "blocked landing kills the locust hop"
    );
    // …and the attack map agrees the screen is then safe.
    assert!(!pos2.is_attacked(&g2, g2.board.sq(3, 5), 0));
    // Screen on the board edge (landing off-board): no hop either.
    let (g3, mut pos3) = locust_game(vec![(1, 0, 3, 3), (2, 1, 3, 7)]);
    let l3 = g3.board.sq(3, 3);
    assert!(
        !legal_moves(&g3, &mut pos3).iter().any(|m| m.from == l3),
        "no landing square beyond the edge"
    );
}

// ---------------------------------------------------------------------------
// 7. Range-limited riders: the R4 atom
// ---------------------------------------------------------------------------

fn shortrook_game() -> (GameDef, Position) {
    let types = vec![
        PieceTypeDef::new("king", 'K', vec![MoveBit::leaper(0, 1), MoveBit::leaper(1, 1)])
            .royal(),
        PieceTypeDef::new("rook", 'R', vec![MoveBit::rider(0, 1)]),
        PieceTypeDef::new("shortrook", 'S', vec![MoveBit::rider(0, 1).max_steps(4)]),
    ];
    // Kings and the reference rook placed OFF the short rook's rays so
    // nothing checks anyone and a8's east/south rays stay clean.
    let g = GameDef::new(
        BoardDef { w: 8, h: 8 },
        2,
        types,
        vec![],
        chess_policy(),
        vec![(0, 0, 0, 0), (0, 1, 6, 7), (2, 0, 0, 7), (1, 0, 7, 0)],
    );
    let pos = Position::startpos(&g);
    (g, pos)
}

#[test]
fn short_rook_reaches_exactly_four_squares() {
    let (g, mut pos) = shortrook_game();
    let ssq = g.board.sq(0, 7); // a8: rays run east (7 squares) and south (7)
    let dests: Vec<u16> = legal_moves(&g, &mut pos)
        .into_iter()
        .filter(|m| m.from == ssq)
        .map(|m| m.to)
        .collect();
    assert_eq!(dests.len(), 8, "4 east + 4 south, no more: {dests:?}");
    assert!(dests.contains(&g.board.sq(4, 7)), "step 4 reachable");
    assert!(!dests.contains(&g.board.sq(5, 7)), "step 5 out of range");
    // Attack maps honor the cap too.
    assert!(pos.is_attacked(&g, g.board.sq(0, 3), 0), "4 down: in range");
    assert!(!pos.is_attacked(&g, g.board.sq(0, 2), 0), "5 down: beyond the cap");
}

#[test]
fn short_rook_prices_below_the_full_rook() {
    use botboard_core::cost::cost_prior;
    let (g, _) = shortrook_game();
    let full = cost_prior(&g, 1, &cwda::W);
    let short = cost_prior(&g, 2, &cwda::W);
    assert!(
        short < full,
        "the mobility integral must price R4 ({short:.2}) below R ({full:.2})"
    );
    assert!(short > 0.5 * full, "…but not absurdly below (sanity)");
}

#[test]
fn short_rook_representations_agree() {
    // The bb fast path must exclude range-limited kernels from its plain
    // tables (like HP-gated ones): perft equality is the proof.
    assert_reps_agree(shortrook_game, 4);
}

// ---------------------------------------------------------------------------
// 8. Chess with Different Armies: composition + fairness measurement
// ---------------------------------------------------------------------------

#[test]
fn cwda_games_build_and_representations_agree() {
    for army in [Army::Clobberers, Army::Rookies, Army::Nutters] {
        let g = cwda::game(Army::Fide, army);
        let mut pos = Position::startpos(&g);
        assert_eq!(g.types.len(), 11);
        assert!(!legal_moves(&g, &mut pos).is_empty(), "{} vs FIDE plays", army.name());
        assert_reps_agree(
            move || {
                let g = cwda::game(Army::Fide, army);
                let pos = Position::startpos(&g);
                (g, pos)
            },
            2,
        );
    }
}

#[test]
fn cwda_cost_table_always_run() {
    // Measurement (a): cost_prior totals per army with the standard
    // weights. Printed for the record; asserted only for sanity — the
    // numbers are the finding, not a target.
    println!("\nCwDA cost priors (w_move = w_attack = 0.35):");
    println!("{:<24} {:>10} {:>10} {:>10} {:>10} {:>8} {:>10}",
        "army", "Q-slot", "R-slot", "B-slot", "N-slot", "pawn", "TOTAL");
    for a in cwda::ALL {
        let sheet = cwda::army_costs(a, &cwda::W);
        println!(
            "{:<24} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>8.2} {:>10.2}",
            a.name(),
            sheet.pieces[0].1,
            sheet.pieces[1].1,
            sheet.pieces[2].1,
            sheet.pieces[3].1,
            sheet.pieces[4].1,
            sheet.total
        );
        for (name, cost, _) in &sheet.pieces {
            assert!(*cost >= 1.0, "{name} under the cost floor");
        }
        assert!(sheet.total > 0.0);
    }
    // One structural sanity: FIDE's queen outprices FIDE's knight.
    let fide = cwda::army_costs(Army::Fide, &cwda::W);
    assert!(fide.pieces[0].1 > fide.pieces[3].1);
}

#[test]
fn cwda_paired_match_always_run() {
    // Measurement (b), small always-run version: 2 pairs, depth 3. The
    // numbers speak; nothing is asserted beyond the games finishing.
    let (pairs, depth, nodes, plies) = (2u32, 3i32, 4_000u64, 160u32);
    println!("\nCwDA paired matches vs FIDE ({pairs} pairs, depth {depth}, {nodes} nodes):");
    for b in [Army::Clobberers, Army::Rookies, Army::Nutters] {
        let (wf, wb, dr) = cwda::paired_match(Army::Fide, b, pairs, depth, nodes, 17, plies);
        let total = wf + wb + dr;
        assert_eq!(total, pairs * 2, "every game reaches a result bucket");
        println!(
            "  FIDE {wf} — {dr} — {wb} {}  (FIDE score {:.0}%)",
            b.name(),
            100.0 * (wf as f64 + dr as f64 * 0.5) / total as f64
        );
    }
}

#[test]
#[ignore] // heavy: run with `cargo test --release -- --ignored`
fn cwda_paired_match_full() {
    // Measurement (b) at full strength: ≥8 pairs, depth 4, 20k nodes.
    let (pairs, depth, nodes, plies) = (8u32, 4i32, 20_000u64, 300u32);
    println!("\nCwDA paired matches vs FIDE ({pairs} pairs, depth {depth}, {nodes} nodes):");
    for b in [Army::Clobberers, Army::Rookies, Army::Nutters] {
        let (wf, wb, dr) = cwda::paired_match(Army::Fide, b, pairs, depth, nodes, 17, plies);
        let total = wf + wb + dr;
        println!(
            "  FIDE {wf} — {dr} — {wb} {}  (FIDE score {:.1}%)",
            b.name(),
            100.0 * (wf as f64 + dr as f64 * 0.5) / total as f64
        );
    }
}
