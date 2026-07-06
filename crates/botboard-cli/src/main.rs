//! Botboard CLI: perft / divide / board display over the headless core.

use std::time::Instant;

use botboard_core::fen::chess_from_fen;
use botboard_core::game::GameDef;
use botboard_core::perft::{divide, perft};
use botboard_core::position::{Loc, Position};
use botboard_core::variants;

fn game_for(name: &str) -> GameDef {
    match name {
        "chess" => variants::chess::game(),
        "xiangqi" => variants::xiangqi::game(),
        "shogi" => variants::shogi::game(),
        _ => {
            eprintln!("unknown variant: {name} (chess|xiangqi|shogi)");
            std::process::exit(1);
        }
    }
}

fn print_board(g: &GameDef, pos: &Position) {
    for y in (0..g.board.h).rev() {
        print!("{:2} ", y + 1);
        for x in 0..g.board.w {
            match pos.piece_at(g.board.sq(x, y)) {
                None => print!(" ."),
                Some(p) => {
                    let c = g.types[p.t as usize].glyph;
                    print!(" {}", if p.side == 0 { c } else { c.to_ascii_lowercase() });
                }
            }
        }
        println!();
    }
    print!("   ");
    for x in 0..g.board.w {
        print!(" {}", (b'a' + x) as char);
    }
    println!("\nside to move: {}", pos.stm);
    for s in 0..g.sides {
        let hand: Vec<String> = (0..g.types.len())
            .filter(|&t| pos.hands[s as usize][t] > 0)
            .map(|t| format!("{}x{}", g.types[t].glyph, pos.hands[s as usize][t]))
            .collect();
        if !hand.is_empty() {
            println!("hand[{s}]: {}", hand.join(" "));
        }
        let _ = Loc::Dead;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: botboard <perft|divide|show> <variant> [depth] [--fen FEN]");
        std::process::exit(1);
    }
    let cmd = args[1].as_str();
    let g = game_for(&args[2]);
    let depth: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let fen = args
        .iter()
        .position(|a| a == "--fen")
        .and_then(|i| args.get(i + 1).cloned());

    let mut pos = match fen {
        Some(f) => chess_from_fen(&g, &f).unwrap_or_else(|e| {
            eprintln!("bad FEN: {e}");
            std::process::exit(1);
        }),
        None => Position::startpos(&g),
    };

    match cmd {
        "show" => print_board(&g, &pos),
        "perft" => {
            for d in 1..=depth {
                let t0 = Instant::now();
                let n = perft(&g, &mut pos, d);
                let dt = t0.elapsed().as_secs_f64();
                println!(
                    "perft({d}) = {n}  [{dt:.2}s, {:.0} kn/s]",
                    n as f64 / dt / 1000.0
                );
            }
        }
        "divide" => {
            let mut total = 0;
            for (m, n) in divide(&g, &mut pos, depth) {
                println!("{m}: {n}");
                total += n;
            }
            println!("total: {total}");
        }
        _ => eprintln!("unknown command: {cmd}"),
    }
}
