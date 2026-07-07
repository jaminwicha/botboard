# Subterranean Robot Wars — Game & Systems Specification

**The roguelike/metroidvania campaign layer that builds on *Robot Wars* and the Botboard engine.**

`Version 0.3 · June 2026`

> **Version 0.3.** Companion to the [Botboard Technical Specification v0.3](#) (cited as *Botboard Spec §N*). This revision follows the engine's move to a single belief-gated **information continuum** (Botboard Spec §8): it surfaces the codex/recon mechanic as belief collapse, and adds **information level as a first-class difficulty and pacing lever**. Everything else carries from v0.2. Changes are traced in [Appendix D — Provenance & Changelog](#appendix-d--provenance--changelog).

---

## Table of Contents

1. [Introduction & Position in the Stack](#1-introduction--position-in-the-stack)
2. [What It Inherits: the Robot Wars Combat Substrate](#2-what-it-inherits-the-robot-wars-combat-substrate)
3. [Design Pillars & Player Fantasy](#3-design-pillars--player-fantasy)
4. [Campaign Structure: Roguelike Meets Metroidvania](#4-campaign-structure-roguelike-meets-metroidvania)
5. [The Core Loop](#5-the-core-loop)
6. [Rooms, Encounters & Difficulty](#6-rooms-encounters--difficulty)
7. [The Controller & Commanding Your Army](#7-the-controller--commanding-your-army)
8. [Card Collection & the Bit Economy](#8-card-collection--the-bit-economy)
9. [Factions, Clans & Alignment](#9-factions-clans--alignment)
10. [Information, Fog & Stealth as Gameplay](#10-information-fog--stealth-as-gameplay)
11. [Progression & Meta-Progression](#11-progression--meta-progression)
12. [Procedural Content Generation](#12-procedural-content-generation)
13. [AI & Content Balancing via Self-Play](#13-ai--content-balancing-via-self-play)
14. [Aesthetic & Presentation](#14-aesthetic--presentation)
15. [Roadmap & Dependencies](#15-roadmap--dependencies)
16. [Open Questions & Risks](#16-open-questions--risks)
- [Appendix A — Leverage Map](#appendix-a--leverage-map)
- [Appendix B — Starter Ability & Terrain Catalog](#appendix-b--starter-ability--terrain-catalog)
- [Appendix C — Example: a Generated Robot Card](#appendix-c--example-a-generated-robot-card)
- [Appendix D — Provenance & Changelog](#appendix-d--provenance--changelog)

---

## 1. Introduction & Position in the Stack

*Subterranean Robot Wars* (SRW) is a single-player, run-based roguelike with metroidvania exploration, in which every fight is a tactical robot battle resolved by the Botboard engine. The player is a newly-conscious controller robot descending through an underground world of warring machines, defeating and dismantling robots to rebuild their own army and pressing deeper.

The product is three layers, each owning a distinct concern:

```
┌────────────────────────────────────────────────────────────────────────┐
│ SUBTERRANEAN ROBOT WARS — roguelike / metroidvania campaign              │
│   dungeon of rooms · runs & meta-progression · card collection ·         │
│   controller narrative · factions · fog-as-discovery · economy           │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │ composes encounters out of
┌────────────────────────────────────▼───────────────────────────────────┐
│ ROBOT WARS — the robot-combat game                                       │
│   one battle = one Botboard game between robot armies on a room-board     │
│   armies · controllers (royal pieces) · terrain · abilities · win cond.   │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │ runs on
┌────────────────────────────────────▼───────────────────────────────────┐
│ BOTBOARD — the headless engine  (see Botboard Spec)                      │
│   Bots/Bits · move-gen · learned eval · belief-gated search ladder · self-play │
└──────────────────────────────────────────────────────────────────────────┘
```

The division of responsibility is strict: Botboard decides what is legal and who wins a position; Robot Wars frames a single battle; SRW is everything outside the battle — how battles are strung into a dungeon, what the player keeps, how they grow, and why they care.

SRW adds **no new rules of movement or capture.** Every robot, ability, terrain feature, and win condition it uses is expressed in the engine's existing Bit/policy vocabulary (Botboard Spec §3, §5). What SRW contributes is structure, economy, progression, narrative, and content — and a set of engine capabilities re-pointed from "play strongly" to "generate and balance a game."

---

## 2. What It Inherits: the Robot Wars Combat Substrate

A **Robot Wars battle** is one Botboard game. SRW never re-implements combat; it instantiates battles and reads their outcomes.

| Robot Wars concept | Botboard mechanism it is built from |
|---|---|
| A robot | a **Bot** — a set of Bits (Botboard Spec §3) |
| A robot's powers | its **Bits**: geometry (Axis A) and effects (Axis B) |
| The room you fight in | a **board**, possibly variable-size or arbitrarily-shaped, with terrain |
| Walls, pits, doors | **terrain** in the occupancy/terrain layer (Botboard Spec §7.1) |
| Lasers, shields, hacking, resurrection, spying, wall/pit creation | **Axis-B effects** (Botboard Spec §3.2) |
| Armor, hit-points, immunities | **capture semantics** + per-instance state arrays (Botboard Spec §7.3) |
| The player / each enemy commander | a **controller** = a royal piece (Botboard Spec §5) |
| A multi-commander brawl | **1–4 player** play (Botboard Spec §8.7) |
| Not knowing an enemy's powers | the **mask substrate** + the belief-gated search ladder (Botboard Spec §8) |
| The enemy commander's tactics | the engine's **search ladder** at a tuned competence tier |
| "Is this army fair?" | the anchored **cost model** + self-play correction (Botboard Spec §4) |

Because the substrate is the engine, SRW content is authored in the engine's terms: a new enemy robot *is* a Bit-set; a hazard room *is* a terrain layout; difficulty *is* army cost, AI competence, and — new in v0.3 — how much information each side has (§6, §10).

---

## 3. Design Pillars & Player Fantasy

- **You are a mind in a machine.** Being a *controller* — able to command, hack, and rebuild robots — is the core power fantasy and the diegetic justification for every system.
- **Defeat is dismantling.** Winning yields the parts (Bits) and sometimes the whole army of the defeated.
- **Knowledge is power.** You do not know what an enemy robot can do until it acts — the same hidden-identity structure that defines Stratego, the game the engine's AI is built around (Botboard Spec §8). And knowledge cuts both ways: enemies that have fought you before know *your* army too (§10).
- **Descent with consequence.** A run is a descent; death feeds meta-progression. The world is ability-gated (metroidvania), not linear.
- **Emergent, not scripted, opposition.** Clans are driven by the engine's AI and fight each other as readily as the player.

---

## 4. Campaign Structure: Roguelike Meets Metroidvania

**The world** is a subterranean network organized by **depth**, divided into **sectors/biomes** that differ in terrain palette, enemy clans, hazards, and aesthetic. Each sector is a graph of **rooms** connected by **corridors**. Rooms are boards; corridors are the map graph; some passages require a capability (drill, flight/hover, a hack tier, a key-item) — the terrain-permission Bits (Botboard Spec §3.1) — so acquiring a robot can unlock blocked regions.

**Two persistence horizons:** within a run (collected robots, army, consumables, and *revealed knowledge* persist and compound) and across runs (meta-progression, §11). Dungeons are procedurally generated (§12) within sector rules, preserving metroidvania landmarks for legibility.

---

## 5. The Core Loop

```
  MOMENT  (a room)             RUN  (a descent)               META  (across runs)
  ┌────────────────┐           ┌──────────────────┐           ┌──────────────────┐
  │ enter room     │           │ choose a path     │           │ unlock Bits /     │
  │  → battle or   │  ───────► │ descend deeper     │ ───────► │  robots / biomes  │
  │    event       │           │ collect Bits/cards │           │ persistent upgrades│
  │  → take loot   │ ◄───────  │ rebuild your army  │ ◄─────── │ codex / mastery   │
  │  → reveal foes │           │ die or extract     │           │ harder ascensions │
  └────────────────┘           └──────────────────┘           └──────────────────┘
```

- **Moment loop:** approach, read what you can (fog permitting), fight or resolve, collect, update knowledge. One Botboard battle.
- **Run loop:** navigate, spend, manage attrition, push or consolidate, end on death or extraction.
- **Meta loop:** convert run results into permanent unlocks that widen future runs.

---

## 6. Rooms, Encounters & Difficulty

A room is generated with a **board layout**, an **encounter**, and a **reward**:

| Room type | Battle? | Description / leverage |
|---|---|---|
| **Skirmish** | yes | a standard battle vs a clan army sized to depth. |
| **Elite / Commander** | yes | an enemy **controller** present; defeating it can win its whole army (§7). |
| **Boss** | yes | a unique high-cost robot or multi-controller set-piece; sector gate. |
| **Ambush** | yes | starts in **darkness/fog**; enemies hidden until they act (§10). |
| **Puzzle / Vault** | no/scripted | terrain + ability gating with no live enemy. |
| **Forge** | no | combine/upgrade **Bits** onto your robots (§8). |
| **Shop / Cache** | no | acquire specific robots or Bits. |
| **Shrine / Event** | no | risk/reward choices, run-scoped blessings/curses, lore. |

**Difficulty has three legible dials**, all expressed in engine terms:

1. **Army cost budget** — deeper rooms field higher-total-cost armies (Botboard Spec §4).
2. **AI competence** — the search ladder is run at a tuned strength tier (shallower/noisier early, stronger for elites and bosses).
3. **Information asymmetry (new in v0.3).** How much each side knows about the other is now a *designable* parameter. A *fresh* enemy fights you blind (it must reason in the engine's expensive hidden-information rungs, and plays more cautiously); a *veteran* or *elite* enemy has **reconned your army** — its belief about you is sharp, so it plays harder and the engine runs it in its cheap, strong perfect-information gear (Botboard Spec §8.8). Likewise, a room can withhold or grant the *player* information (darkness, intel caches, scouted maps). This gives the designer a difficulty and pacing lever orthogonal to raw power — a feared elite is frightening partly because *it already knows how you fight*.

Because all three dials are engine-native and the cost/competence dials are validated by self-play, difficulty is tuned by legible knobs rather than hand-authored stat blocks.

---

## 7. The Controller & Commanding Your Army

The player is a **controller robot**, present on the board as a royal piece (Botboard Spec §5). **Commanding:** the player moves their robots; the controller is one piece among them. **Controller abilities** are Axis-B effects (Botboard Spec §3.2): **hack** (flip an enemy, gated by a target predicate + success chance), **resurrect**, **spy** (reveal a full move-set), **create walls / dig pits** — costed, limited "spells." **Stakes:** if the controller is captured/destroyed, the run (or battle) is lost. **Capturing enemy controllers** can transfer their army at a randomized success rate — the headline reward of elite rooms.

**Army management:** the player holds a **roster** and **deploys** a subset within a deployment budget; the bench and the hand for drops (Botboard Spec §3.3) are the controller's logistics.

---

## 8. Card Collection & the Bit Economy

Robots are **collectible cards**; a card is a **Bot**, i.e. a Bit-set (Botboard Spec §3). **Acquisition:** salvage (defeated robots drop Bits, forged onto yours), capture (whole robots/armies), shops/caches, and drafts. **Army-building** is capped by a **cost budget** using the engine's anchored prior corrected by self-play (Botboard Spec §4) — heterogeneous armies stay fair without per-card hand-balancing. **Rarity** maps to cost/complexity tiers (Appendix B), *derived* from the cost model. **Synthesis (Forge):** combining Bits builds bespoke robots; the cost model prices the result live and self-play-derived values warn when a combination is over- or under-tuned.

---

## 9. Factions, Clans & Alignment

The underground is populated by mutually-opposed **robot clans**, realized through the engine's **multiplayer free-for-all AI** (Botboard Spec §8.7). **Clan identity = a signature Bit palette** (artillery, infiltration, siege), making clans mechanically distinct and teachable. Clans **wander, hold territory, and fight each other and the player**; three-way encounters are tactical opportunities. **Alignment & reputation** shift with player actions, enabling alliances or inviting ambush — emergent from the multiplayer AI, not scripts.

---

## 10. Information, Fog & Stealth as Gameplay

The engine's defining mechanic — **a piece's rules are hidden until observed** (Botboard Spec §8) — is SRW's signature system, and in v0.3 it is grounded in the engine's full **information continuum** rather than a single hidden mode.

- **Discovery as play.** You don't know an unfamiliar robot's powers until it moves, attacks, or you reveal it. Early turns against new enemies are about *learning*; a known enemy is a weaker enemy.
- **Scouting & spying.** **Spy** collapses a belief to certainty; baiting an enemy into revealing itself is a skill.
- **Fog, darkness & stealth** are the engine's **mask substrate** (Botboard Spec §8.6): the player's view is the ground-truth board with concealment masks applied; each enemy carries a knowledge mask of which rules you've seen.
- **The codex (knowledge meta-progression).** Identified robots are recorded in a **bestiary**; their movesets stay revealed in future encounters. Accumulated knowledge is persistent power.

**The codex is belief collapse — and so is enemy recon.** This is the systems-level heart of v0.3. Information held about an army *is* the sharpness of a belief, the very dial the engine uses to choose how it thinks (Botboard Spec §8.1). Two consequences fall straight out:

- **Scouting makes the engine cheaper *and* the enemy more dangerous in a rematch.** When you have reconned an enemy (codex) you fight informed; when an enemy has reconned you (a veteran/elite, or a clan you've fought repeatedly this run), *its* belief about your army is sharp, so the engine drops into its strong, fast perfect-information gear against you and plays harder. The game mechanic (scout / be scouted) and the engine optimization (slide down to chess-engine mode) are the same variable.
- **A run can model enemies "learning" you.** Because belief sharpness persists, a clan you keep fighting through a run accumulates intel on your army — later encounters with that clan are tougher and run in the engine's cheaper rungs, a natural rising-tension arc with no scripting.

**PvE intelligence** uses the engine's belief-gated ladder at per-encounter competence tiers: cheap perfect-information search where the enemy knows you (rematches, elites), the sound mid-ladder where information is partial, and the search-free **R-NaD** policy for genuinely blind cold-opens against unknown clans (Botboard Spec §8.2). Design intent: make *information itself* a resource the player gathers, spends, and is denied — and a thing the enemy gathers about the player in turn.

---

## 11. Progression & Meta-Progression

**Within a run (power):** the army strengthens as Bits/robots are collected and forged; the deployment budget may rise at depth milestones; revealed knowledge accumulates. Growth is bounded by the cost budget.

**Across runs (breadth & permanence):** new **Bits/robots/rarities** enter the pools; new **starting controllers/armies and run modifiers**; opened **sector gates and shortcuts**; the persistent **codex/mastery** bestiary (rewards for fully cataloguing a clan); and an optional **ascension ladder** of difficulty modifiers — which can now include *information* modifiers (enemies start with more intel on you, fewer scouted maps). Meta-progression widens *options and knowledge* rather than handing out flat power.

---

## 12. Procedural Content Generation

Procedural generation is leveraged at three levels: **procedural robots** (sampled valid, themed **Bit-sets**, kept in power bands by the anchored cost model, styled by clan palette — stats are priced Bits, not invented); **procedural dungeons** (room graph, board layouts, terrain, encounter/reward placement, preserving landmarks); and **procedural visuals** (16-bit sprites generated to *communicate function*, so a sprite's silhouette telegraphs its Bit profile before the fog lifts). Generated content is **pre-validated by the engine** (§13), and the play-testers doing so must **generalize across novel pieces** — which the engine's generalizing evaluation is built to meet (Botboard Spec §7.4).

---

## 13. AI & Content Balancing via Self-Play

The engine's **self-play farm** (Botboard Spec §9) is reused as SRW's **content-QA and difficulty pipeline** — mirroring established practice (asymmetric-game point-cost estimation from self-play; evolutionary CCG play-testing).

- **PvE controllers** are the engine's belief-gated ladder at tuned competence tiers — and at tuned *information* tiers (§6, §10), a second knob the continuum makes available.
- **Generated-content vetting.** New robots, armies, and dungeons are play-tested by self-play *before shipping*; the loop that measures piece values (Botboard Spec §4) flags over/under-tuned robots, degenerate combinations, and unwinnable or trivial encounters.
- **Cost-model feedback.** Realized win rates correct the cost prior, keeping budgets, rarity tiers, and procedural bands honest.
- **Live difficulty tuning.** Aggregate self-play and (later) player telemetry adjust encounter cost budgets, controller competence, and information asymmetry per depth.

The caveat the research is explicit about: play-testing agents must generalize to new content, or they will mis-judge it — which is why the engine's evaluator is built to generalize. The ML core earns its cost twice: once as the opponent, once as the balancing system that makes infinite procedural content shippable.

---

## 14. Aesthetic & Presentation

- **16-bit / SNES-era pixel art** — blocky, readable robots, rooms, and UI.
- **Subterranean atmosphere** — distinct sector palettes signaling depth, hazard, and clan territory.
- **Readability first.** Because robots are procedural and powers initially hidden, presentation must let players *read intent*: silhouette telegraphs role, revealed movesets show clearly, fog/stealth states are unambiguous, an army's threat is glanceable — and the UI should hint when an enemy is *informed about you* (a reconned elite), since that changes how it will play.
- **The controller UI** centers the command fantasy: roster, deployment, the hand for drops, and the controller's abilities as a clear, limited toolkit.

---

## 15. Roadmap & Dependencies

SRW depends on Botboard/Robot Wars maturity. The engine phases (Botboard Spec §12) are the hard prerequisites:

```
 BOTBOARD prereqs ───────────────────────────────►  SRW build order
 P0 engine core + prototypes ┐
 P1 cost + embeddings/anchors  │  enable army-building + procedural balance
 P2 AI ladder (perfect→determinize→GT-CFR→R-NaD→FFA) │  enable PvE controllers, clans, info-tiers
 P3 fog / stealth (masks) + terrain ┘  enable discovery + gating + hazard rooms
```

**SRW's own build order:**
1. **Vertical slice** — one sector, skirmish + elite rooms, a small robot pool, salvage→forge→deploy, a controller with two abilities, a short run. Proves the moment+run loops on real battles.
2. **Full run loop** — sector graph generation, room archetypes, drafts/shops, capture-the-controller, basic meta-unlocks.
3. **Information layer** — fog/darkness rooms, spy/stealth (the mask substrate), the codex/bestiary, and the enemy-recon difficulty lever. (Requires Botboard P2–P3.)
4. **Factions** — multiple clans, wandering/three-way encounters, alignment. (Requires Botboard P2 multiplayer.)
5. **Procedural content + self-play balancing** — procedural robots/dungeons/visuals with self-play vetting; ascension ladder; meta-progression breadth.
6. **Polish & economy tuning** — telemetry-driven difficulty, rarity, and pacing.

---

## 16. Open Questions & Risks

- **Pacing of fog.** Discovery is the signature pleasure but can frustrate; how much to hide, how cheap to make revealing. Needs playtesting.
- **Legibility of enemy recon.** The "this elite already knows your army" lever is powerful but invisible unless surfaced; the UI must communicate it without exposing the machinery.
- **Collection power-creep vs the cost model.** As the pool grows, can the anchored cost model + self-play hold the line? The pairwise synergy term can't see all higher-order combos (Botboard Spec §4.1), so self-play vetting is the backstop.
- **Procedural robot readability.** Can generated sprites telegraph Bit profiles well enough to reason under fog?
- **Run length & extraction.** Right run length; whether extraction exists or death is the only exit.
- **Three-way encounter clarity.** Multiplayer brawls are exciting but hard to read; UI and AI behavior must make them legible.
- **Engine dependency risk.** SRW's distinctive systems sit on the engine's hardest, latest capabilities (the search ladder, the mask substrate, generalizing evaluation); slips in Botboard P2/P3 directly gate SRW.

---

## Appendix A — Leverage Map

| SRW system | Leverages (Botboard) | Notes |
|---|---|---|
| A battle | a Botboard game | SRW never re-implements rules |
| Robots / cards | Bots = Bit-sets (§3) | content = engine vocabulary |
| Forge / synthesis | the cost model (§4) | prices bespoke builds live |
| Army-building budget, rarity | anchored prior + self-play (§4, §9) | fairness without hand-balancing |
| Controller & abilities | royalty (§5) + Axis-B effects (§3.2) | hack/spy/resurrect/wall/pit |
| HP / armor / healing robots | per-instance state arrays (§7.3) | cheap to simulate, branchless |
| Capture-the-controller | royal capture + capture semantics | run power spikes |
| Terrain rooms & metroidvania gates | terrain layer (§7.1), path Bits (§3.1) | drill/flight/hack as keys |
| Fog, stealth, discovery | mask substrate + the search ladder (§8) | the signature system |
| Codex / enemy recon / difficulty-by-info | belief sharpness = the engine's dial (§8.1, §8.8) | scouting ↔ engine gear are one variable |
| Factions / clans / three-way | multiplayer belief AI (§8.7) | living, self-interested world |
| PvE difficulty tiers | ladder competence + information tier (§8) | two principled dials |
| Procedural robots | Bit generator + anchored cost model (§4) | stats = priced Bits |
| Content vetting & balancing | self-play farm (§9) | QA for infinite content |

---

## Appendix B — Starter Ability & Terrain Catalog

A first catalog of Axis-B abilities and terrain, adapted from the Gemini spec as seed content. All costs are **placeholders** — initial priors only; the engine's anchored cost model and self-play set real values (Botboard Spec §4). All are expressed in existing engine primitives.

### Offensive & utility abilities
| Ability | Effect | Engine realization | Cost prior |
|---|---|---|---|
| Piercing | ranged attack ignores blockers | slider/laser ignores occupancy on its ray | ×1.3 |
| Stealth | concealed on opponents' view until adjacent | concealment mask in the belief substrate (§8.6) | ×1.4 |
| Hover / Flight | ignores pit tiles when moving | terrain permission Bit (§3.1) | ×1.1 |
| EMP Aura | enemies within 2 tiles cannot use action Bits | global EMP mask; action Bits suppressed inside it | +6.5 |
| Hologram (Decoy) | a fake clone that moves but cannot attack; vanishes if hit | exists in opponents' belief view; ground-truth HP 0 | +2.5 |
| Welding Torch | heals an adjacent ally 1 HP | increments the ally's `hp[]` entry (§7.3) | +3.0 |
| Mine-Layer | leaves a hidden trap on a vacated square | trap on the square, absent from the enemy view | (tbd) |
| Overclock (Kamikaze) | acts twice in a turn, takes 1 self-damage | compound action + self HP decrement | (tbd) |

### Terrain / tile types
| Tile | Effect |
|---|---|
| Floor | normal traversal. |
| Wall | impassable unless the mover has **drill**. |
| Pit | impassable unless the mover has **flight/hover**. |
| Ice | a slider entering must continue until it hits a wall, floor, or piece. |
| Tall Grass / Smoke | a robot standing here gains **stealth** unless adjacent. |
| Acid / Lava | passable, but ending a turn here costs 1 HP. |

### Rarity ↔ cost tiers (placeholder bands)
| Tier | Role | Cost band |
|---|---|---|
| 1 | pawn-like / chaff | ~0.5–2.0 |
| 2 | tactical specialist | ~3.0–6.0 |
| 3 | queen-sized | ~8.0–12.0 |
| 4 | boss / super-heavy | ~15.0+ |

---

## Appendix C — Example: a Generated Robot Card

```
  ╔══════════════════════════════════════╗
  ║  SCUTTLE-LANCE          [Rare]        ║   Clan:  Foundry Siege
  ║  ────────────────────────────────────║   Cost:  ~4.1  (anchored cost model)
  ║  Bits:                                ║   Role:  ranged skirmisher
  ║   • rider(0,1)        move-only       ║
  ║   • hopper(0,1)       capture-only    ║   Sprite telegraphs: long, low,
  ║   • laser  (range 2, coupled retreat) ║   barrel-forward  →  reads "ranged"
  ║   • armor  (2 HP)                     ║
  ║  Hidden until observed in play.       ║   Belief narrows as it moves/fires.
  ╚══════════════════════════════════════╝
```

Its **stats are its Bits**; its **cost** is the engine's valuation of that set; its **rarity** follows from cost/complexity (§8); its **AI** is the shared belief-gated ladder; its **balance** was vetted by self-play (§13); its **HP** lives in a state array (Botboard Spec §7.3); and to a player meeting it cold it begins **hidden** — though to a clan that has fought this player before, *its* belief about the player's army is already sharp (§10).

---

## Appendix D — Provenance & Changelog

### New in v0.3 (this revision)
- **Information level as a first-class difficulty and pacing lever (§6, §10).** Difficulty now has three engine-native dials — army cost, AI competence, and **information asymmetry** — the last falling directly out of the engine's belief-sharpness dial (Botboard Spec §8).
- **Codex/recon reframed as belief collapse (§10).** Scouting an enemy and being scouted by an enemy are the *same variable* the engine uses to choose its search gear: informed enemies (rematches, elites) play harder and run in the engine's cheap perfect-information rungs; cold-opens run in the expensive hidden rungs. A run can model clans "learning" the player's army as a natural rising-tension arc.
- Cross-references updated to the engine's single belief-gated **search ladder** (Botboard Spec §8) in place of v0.2's separate AI lineages.

### Carried from v0.2 (unchanged)
The three-layer framing and strict engine/game division; the nested core loop; room/encounter archetypes; the controller fantasy and capture-the-controller; the Bit-economy/collection spine; factions via multiplayer AI; knowledge-as-progression; procedural generation; self-play content balancing; the leverage-map discipline; the Gemini-derived ability/terrain catalog (Appendix B); HP-as-state-array; and the mask-based concealment substrate. See v0.2 Appendix D for that history.

---

*Subterranean Robot Wars — Game & Systems Specification, Version 0.3. Complementary to the Botboard Technical Specification v0.3; the codex and enemy reconnaissance are the same belief-sharpness dial the engine uses to slide between its imperfect- and perfect-information gears.*
