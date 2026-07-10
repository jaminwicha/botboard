//! Headless per-ply profiling harness for the SRW battle surface.
//!
//! Drives the exact production path (`srw_create` → `srw_ai_move` →
//! `srw_apply`) over a setup JSON and reports where the time goes: per-ply
//! wall time, ladder rung, OOS walk-node counts, and branching mass. Wall
//! clocks live only in this harness — the engine stays clock-free.
//!
//! Usage: srw_profile <setup.json> [--max-plies N] [--csv]

use std::ffi::{CStr, CString};
use std::time::Instant;

use botboard::srw::{
    srw_ai_move, srw_apply, srw_create, srw_destroy, srw_end_reason, srw_entropy,
    srw_last_move_stats, srw_plies, srw_status, srw_stm,
};

const RUNG_NAMES: [&str; 4] = ["R0", "R1", "R2", "R3"];

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: srw_profile <setup.json> [--max-plies N] [--csv]");
        std::process::exit(2);
    };
    let mut max_plies = u32::MAX;
    let mut csv = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--max-plies" => {
                max_plies = args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--max-plies needs a number");
                    std::process::exit(2);
                })
            }
            "--csv" => csv = true,
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    let setup = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("read {path}: {e}");
        std::process::exit(2);
    });
    let setup_c = CString::new(setup).expect("setup JSON contains NUL");
    let b = srw_create(setup_c.as_ptr());
    if b.is_null() {
        eprintln!("srw_create rejected the setup");
        std::process::exit(2);
    }

    if csv {
        println!(
            "ply,stm,rung,micros,entropy,walk_nodes,moves_generated,avg_branching,\
             update_nodes,sample_nodes,depth0_leaves,terminal_nodes,table_nodes"
        );
    }

    let mut buf = [0i8; 64];
    let mut stats = [0u64; 9];
    let mut rung_count = [0u64; 4];
    let mut rung_micros = [0u128; 4];
    let mut played = 0u32;
    let total_start = Instant::now();

    while srw_status(b) == 0 && played < max_plies {
        let stm = srw_stm(b);
        let entropy = srw_entropy(b, stm);
        let t = Instant::now();
        let rung = srw_ai_move(b, buf.as_mut_ptr(), buf.len() as i32);
        let micros = t.elapsed().as_micros();
        if rung < 0 {
            eprintln!("srw_ai_move failed: {rung}");
            break;
        }
        let have_stats = srw_last_move_stats(b, stats.as_mut_ptr(), stats.len() as i32) > 0;
        if !have_stats {
            stats = [0; 9];
            stats[0] = rung as u64;
        }
        let mv = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
        let branching =
            if stats[2] > 0 { stats[7] as f64 / stats[2] as f64 } else { 0.0 };
        let ply = srw_plies(b);
        if csv {
            println!(
                "{ply},{stm},{rung},{micros},{entropy:.2},{},{},{branching:.2},{},{},{},{},{}",
                stats[2], stats[7], stats[5], stats[6], stats[4], stats[3], stats[8]
            );
        } else {
            println!(
                "ply {ply:>4} side {stm} {} {micros:>9}us  H={entropy:>6.2}  \
                 walk={:>8} b={branching:>5.2} tbl={:>6}  {mv}",
                RUNG_NAMES[rung as usize % 4],
                stats[2],
                stats[8]
            );
        }
        rung_count[rung as usize % 4] += 1;
        rung_micros[rung as usize % 4] += micros;

        let rc = srw_apply(b, buf.as_ptr());
        if rc != 0 {
            eprintln!("srw_apply failed: {rc}");
            break;
        }
        played += 1;
    }

    let status = srw_status(b);
    let reason = srw_end_reason(b);
    let plies = srw_plies(b);
    let total = total_start.elapsed();
    srw_destroy(b);

    eprintln!("---");
    eprintln!(
        "status {status} (0 ongoing, 1+side win, 9 draw)  end_reason {reason} \
         (1 elim, 2 rep, 3 ply-cap, 4 no-progress, 5 mate)  plies {plies}  \
         total {:.2}s",
        total.as_secs_f64()
    );
    for r in 0..4 {
        if rung_count[r] > 0 {
            eprintln!(
                "  {}: {:>4} moves  {:>10.1}ms total  {:>9.1}ms/move",
                RUNG_NAMES[r],
                rung_count[r],
                rung_micros[r] as f64 / 1000.0,
                rung_micros[r] as f64 / 1000.0 / rung_count[r] as f64
            );
        }
    }
}
