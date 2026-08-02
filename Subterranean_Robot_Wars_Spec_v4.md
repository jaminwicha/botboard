# Subterranean Robot Wars — Game & Systems Specification

**The roguelike/metroidvania campaign layer that builds on *Robot Wars* and the Botboard engine.**

`Version 0.4 · August 2026`

> **Version 0.4.** Companion to the [Botboard Technical Specification v0.4](#) (cited as *Botboard Spec §N*). This is an **implementation catch-up revision**: the battle surface shipped ahead of the spec, and this revision documents it. It (1) documents the **battle authoring surface as shipped** — the setup JSON, the expanded move/ability/terrain vocabulary, per-battle **custom abilities and terrains** (Bits 2.0 Stage 4), and per-battle **net checkpoints** (new Appendix E); (2) promotes **spy** and the **codex wire-up** from design intent to shipped mechanics (§7, §10); and (3) **rules on capture-the-controller army transfer** as a campaign-layer composition (§7). Everything else carries from v0.3. Changes are traced in [Appendix D — Provenance & Changelog](#appendix-d--provenance--changelog).

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
- [Appendix E — The Battle Authoring Surface (as shipped)](#appendix-e--the-battle-authoring-surface-as-shipped)

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

SRW adds **no new rules of movement or capture.** Every robot, ability, terrain feature, and win condition it uses is expressed in the engine's existing Bit/policy vocabulary (Botboard Spec §3, §5) — and, new in v0.4, that vocabulary is partly **authorable per battle**: the campaign can ship custom Axis-B effects and terrain kinds as validated data rows in the battle setup itself (Appendix E), without an engine release. What SRW contributes is structure, economy, progression, narrative, and content — and a set of engine capabilities re-pointed from "play strongly" to "generate and balance a game."

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
| A clan's bespoke gimmick (new in v0.4) | **custom ability/terrain rows** authored in the battle setup (Bits 2.0 Stage 4; Appendix E) |

Because the substrate is the engine, SRW content is authored in the engine's terms: a new enemy robot *is* a Bit-set; a hazard room *is* a terrain layout; difficulty *is* army cost, AI competence, and how much information each side has (§6, §10). The full authoring surface — every setup key, the shipped move/ability/terrain vocabulary, and the custom-content rows — is catalogued in **Appendix E**.

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
3. **Information asymmetry.** How much each side knows about the other is a *designable* parameter. A *fresh* enemy fights you blind (it must reason in the engine's expensive hidden-information rungs, and plays more cautiously); a *veteran* or *elite* enemy has **reconned your army** — its belief about you is sharp, so it plays harder and the engine runs it in its cheap, strong perfect-information gear (Botboard Spec §8.8). Likewise, a room can withhold or grant the *player* information (darkness, intel caches, scouted maps). This gives the designer a difficulty and pacing lever orthogonal to raw power — a feared elite is frightening partly because *it already knows how you fight*.

All three dials are shipped battle-surface parameters: the cost budget prices through `srw_price`, competence is the per-side `tiers` array, and information asymmetry is the per-side `intel` array plus — new in v0.4 — the persistent `codex` warm-start (§10, Appendix E). Because all three are engine-native and the cost/competence dials are validated by self-play, difficulty is tuned by legible knobs rather than hand-authored stat blocks.

---

## 7. The Controller & Commanding Your Army

The player is a **controller robot**, present on the board as a royal piece (Botboard Spec §5). **Commanding:** the player moves their robots; the controller is one piece among them. **Controller abilities** are Axis-B effects (Botboard Spec §3.2): **hack** (flip an enemy, gated by a target predicate), **resurrect**, **spy** (reveal a robot's identity — *shipped in v0.4*: a board-null action whose entire payload is belief collapse and stealth-piercing for the caster's side, §10), **create walls / dig pits** — costed, limited "spells." **Stakes:** if the controller is captured/destroyed, the run (or battle) is lost.

**Capturing enemy controllers** can transfer their army at a randomized success rate — the headline reward of elite rooms. **Ruling (v0.4):** army transfer is a **campaign-layer composition, not an engine primitive.** The engine already composes every battle from a JSON roster (placements of typed pieces); "winning an army" means the campaign adds the defeated side's roster entries to the player's collection and fields them in *later* setups. The randomized success rate is campaign-seeded RNG applied at reward time. Nothing in the engine reassigns pieces mid-battle, and nothing needs to: within the battle that wins the army, victory ends the game; the transfer is bookkeeping between battles, which the campaign layer owns. (A mid-battle mass-defection mechanic, if ever wanted, would be a distinct future engine feature — it is explicitly **not** implied by this reward.)

**Army management:** the player holds a **roster** and **deploys** a subset within a deployment budget; the bench and the hand for drops (Botboard Spec §3.3) are the controller's logistics.

---

## 8. Card Collection & the Bit Economy

Robots are **collectible cards**; a card is a **Bot**, i.e. a Bit-set (Botboard Spec §3). **Acquisition:** salvage (defeated robots drop Bits, forged onto yours), capture (whole robots/armies), shops/caches, and drafts. **Army-building** is capped by a **cost budget** using the engine's anchored prior corrected by self-play (Botboard Spec §4) — heterogeneous armies stay fair without per-card hand-balancing. **Rarity** maps to cost/complexity tiers (Appendix B), *derived* from the cost model. **Synthesis (Forge):** combining Bits builds bespoke robots; the cost model prices the result live and self-play-derived values warn when a combination is over- or under-tuned.

**Custom-content pricing (new in v0.4):** custom abilities carry a **required cost hint** (a flat prior term plus a utility multiplier) validated at the battle boundary, so forged or clan-bespoke effects stay inside the same budget discipline as stdlib content (Appendix E).

---

## 9. Factions, Clans & Alignment

The underground is populated by mutually-opposed **robot clans**, realized through the engine's **multiplayer free-for-all AI** (Botboard Spec §8.7). **Clan identity = a signature Bit palette** (artillery, infiltration, siege), making clans mechanically distinct and teachable — and, with v0.4's custom rows, a clan can own a **signature bespoke effect or terrain** (a vampiric strike, a lava biome) authored as data in its battles' setups. Clans **wander, hold territory, and fight each other and the player**; three-way encounters are tactical opportunities. **Alignment & reputation** shift with player actions, enabling alliances or inviting ambush — emergent from the multiplayer AI, not scripts.

---

## 10. Information, Fog & Stealth as Gameplay

The engine's defining mechanic — **a piece's rules are hidden until observed** (Botboard Spec §8) — is SRW's signature system, grounded in the engine's full **information continuum** rather than a single hidden mode.

- **Discovery as play.** You don't know an unfamiliar robot's powers until it moves, attacks, or you reveal it. Early turns against new enemies are about *learning*; a known enemy is a weaker enemy.
- **Scouting & spying.** **Spy** collapses a belief to certainty; baiting an enemy into revealing itself is a skill. *Shipped in v0.4:* spy is a stdlib ability (`"spy"` with a range), generated like any Axis-B action. It is **board-null** — it moves nothing, damages nothing, and spends the turn — and on apply the caster's side learns the target's true identity and **pierces its stealth permanently**. Because it is board-null, the perfect-information searcher correctly treats it as a tempo loss; it is a *player* verb and a scripted-encounter verb, priced by a small flat cost term.
- **Fog, darkness & stealth** are the engine's **mask substrate** (Botboard Spec §8.6): the player's view is the ground-truth board with concealment masks applied; each enemy carries a knowledge mask of which rules you've seen.
- **The codex (knowledge meta-progression).** Identified robots are recorded in a **bestiary**; their movesets stay revealed in future encounters. Accumulated knowledge is persistent power. *Shipped in v0.4, end to end:* after (or during) a battle, the campaign exports any observer's accumulated belief with **`srw_codex`**; feeding that export back verbatim in a later setup's **`"codex"`** array warm-starts the rematch belief by intersection with the cold open — strictly sharper, never wrong, per-piece fallback when the enemy changed a slot, and applied before `intel` so fresh certain intel wins over stale recon (Appendix E).

**The codex is belief collapse — and so is enemy recon.** Information held about an army *is* the sharpness of a belief, the very dial the engine uses to choose how it thinks (Botboard Spec §8.1). Two consequences fall straight out:

- **Scouting makes the engine cheaper *and* the enemy more dangerous in a rematch.** When you have reconned an enemy (codex) you fight informed; when an enemy has reconned you (a veteran/elite, or a clan you've fought repeatedly this run), *its* belief about your army is sharp, so the engine drops into its strong, fast perfect-information gear against you and plays harder. The game mechanic (scout / be scouted) and the engine optimization (slide down to chess-engine mode) are the same variable.
- **A run can model enemies "learning" you.** Because belief sharpness persists — now literally, as exported codex JSON the campaign stores per clan — a clan you keep fighting through a run accumulates intel on your army; later encounters with that clan are tougher and run in the engine's cheaper rungs, a natural rising-tension arc with no scripting.

**PvE intelligence** uses the engine's belief-gated ladder at per-encounter competence tiers: cheap perfect-information search where the enemy knows you (rematches, elites), the sound mid-ladder where information is partial, and the search-free **R-NaD** policy for genuinely blind cold-opens against unknown clans (Botboard Spec §8.2). *(Scope note, unchanged from the implementation: battles with 3–4 sides run the committed FFA baseline rather than the belief ladder — the full information continuum applies to two-sided battles.)* Design intent: make *information itself* a resource the player gathers, spends, and is denied — and a thing the enemy gathers about the player in turn.

---

## 11. Progression & Meta-Progression

**Within a run (power):** the army strengthens as Bits/robots are collected and forged; the deployment budget may rise at depth milestones; revealed knowledge accumulates. Growth is bounded by the cost budget.

**Across runs (breadth & permanence):** new **Bits/robots/rarities** enter the pools; new **starting controllers/armies and run modifiers**; opened **sector gates and shortcuts**; the persistent **codex/mastery** bestiary (rewards for fully cataloguing a clan) — realized as stored `srw_codex` exports keyed by clan (§10); and an optional **ascension ladder** of difficulty modifiers — which can include *information* modifiers (enemies start with more intel on you, fewer scouted maps). Meta-progression widens *options and knowledge* rather than handing out flat power.

---

## 12. Procedural Content Generation

Procedural generation is leveraged at three levels: **procedural robots** (sampled valid, themed **Bit-sets**, kept in power bands by the anchored cost model, styled by clan palette — stats are priced Bits, not invented); **procedural dungeons** (room graph, board layouts, terrain, encounter/reward placement, preserving landmarks); and **procedural visuals** (16-bit sprites generated to *communicate function*, so a sprite's silhouette telegraphs its Bit profile before the fog lifts). Generated content is **pre-validated by the engine** (§13), and the play-testers doing so must **generalize across novel pieces** — which the engine's generalizing evaluation is built to meet (Botboard Spec §7.4). *(v0.4 note: custom abilities feed the evaluator through the descriptor slot of their nearest stdlib kin — a documented approximation until a wider descriptor version; their pricing flows through the required cost hint.)*

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

SRW depends on Botboard/Robot Wars maturity. The engine phases (Botboard Spec §12) are the hard prerequisites — **all shipped as of v0.4**, including the information layer's engine half (spy + codex over the FFI):

```
 BOTBOARD prereqs ───────────────────────────────►  SRW build order
 P0 engine core + prototypes ┐
 P1 cost + embeddings/anchors  │  enable army-building + procedural balance
 P2 AI ladder (perfect→determinize→GT-CFR→R-NaD→FFA) │  enable PvE controllers, clans, info-tiers
 P3 fog / stealth (masks) + terrain ┘  enable discovery + gating + hazard rooms
```

**SRW's own build order:**
1. **Vertical slice** — one sector, skirmish + elite rooms, a small robot pool, salvage→forge→deploy, a controller with two abilities, a short run. Proves the moment+run loops on real battles.
2. **Full run loop** — sector graph generation, room archetypes, drafts/shops, capture-the-controller (the §7 roster-transfer ruling), basic meta-unlocks.
3. **Information layer** — fog/darkness rooms, spy/stealth (the mask substrate), the codex/bestiary (`srw_codex` + `"codex"` warm-starts), and the enemy-recon difficulty lever. *(The engine side is complete; this phase is campaign UI + storage.)*
4. **Factions** — multiple clans, wandering/three-way encounters, alignment. (Requires Botboard P2 multiplayer — shipped as the FFA baseline.)
5. **Procedural content + self-play balancing** — procedural robots/dungeons/visuals with self-play vetting; ascension ladder; meta-progression breadth; clan-signature custom rows (Appendix E).
6. **Polish & economy tuning** — telemetry-driven difficulty, rarity, and pacing.

---

## 16. Open Questions & Risks

- **Pacing of fog.** Discovery is the signature pleasure but can frustrate; how much to hide, how cheap to make revealing. Spy's cost prior (currently a small flat term) and per-controller spy ranges are the tuning surface. Needs playtesting.
- **Legibility of enemy recon.** The "this elite already knows your army" lever is powerful but invisible unless surfaced; the UI must communicate it without exposing the machinery.
- **Collection power-creep vs the cost model.** As the pool grows, can the anchored cost model + self-play hold the line? The pairwise synergy term can't see all higher-order combos (Botboard Spec §4.1), so self-play vetting is the backstop. Custom-row cost hints are author-supplied and therefore *gameable by content authors* — self-play vetting must cover custom content too.
- **Procedural robot readability.** Can generated sprites telegraph Bit profiles well enough to reason under fog?
- **Run length & extraction.** Right run length; whether extraction exists or death is the only exit.
- **Three-way encounter clarity.** Multiplayer brawls are exciting but hard to read; UI and AI behavior must make them legible. (And they run the FFA baseline, not the belief ladder — information tricks read differently there.)
- ~~Engine dependency risk.~~ Retired in v0.4: the engine capabilities SRW's distinctive systems need (the search ladder, the mask substrate, generalizing evaluation, spy/codex, custom content) are shipped and tested; remaining engine work is scale and quality (net strength), not capability.

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
| Capture-the-controller | royal capture + campaign roster transfer (§7 ruling) | run power spikes |
| Terrain rooms & metroidvania gates | terrain layer (§7.1), path Bits (§3.1) | drill/flight/hack as keys |
| Fog, stealth, discovery | mask substrate + the search ladder (§8) | the signature system |
| Codex / enemy recon / difficulty-by-info | belief sharpness (§8.1, §8.8) via `srw_codex` + `"codex"` | scouting ↔ engine gear are one variable |
| Spy | the stdlib `"spy"` ability (board-null belief collapse) | shipped v0.4 |
| Clan-signature effects & biomes | custom ability/terrain rows (Bits 2.0 Stage 4; App. E) | data, not engine releases |
| Factions / clans / three-way | multiplayer FFA AI (§8.7) | living, self-interested world |
| PvE difficulty tiers | ladder competence + information tier (§8) | two principled dials |
| Procedural robots | Bit generator + anchored cost model (§4) | stats = priced Bits |
| Content vetting & balancing | self-play farm (§9) | QA for infinite content |

---

## Appendix B — Starter Ability & Terrain Catalog

A first catalog of Axis-B abilities and terrain, adapted from the Gemini spec as seed content. All costs are **placeholders** — initial priors only; the engine's anchored cost model and self-play set real values (Botboard Spec §4). All are expressed in existing engine primitives.

> **v0.4 status:** this catalog is now a **subset** of the shipped vocabulary. Everything below is implemented; the shipped surface additionally includes swap, push, spy, destructible block tiers, conveyor belts, drill permission, per-move HP gates, hopper landing modes (grasshopper/locust), range-limited riders, and per-battle **custom** abilities/terrains. Appendix E is the complete as-shipped reference.

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
| Spy *(v0.4)* | reveal an enemy robot's identity | board-null stdlib ability; belief collapse + stealth pierce for the caster's side | +0.5 flat |

### Terrain / tile types
| Tile | Effect |
|---|---|
| Floor | normal traversal. |
| Wall | impassable unless the mover has **drill**. |
| Pit | impassable unless the mover has **flight/hover**. |
| Ice | a slider entering must continue until it hits a wall, floor, or piece. |
| Tall Grass / Smoke | a robot standing here gains **stealth** unless adjacent. |
| Acid / Lava | passable, but ending a turn here costs 1 HP. |
| Destructible blocks *(shipped)* | 3 laser-crackable tiers; drillers pass through. |
| Conveyors *(shipped)* | 4 directions; rewrite the mover's destination like ice. |
| *Custom rows (v0.4)* | per-battle authored: blocks/on-land/carry/conceal properties (App. E). |

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

### New in v0.4 (this revision — implementation catch-up)
- **Appendix E added: the battle authoring surface as shipped.** The setup JSON schema; move vocabulary batches 2–3 (drill, destructible blocks, conveyors, swap, push, per-move HP gates, hopper landing modes, range-limited riders); per-battle **custom ability and terrain rows** (Bits 2.0 Stage 4) with loud boundary validation and required cost hints; per-battle **net checkpoints**; battle telemetry.
- **Spy shipped (§7, §10, App. B).** A stdlib board-null ability: belief collapse + permanent stealth pierce for the caster's side; priced as a small flat term and excluded from mobility-based pricing.
- **Codex wired end to end (§10, §11).** `srw_codex` exports an observer's belief; the setup `"codex"` array warm-starts rematches by intersection (before `intel`). The bestiary and clan-recon arcs are now storage + UI, not engine work.
- **Capture-the-controller ruled (§7).** Army transfer is campaign-layer roster bookkeeping between battles over the existing composition surface; explicitly not a mid-battle engine mechanic.
- **Engine dependency risk retired (§15, §16).** All engine prerequisites are shipped and tested; residual engine work is strength/scale (net promotion), not capability.
- Scope note recorded (§10): 3–4-sided battles run the FFA baseline, not the belief ladder.

### New in v0.3
- **Information level as a first-class difficulty and pacing lever (§6, §10).** Difficulty has three engine-native dials — army cost, AI competence, and **information asymmetry** — the last falling directly out of the engine's belief-sharpness dial (Botboard Spec §8).
- **Codex/recon reframed as belief collapse (§10).** Scouting an enemy and being scouted by an enemy are the *same variable* the engine uses to choose its search gear: informed enemies (rematches, elites) play harder and run in the engine's cheap perfect-information rungs; cold-opens run in the expensive hidden rungs. A run can model clans "learning" the player's army as a natural rising-tension arc.
- Cross-references updated to the engine's single belief-gated **search ladder** (Botboard Spec §8) in place of v0.2's separate AI lineages.

### Carried from v0.2 (unchanged)
The three-layer framing and strict engine/game division; the nested core loop; room/encounter archetypes; the controller fantasy and capture-the-controller; the Bit-economy/collection spine; factions via multiplayer AI; knowledge-as-progression; procedural generation; self-play content balancing; the leverage-map discipline; the Gemini-derived ability/terrain catalog (Appendix B); HP-as-state-array; and the mask-based concealment substrate. See v0.2 Appendix D for that history.

---

## Appendix E — The Battle Authoring Surface (as shipped)

The normative reference is the FFI module documentation (`crates/botboard-ffi/src/srw.rs`) and its acceptance suite (`srw_suite.rs`); this appendix is the campaign author's map of it. A battle is composed from **one JSON setup** passed to `srw_create`; validation is loud (a NULL handle plus a precise fault string via `srw_last_error`).

### Setup keys
| Key | Meaning |
|---|---|
| `seed`, `max_plies` | deterministic battle seed; ply cap (draw at cap). |
| `board` | `{w, h}` room dimensions. |
| `sides` | 2–4. Two-sided battles run the belief-gated ladder; 3–4 run the FFA baseline. |
| `types` | the Bit-set piece types: `moves` (Axis A), `abilities` (Axis B), flags (`royal`, `hp`, `stealth`, `hologram`, `flight`, `drill`, `overclock`, `emp`). |
| `placements` | typed, sided piece placements. |
| `terrain` | cells of stdlib kinds (wall, pit, ice, grass, acid, block1–3, conv_n/e/s/w) or custom terrain ids. |
| `intel` | per-observer belief collapse at setup: `known_types` or `reveal_all` (which also pierces stealth). |
| `codex` | *(v0.4)* per-observer rematch warm-start: `srw_codex` exports fed back verbatim; intersection with cold-open, applied before `intel`. |
| `tiers` | per-side AI competence (0–3 → ladder configs). |
| `material` | optional per-side material table override. |
| `net` | optional BBNET checkpoint path; quantized against this battle's GameDef; bad paths fail the build loudly. |
| `abilities` | *(Stage 4)* custom ability rows — see below. |
| `terrains` | *(Stage 4)* custom terrain rows — see below. |

### Move vocabulary (Axis A, batches 1–3)
Leaper / rider / hopper geometry with direction filters and move/capture/both modes; per-move `min_hp`/`max_hp` state gates; rider `max_steps` (range-limited riders); hopper `landing` modes `cannon` (xiangqi), `beyond` (grasshopper), `locust` (captures the screen, lands beyond). Castling, en passant, double-step, drops, promotion, and overclock compounds are stdlib move scripts.

### Ability vocabulary (Axis B, stdlib)
`heal`, `wall`, `pit`, `laser` (range/pierce/retreat), `resurrect`, `hack`, `mine`, `swap`, `push`, and *(v0.4)* `spy` — all parameterized per piece (`range` etc.). Effect codes 0–11 are frozen wire ids in `srw_legal_info`.

### Custom abilities (`"abilities"`, Bits 2.0 Stage 4)
One row = one bespoke effect; the row IS the definition (piece entries reference it by id). Target: `who` (enemy/friendly/empty) × reach (Chebyshev `range` or `ray {max, pierce}`) × preds (`damaged`, `hp_eq1`, `nonroyal`, `bare`). Ops (against the target, in order): `hp_add` (lethal at 0), `capture_at`, `flip_side`, `set_terrain`, `terrain_step`; self-ops: `hp_add`. **Required**: a `cost` hint (flat + multiplier) and a `descriptor_slot` naming the nearest stdlib kin for the evaluator. Custom effect codes surface at 32+ (`EFFECT_CUSTOM_BASE`), stable within a battle. Relocation-coupled shapes (swap/push/retreat analogues) and custom *move* scripts remain stdlib-only.

### Custom terrains (`"terrains"`, Bits 2.0 Stage 4)
≤ 16 rows per battle: `blocks` (ground/flight/drill permission classes), `on_land` (op list + gate + consumed), `carry` (slide/belt destination rewriting), `conceal` (standing / owner-secret), `tiers` (0/1 destructible). Wire codes allocate above the stdlib band, stable within a battle.

### Reading the battle
`srw_legal_count`/`srw_legal_move`/`srw_legal_info` (moves + kind/effect codes), `srw_piece_info`, `srw_terrain` / `srw_terrain_for` (per-observer masking), `srw_visible` / `srw_revealed` (presence vs identity), `srw_entropy` (belief sharpness — the "how informed" UI signal), `srw_codex` (belief export), `srw_ai_move` (returns the rung used — the UI's "how hard is it thinking" signal), `srw_end_reason`, `srw_last_move_stats`, `srw_price` (army pricing for budgets/forge).

---

*Subterranean Robot Wars — Game & Systems Specification, Version 0.4. Complementary to the Botboard Technical Specification; the codex and enemy reconnaissance are the same belief-sharpness dial the engine uses to slide between its imperfect- and perfect-information gears — and as of this revision, that dial is wired end to end.*
