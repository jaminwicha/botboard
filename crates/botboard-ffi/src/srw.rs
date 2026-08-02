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
//!   "material": [0, 210],
//!   "net": "artifacts/srw_net_v1.bin"
//! }
//! ```
//!
//! Additive vocabulary (batch 2): type flag `"drill": true` (walls and
//! destructible blocks neither block paths nor bar landings; pits still
//! do unless the type also flies); per-move optional `"min_hp"` /
//! `"max_hp"` HP gates (0/absent = unbounded — the §3.1 state condition);
//! ability kinds `"swap"` (exchange with a friendly in range) and
//! `"push"` (shove an enemy one square away); terrain kinds `"block1"` /
//! `"block2"` / `"block3"` (destructible block tiers — laser-crackable,
//! driller-passable) and `"conv_n"` / `"conv_e"` / `"conv_s"` /
//! `"conv_w"` (conveyor belts; N is +y, side 0's forward).
//!
//! Additive vocabulary (batch 3, the fairy-chess atoms): per-move
//! `"landing": "cannon" | "beyond" | "locust"` selects the hopper landing
//! mode (default `"cannon"` — the xiangqi behavior; `"beyond"` is the
//! grasshopper, defaulting the Bit's mode to both move and capture;
//! `"locust"` captures the screen itself, landing just past it — its
//! moves render as `"e2e5xe4"` and surface as kind 7 in
//! `srw_legal_info`); per-move `"max_steps": N` caps a rider at N steps
//! along each ray (0/absent = unlimited — the R4 short-rook atom).
//!
//! `"net"` (optional): path to a BBNET checkpoint (§10.6). The float
//! weights quantize against THIS battle's GameDef — descriptors are
//! Bit-derived (§7.4), so one checkpoint serves any composed army — and
//! every side's searcher evaluates with the deterministic-grade net.
//! A missing or invalid checkpoint is a build error, never a silent
//! fallback to the linear eval.
//!
//! Spy + codex (SRW §7, §8.8, §10–§11) — the recon arc end to end:
//! ability kind `"spy"` {range} is the active belief-collapse verb — a
//! board-null action (notation `"e2!spy:e3"`, effect code 11) that, on
//! apply, collapses the caster side's belief about the target to its
//! true identity and pierces its stealth for that side permanently.
//! `srw_codex(battle, observer, buf, cap)` exports that observer's
//! current belief as codex JSON; the optional top-level `"codex"` setup
//! array takes those exports back verbatim
//! (`"codex": [{"observer": 1, "candidates": [[0,2],[1],...]}]`) and
//! warm-starts the rematch: intersection with the cold-open belief,
//! per-piece fallback when the enemy changed a slot, applied before
//! `intel` so fresh certain intel wins over stale recon.
//!
//! Custom effects and terrains (Bits 2.0 Stage 4) — two OPTIONAL
//! top-level arrays make Axis-B content authorable per battle:
//!
//! ```json
//! "abilities": [
//!   {"id": "vamp-strike",
//!    "target": {"who": "enemy", "range": 2, "pred": ["nonroyal"]},
//!    "ops": [{"op": "hp_add", "n": -1}],
//!    "self_ops": [{"op": "hp_add", "n": 1, "cap": true}],
//!    "cost": {"flat": 3.0, "mult": 1.0},
//!    "descriptor_slot": "laser"}
//! ],
//! "terrains": [
//!   {"id": "lava", "code": "auto",
//!    "blocks": {"ground": false, "flight": false, "drill": false},
//!    "on_land": {"ops": [{"op": "hp_add", "n": -1}], "gate": "anyone",
//!                 "consumed": false},
//!    "carry": null, "conceal": null, "tiers": 0}
//! ]
//! ```
//!
//! **Custom abilities.** `target.who` is `"enemy" | "friendly" |
//! "empty"`; reach is either `"range": N` (chebyshev point, 1..=16) or
//! `"ray": {"max": N, "pierce": bool}` (queen-line beam, enemies only —
//! blocking terrain stops it, a friendly stops a non-piercing beam).
//! `pred` (piece targets): `"damaged"`, `"hp_eq1"`, `"nonroyal"`; empty
//! targets are always bare floor (`"bare"` is accepted, and implied).
//! `ops` (1..=8, applied to the target square in order): `hp_add`
//! {n ∈ ±1..=8, optional cap} — damage is LETHAL at 0 (strike-to-kill
//! through the capture fate), `capture_at` (§3.2 strike-or-kill, enemy
//! targets), `flip_side` (enemy targets), `set_terrain` {kind: an
//! authorable stdlib kind, `"mine"` for the caster-banded mine, or a
//! custom terrain id — empty targets}, `terrain_step` (erode a block).
//! `self_ops` (0..=8, against the caster): `hp_add` only. `cost`
//! (REQUIRED): flat 0..=100 joins the flat priors, mult 0.1..=10 joins
//! M_utility. `descriptor_slot` (REQUIRED): a stdlib kin NAME ("heal",
//! "laser", "wall", "pit", "mine", "resurrect", "hack" — or "swap" /
//! "push" / "none" for no dim), mapping the row into the frozen 21-dim
//! NNUE layout. Piece `"abilities"` entries may then reference the id:
//! `{"kind": "vamp-strike"}` (other keys ignored — the row IS the
//! definition). Notation is `"e2!vamp-strike:e3"`; `srw_legal_info`
//! effect codes for custom rows live in a fixed band above the stdlib
//! codes: custom row `i` surfaces as `EFFECT_CUSTOM_BASE (32) + i`,
//! stable within the battle (order = array order).
//! Custom MOVE scripts (new special-move generators) are OUT of scope:
//! gates/bindings compose engine-side only. Relocation-coupled custom
//! selector shapes (swap/push/retreat/resurrect analogues) are likewise
//! stdlib-only this stage.
//!
//! **Custom terrains.** ≤ 16 rows per battle; internal codes and wire
//! codes allocate upward from the stdlib bands (`srw_terrain` code =
//! `14 + i`, stable within the battle). `code` must be `"auto"` (or
//! absent). `blocks` gives the three permission-class flags; `on_land`
//! is `hp_add` literals (±1..=8; damage lethal at 0, healing capped at
//! type max) with `gate` `"anyone" | "enemy_of_owner"` and `consumed`;
//! `carry` is `null | {"slide": {"riders_only": bool}} | {"belt":
//! {"dx": d, "dy": d}}`; `conceal` is `null | "standing" |
//! "owner_secret"`; `tiers` is 0 or 1 (a tier-1 custom block erodes to
//! floor; multi-tier bands are stdlib-only). Rows with an
//! `"enemy_of_owner"` gate or `"owner_secret"` conceal REQUIRE
//! `"owner": side` — customs are owner-fixed, not code-banded. Custom
//! ids may then appear in `"terrain"` cells.
//!
//! **Validation is loud**: any unknown op/pred/selector, out-of-band
//! amount, id collision (with the stdlib or another custom), missing
//! cost hint / descriptor slot, over-cap terrain count, or empty op
//! list makes `srw_create` return NULL; `srw_last_error` then names the
//! fault precisely.

use std::ffi::{c_char, c_int, CStr};
use std::ptr;
use std::sync::Mutex;

use botboard_core::belief::Belief;
use botboard_core::bits::{AbilityBit, HopMode, Mode, MoveBit, PathRule};
use botboard_core::cost::{cost_prior, material_table, CostWeights};
use botboard_core::effects::{kin_slot, CostHint, CustomAbility, CustomReach, Pred, Who};
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
use botboard_core::ops::{HpAmt, MicroOp, SqRef, TerrainKind};
use botboard_core::position::{Loc, Position, T_NONE, T_WALL};
use botboard_core::rng::Rng;
use botboard_core::search::Searcher;
use botboard_core::terrain_defs::{
    self, Blocks, Carry, Conceal, CustomOnLand, CustomTerrain, LandGate, MAX_CUSTOM_TERRAIN,
};

use crate::json::{parse, Json};
use crate::write_str;

/// The last `srw_create` build failure, readable via [`srw_last_error`]
/// — validation is loud: NULL plus a precise fault string.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

/// The fixed pricing weights shared with the engine CLI's army generator:
/// cross-battle comparable, deterministic, re-fit is a campaign concern.
const W: CostWeights = CostWeights { w_move: 0.35, w_attack: 0.35 };

/// `srw_legal_info` effect-code band for Stage-4 custom rows: custom row
/// `i` surfaces as `EFFECT_CUSTOM_BASE + i` (stable within a battle;
/// the gap above the stdlib codes is headroom for stdlib growth).
pub const EFFECT_CUSTOM_BASE: c_int = 32;

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
    // Hopper landing mode (batch 3): "cannon" (default, xiangqi-compatible),
    // "beyond" (grasshopper), "locust" (capture the screen). Applied before
    // the mode key so an explicit "mode" still overrides the grasshopper's
    // widened default.
    b = match j.str_or("landing", "cannon") {
        "beyond" => b.landing(HopMode::BeyondScreen),
        "locust" => b.landing(HopMode::Locust),
        _ => b,
    };
    let default_mode = match b.mode {
        Mode::Move => "move",
        Mode::Capture => "capture",
        Mode::Both => "both",
    };
    b = b.mode(match j.str_or("mode", default_mode) {
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
    // Optional HP gates (§3.1 state condition), additive schema: absent
    // or 0 = unbounded on that end.
    let min_hp = j.num("min_hp", 0.0) as i16;
    if min_hp > 0 {
        b = b.min_hp(min_hp);
    }
    let max_hp = j.num("max_hp", 0.0) as i16;
    if max_hp > 0 {
        b = b.max_hp(max_hp);
    }
    // Range-limited rider (batch 3): 0/absent = unlimited.
    let max_steps = j.num("max_steps", 0.0) as u8;
    if max_steps > 0 {
        b = b.max_steps(max_steps);
    }
    Some(b)
}

/// Kind lookup for a piece "abilities" entry: the stdlib vocabulary, or
/// a Stage-4 custom row by id (the row IS the definition — per-piece
/// parameters on a custom reference are ignored). Unknown kinds keep
/// the historical silent-drop acceptance.
fn parse_ability(j: &Json, customs: &[CustomAbility]) -> Option<AbilityBit> {
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
        "swap" => Some(AbilityBit::Swap { range }),
        "push" => Some(AbilityBit::Push { range }),
        "spy" => Some(AbilityBit::Spy { range }),
        other => customs
            .iter()
            .position(|c| c.id == other)
            .map(|i| AbilityBit::Custom(i as u16)),
    }
}

// ---------------------------------------------------------------------------
// Stage-4 custom-content parsing + validation (loud: every fault is a
// precise Err that srw_create surfaces through srw_last_error).
// ---------------------------------------------------------------------------

/// Custom ids: 1..=32 chars of [a-z0-9_-] — safe in the move notation
/// ("e2!<id>:e3") and the terrain cell vocabulary.
fn valid_custom_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn parse_custom_terrains(spec: &Json, sides: u8) -> Result<Vec<CustomTerrain>, String> {
    let Some(arr) = spec.get("terrains").and_then(|a| a.as_arr()) else {
        return Ok(Vec::new());
    };
    if arr.len() > MAX_CUSTOM_TERRAIN {
        return Err(format!(
            "terrains: at most {MAX_CUSTOM_TERRAIN} custom rows per battle, got {}",
            arr.len()
        ));
    }
    let mut out: Vec<CustomTerrain> = Vec::new();
    for tj in arr {
        let id = tj.str_or("id", "").to_string();
        if !valid_custom_id(&id) {
            return Err(format!("terrains: bad id {id:?} (want 1..=32 chars of [a-z0-9_-])"));
        }
        if terrain_defs::ROWS.iter().any(|r| r.id == id) {
            return Err(format!("terrains: id {id:?} collides with a stdlib terrain"));
        }
        if out.iter().any(|c| c.id == id) {
            return Err(format!("terrains: duplicate id {id:?}"));
        }
        match tj.get("code") {
            None => {}
            Some(v) if v.as_str() == Some("auto") => {}
            Some(_) => {
                return Err(format!(
                    "terrains[{id}]: code must be \"auto\" — internal and wire codes \
                     allocate upward from the stdlib bands"
                ))
            }
        }
        let blocks = match tj.get("blocks") {
            None => Blocks { ground: false, flight: false, drill: false },
            Some(b) => Blocks {
                ground: b.bool_or("ground", false),
                flight: b.bool_or("flight", false),
                drill: b.bool_or("drill", false),
            },
        };
        let owner = match tj.get("owner") {
            None | Some(Json::Null) => None,
            Some(v) => {
                let s = v
                    .as_i64()
                    .ok_or(format!("terrains[{id}]: owner must be a side number"))?;
                if s < 0 || s >= sides as i64 {
                    return Err(format!(
                        "terrains[{id}]: owner {s} out of range for {sides} sides"
                    ));
                }
                Some(s as Side)
            }
        };
        let on_land = match tj.get("on_land") {
            None | Some(Json::Null) => None,
            Some(oj) => {
                let ops_j = oj
                    .get("ops")
                    .and_then(|o| o.as_arr())
                    .ok_or(format!("terrains[{id}]: on_land needs an ops array"))?;
                if ops_j.is_empty() || ops_j.len() > 8 {
                    return Err(format!(
                        "terrains[{id}]: on_land op list must number 1..=8, got {}",
                        ops_j.len()
                    ));
                }
                let mut ops = Vec::new();
                for opj in ops_j {
                    let name = opj.str_or("op", "");
                    if name != "hp_add" {
                        return Err(format!(
                            "terrains[{id}]: unknown on_land op {name:?} (vocabulary: hp_add)"
                        ));
                    }
                    let n = opj.num("n", 0.0) as i16;
                    if n == 0 || n.abs() > 8 {
                        return Err(format!(
                            "terrains[{id}]: on_land hp_add n must be ±1..=8, got {n}"
                        ));
                    }
                    ops.push(MicroOp::HpAdd { n: HpAmt::Lit(n), cap: n > 0 });
                }
                let gate = match oj.str_or("gate", "anyone") {
                    "anyone" => LandGate::Anyone,
                    "enemy_of_owner" => {
                        if owner.is_none() {
                            return Err(format!(
                                "terrains[{id}]: gate enemy_of_owner requires \"owner\""
                            ));
                        }
                        LandGate::EnemyOfOwner
                    }
                    other => {
                        return Err(format!(
                            "terrains[{id}]: unknown gate {other:?} (anyone|enemy_of_owner)"
                        ))
                    }
                };
                Some(CustomOnLand { ops, gate, consumed: oj.bool_or("consumed", false) })
            }
        };
        let carry = match tj.get("carry") {
            None | Some(Json::Null) => Carry::None,
            Some(cj) => {
                if let Some(sj) = cj.get("slide") {
                    Carry::Slide { riders_only: sj.bool_or("riders_only", true) }
                } else if let Some(bj) = cj.get("belt") {
                    let (dx, dy) = (bj.num("dx", 0.0) as i8, bj.num("dy", 0.0) as i8);
                    if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dy) || (dx == 0 && dy == 0)
                    {
                        return Err(format!(
                            "terrains[{id}]: belt direction must be a unit step, got ({dx},{dy})"
                        ));
                    }
                    Carry::Belt { dx, dy }
                } else {
                    return Err(format!(
                        "terrains[{id}]: carry must be null, {{\"slide\":..}} or {{\"belt\":..}}"
                    ));
                }
            }
        };
        let conceal = match tj.get("conceal") {
            None | Some(Json::Null) => Conceal::None,
            Some(v) => match v.as_str() {
                Some("standing") => Conceal::Standing,
                Some("owner_secret") => {
                    if owner.is_none() {
                        return Err(format!(
                            "terrains[{id}]: conceal owner_secret requires \"owner\""
                        ));
                    }
                    Conceal::OwnerSecret
                }
                _ => {
                    return Err(format!(
                        "terrains[{id}]: unknown conceal (null|standing|owner_secret)"
                    ))
                }
            },
        };
        let tiers = tj.num("tiers", 0.0) as i64;
        if !(0..=1).contains(&tiers) {
            return Err(format!(
                "terrains[{id}]: custom tiers must be 0 or 1 (multi-tier bands are stdlib-only)"
            ));
        }
        out.push(CustomTerrain { id, blocks, on_land, carry, conceal, owner, tiers: tiers as u8 });
    }
    Ok(out)
}

/// One custom-ability op. `self_op` restricts to the caster vocabulary;
/// piece-vs-empty targeting compatibility is validated here so the
/// interpreter never sees an op its target cannot satisfy.
fn parse_custom_op(
    id: &str,
    opj: &Json,
    who: Who,
    terrains: &[CustomTerrain],
    self_op: bool,
) -> Result<MicroOp, String> {
    let name = opj.str_or("op", "");
    if self_op && name != "hp_add" {
        return Err(format!(
            "abilities[{id}]: self_ops vocabulary is hp_add only, got {name:?}"
        ));
    }
    let piece_target = matches!(who, Who::Enemy | Who::Friendly);
    match name {
        "hp_add" => {
            if !self_op && !piece_target {
                return Err(format!(
                    "abilities[{id}]: hp_add needs a piece target (who enemy|friendly)"
                ));
            }
            let n = opj.num("n", 0.0) as i16;
            if n == 0 || n.abs() > 8 {
                return Err(format!("abilities[{id}]: hp_add n must be ±1..=8, got {n}"));
            }
            // Heals cap at type max by default; "cap" overrides.
            Ok(MicroOp::HpAdd { n: HpAmt::Lit(n), cap: opj.bool_or("cap", n > 0) })
        }
        "capture_at" => {
            if who != Who::Enemy {
                return Err(format!("abilities[{id}]: capture_at needs who \"enemy\""));
            }
            Ok(MicroOp::CaptureAt { at: SqRef::To })
        }
        "flip_side" => {
            if who != Who::Enemy {
                return Err(format!("abilities[{id}]: flip_side needs who \"enemy\""));
            }
            Ok(MicroOp::FlipSide)
        }
        "terrain_step" => Ok(MicroOp::TerrainStep),
        "set_terrain" => {
            if who != Who::EmptyCell {
                return Err(format!("abilities[{id}]: set_terrain needs who \"empty\""));
            }
            let kind = opj.str_or("kind", "");
            let tk = match kind {
                "mine" => TerrainKind::OwnerMine,
                other => match terrain_defs::by_id(other) {
                    Some(code) => TerrainKind::Code(code),
                    None => match terrains.iter().position(|c| c.id == other) {
                        Some(i) => TerrainKind::Code((terrain_defs::NT + i) as u8),
                        None => {
                            return Err(format!(
                                "abilities[{id}]: set_terrain kind {other:?} names no \
                                 authorable terrain"
                            ))
                        }
                    },
                },
            };
            Ok(MicroOp::SetTerrain { kind: tk })
        }
        other => Err(format!(
            "abilities[{id}]: unknown op {other:?} (vocabulary: hp_add, capture_at, \
             flip_side, set_terrain, terrain_step)"
        )),
    }
}

fn parse_custom_abilities(
    spec: &Json,
    terrains: &[CustomTerrain],
) -> Result<Vec<CustomAbility>, String> {
    let Some(arr) = spec.get("abilities").and_then(|a| a.as_arr()) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<CustomAbility> = Vec::new();
    for aj in arr {
        let id = aj.str_or("id", "").to_string();
        if !valid_custom_id(&id) {
            return Err(format!("abilities: bad id {id:?} (want 1..=32 chars of [a-z0-9_-])"));
        }
        if botboard_core::effects::STDLIB_ABILITY_IDS.contains(&id.as_str()) {
            return Err(format!("abilities: id {id:?} collides with a stdlib ability"));
        }
        if out.iter().any(|c| c.id == id) {
            return Err(format!("abilities: duplicate id {id:?}"));
        }
        let tj = aj
            .get("target")
            .ok_or(format!("abilities[{id}]: missing target selector"))?;
        let who = match tj.str_or("who", "") {
            "enemy" => Who::Enemy,
            "friendly" => Who::Friendly,
            "empty" => Who::EmptyCell,
            other => {
                return Err(format!(
                    "abilities[{id}]: unknown target.who {other:?} (enemy|friendly|empty)"
                ))
            }
        };
        let reach = if let Some(rj) = tj.get("ray") {
            if who != Who::Enemy {
                return Err(format!("abilities[{id}]: ray reach requires who \"enemy\""));
            }
            let max = rj.num("max", 0.0) as i64;
            if !(1..=16).contains(&max) {
                return Err(format!("abilities[{id}]: ray.max must be 1..=16, got {max}"));
            }
            CustomReach::Ray { max: max as u8, pierce: rj.bool_or("pierce", false) }
        } else {
            let range = tj.num("range", 0.0) as i64;
            if !(1..=16).contains(&range) {
                return Err(format!(
                    "abilities[{id}]: target.range must be 1..=16 (or give a \"ray\"), got {range}"
                ));
            }
            CustomReach::Point { range: range as u8 }
        };
        let mut preds = Vec::new();
        if let Some(parr) = tj.get("pred").and_then(|p| p.as_arr()) {
            for pj in parr {
                let ps = pj.as_str().unwrap_or("");
                match ps {
                    "damaged" | "hp_eq1" | "nonroyal" => {
                        if who == Who::EmptyCell {
                            return Err(format!(
                                "abilities[{id}]: pred {ps:?} needs a piece target"
                            ));
                        }
                        preds.push(match ps {
                            "damaged" => Pred::Damaged,
                            "hp_eq1" => Pred::HpEq1,
                            _ => Pred::NonRoyal,
                        });
                    }
                    // Bare floor is the empty-target rule anyway; accept
                    // the name as documentation.
                    "bare" => {
                        if who != Who::EmptyCell {
                            return Err(format!(
                                "abilities[{id}]: pred \"bare\" is for empty targets"
                            ));
                        }
                    }
                    other => {
                        return Err(format!(
                            "abilities[{id}]: unknown pred {other:?} \
                             (damaged|hp_eq1|nonroyal|bare)"
                        ))
                    }
                }
            }
        }
        let ops_j = aj
            .get("ops")
            .and_then(|o| o.as_arr())
            .ok_or(format!("abilities[{id}]: missing ops array"))?;
        if ops_j.is_empty() {
            return Err(format!("abilities[{id}]: a custom ability must include at least one op"));
        }
        if ops_j.len() > 8 {
            return Err(format!(
                "abilities[{id}]: op list length must be ≤ 8, got {}",
                ops_j.len()
            ));
        }
        let ops = ops_j
            .iter()
            .map(|o| parse_custom_op(&id, o, who, terrains, false))
            .collect::<Result<Vec<_>, _>>()?;
        let self_j = aj.get("self_ops").and_then(|o| o.as_arr()).unwrap_or(&[]);
        if self_j.len() > 8 {
            return Err(format!(
                "abilities[{id}]: self_ops length must be ≤ 8, got {}",
                self_j.len()
            ));
        }
        let self_ops = self_j
            .iter()
            .map(|o| parse_custom_op(&id, o, who, terrains, true))
            .collect::<Result<Vec<_>, _>>()?;
        let cj = aj
            .get("cost")
            .ok_or(format!("abilities[{id}]: cost hint required ({{\"flat\", \"mult\"}})"))?;
        let flat = cj
            .get("flat")
            .and_then(|v| v.as_f64())
            .ok_or(format!("abilities[{id}]: cost.flat required"))?;
        let mult = cj
            .get("mult")
            .and_then(|v| v.as_f64())
            .ok_or(format!("abilities[{id}]: cost.mult required"))?;
        if !flat.is_finite() || !(0.0..=100.0).contains(&flat) {
            return Err(format!("abilities[{id}]: cost.flat must be 0..=100, got {flat}"));
        }
        if !mult.is_finite() || !(0.1..=10.0).contains(&mult) {
            return Err(format!("abilities[{id}]: cost.mult must be 0.1..=10, got {mult}"));
        }
        let slot_name = aj
            .get("descriptor_slot")
            .and_then(|v| v.as_str())
            .ok_or(format!("abilities[{id}]: descriptor_slot required (a stdlib kin name)"))?;
        let slot = kin_slot(slot_name).ok_or(format!(
            "abilities[{id}]: descriptor_slot {slot_name:?} is not a stdlib kin"
        ))?;
        out.push(CustomAbility {
            id,
            who,
            reach,
            preds,
            ops,
            self_ops,
            cost: CostHint { flat, mult },
            descriptor_slot: slot,
        });
    }
    Ok(out)
}

fn parse_types(spec: &Json, customs: &[CustomAbility]) -> Result<Vec<PieceTypeDef>, String> {
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
        if tj.bool_or("drill", false) {
            def = def.drill();
        }
        let emp = tj.num("emp", 0.0) as u8;
        if emp > 0 {
            def = def.emp(emp);
        }
        let abilities: Vec<AbilityBit> = tj
            .get("abilities")
            .and_then(|a| a.as_arr())
            .map(|a| a.iter().filter_map(|j| parse_ability(j, customs)).collect())
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
    // Stage-4 custom content: terrains first (ability set_terrain ops may
    // reference custom terrain ids), then abilities, then the types that
    // reference them.
    let custom_terrains = parse_custom_terrains(&spec, sides)?;
    let custom_effects = parse_custom_abilities(&spec, &custom_terrains)?;
    let types = parse_types(&spec, &custom_effects)?;

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
    let g = GameDef::with_customs(
        board,
        sides,
        types,
        Vec::new(),
        policy,
        start,
        custom_effects,
        custom_terrains,
    );

    let material = match spec.get("material").and_then(|m| m.as_arr()) {
        Some(arr) if arr.len() == g.types.len() => {
            arr.iter().map(|v| v.as_i64().unwrap_or(0) as i32).collect()
        }
        _ => material_table(&g, &W),
    };

    // Optional deterministic-grade net (§10.6): quantized against this
    // battle's GameDef so the Bit-derived descriptors (§7.4) match the
    // composed types. Bad path or bad bytes fail the build — no silent
    // fallback.
    let qnet = match spec.get("net") {
        None => None,
        Some(v) => {
            let path = v.as_str().ok_or("net must be a checkpoint path string")?;
            let data = std::fs::read(path)
                .map_err(|e| format!("net checkpoint {path}: {e}"))?;
            let fnet = botboard_core::nnue::FloatNet::from_bytes(&data)
                .map_err(|e| format!("net checkpoint {path}: {e}"))?;
            Some(botboard_core::nnue::QuantNet::from_float(&g, &fnet))
        }
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
            // Registry-id lookup (Bits 2.0 Stage 3/4): the row ids ARE
            // the setup-JSON terrain kinds — stdlib authorable rows plus
            // this battle's custom rows. Owner-band rows (mines) and the
            // absence row are not authorable; unknown kinds keep the
            // historical wall default.
            let kind = g.terrain_by_id(cj.str_or("kind", "wall")).unwrap_or(T_WALL);
            // Blocking terrain cannot share a cell with a piece; the
            // passable kinds (ice, grass, acid) are stood upon by design.
            if pos.board[sq] >= 0 && g.terrain_blocks(kind) {
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
    // Persistent codex (§8.8, §10–§11): each entry warm-starts an
    // observer's belief from an earlier battle's `srw_codex` export —
    // intersected with the fresh cold-open set; empty intersections fall
    // back per piece (the enemy may have changed that slot). Applied
    // BEFORE intel, so certain fresh intel wins over stale recon.
    if let Some(codex) = spec.get("codex").and_then(|c| c.as_arr()) {
        for cj in codex {
            let Some(o) = cj.get("observer").and_then(|v| v.as_i64()) else {
                return Err("codex: each entry needs an \"observer\" side number".into());
            };
            if o < 0 || o >= sides as i64 {
                return Err(format!("codex: observer {o} out of range for {sides} sides"));
            }
            let Some(cands) = cj.get("candidates").and_then(|c| c.as_arr()) else {
                return Err(format!(
                    "codex[observer {o}]: needs a \"candidates\" array of type-id arrays \
                     (the srw_codex export shape)"
                ));
            };
            let candidates: Vec<Vec<TypeId>> = cands
                .iter()
                .map(|row| {
                    row.as_arr()
                        .map(|r| {
                            r.iter().filter_map(|v| v.as_i64().map(|t| t as TypeId)).collect()
                        })
                        .ok_or(format!(
                            "codex[observer {o}]: candidates rows must be arrays of type ids"
                        ))
                })
                .collect::<Result<_, _>>()?;
            let prior = Belief { observer: o as Side, candidates };
            beliefs[o as usize] =
                botboard_core::codex::warm_start(&g, &pos, o as Side, &prior);
        }
    }
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

    let searchers = (0..sides)
        .map(|_| {
            Searcher::new(
                &g,
                match &qnet {
                    Some(q) => Eval::with_net(material.clone(), q.clone()),
                    None => Eval::new(material.clone()),
                },
            )
        })
        .collect();

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
            // Owner-secret terrain (registry conceal row, stdlib band or
            // a custom row's fixed authored owner): invisible to everyone
            // but its owner until it triggers.
            let t = world.terrain[sq];
            if self.g.terrain_conceal(t) == Conceal::OwnerSecret
                && self.g.terrain_owner(t) != Some(s)
            {
                world.terrain[sq] = T_NONE;
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
        let standing_conceal =
            self.g.terrain_conceal(self.pos.terrain[sq as usize]) == Conceal::Standing;
        if p.side == observer || !standing_conceal {
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
        // Spy (§7, §10): the active belief-collapse verb. The move is
        // board-null by construction; the information effect lands here —
        // the caster's side learns the target's identity and pierces its
        // stealth permanently (identity IS the cover blown).
        if mv.effect == Effect::Spy {
            let actor = pre.stm as usize;
            let ti = pre.board[mv.to as usize];
            if ti >= 0 {
                let ti = ti as usize;
                self.beliefs[actor].candidates[ti] = vec![self.pos.pieces[ti].t];
                self.revealed[actor][ti] = true;
            }
        }
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

/// Create a battle from setup JSON. NULL on parse/validation failure —
/// `srw_last_error` then names the fault precisely.
#[no_mangle]
pub extern "C" fn srw_create(setup_json: *const c_char) -> *mut SrwBattle {
    let Some(s) = (unsafe { cstr(setup_json) }) else {
        if let Ok(mut e) = LAST_ERROR.lock() {
            *e = "setup_json is null or not UTF-8".into();
        }
        return ptr::null_mut();
    };
    match build(s) {
        Ok(b) => {
            if let Ok(mut e) = LAST_ERROR.lock() {
                e.clear();
            }
            Box::into_raw(Box::new(b))
        }
        Err(msg) => {
            if let Ok(mut e) = LAST_ERROR.lock() {
                *e = msg;
            }
            ptr::null_mut()
        }
    }
}

/// Copy the reason the most recent `srw_create` returned NULL into
/// `buf` (NUL-terminated UTF-8). Returns the string length (0 when the
/// last create succeeded / nothing recorded), or negative on a buffer
/// problem. The buffer is process-wide state: read it on the same
/// thread flow that observed the NULL.
#[no_mangle]
pub extern "C" fn srw_last_error(buf: *mut c_char, cap: c_int) -> c_int {
    let s = LAST_ERROR.lock().map(|e| e.clone()).unwrap_or_default();
    write_str(&s, buf, cap)
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

/// Export `observer`'s CURRENT belief as codex JSON (§8.8, §10–§11):
/// accumulated reconnaissance made durable. The campaign layer stores it
/// and feeds it back verbatim as an entry in a later battle's `"codex"`
/// setup array to warm-start the rematch (intersection with cold-open —
/// strictly sharper, never wrong). Returns bytes written (excl. NUL),
/// -1 bad handle, -2 bad observer, or the buffer-overflow negative.
#[no_mangle]
pub extern "C" fn srw_codex(
    b: *mut SrwBattle,
    observer: c_int,
    buf: *mut c_char,
    cap: c_int,
) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    if observer < 0 {
        return -2;
    }
    let Some(bl) = b.beliefs.get(observer as usize) else { return -2 };
    write_str(&botboard_core::codex::belief_to_json(bl), buf, cap)
}

/// Shared FFI terrain encoding: 0 none, 1 wall, 2 pit, 3 mine, 4 ice,
/// 5 tall grass, 6 acid, 7/8/9 destructible block tier 1/2/3,
/// 10/11/12/13 conveyor N/E/S/W (N = +y, side 0's forward). The code IS
/// the registry row's stable wire id (all four owner-band mine rows
/// share code 3). Stage-4 custom rows allocate upward from the stdlib
/// band: custom row `i` surfaces as `14 + i` — stable within a battle
/// (order = the setup's "terrains" array order).
fn terrain_code(g: &GameDef, t: u8) -> c_int {
    g.terrain_wire_code(t) as c_int
}

/// Ground-truth terrain: 0 none, 1 wall, 2 pit, 3 mine, 4 ice,
/// 5 tall grass, 6 acid, 7/8/9 block tier 1/2/3, 10..=13 conveyor NESW,
/// 14+ this battle's custom rows in setup order.
#[no_mangle]
pub extern "C" fn srw_terrain(b: *mut SrwBattle, sq: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    b.pos.terrain.get(sq as usize).map_or(-2, |&t| terrain_code(&b.g, t))
}

/// Terrain as `observer` sees it: owner-secret rows (mines, custom
/// `"owner_secret"` rows) are invisible (0) until they blow. 0 none,
/// 1 wall, 2 pit, 3 own mine, 4 ice, 5 grass, 6 acid, 7/8/9 block tier
/// 1/2/3, 10..=13 conveyor NESW, 14+ custom rows.
#[no_mangle]
pub extern "C" fn srw_terrain_for(b: *mut SrwBattle, observer: c_int, sq: c_int) -> c_int {
    let Some(b) = (unsafe { b.as_ref() }) else { return -1 };
    b.pos.terrain.get(sq as usize).map_or(-2, |&t| {
        match b.g.terrain_conceal(t) {
            // Owner-secret rows surface their wire code to the owner
            // and bare floor to everyone else.
            Conceal::OwnerSecret => {
                if b.g.terrain_owner(t) == Some(observer as Side) {
                    terrain_code(&b.g, t)
                } else {
                    0
                }
            }
            _ => terrain_code(&b.g, t),
        }
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
/// 6 compound 7 locust hop (captures the screen at `aux`, landing on the
/// empty `to` beyond it); effect: 0 none 1 heal 2 wall 3 pit 4 laser (a laser whose
/// `to` holds no piece is terrain damage on a destructible block) 5 twice
/// 6 resurrect (out[5] then carries the revived type id, not a promo)
/// 7 hack 8 mine-lay 9 swap (exchange caster with the friendly at `to`)
/// 10 push (shove the enemy at `to` onto `aux`); 11+ this battle's
/// CUSTOM abilities, allocated upward from the stdlib band — custom row
/// `i` (setup "abilities" array order) surfaces as `11 + i`, stable
/// within the battle.
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
        MoveKind::Locust => 7,
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
        Effect::Swap => 9,
        Effect::Push => 10,
        Effect::Spy => 11,
        // Stage-4 custom rows: a fixed band above the stdlib codes
        // (moved from 11+i when spy claimed 11 — the band base leaves
        // headroom for stdlib growth; codes stay stable within a battle).
        Effect::Custom(i) => EFFECT_CUSTOM_BASE + i as c_int,
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
    // Stage-4 customs price through the same registry: parse them so
    // custom cost hints reach cost_prior.
    let Ok(custom_terrains) = parse_custom_terrains(&spec, 2) else { return -4 };
    let Ok(custom_effects) = parse_custom_abilities(&spec, &custom_terrains) else { return -4 };
    let Ok(types) = parse_types(&spec, &custom_effects) else { return -4 };
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
    let g = GameDef::with_customs(
        board,
        2,
        types,
        Vec::new(),
        policy,
        Vec::new(),
        custom_effects,
        custom_terrains,
    );
    for t in 0..n {
        let c = cost_prior(&g, t as TypeId, &W);
        unsafe { *out.add(t) = c };
    }
    n as c_int
}
