//! Botboard CLI: the simple UI over the headless core (§10.1 discipline:
//! all rules/state/RNG live in botboard-core; this layer is presentation,
//! input, and orchestration only).
//!
//! Commands:
//!   show|perft|divide <variant> [depth] [--fen FEN]
//!   play <variant> [--depth D] [--hidden] [--seed S]   interactive vs AI
//!   selfplay <variant> [--games N] [--depth D]          watch/aggregate
//!   cost <variant>                                      cost-model report
//!   league <variant> [--games N]                        population + Nash
//!   armies [--budget B] [--seed S]                      generate + price

use std::io::{self, BufRead, Write};
use std::time::Instant;

use botboard_core::belief::Belief;
use botboard_core::cost::{cost_prior, fit_weights, material_table, mobility_integral, Anchor};
use botboard_core::eval::Eval;
use botboard_core::game::GameDef;
use botboard_core::ladder::{choose_move, LadderConfig};
use botboard_core::league::{default_population, nash_averaging, profiles_json, round_robin};
use botboard_core::movegen::{legal_moves, status, Status};
use botboard_core::moves::move_str;
use botboard_core::perft::{divide, perft};
use botboard_core::position::Position;
use botboard_core::rng::Rng;
use botboard_core::search::Searcher;
use botboard_core::selfplay::{play_game, random_army, Outcome};
use botboard_core::variants;
use botboard_core::fen::chess_from_fen;

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

fn anchors_for(g: &GameDef, name: &str) -> Vec<Anchor> {
    match name {
        "chess" => {
            use variants::chess as c;
            vec![
                Anchor { t: c::P, value: 1.0 },
                Anchor { t: c::N, value: 3.0 },
                Anchor { t: c::B, value: 3.0 },
                Anchor { t: c::R, value: 5.0 },
                Anchor { t: c::Q, value: 9.0 },
            ]
        }
        _ => (0..g.types.len() as u8)
            .filter(|&t| !g.types[t as usize].royal)
            .map(|t| Anchor { t, value: 3.0 })
            .collect(),
    }
}

fn material_for(g: &GameDef, name: &str) -> Vec<i32> {
    material_table(g, &fit_weights(g, &anchors_for(g, name)))
}

fn print_board(g: &GameDef, pos: &Position, hide_side: Option<u8>) {
    for y in (0..g.board.h).rev() {
        print!("{:2} ", y + 1);
        for x in 0..g.board.w {
            match pos.piece_at(g.board.sq(x, y)) {
                None => print!(" ."),
                Some(p) => {
                    let mut c = g.types[p.t as usize].glyph;
                    if hide_side == Some(p.side) && !g.types[p.t as usize].royal {
                        c = '?';
                    }
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
    println!();
    for s in 0..g.sides {
        let hand: Vec<String> = (0..g.types.len())
            .filter(|&t| pos.hands[s as usize][t] > 0)
            .map(|t| format!("{}x{}", g.types[t].glyph, pos.hands[s as usize][t]))
            .collect();
        if !hand.is_empty() {
            println!("hand[{s}]: {}", hand.join(" "));
        }
    }
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn eval_for(g: &GameDef, name: &str, args: &[String]) -> Eval {
    let material = material_for(g, name);
    let net_path: String = opt(args, "--net", String::new());
    if net_path.is_empty() {
        return Eval::new(material);
    }
    let data = std::fs::read(&net_path).expect("read net checkpoint");
    let fnet = botboard_core::nnue::FloatNet::from_bytes(&data).expect("parse checkpoint");
    let qnet = botboard_core::nnue::QuantNet::from_float(g, &fnet);
    println!("[deterministic-grade net loaded from {net_path}]");
    Eval::with_net(material, qnet)
}

fn cmd_play(g: &GameDef, name: &str, args: &[String]) {
    let depth: i32 = opt(args, "--depth", 4);
    let seed: u64 = opt(args, "--seed", 1);
    let hidden = flag(args, "--hidden");
    let mut searcher = Searcher::new(g, eval_for(g, name, args));
    let mut pos = Position::startpos(g);
    let mut history: Vec<u64> = Vec::new();
    let mut rng = Rng::new(seed);
    let cfg = LadderConfig { depth, ..Default::default() };
    // In hidden mode the engine plays on a cold-open belief about YOUR
    // pieces; your view hides ITS pieces (§8.6 observed-view discipline).
    let mut engine_belief = Belief::cold_open(g, &pos, 1);

    println!("You are side 0 (uppercase). Enter moves like e2e4, e7e8q, P@e5.");
    println!("Commands: 'moves' lists legal moves, 'quit' exits.\n");

    let stdin = io::stdin();
    loop {
        match status(g, &mut pos) {
            Status::Ongoing => {}
            Status::Win(s) => {
                print_board(g, &pos, None);
                println!("{} wins!", if s == 0 { "You" } else { "Engine" });
                return;
            }
            Status::Draw => {
                print_board(g, &pos, None);
                println!("Draw.");
                return;
            }
        }
        if pos.stm == 0 {
            print_board(g, &pos, if hidden { Some(1) } else { None });
            let moves = legal_moves(g, &mut pos);
            print!("> ");
            io::stdout().flush().ok();
            let Some(Ok(line)) = stdin.lock().lines().next() else { return };
            let line = line.trim().to_string();
            match line.as_str() {
                "quit" => return,
                "moves" => {
                    let strs: Vec<String> =
                        moves.iter().map(|m| move_str(g, m)).collect();
                    println!("{}", strs.join(" "));
                    continue;
                }
                _ => {}
            }
            let Some(mv) = moves.iter().find(|m| move_str(g, m) == line).copied() else {
                println!("illegal or unknown move: {line}");
                continue;
            };
            let pre = pos.clone();
            pos.make(g, &mv);
            engine_belief.observe(g, &pre, &mv);
        } else {
            let t0 = Instant::now();
            let mv = if hidden {
                choose_move(g, &mut pos, &engine_belief, &mut searcher, &cfg, &mut rng)
                    .map(|(m, rung)| {
                        println!("[engine rung: {rung:?}]");
                        m
                    })
            } else {
                searcher.history = history.clone();
                searcher.search(g, &mut pos, depth, 2_000_000).best
            };
            let Some(mv) = mv else {
                println!("Engine has no move.");
                return;
            };
            println!("engine plays {} ({:.1}s)", move_str(g, &mv), t0.elapsed().as_secs_f64());
            pos.make(g, &mv);
        }
        history.push(pos.hash);
        if history.iter().filter(|&&h| h == *history.last().unwrap()).count() >= 3 {
            print_board(g, &pos, None);
            println!("Draw by threefold repetition.");
            return;
        }
    }
}

fn cmd_selfplay(g: &GameDef, name: &str, args: &[String]) {
    let games: u32 = opt(args, "--games", 4);
    let depth: i32 = opt(args, "--depth", 3);
    let material = material_for(g, name);
    let (mut w0, mut w1, mut d) = (0, 0, 0);
    let t0 = Instant::now();
    for i in 0..games {
        let mut s0 = Searcher::new(g, Eval::new(material.clone()));
        let mut s1 = Searcher::new(g, Eval::new(material.clone()));
        let rec = play_game(
            g,
            &mut s0,
            &mut s1,
            (depth, depth),
            (100_000, 100_000),
            100 + i as u64,
            6,
            300,
            0,
        );
        match rec.outcome {
            Outcome::Win(0) => w0 += 1,
            Outcome::Win(_) => w1 += 1,
            Outcome::Draw => d += 1,
        }
        println!("game {}: {:?} in {} plies", i + 1, rec.outcome, rec.plies);
    }
    println!(
        "\n{games} games: side0 +{w0} side1 +{w1} ={d}  ({:.1}s)",
        t0.elapsed().as_secs_f64()
    );
}

fn cmd_cost(g: &GameDef, name: &str) {
    let anchors = anchors_for(g, name);
    let w = fit_weights(g, &anchors);
    println!("fitted weights: w_move={:.3} w_attack={:.3}\n", w.w_move, w.w_attack);
    println!("{:<18} {:>9} {:>10} {:>8}", "type", "avg_moved", "avg_attack", "C_prior");
    for t in 0..g.types.len() as u8 {
        let m = mobility_integral(g, t, 0);
        let c = cost_prior(g, t, &w);
        let royal = if g.types[t as usize].royal { " (royal)" } else { "" };
        println!(
            "{:<18} {:>9.2} {:>10.2} {:>8.2}{royal}",
            g.types[t as usize].name, m.avg_moved, m.avg_attacked, c
        );
    }
}

fn cmd_league(g: &GameDef, name: &str, args: &[String]) {
    let games: u32 = opt(args, "--games", 2);
    let material = material_for(g, name);
    let pop = default_population();
    println!("round-robin: {} members × {games} games/pair…", pop.len());
    let payoff = round_robin(g, &pop, &material, games, 2, 20_000, 7);
    let (mix, rating) = nash_averaging(&payoff, 5000);
    let json = profiles_json(&pop, &mix, &rating, &payoff);
    std::fs::write("league_profiles.json", &json).ok();
    println!("{json}");
    println!("(written to league_profiles.json)");
}

/// Promotion gate (scale-ladder rung 1): two checkpoints play a paired-
/// opening match — every random opening is played twice with colors
/// swapped, so the stronger evaluator, not the draw of openings, decides.
fn cmd_netmatch(g: &GameDef, name: &str, args: &[String]) {
    let pairs: u32 = opt(args, "--pairs", 10);
    let depth: i32 = opt(args, "--depth", 4);
    let nodes: u64 = opt(args, "--nodes", 20_000);
    let seed: u64 = opt(args, "--seed", 3);
    let a_path: String = opt(args, "--a", String::new());
    let b_path: String = opt(args, "--b", String::new());
    let load = |path: &str| -> Eval {
        let material = material_for(g, name);
        if path.is_empty() {
            return Eval::new(material);
        }
        let data = std::fs::read(path).expect("read net checkpoint");
        let fnet =
            botboard_core::nnue::FloatNet::from_bytes(&data).expect("parse checkpoint");
        Eval::with_net(material, botboard_core::nnue::QuantNet::from_float(g, &fnet))
    };
    let (mut wa, mut wb, mut dr) = (0u32, 0u32, 0u32);
    for pair in 0..pairs {
        // A short random opening, replayed for both color assignments.
        let mut rng = Rng::new(seed.wrapping_add(pair as u64 * 7919));
        let mut opening: Vec<botboard_core::moves::Move> = Vec::new();
        {
            let mut pos = Position::startpos(g);
            for _ in 0..4 {
                let ms = legal_moves(g, &mut pos);
                if ms.is_empty() {
                    break;
                }
                let mv = ms[(rng.next_u64() % ms.len() as u64) as usize];
                pos.make(g, &mv);
                opening.push(mv);
            }
        }
        for a_first in [true, false] {
            let mut sa = Searcher::new(g, load(&a_path));
            let mut sb = Searcher::new(g, load(&b_path));
            let mut pos = Position::startpos(g);
            for mv in &opening {
                pos.make(g, mv);
            }
            let mut plies = 0;
            let verdict = loop {
                match botboard_core::movegen::status(g, &mut pos) {
                    botboard_core::movegen::Status::Win(side) => break Some(side),
                    botboard_core::movegen::Status::Draw => break None,
                    botboard_core::movegen::Status::Ongoing => {}
                }
                if plies >= 200 {
                    break None;
                }
                let a_to_move = (pos.stm == 0) == a_first;
                let s = if a_to_move { &mut sa } else { &mut sb };
                let Some(mv) = s.search(g, &mut pos, depth, nodes).best else {
                    break None;
                };
                pos.make(g, &mv);
                plies += 1;
            };
            match verdict {
                Some(side) if (side == 0) == a_first => wa += 1,
                Some(_) => wb += 1,
                None => dr += 1,
            }
        }
        println!("pair {pair}: A {wa}  B {wb}  draws {dr}");
    }
    let total = wa + wb + dr;
    println!(
        "netmatch: A ({a_path}) {wa} — {dr} — {wb} B ({b_path})  score {:.1}%",
        100.0 * (wa as f64 + dr as f64 * 0.5) / total as f64
    );
}

fn cmd_armies(args: &[String]) {
    let budget: f64 = opt(args, "--budget", 20.0);
    let seed: u64 = opt(args, "--seed", 1);
    let mut rng = Rng::new(seed);
    let w = botboard_core::cost::CostWeights { w_move: 0.35, w_attack: 0.35 };
    let (g, gen) = random_army(budget, &mut rng, &w);
    println!("generated piece types (budget {budget}):");
    for p in &gen {
        println!("  [{}] cost {:.2}  {}", g.types[p.type_id as usize].glyph, p.cost, p.bits_desc);
    }
    let mut pos = Position::startpos(&g);
    print_board(&g, &pos, None);
    println!("legal moves from start: {}", legal_moves(&g, &mut pos).len());
}

/// Prototype 1 (spec §7.1/§12): the representation crossover, measured.
/// Runs perft per variant on both board classes — mailbox+piece-list vs the
/// wide-bitboard (u128) kernel path — and reports the speedup.
fn cmd_bench() {
    println!(
        "{:<10} {:>7} {:>12} {:>12} {:>12} {:>8}",
        "variant", "cells", "perft-nodes", "mailbox kn/s", "bitboard kn/s", "speedup"
    );
    for (name, depth) in [("chess", 5u32), ("xiangqi", 4), ("shogi", 4)] {
        let mut g = game_for(name);
        let mut rates = [0f64; 2];
        let mut nodes = 0;
        for (i, use_bb) in [(0usize, false), (1, true)] {
            g.use_bitboards = use_bb;
            let mut pos = Position::startpos(&g);
            let t0 = Instant::now();
            nodes = perft(&g, &mut pos, depth);
            rates[i] = nodes as f64 / t0.elapsed().as_secs_f64() / 1000.0;
        }
        println!(
            "{:<10} {:>7} {:>12} {:>12.0} {:>12.0} {:>7.2}x",
            name,
            g.board.ncells(),
            nodes,
            rates[0],
            rates[1],
            rates[1] / rates[0]
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: botboard <show|perft|divide|play|selfplay|cost|league|armies|bench> [variant] [args]"
        );
        std::process::exit(1);
    }
    let cmd = args[1].as_str();

    if cmd == "armies" {
        return cmd_armies(&args[2..]);
    }
    if cmd == "bench" {
        return cmd_bench();
    }
    if cmd == "train-net" {
        let variant = args.get(2).map(String::as_str).unwrap_or("chess");
        let g = game_for(variant);
        let rest: Vec<String> = args[3.min(args.len())..].to_vec();
        let games: u32 = opt(&rest, "--games", 24);
        let epochs: u32 = opt(&rest, "--epochs", 6);
        let out: String = opt(&rest, "--out", format!("{variant}_net.bin"));
        let material = material_for(&g, variant);
        let mut net = botboard_core::nnue::FloatNet::new(7);
        println!("training on {games} self-play games ({epochs} epochs)…");
        let rep = botboard_core::nnue::train_from_selfplay(
            &g, &mut net, &material, games, 2, epochs, 0.01, 11,
        );
        println!(
            "samples: {}  loss: {:.4} → {:.4}",
            rep.samples, rep.first_loss, rep.last_loss
        );
        std::fs::write(&out, net.to_bytes()).expect("write checkpoint");
        println!("checkpoint written to {out} (deterministic grade derives at load)");
        return;
    }

    let variant = args.get(2).map(String::as_str).unwrap_or("chess");
    let g = game_for(variant);
    let rest: Vec<String> = args[3.min(args.len())..].to_vec();

    match cmd {
        "show" => {
            let pos = Position::startpos(&g);
            print_board(&g, &pos, None);
        }
        "perft" | "divide" => {
            let depth: u32 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(1);
            let fen = rest
                .iter()
                .position(|a| a == "--fen")
                .and_then(|i| rest.get(i + 1).cloned());
            let mut pos = match fen {
                Some(f) => chess_from_fen(&g, &f).unwrap_or_else(|e| {
                    eprintln!("bad FEN: {e}");
                    std::process::exit(1);
                }),
                None => Position::startpos(&g),
            };
            if cmd == "perft" {
                for d in 1..=depth {
                    let t0 = Instant::now();
                    let n = perft(&g, &mut pos, d);
                    let dt = t0.elapsed().as_secs_f64();
                    println!(
                        "perft({d}) = {n}  [{dt:.2}s, {:.0} kn/s]",
                        n as f64 / dt / 1000.0
                    );
                }
            } else {
                let mut total = 0;
                for (m, n) in divide(&g, &mut pos, depth) {
                    println!("{m}: {n}");
                    total += n;
                }
                println!("total: {total}");
            }
        }
        "play" => cmd_play(&g, variant, &rest),
        "selfplay" => cmd_selfplay(&g, variant, &rest),
        "cost" => cmd_cost(&g, variant),
        "league" => cmd_league(&g, variant, &rest),
        "netmatch" => cmd_netmatch(&g, variant, &rest),
        _ => eprintln!("unknown command: {cmd}"),
    }
}
