//! The C ABI boundary (§10.2): a flat `extern "C"` surface over the Rust
//! core, opaque-handle pattern, coarse commands. The same surface serves a
//! C# application layer (csbindgen/LibraryImport) and a Python training
//! harness — the core owns all rules, state, and RNG (§10.1).
//!
//! Conventions: functions return 0 on success, negative on error; move and
//! board strings are NUL-terminated UTF-8 in caller-supplied buffers.

use std::ffi::{c_char, c_int, CStr};
use std::ptr;

use botboard_core::cost::{fit_weights, material_table, Anchor};
use botboard_core::eval::Eval;
use botboard_core::game::GameDef;
use botboard_core::movegen::{legal_moves, status, Status};
use botboard_core::moves::move_str;
use botboard_core::perft::perft;
use botboard_core::position::Position;
use botboard_core::search::Searcher;
use botboard_core::variants;

pub struct Engine {
    g: GameDef,
    pos: Position,
    searcher: Searcher,
    history: Vec<u64>,
}

fn variant_material(g: &GameDef, variant: &str) -> Vec<i32> {
    // Anchored prior per variant; chess also gets its classical anchors.
    let anchors: Vec<Anchor> = match variant {
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
    };
    material_table(g, &fit_weights(g, &anchors))
}

/// Create an engine for a named variant ("chess" | "xiangqi" | "shogi").
/// Returns NULL on unknown names. The caller owns the handle and must free
/// it with `bb_destroy`.
#[no_mangle]
pub extern "C" fn bb_create(variant: *const c_char) -> *mut Engine {
    let name = unsafe {
        if variant.is_null() {
            return ptr::null_mut();
        }
        match CStr::from_ptr(variant).to_str() {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };
    let g = match name {
        "chess" => variants::chess::game(),
        "xiangqi" => variants::xiangqi::game(),
        "shogi" => variants::shogi::game(),
        _ => return ptr::null_mut(),
    };
    let material = variant_material(&g, name);
    let pos = Position::startpos(&g);
    let searcher = Searcher::new(&g, Eval::new(material));
    Box::into_raw(Box::new(Engine { g, pos, searcher, history: Vec::new() }))
}

#[no_mangle]
pub extern "C" fn bb_destroy(e: *mut Engine) {
    if !e.is_null() {
        drop(unsafe { Box::from_raw(e) });
    }
}

#[no_mangle]
pub extern "C" fn bb_reset(e: *mut Engine) -> c_int {
    let Some(e) = (unsafe { e.as_mut() }) else { return -1 };
    e.pos = Position::startpos(&e.g);
    e.history.clear();
    0
}

/// Number of legal moves in the current position.
#[no_mangle]
pub extern "C" fn bb_legal_count(e: *mut Engine) -> c_int {
    let Some(e) = (unsafe { e.as_mut() }) else { return -1 };
    legal_moves(&e.g, &mut e.pos).len() as c_int
}

/// Write the i-th legal move's string into `buf` (capacity `cap`).
#[no_mangle]
pub extern "C" fn bb_legal_move(e: *mut Engine, i: c_int, buf: *mut c_char, cap: c_int) -> c_int {
    let Some(e) = (unsafe { e.as_mut() }) else { return -1 };
    let moves = legal_moves(&e.g, &mut e.pos);
    let Some(mv) = moves.get(i as usize) else { return -2 };
    write_str(&move_str(&e.g, mv), buf, cap)
}

/// Apply a move given in move-string form ("e2e4", "P@e5", "e7e8q").
#[no_mangle]
pub extern "C" fn bb_apply(e: *mut Engine, mv: *const c_char) -> c_int {
    let Some(e) = (unsafe { e.as_mut() }) else { return -1 };
    let s = unsafe {
        if mv.is_null() {
            return -2;
        }
        match CStr::from_ptr(mv).to_str() {
            Ok(s) => s,
            Err(_) => return -2,
        }
    };
    let moves = legal_moves(&e.g, &mut e.pos);
    let Some(m) = moves.iter().find(|m| move_str(&e.g, m) == s) else { return -3 };
    e.pos.make(&e.g, m);
    e.history.push(e.searcher.zob.hash(&e.g, &e.pos));
    0
}

/// Search the current position; writes the best move string. `depth` plies,
/// `node_budget` ≤ 0 means unlimited. Deterministic-grade (§10.6): integer
/// eval, same inputs → same move on every machine.
#[no_mangle]
pub extern "C" fn bb_bestmove(
    e: *mut Engine,
    depth: c_int,
    node_budget: i64,
    buf: *mut c_char,
    cap: c_int,
) -> c_int {
    let Some(e) = (unsafe { e.as_mut() }) else { return -1 };
    let budget = if node_budget <= 0 { u64::MAX } else { node_budget as u64 };
    e.searcher.history = e.history.clone();
    let r = e.searcher.search(&e.g, &mut e.pos, depth.max(1), budget);
    match r.best {
        Some(mv) => write_str(&move_str(&e.g, &mv), buf, cap),
        None => -2,
    }
}

/// 0 = ongoing, 1 = side-0 win, 2 = side-1 win, 3 = draw.
#[no_mangle]
pub extern "C" fn bb_status(e: *mut Engine) -> c_int {
    let Some(e) = (unsafe { e.as_mut() }) else { return -1 };
    match status(&e.g, &mut e.pos) {
        Status::Ongoing => 0,
        Status::Win(s) => 1 + s as c_int,
        Status::Draw => 3,
    }
}

/// Side to move (0-based), or -1.
#[no_mangle]
pub extern "C" fn bb_side_to_move(e: *mut Engine) -> c_int {
    unsafe { e.as_ref() }.map_or(-1, |e| e.pos.stm as c_int)
}

/// Perft node count (validation entry point for bindings).
#[no_mangle]
pub extern "C" fn bb_perft(e: *mut Engine, depth: c_int) -> i64 {
    let Some(e) = (unsafe { e.as_mut() }) else { return -1 };
    perft(&e.g, &mut e.pos, depth.max(0) as u32) as i64
}

fn write_str(s: &str, buf: *mut c_char, cap: c_int) -> c_int {
    let bytes = s.as_bytes();
    if buf.is_null() || (bytes.len() + 1) as c_int > cap {
        return -10;
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
        *buf.add(bytes.len()) = 0;
    }
    bytes.len() as c_int
}
