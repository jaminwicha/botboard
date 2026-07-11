//! The SRW battle surface (Robot Wars layer over the engine, SRW Spec §2):
//! composes one battle from a JSON setup — board, terrain, Bit-set piece
//! types, placements, 2–4 sides — and drives it with the engine's
//! belief-gated ladder. Information asymmetry (SRW §6, §10) enters as
//! per-side intel that collapses cold-open beliefs; competence enters as
//! per-side ladder tiers. The campaign layer (C#/Godot) never re-implements
//! rules: it reads state through this surface and applies engine-legal
//! moves only.
//!
//! Setup schema (all costs/derived data live engine-side):
//! ```json
//! {
//!   "seed": 1, "max_plies": 400,
//!   "board": {"w": 8, "h": 8},
//!   "sides": 2,
//!   "types": [
//!     {"name": "controller", "glyph": "C", "royal": true, "hp": 1,
//!      "moves": [{"geom": "leaper", "m": 0, "n": 1},
//!                 {"geom": "leaper", "m": 1, "n": 1, "mode": "both"}],
//!      "abilities": [{"kind": "wall", "range": 2}, {"kind": "pit", "range": 2}]},
//!     {"name": "scuttle-lance", "glyph": "S", "hp": 2, "overclock": false,
//!      "moves": [{"geom": "rider", "m": 0, "n": 1, "mode": "move"},
//!                 {"geom": "hopper", "m": 0, "n": 1}],
//!      "abilities": [{"kind": "laser", "range": 2, "retreat": true}]}
//!   ],
//!   "placements": [{"t": 0, "side": 0, "x": 4, "y": 0}],
//!   "terrain": [{"x": 3, "y": 4, "kind": "wall"}],
//!   "intel": [{"observer": 1, "known_types": [1], "reveal_all": false}],
//!   "tiers": [1, 1],
//!   "material": [0, 210]
//! }
//! ```

use std::ffi::{c_char, c_int, CStr};
use std::ptr;

use botboard_core::belief::Belief;
use botboard_core::bits::{AbilityBit, Mode, MoveBit, PathRule};
use botboard_core::cost::{cost_prior, material_table, CostWeights};
use botboard_core::eval::Eval;
use botboard_core::ffa::choose_ffa_move;
use botboard_core::game::{
    BoardDef, CaptureFate, GameDef, PassPolicy, PieceTypeDef, Policy, Side, StalematePolicy,
    TurnPolicy, TypeId,
};
use botboard_core::geometry::DirFilter;
use botboard_core::ladder::{choose_move_traced, LadderConfig, LadderTrace, Rung};
use botboard_core::movegen::{legal_moves, status, Status};
use botboard_core::moves::{move_str, Effect, MoveKind, NO_SQ};
use botboard_core::position::{mine_owner, Loc, Position, T_ACID, T_GRASS, T_ICE, T_MINE0, T_NONE, T_PIT, T_WALL};
use botboard_core::rng::Rng;
use botboard_core::search::Searcher;

use crate::json::{parse, Json};
use crate::write_str;

/// The fixed pricing weights shared with the engine CLI's army generator:
/// cross-battle comparable, deterministic, re-fit is a campaign concern.
const W: CostWeights = CostWeights { w_move: 0.35, w_attack: 0.35 };

pub struct SrwBattle {
    g: GameDef,
    pos: Position,
    searchers: Vec<Searcher>,
    beliefs: Vec<Belief>,
    cfgs: Vec<LadderConfig>,
    rng: Rng,
    history: Vec<u64>,
    alive: Vec<bool>,
    plies: u32,
    max_plies: u32,
    /// Encoded terminal status once decided out-of-band (royal death,
    /// FFA elimination, repetition, ply cap): 1+side win, 9 draw.
    over: Option<c_int>,
    /// Why the battle ended: 0 not ended, 1 elimination (royal death /
    /// stuck seat), 2 threefold repetition, 3 ply cap, 4 no-progress
    /// adjudication, 5 mate/stalemate.
    end_reason: c_int,
    /// Counters from the most recent `ai_move` ladder decision.
    last_trace: Option<LadderTrace>,
    /// Presence masks (SRW §8.6 / Appendix B): revealed[observer][piece].
    /// Stealth pieces start hidden from enemy observers; adjacency, taking
    /// or dealing damage, and full recon reveal them permanently.
    revealed: Vec<Vec<bool>>,
}

/// Competence tiers (SRW §6 dial 2): the ladder run at tuned strength.
fn tier_cfg(tier: i64) -> LadderConfig {
    match tier {
        0 => LadderConfig {
            depth: 2,
            node_budget: 5_000,
            determinizations: 2,
            oos_iterations: 64,
            oos_depth: 2,
            ..Default::default()
        },
        1 => LadderConfig {
            depth: 3,
            node_budget: 20_000,
            determinizations: 4,
            oos_iterations: 128,
            oos_depth: 3,
            ..Default::default()
        },
        2 => LadderConfig {
            depth: 4,
            node_budget: 80_000,
            determinizations: 8,
            oos_iterations: 256,
            oos_depth: 4,
            ..Default::default()
        },
        _ => LadderConfig {
            depth: 5,
            node_budget: 250_000,
            determinizations: 12,
            oos_iterations: 384,
            oos_depth: 4,
            ..Default::default()
        },
    }
}

fn parse_move_bit(j: &Json) -> Option<MoveBit> {
    let m = j.num("m", 0.0) as i8;
    let n = j.num("n", 1.0) as i8;
    let mut b = match j.str_or("geom", "leaper") {
        "rider" => MoveBit::rider(m, n),
        "hopper" => MoveBit::hopper(m, n),
        _ => MoveBit::leaper(m, n),
    };
    b = b.mode(match j.str_or("mode", "both") {
        "move" => Mode::Move,
        "capture" => Mode::Capture,
        _ => Mode::Both,
    });
    b = b.dirs(match j.str_or("dirs", "all") {
        "forward" => DirFilter::Forward,
        "backward" => DirFilter::Backward,
        "sideways" => DirFilter::Sideways,
        "vertical" => DirFilter::Vertical,
        _ => DirFilter::All,
    });
    b = b.path(match j.str_or("path", "none") {
        "midpoint" => PathRule::Midpoint,
        "leg" => PathRule::Leg,
        _ => PathRule::None,
    });
    Some(b)
}

fn parse_ability(j: &Json) -> Option<AbilityBit> {
    let range = j.num("range", 1.0) as u8;
    match j.str_or("kind", "") {
        "heal" => Some(AbilityBit::Heal { amount: j.num("amount", 1.0) as i16, range }),
        "wall" => Some(AbilityBit::CreateWall { range }),
        "pit" => Some(AbilityBit::DigPit { range }),
        "laser" => Some(AbilityBit::Laser {
            range,
            retreat: j.bool_or("retreat", true),
            pierce: j.bool_or("pierce", false),
        }),
        "resurrect" => Some(AbilityBit::Resurrect { range }),
        "hack" => Some(AbilityBit::Hack { range }),
        "mine" => Some(AbilityBit::MineLayer { range }),
        _ => None,
    }
}

fn parse_types(spec: &Json) -> Result<Vec<PieceTypeDef>, String> {
    let mut types = Vec::new();
    for tj in spec.get("types").and_then(|t| t.as_arr()).ok_or("missing types")? {
        // GameDef wants 'static names; battle handles are per-encounter,
        // so the leak is bounded by types-per-session.
        let name: &'static str =
            Box::leak(tj.str_or("name", "robot").to_string().into_boxed_str());
        let glyph = tj.str_or("glyph", "R").chars().next().unwrap_or('R');
        let bits: Vec<MoveBit> = tj
            .get("moves")
            .and_then(|m| m.as_arr())
            .map(|a| a.iter().filter_map(parse_move_bit).collect())
            .unwrap_or_default();
        let mut def = PieceTypeDef::new(name, glyph, bits);
        if tj.bool_or("royal", false) {
            def = def.royal();
        }
        let hp = tj.num("hp", 1.0) as i16;
        if hp > 1 {
            def = def.hp(hp);
        }
        if tj.bool_or("overclock", false) {
            def = def.overclock();
        }
        if tj.bool_or("hologram", false) {
            def = def.hologram();
        }
        if tj.bool_or("stealth", false) {
            def = def.stealth();
        }
        if tj.bool_or("flight", false) {
            def = def.flight();
        }
        let emp = tj.num("emp", 0.0) as u8;
        if emp > 0 {
            def = def.emp(emp);
        }
        let abilities: Vec<AbilityBit> = tj
            .get("abilities")
            .and_then(|a| a.as_arr())
            .map(|a| a.iter().filter_map(parse_ability).collect())
            .unwrap_or_default();
        if !abilities.is_empty() {
            def = def.abilities(abilities);
        }
        types.push(def);
    }
    if types.is_empty() {
        return Err("no types".into());
    }
    Ok(types)
}

fn build(spec_str: &str) -> Result<SrwBattle, String> {
    let spec = parse(spec_str)?;
    let board = BoardDef {
        w: spec.get("board").map_or(8.0, |b| b.num("w", 8.0)) as u8,
        h: spec.get("board").map_or(8.0, |b| b.num("h", 8.0)) as u8,
    };
    if board.ncells() == 0 || board.ncells() > 128 {
        return Err("board must be 1..=128 cells".into());
    }
    let sides = spec.num("sides", 2.0) as u8;
    if !(2..=4).contains(&sides) {
        return Err("sides must be 2..=4".into());
    }
    let types = parse_types(&spec)?;

    let mut start: Vec<(TypeId, Side, u8, u8)> = Vec::new();
    for pj in spec.get("placements").and_then(|p| p.as_arr()).ok_or("missing placements")? {
        let t = pj.num("t", 0.0) as usize;
        let side = pj.num("side", 0.0) as u8;
        if t >= types.len() || side >= sides {
            return Err("placement out of range".into());
        }
        start.push((t as TypeId, side, pj.num("x", 0.0) as u8, pj.num("y", 0.0) as u8));
    }

    let policy = Policy {
        stalemate: StalematePolicy::Loss, // a stuck robot army powers down
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };
    let g = GameDef::new(board, sides, types, Vec::new(), policy, start);

    let material = match spec.get("material").and_then(|m| m.as_arr()) {
        Some(arr) if arr.len() == g.types.len() => {
            arr.iter().map(|v| v.as_i64().unwrap_or(0) as i32).collect()
        }
        _ => material_table(&g, &W),
    };

    let mut pos = Position::startpos(&g);
    if let Some(cells) = spec.get("terrain").and_then(|t| t.as_arr()) {
        for cj in cells {
            let x = cj.num("x", 0.0) as u8;
            let y = cj.num("y", 0.0) as u8;
            if x >= g.board.w || y >= g.board.h {
                return Err("terrain out of range".into());
            }
            let sq = g.board.sq(x, y) as usize;
            let kind = match cj.str_or("kind", "wall") {
                "pit" => T_PIT,
                "ice" => T_ICE,
                "grass" => T_GRASS,
                "acid" => T_ACID,
                _ => T_WALL,
            };
            // Blocking terrain cannot share a cell with a piece; the
            // passable kinds (ice, grass, acid) are stood upon by design.
            if pos.board[sq] >= 0 && botboard_core::position::terrain_blocks(kind) {
                return Err("blocking terrain on occupied cell".into());
            }
            pos.terrain[sq] = kind;
        }
        pos.rehash(&g);
    }

    // Beliefs: cold open per side, then collapse per the intel dial
    // (SRW §10: codex/recon = belief collapse; the engine gear follows).
    let mut beliefs: Vec<Belief> =
        (0..sides).map(|s| Belief::cold_open(&g, &pos, s)).collect();
    if let Some(intel) = spec.get("intel").and_then(|i| i.as_arr()) {
        for ij in intel {
            let o = ij.num("observer", 0.0) as usize;
            if o >= sides as usize {
                continue;
            }
            let all = ij.bool_or("reveal_all", false);
            let known: Vec<TypeId> = ij
                .get("known_types")
                .and_then(|k| k.as_arr())
                .map(|a| a.iter().filter_map(|v| v.as_i64().map(|t| t as TypeId)).collect())
                .unwrap_or_default();
            for i in 0..pos.pieces.len() {
                if beliefs[o].candidates[i].len() > 1 {
                    let truth = pos.pieces[i].t;
                    if all || known.contains(&truth) {
                        beliefs[o].candidates[i] = vec![truth];
                    }
                }
            }
        }
    }

    let cfgs: Vec<LadderConfig> = (0..sides as usize)
        .map(|s| {
            let t = spec
                .get("tiers")
                .and_then(|a| a.as_arr())
                .and_then(|a| a.get(s))
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            tier_cfg(t)
        })
        .collect();

    let searchers =
        (0..sides).map(|_| Searcher::new(&g, Eval::new(material.clone()))).collect();

    // Presence masks: own pieces and non-stealth types are always seen;
    // full recon (reveal_all intel) pierces stealth too — the spy-drone
    // counter-play.
    let mut revealed: Vec<Vec<bool>> = (0..sides as usize)
        .map(|o| {
            pos.pieces
                .iter()
                .map(|p| {
                    p.side as usize == o || !g.types[p.t as usize].stealth
                })
                .collect()
        })
        .collect();
    if let Some(intel) = spec.get("intel").and_then(|i| i.as_arr()) {
        for ij in intel {
            let o = ij.num("observer", 0.0) as usize;
            if o < sides as usize && ij.bool_or("reveal_all", false) {
                revealed[o].iter_mut().for_each(|r| *r = true);
            }
        }
    }

    Ok(SrwBattle {
        pos,
        searchers,
        beliefs,
        cfgs,
        rng: Rng::new(spec.num("seed", 1.0) as u64),
        history: Vec::new(),
        alive: vec![true; sides as usize],
        plies: 0,
        max_plies: spec.num("max_plies", 400.0) as u32,
        over: None,
        end_reason: 0,
        last_trace: None,
        revealed,
        g,
    })
}

impl SrwBattle {
    /// The world side `s` gets to think in (§8.6 masks): ground truth
    /// minus unrevealed enemy stealth pieces and unseen enemy mines. The
    /// arbiter still adjudicates on truth — bumping into a cloaked robot
    /// resolves as the strike it really is.
    fn masked_world(&self, s: Side) -> Position {
        let mut world = self.pos.clone();
        for i in 0..world.pieces.len() {
            let p = world.pieces[i];
            if p.side != s && !self.visible_to(s as usize, i) {
                if let Loc::Board(sq) = p.loc {
                    world.board[sq as usize] = -1;
                    world.pieces[i].loc = Loc::Dead;
                }
            }
        }
        for sq in 0..world.terrain.len() {
            if let Some(owner) = mine_owner(world.terrain[sq]) {
                if owner != s {
                    world.terrain[sq] = T_NONE;
                }
            }
        }
        world.rehash(&self.g);
        world
    }

    /// Tall grass (Appendix B): live camouflage, not identity — a robot
    /// standing in the stalks is unseen by observers with nobody adjacent.
    /// Positional and memoryless: leave the grass and you are plain again,
    /// slip back in and you vanish again.
    fn concealed_by_grass(&self, observer: Side, i: usize) -> bool {
        let p = self.pos.pieces[i];
        let Loc::Board(sq) = p.loc else { return false };
        if p.side == observer || self.pos.terrain[sq as usize] != T_GRASS {
            return false;
        }
        let (x, y) = self.g.board.xy(sq);
        !self.pos.pieces.iter().any(|q| {
            let Loc::Board(qsq) = q.loc else { return false };
            if q.side != observer {
                return false;
            }
            let (qx, qy) = self.g.board.xy(qsq);
            (qx - x).abs().max((qy - y).abs()) <= 1
        })
    }

    /// One visibility truth for the FFI, the masked AI worlds, and the
    /// renderer: the permanent stealth mask AND the live grass check.
    fn visible_to(&self, observer: usize, i: usize) -> bool {
        self.revealed[observer][i]
            && !self.concealed_by_grass(observer as Side, i)
    }

    /// Update presence masks after a real move: adjacency spots a cloaked
    /// robot; taking or dealing damage blows its cover for everyone.
    fn reveal_pass(&mut self, pre: &Position) {
        // Damage in either direction reveals to all observers.
        for i in 0..self.pos.pieces.len() {
            let damaged = self.pos.hp[i] != pre.hp[i]
                || (matches!(pre.pieces[i].loc, Loc::Board(_))
                    && matches!(self.pos.pieces[i].loc, Loc::Dead));
            if damaged {
                for o in 0..self.revealed.len() {
                    self.revealed[o][i] = true;
                }
            }
        }
        // Chebyshev adjacency to any enemy piece reveals — permanently.
        for i in 0..self.pos.pieces.len() {
            let p = self.pos.pieces[i];
            let Loc::Board(sq) = p.loc else { continue };
            if !self.g.types[p.t as usize].stealth {
                continue;
            }
            let (x, y) = self.g.board.xy(sq);
            for q in self.pos.pieces.iter() {
                let Loc::Board(qsq) = q.loc else { continue };
                if q.side == p.side {
                    continue;
                }
                let (qx, qy) = self.g.board.xy(qsq);
                if (qx - x).abs().max((qy - y).abs()) <= 1 {
                    self.revealed[q.side as usize][i] = true;
                }
            }
        }
    }

    fn live_royal(&self, s: Side) -> bool {
        self.pos.pieces.iter().any(|p| {
            p.side == s
                && self.g.types[p.t as usize].royal
                && matches!(p.loc, Loc::Board(_))
        })
    }

    fn eliminate(&mut self, s: Side) {
        self.alive[s as usize] = false;
        for p in self.pos.pieces.iter_mut() {
            if p.side == s {
                if let Loc::Board(_) = p.loc {
                    p.loc = Loc::Dead;
                }
            }
        }
        for t in 0..self.g.types.len() {
            self.pos.hands[s as usize][t] = 0;
        }
        for sq in 0..self.pos.board.len() {
            self.pos.board[sq] = -1;
        }
        for i in 0..self.pos.pieces.len() {
            if let Loc::Board(sq) = self.pos.pieces[i].loc {
                self.pos.board[sq as usize] = i as i32;
            }
        }
        self.pos.rehash(&self.g);
    }

    /// Royal-death eliminations, dead-seat skipping, FFA stuck-player
    /// elimination, and end-of-game detection (the FFA baseline ruling +
    /// laser royal-snipes, which end a battle without checkmate).
    fn upkeep(&mut self) {
        if self.over.is_some() {
            return;
        }
        for s in 0..self.g.sides {
            if self.alive[s as usize] && !self.live_royal(s) {
                self.eliminate(s);
            }
        }
        if self.g.sides > 2 {
            let mut guard = 0;
            loop {
                guard += 1;
                if guard > 16 {
                    break;
                }
                let live = self.alive.iter().filter(|&&a| a).count();
                if live <= 1 {
                    break;
                }
                let stm = self.pos.stm;
                if !self.alive[stm as usize] {
                    let next = (stm + 1) % self.g.sides;
                    self.pos.set_stm(&self.g, next);
                    continue;
                }
                if legal_moves(&self.g, &mut self.pos).is_empty() {
                    self.eliminate(stm);
                    let next = (stm + 1) % self.g.sides;
                    self.pos.set_stm(&self.g, next);
                    continue;
                }
                break;
            }
        }
        let live = self.alive.iter().filter(|&&a| a).count();
        if live == 1 {
            let w = self.alive.iter().position(|&a| a).unwrap() as c_int;
            self.over = Some(1 + w);
            self.end_reason = 1;
        } else if live == 0 {
            self.over = Some(9);
            self.end_reason = 1;
        }
    }

    fn status_code(&mut self) -> c_int {
        self.upkeep();
        if let Some(code) = self.over {
            return code;
        }
        if self.g.sides == 2 {
            match status(&self.g, &mut self.pos) {
                Status::Ongoing => 0,
                Status::Win(s) => {
                    self.end_reason = 5;
                    1 + s as c_int
                }
                Status::Draw => {
                    self.end_reason = 5;
                    9
                }
            }
        } else {
            0
        }
    }

    fn apply(&mut self, s: &str) -> c_int {
        if self.status_code() != 0 {
            return -4;
        }
        let moves = legal_moves(&self.g, &mut self.pos);
        let Some(mv) = moves.iter().find(|m| move_str(&self.g, m) == s).copied() else {
            return -3;
        };
        let pre = self.pos.clone();
        self.pos.make(&self.g, &mv);
        self.plies += 1;
        // Every observer updates on the public action (§8.6 discipline).
        for b in self.beliefs.iter_mut() {
            b.observe(&self.g, &pre, &mv);
        }
        self.reveal_pass(&pre);
        self.history.push(self.pos.hash);
        if self.history.iter().filter(|&&h| h == self.pos.hash).count() >= 3 {
            self.over = Some(9);
            self.end_reason = 2;
        } else if self.plies >= self.max_plies {
            self.over = Some(9);
            self.end_reason = 3;
        }
        self.upkeep();
        0
    }

    /// Choose (but do not apply) a move for the side to move. Returns the
    /// rung used (0..=3) or negative on error. Two-player battles run the
    /// belief-gated ladder; N-player runs the committed FFA baseline.
    fn ai_move(&mut self, buf: *mut c_char, cap: c_int) -> c_int {
        if self.status_code() != 0 {
            return -4;
        }
        let stm = self.pos.stm as usize;
        if self.g.sides == 2 {
            // Think in the masked world (hidden mines/stealth absent) …
            let mut world = self.masked_world(stm as Side);
            let masked_choice = choose_move_traced(
                &self.g,
                &mut world,
                &self.beliefs[stm],
                &mut self.searchers[stm],
                &self.cfgs[stm],
                &mut self.rng,
            );
            // A masked stalemate (the only real moves touch hidden pieces)
            // falls back to an informed re-think, like the blocked-plan
            // case below — the arbiter knows moves exist.
            let Some((mut mv, trace)) = masked_choice.or_else(|| {
                choose_move_traced(
                    &self.g,
                    &mut self.pos,
                    &self.beliefs[stm],
                    &mut self.searchers[stm],
                    &self.cfgs[stm],
                    &mut self.rng,
                )
            }) else {
                return -2;
            };
            // … but the arbiter rules on truth. A plan a cloaked robot
            // blocks falls back to an informed re-think (the piece was
            // brushed; the reveal passes will catch it).
            if !legal_moves(&self.g, &mut self.pos).contains(&mv) {
                let Some((tmv, _)) = choose_move_traced(
                    &self.g,
                    &mut self.pos,
                    &self.beliefs[stm],
                    &mut self.searchers[stm],
                    &self.cfgs[stm],
                    &mut self.rng,
                ) else {
                    return -2;
                };
                mv = tmv;
            }
            let rung = trace.rung;
            self.last_trace = Some(trace);
            let r = write_str(&move_str(&self.g, &mv), buf, cap);
            if r < 0 {
                return r;
            }
            match rung {
                Rung::R0PerfectInfo => 0,
                Rung::R1Determinize => 1,
                Rung::R2Sound => 2,
                Rung::R3Policy => 3,
            }
        } else {
            let eval = self.searchers[stm].eval.clone();
            let mut world = self.masked_world(stm as Side);
            let Some(mut mv) = choose_ffa_move(&self.g, &mut world, &eval, &mut self.rng)
                .or_else(|| choose_ffa_move(&self.g, &mut self.pos, &eval, &mut self.rng))
            else {
                return -2;
            };
            if !legal_moves(&self.g, &mut self.pos).contains(&mv) {
                let Some(tmv) = choose_ffa_move(&self.g, &mut self.pos, &eval, &mut self.rng)
                else {
                    return -2;
                };
                mv = tmv;
            }
            let r = write_str(&move_str(&self.g, &mv), buf, cap);
            if r < 0 {
                return r;
            }
            0
        }
    }
}

// ---------------------------------------------------------------------------
// extern "C" surface
// ---------------------------------------------------------------------------

unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

/// Create a battle from setup JSON. NULL on parse/validation failure.
#[no_mangle]
pub extern "C" fn srw_create(setup_json: *const c_char) -> *mut SrwBattle {
    let Some(s) = (unsafe { cstr(setup_json) }) else { return ptr::null_mut() };
    match build(s) {
        Ok(b) => Box::into_raw(Box::new(b)),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn srw_destroy(b: *mut SrwBattle) {
    if !b.is_null() {
        drop(unsafe { Box::from_raw(b) });
    }
}

/// 0 ongoing, 1+side winner, 9 draw, negative error.
#[no_mangle]
pub extern "C" fn srw_status(b: *mut SrwBattle) -> c_int {
    let Some(b) = (unsafe { b.as_mut() }) else { return -1 };
    b.status_code()
}

#[no_mangle]
pub extern "C" fn srw_stm(b: *mut SrwBattle) -> c_int {
    unsafe { b.as_ref() }.map_or(-1, |b| b.pos.stm as c_int)
}

#[no_mangle]
pub extern "C" fn srw_alive(b: *mut SrwBattle, side: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    b.alive.get(side as usize).map_or(-1, |&a| a as c_int)
}

#[no_mangle]
pub extern "C" fn srw_plies(b: *mut SrwBattle) -> c_int {
    unsafe { b.as_ref() }.map_or(-1, |b| b.plies as c_int)
}

/// out[4] = [w, h, sides, ntypes].
#[no_mangle]
pub extern "C" fn srw_dims(b: *mut SrwBattle, out: *mut c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    if out.is_null() {
        return -2;
    }
    unsafe {
        *out = b.g.board.w as c_int;
        *out.add(1) = b.g.board.h as c_int;
        *out.add(2) = b.g.sides as c_int;
        *out.add(3) = b.g.types.len() as c_int;
    }
    0
}

#[no_mangle]
pub extern "C" fn srw_type_name(
    b: *mut SrwBattle,
    t: c_int,
    buf: *mut c_char,
    cap: c_int,
) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    let Some(ty) = b.g.types.get(t as usize) else { return -2 };
    write_str(ty.name, buf, cap)
}

/// Glyph char code, or negative.
#[no_mangle]
pub extern "C" fn srw_type_glyph(b: *mut SrwBattle, t: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    b.g.types.get(t as usize).map_or(-2, |ty| ty.glyph as c_int)
}

#[no_mangle]
pub extern "C" fn srw_piece_count(b: *mut SrwBattle) -> c_int {
    unsafe { b.as_ref() }.map_or(-1, |b| b.pos.pieces.len() as c_int)
}

/// out[6] = [sq(-1 off board), side, type, hp, loc(0 dead/1 board/2 hand), moved].
#[no_mangle]
pub extern "C" fn srw_piece_info(b: *mut SrwBattle, i: c_int, out: *mut c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    let Some(p) = b.pos.pieces.get(i as usize) else { return -2 };
    if out.is_null() {
        return -3;
    }
    let (sq, loc) = match p.loc {
        Loc::Board(sq) => (sq as c_int, 1),
        Loc::Hand(_) => (-1, 2),
        Loc::Dead => (-1, 0),
    };
    unsafe {
        *out = sq;
        *out.add(1) = p.side as c_int;
        *out.add(2) = p.t as c_int;
        *out.add(3) = b.pos.hp[i as usize] as c_int;
        *out.add(4) = loc;
        *out.add(5) = p.moved as c_int;
    }
    0
}

/// Is piece `i` identity-revealed to `observer`? (1 yes / 0 still hidden.)
/// Own pieces and royals are always public (§8.6).
#[no_mangle]
pub extern "C" fn srw_revealed(b: *mut SrwBattle, observer: c_int, i: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    let Some(belief) = b.beliefs.get(observer as usize) else { return -2 };
    belief.candidates.get(i as usize).map_or(-3, |c| (c.len() <= 1) as c_int)
}

/// Belief sharpness (§8.1) of `observer` — the UI's "how informed" signal
/// and the campaign's recon telemetry.
#[no_mangle]
pub extern "C" fn srw_entropy(b: *mut SrwBattle, observer: c_int) -> f64 {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1.0 };
    b.beliefs.get(observer as usize).map_or(-1.0, |bl| bl.entropy())
}

/// Ground-truth terrain: 0 none, 1 wall, 2 pit, 3 mine, 4 ice,
/// 5 tall grass, 6 acid.
#[no_mangle]
pub extern "C" fn srw_terrain(b: *mut SrwBattle, sq: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    b.pos.terrain.get(sq as usize).map_or(-2, |&t| match t {
        T_WALL => 1,
        T_PIT => 2,
        T_ICE => 4,
        T_GRASS => 5,
        T_ACID => 6,
        t if mine_owner(t).is_some() => 3,
        _ => 0,
    })
}

/// Terrain as `observer` sees it: enemy mines are invisible (0) until
/// they blow. 0 none, 1 wall, 2 pit, 3 own mine, 4 ice, 5 grass, 6 acid.
#[no_mangle]
pub extern "C" fn srw_terrain_for(b: *mut SrwBattle, observer: c_int, sq: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    b.pos.terrain.get(sq as usize).map_or(-2, |&t| match t {
        T_WALL => 1,
        T_PIT => 2,
        T_ICE => 4,
        T_GRASS => 5,
        T_ACID => 6,
        t if mine_owner(t) == Some(observer as Side) => 3,
        _ => 0,
    })
}

/// Is piece `i` present in `observer`'s view? Stealth pieces (Appendix B)
/// are hidden from enemies until adjacency, damage, or full recon reveals
/// them. 1 visible / 0 concealed.
#[no_mangle]
pub extern "C" fn srw_visible(b: *mut SrwBattle, observer: c_int, i: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    let Some(row) = b.revealed.get(observer as usize) else { return -2 };
    if row.get(i as usize).is_none() {
        return -3;
    }
    b.visible_to(observer as usize, i as usize) as c_int
}

#[no_mangle]
pub extern "C" fn srw_legal_count(b: *mut SrwBattle) -> c_int {
    let Some(b) = (unsafe { b.as_mut() }) else { return -1 };
    legal_moves(&b.g, &mut b.pos).len() as c_int
}

#[no_mangle]
pub extern "C" fn srw_legal_move(
    b: *mut SrwBattle,
    i: c_int,
    buf: *mut c_char,
    cap: c_int,
) -> c_int {
    let Some(b) = (unsafe { b.as_mut() }) else { return -1 };
    let moves = legal_moves(&b.g, &mut b.pos);
    let Some(mv) = moves.get(i as usize) else { return -2 };
    write_str(&move_str(&b.g, mv), buf, cap)
}

/// out[6] = [from(-1 drop), to, kind, aux(-1 none), effect, promo(-1 none)].
/// kind: 0 normal 1 double-step 2 en-passant 3 castle 4 drop 5 ability
/// 6 compound; effect: 0 none 1 heal 2 wall 3 pit 4 laser 5 twice
/// 6 resurrect (out[5] then carries the revived type id, not a promo)
/// 7 hack 8 mine-lay.
#[no_mangle]
pub extern "C" fn srw_legal_info(b: *mut SrwBattle, i: c_int, out: *mut c_int) -> c_int {
    let Some(b) = (unsafe { b.as_mut() }) else { return -1 };
    let moves = legal_moves(&b.g, &mut b.pos);
    let Some(mv) = moves.get(i as usize) else { return -2 };
    if out.is_null() {
        return -3;
    }
    let kind = match mv.kind {
        MoveKind::Normal => 0,
        MoveKind::DoubleStep => 1,
        MoveKind::EnPassant => 2,
        MoveKind::Castle => 3,
        MoveKind::Drop => 4,
        MoveKind::Ability => 5,
        MoveKind::Compound => 6,
    };
    let effect = match mv.effect {
        Effect::None => 0,
        Effect::Heal(_) => 1,
        Effect::Wall => 2,
        Effect::Pit => 3,
        Effect::Laser => 4,
        Effect::Twice => 5,
        Effect::Resurrect => 6,
        Effect::Hack => 7,
        Effect::Mine => 8,
    };
    unsafe {
        *out = if mv.from == NO_SQ { -1 } else { mv.from as c_int };
        *out.add(1) = mv.to as c_int;
        *out.add(2) = kind;
        *out.add(3) = if mv.aux == NO_SQ { -1 } else { mv.aux as c_int };
        *out.add(4) = effect;
        *out.add(5) = if mv.effect == Effect::Resurrect {
            mv.drop_type as c_int
        } else {
            mv.promo.map_or(-1, |t| t as c_int)
        };
    }
    0
}

/// Apply a move by string. 0 ok; -3 illegal; -4 game over.
#[no_mangle]
pub extern "C" fn srw_apply(b: *mut SrwBattle, mv: *const c_char) -> c_int {
    let Some(b) = (unsafe { b.as_mut() }) else { return -1 };
    let Some(s) = (unsafe { cstr(mv) }) else { return -2 };
    b.apply(s)
}

/// Choose a move for the side to move (does not apply it). Writes the move
/// string; returns the ladder rung used (0..=3) or negative error.
#[no_mangle]
pub extern "C" fn srw_ai_move(b: *mut SrwBattle, buf: *mut c_char, cap: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_mut() }) else { return -1 };
    b.ai_move(buf, cap)
}

/// Why the battle ended: 0 not ended, 1 elimination (royal death / stuck
/// seat), 2 threefold repetition, 3 ply cap, 4 no-progress adjudication,
/// 5 mate/stalemate. Call after `srw_status` reports non-zero.
#[no_mangle]
pub extern "C" fn srw_end_reason(b: *mut SrwBattle) -> c_int {
    unsafe { b.as_ref() }.map_or(-1, |b| b.end_reason)
}

/// Counters from the most recent `srw_ai_move` ladder decision, written as
/// u64s: [rung, oos_iterations, walk_nodes, terminal_nodes, depth0_leaves,
/// update_nodes, sample_nodes, moves_generated, table_nodes]. The OOS
/// fields are zero unless the decision dispatched to rung 2. Returns the
/// number of values written, or negative (-3 if no decision yet).
#[no_mangle]
pub extern "C" fn srw_last_move_stats(b: *mut SrwBattle, out: *mut u64, cap: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    if out.is_null() || cap < 9 {
        return -2;
    }
    let Some(trace) = b.last_trace else { return -3 };
    let s = trace.oos.unwrap_or_default();
    let vals: [u64; 9] = [
        trace.rung as u64,
        s.iterations,
        s.walk_nodes,
        s.terminal_nodes,
        s.depth0_leaves,
        s.update_nodes,
        s.sample_nodes,
        s.moves_generated,
        s.table_nodes,
    ];
    for (i, v) in vals.iter().enumerate() {
        unsafe { *out.add(i) = *v };
    }
    vals.len() as c_int
}

/// Price the setup's piece types with the engine cost model (SRW §8: the
/// forge and army budgets read this — stats are priced Bits). Writes one
/// cost per type into `out` (royals 0.0); returns the count or negative.
#[no_mangle]
pub extern "C" fn srw_price(setup_json: *const c_char, out: *mut f64, cap: c_int) -> c_int {
    let Some(s) = (unsafe { cstr(setup_json) }) else { return -1 };
    let Ok(spec) = parse(s) else { return -2 };
    let board = BoardDef {
        w: spec.get("board").map_or(8.0, |b| b.num("w", 8.0)) as u8,
        h: spec.get("board").map_or(8.0, |b| b.num("h", 8.0)) as u8,
    };
    if board.ncells() == 0 || board.ncells() > 128 {
        return -3;
    }
    let Ok(types) = parse_types(&spec) else { return -4 };
    let n = types.len();
    if out.is_null() || (n as c_int) > cap {
        return -5;
    }
    let policy = Policy {
        stalemate: StalematePolicy::Loss,
        capture_fate: CaptureFate::Destroy,
        turn: TurnPolicy::Alternate,
        pass: PassPolicy::Forbidden,
    };
    let g = GameDef::new(board, 2, types, Vec::new(), policy, Vec::new());
    for t in 0..n {
        let c = cost_prior(&g, t as TypeId, &W);
        unsafe { *out.add(t) = c };
    }
    n as c_int
}
