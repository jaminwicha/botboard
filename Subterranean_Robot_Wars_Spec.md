# Subterranean Robot Wars — Game & Systems Specification

**The roguelike/metroidvania campaign layer that builds on *Robot Wars* and the Botboard engine.**

`Working Draft · Version 0.1 · June 2026`

**Companion document.** This spec assumes the [Botboard Technical Specification](#) and references it throughout (cited as *Botboard Spec §N*). It does **not** re-describe the engine; it specifies the game and campaign systems layered on top, and is explicit about which engine capability each system leverages.

---

## Table of Contents

1. [Introduction & Position in the Stack](#1-introduction--position-in-the-stack)  
2. [What It Inherits: the Robot Wars Combat Substrate](#2-what-it-inherits-the-robot-wars-combat-substrate)  
3. [Design Pillars & Player Fantasy](#3-design-pillars--player-fantasy)  
4. [Campaign Structure: Roguelike Meets Metroidvania](#4-campaign-structure-roguelike-meets-metroidvania)  
5. [The Core Loop](#5-the-core-loop)  
6. [Rooms & Encounters](#6-rooms--encounters)  
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
- [Appendix B — Example: a Generated Robot Card](#appendix-b--example-a-generated-robot-card)  
- [Appendix C — Glossary](#appendix-c--glossary)

---

## 1\. Introduction & Position in the Stack

*Subterranean Robot Wars* (SRW) is a single-player, run-based roguelike with metroidvania exploration, in which every fight is a tactical robot battle resolved by the Botboard engine. The player is a newly-conscious controller robot descending through an underground world of warring machines, defeating and dismantling robots to rebuild their own army and pressing deeper.

The product is three layers, each owning a distinct concern:

┌────────────────────────────────────────────────────────────────────────┐

│ SUBTERRANEAN ROBOT WARS — roguelike / metroidvania campaign              │

│   dungeon of rooms · runs & meta-progression · card collection ·         │

│   controller narrative · factions · fog-as-discovery · economy           │

└────────────────────────────────────┬───────────────────────────────────┘

                                     │ composes encounters out of

┌────────────────────────────────────▼───────────────────────────────────┐

│ ROBOT WARS — the robot-combat game                                       │

│   one battle \= one Botboard game between robot armies on a room-board     │

│   armies · controllers (royal pieces) · terrain · abilities · win cond.   │

└────────────────────────────────────┬───────────────────────────────────┘

                                     │ runs on

┌────────────────────────────────────▼───────────────────────────────────┐

│ BOTBOARD — the headless engine  (see Botboard Spec)                      │

│   Bots/Bits · move-gen · learned eval · belief-state AI · self-play       │

└──────────────────────────────────────────────────────────────────────────┘

The division of responsibility is strict and worth stating, because it is what keeps SRW buildable: **Botboard** decides what is legal and who wins a position; **Robot Wars** is the framing of a single battle as a game (which armies, on which board, under which win condition, with which controllers); **SRW** is everything outside the battle — how battles are strung into a dungeon, what the player keeps, how they grow, and why they care.

SRW adds **no new rules of movement or capture.** Every robot, ability, terrain feature, and win condition it uses is expressed in the engine's existing Bit/policy vocabulary (*Botboard Spec §3, §5*). What SRW contributes is structure, economy, progression, narrative, and content — and a set of engine capabilities re-pointed from "play strongly" to "generate and balance a game."

---

## 2\. What It Inherits: the Robot Wars Combat Substrate

A **Robot Wars battle** is one Botboard game. SRW never re-implements combat; it instantiates battles and reads their outcomes. The substrate it leans on:

| Robot Wars concept | Botboard mechanism it is built from |
| :---- | :---- |
| A robot | a **Bot** — a set of Bits (*Botboard Spec §3*) |
| A robot's powers | its **Bits**: geometry (Axis A) and effects (Axis B) |
| The room you fight in | a **board**, possibly variable-size or arbitrarily-shaped, with terrain |
| Walls, pits, doors | **terrain** in the occupancy/terrain layer (*Botboard Spec §7.1*) |
| Lasers, shields, hacking, resurrection, spying, wall/pit creation | **Axis-B effects** (*Botboard Spec §3.2*) |
| Armor, immunities | **capture semantics** (hit-count, immunity flags) |
| The player / each enemy commander | a **controller** \= a royal piece (*Botboard Spec §5*, royalty-as-attribute) |
| A multi-commander brawl | **3–4 player** play (*Botboard Spec §8*) |
| Not knowing an enemy's powers | **hidden move-sets / public belief states** (*Botboard Spec §8*) |
| The enemy commander's tactics | the engine's **belief-state AI** |
| "Is this army fair?" | the **cost model** and **self-play** measurement (*Botboard Spec §4, §9*) |

Because the substrate is the engine, SRW content is authored in the engine's terms: designing a new enemy robot *is* defining a Bit-set; designing a hazard room *is* laying out terrain; tuning difficulty *is* selecting armies and controllers. This is the central efficiency of the whole project — the game's content pipeline and the engine's rule system are the same system.

---

## 3\. Design Pillars & Player Fantasy

- **You are a mind in a machine.** The player wakes with true consciousness into a world of robots that merely execute. Being a *controller* — able to command, hack, and rebuild robots — is the core power fantasy and the diegetic justification for every system.  
- **Defeat is dismantling.** Winning a battle yields the parts (Bits) and sometimes the whole army of the defeated. Growth is literally built from fallen enemies.  
- **Knowledge is power.** You do not know what an enemy robot can do until it acts. Scouting, spying, and remembering enemy movesets is a real axis of advantage — the imperfect-information core of the engine surfaced as play.  
- **Descent with consequence.** A roguelike run is a descent; death costs the run's army but feeds meta-progression. The world is a connected, ability-gated space (metroidvania), not a linear ladder.  
- **Emergent, not scripted, opposition.** Enemy clans are driven by the engine's AI and fight each other as readily as the player; encounters arise from systems, not hand-authored scripts.

---

## 4\. Campaign Structure: Roguelike Meets Metroidvania

**The world** is a subterranean network organized by **depth**, divided into **sectors/biomes** (e.g. flooded foundries, collapsed archives, reactor depths) that differ in terrain palette, enemy clans, hazards, and aesthetic. Each sector is a graph of **rooms** connected by **corridors**.

- **Rooms are boards.** Entering a combat room starts a Robot Wars battle on that room's board (*§6*).  
- **Corridors are the map graph.** Branching paths let the player choose risk/reward routes, roguelike-style.  
- **Metroidvania gating.** Some passages require a capability — a robot in your army with **drill** (remove/traverse walls), **flight** (cross pits), a **hack** tier, or a key-item — mirroring the terrain-permission Bits (*Botboard Spec §3.1*). Acquiring a new robot can therefore unlock previously-blocked regions, within and across runs.

**Two persistence horizons:**

- **Within a run:** your collected robots, current army, consumables, and revealed knowledge persist room-to-room and compound. The run is the unit of escalating power.  
- **Across runs:** death (or extraction) ends the run; what carries over is **meta-progression** (*§11*) — unlocked Bits/robots in the pool, new starting options, opened biomes, persistent upgrades, and the codex of discovered robots.

Dungeons are **procedurally generated** (*§12*) within sector-specific rules, so the topology, encounters, and rewards differ each run, while metroidvania landmarks (locked vaults, sector gates) remain legible.

---

## 5\. The Core Loop

Three nested loops, each feeding the next outward:

  MOMENT  (a room)             RUN  (a descent)               META  (across runs)

  ┌────────────────┐           ┌──────────────────┐           ┌──────────────────┐

  │ enter room     │           │ choose a path     │           │ unlock Bits /     │

  │  → battle or   │  ───────► │ descend deeper     │ ───────► │  robots / biomes  │

  │    event       │           │ collect Bits/cards │           │ persistent upgrades│

  │  → take loot   │ ◄───────  │ rebuild your army  │ ◄─────── │ codex / mastery   │

  │  → reveal foes │           │ die or extract     │           │ harder ascensions │

  └────────────────┘           └──────────────────┘           └──────────────────┘

- **Moment loop:** approach a room, read what you can (fog permitting), fight or resolve an event, collect loot, update your knowledge of enemy robots. Tactical, one Botboard battle.  
- **Run loop:** navigate the sector graph, spend rewards on your army, manage attrition, decide when to push deeper versus consolidate, and end the run on death or a chosen extraction. Strategic, deckbuilding-flavored.  
- **Meta loop:** convert run results into permanent unlocks that widen future runs. Progression, mastery, and difficulty laddering.

---

## 6\. Rooms & Encounters

A room is generated with a **board layout** (size, shape, terrain), an **encounter** (what is in it), and a **reward**. Room archetypes:

| Room type | Battle? | Description / leverage |
| :---- | :---- | :---- |
| **Skirmish** | yes | a standard Robot Wars battle vs a clan army sized to current depth. |
| **Elite / Commander** | yes | an enemy **controller** present; defeating it can win its whole army (*§7*). |
| **Boss** | yes | a unique, high-cost robot or a multi-controller set-piece; sector gate. |
| **Ambush** | yes | starts in **darkness/fog**; enemies hidden until they act (*§10*). |
| **Puzzle / Vault** | no or scripted | terrain \+ ability gating with no live enemy — solved with drill/flight/hack; rewards behind it. |
| **Forge** | no | spend resources to combine/upgrade **Bits** onto your robots (*§8*). |
| **Shop / Cache** | no | acquire specific robots or Bits; deterministic choice vs random loot. |
| **Shrine / Event** | no | risk/reward choices, blessings/curses (run-scoped modifiers), lore. |

**Difficulty scaling** is expressed in the engine's own terms: deeper rooms field armies of higher total **cost** (*Botboard Spec §4*), stronger controllers (more search/quality), and harsher terrain. Because army strength is measured by the cost model and validated by self-play, the game can dial difficulty by a single legible knob — the army's cost budget and the controller's competence — rather than hand-tuned stat blocks.

---

## 7\. The Controller & Commanding Your Army

The player is a **controller robot**, present on the battle board as a royal piece (*Botboard Spec §5*). This unifies narrative and mechanics:

- **Commanding.** In a battle the player moves their robots; the controller is one piece among them, with its own movement and abilities.  
- **Controller abilities.** The diegetic powers of a controller are Axis-B effects (*Botboard Spec §3.2*): **hack** (flip an enemy robot to your side, gated by a target predicate and a success chance), **resurrect** a fallen ally, **spy** (reveal an enemy's full move-set), **create walls / dig pits** to reshape the board. These are the controller's "spells," costed and limited.  
- **Stakes.** If the player's controller is captured/destroyed, the run (or the battle) is lost — this is the royal-capture win condition repurposed as run-ending tension. The royalty policy is per-piece, so battles can also be loss-on-controller-capture while ordinary robots fight on.  
- **Capturing enemy controllers.** Defeating an enemy controller can transfer its army to the player at a **randomized success rate** — the headline reward of elite rooms and the primary engine of mid-run power spikes.

**Army management** between battles: the player holds a **roster** (collection) and **deploys** a subset as the battle army, subject to a deployment budget. Deployment, the bench, and the hand for drops (*Botboard Spec §3.3*) are all surfaced as the controller's logistics.

---

## 8\. Card Collection & the Bit Economy

Robots are **collectible cards**; a card is a **Bot**, i.e. a Bit-set (*Botboard Spec §3*). The collection/economy is the roguelike's deckbuilding spine.

**Acquisition**

- **Salvage:** defeating a robot drops one or more of its **Bits**, which can be attached to your robots at a **Forge**.  
- **Capture:** hacking or capturing a robot/controller yields whole robots or armies.  
- **Shops/caches:** targeted acquisition of specific robots or Bits.  
- **Drafts:** between rooms, choose one robot/Bit from a small offered set (classic roguelike pick).

**Army-building constraints**

- A **cost budget** caps deployable army strength, using the engine's **cost prior corrected by self-play** (*Botboard Spec §4, §9*). This is how heterogeneous, player-assembled armies stay fair without per-card hand-balancing.  
- **Rarity** maps to cost/complexity tiers: common robots are cheap, low-Bit-count Bots; rare/legendary robots carry expensive or exotic Axis-B effects. Rarity is therefore *derived* from the cost model, not assigned arbitrarily.

**Synthesis (Forge).** Combining Bits onto a chassis builds bespoke robots — e.g. grafting a **laser** onto a fast **rider**, or **armor** onto a cheap blocker. The cost model prices the result on the fly, and self-play-derived values warn when a combination is over/undertuned, giving the player meaningful, legible build decisions.

---

## 9\. Factions, Clans & Alignment

The underground is populated by mutually-opposed **robot clans**, realized through the engine's **3–4 player free-for-all AI** (*Botboard Spec §8*). Clans are not just reskins:

- **Clan identity \= a signature Bit palette.** A clan has themed armies — e.g. a hopper/cannon artillery clan, a stealth/cloak infiltration clan, a wall-building siege clan — built from characteristic Bits. This makes clans mechanically distinct and teachable.  
- **Wandering, self-interested AI.** Clans roam, hold territory, and fight each other and the player. Encounters can be three-way: arriving mid-battle between two clans is a tactical opportunity (the Pluribus-style multiplayer regime, *Botboard Spec §8*).  
- **Alignment & reputation.** Player actions shift standing with clans; high standing can enable temporary alliances or safe passage, low standing invites ambush. Betrayal and shifting alliances emerge from the multiplayer AI rather than scripts.

This section is a direct consumer of the engine's hardest AI capability: **imperfect-information, multiplayer, hard-to-exploit play**. The free-for-all that is a research challenge in the engine is, here, the source of a living world.

---

## 10\. Information, Fog & Stealth as Gameplay

The engine's defining mechanic — **a piece's rules are hidden until observed** (*Botboard Spec §8*) — is SRW's signature gameplay system, not a backend detail.

- **Discovery as play.** You do not know an unfamiliar robot's powers until it moves, attacks, or you reveal it. Early turns against new enemies are about *learning*, and a known enemy is a weaker enemy.  
- **Scouting & spying.** **Spy** abilities collapse a belief to certainty (reveal a full move-set); positioning to bait an enemy into revealing itself is a skill.  
- **Fog & darkness rooms.** Some rooms limit vision to a range or hide regions; **stealth/cloak** Bits conceal robots; counter-abilities reveal them. These are visibility modifiers on the engine's public state (*Botboard Spec §8*).  
- **The codex (knowledge meta-progression).** Robots you have identified are recorded in a **bestiary**; their movesets stay revealed in future encounters of that type. Accumulated knowledge is a persistent form of power, tying the fog system to meta-progression (*§11*).

Design intent: make *information itself* a resource the player gathers, spends, and is denied — the thing that distinguishes SRW from a deterministic tactics roguelike, and the most direct payoff of building on Botboard specifically.

---

## 11\. Progression & Meta-Progression

**Within a run (power):** the army strengthens as Bits and robots are collected and forged; the deployment budget may rise with depth milestones; revealed knowledge accumulates. Power growth is bounded by the cost budget so runs escalate without trivializing.

**Across runs (breadth & permanence):**

- **Unlock pool:** new Bits, robot chassis, and rarities enter the procedural and shop pools.  
- **Starting options:** new starting controllers/armies and run modifiers.  
- **World access:** opened sector gates and shortcuts (metroidvania persistence at the meta scale).  
- **Codex/mastery:** the discovered-robot bestiary and clan intel persist; mastery rewards for fully cataloguing a clan.  
- **Ascension ladder:** optional escalating difficulty modifiers (tougher cost budgets, smarter controllers, harsher fog) for replay depth.

Meta-progression deliberately widens *options and knowledge* rather than handing out flat power, so that mastery of systems — army-building under the cost model, reading the fog — remains the real progression.

---

## 12\. Procedural Content Generation

SRW is content-hungry; procedural generation is leveraged at three levels, all of which lean on the engine.

- **Procedural robots.** A generator samples valid, themed **Bit-sets** to produce new Bots, with the **cost model** keeping them within target power bands and clan palettes constraining style (*Botboard Spec §4*). A robot's stats are not invented — they are its Bits, priced by the engine.  
- **Procedural dungeons.** Sector rules generate the room graph, board layouts, terrain, and encounter/reward placement, preserving metroidvania landmarks (gates, vaults) for legibility.  
- **Procedural visuals.** Robots render as blocky 16-bit sprites generated to *communicate function* — a sprite's silhouette/motifs reflect its Bit profile (a ranged robot reads as ranged), so players can form hypotheses before the fog lifts. Generated in the manner of large generative collectible sets.

The key discipline: generated content is **pre-validated by the engine** before the player sees it (*§13*), so procedural variety never breaks balance.

---

## 13\. AI & Content Balancing via Self-Play

This is the decisive "leverage" of building on Botboard: the engine's **self-play farm** (*Botboard Spec §9*) is reused as SRW's **content-QA and difficulty pipeline**, not only as a way to play strongly.

- **PvE controllers** are the engine's **belief-state AI** at tuned competence tiers — weaker (shallower search, noisier policy) for early rooms, stronger for elites and bosses — giving a difficulty curve from one principled dial.  
- **Generated-content vetting.** New robots, armies, and dungeons are play-tested by self-play *before shipping into runs*: the same loop that measures piece values (*Botboard Spec §4*) flags over/undertuned robots, degenerate combinations, and unwinnable or trivial encounters.  
- **Cost-model feedback.** Realized win rates from self-play continuously correct the cost prior, which in turn keeps the army-building budget, rarity tiers, and procedural power bands honest as content grows.  
- **Live difficulty tuning.** Aggregate self-play and (later) player telemetry adjust encounter cost budgets and controller competence per depth, keeping the curve fair as the collection meta shifts.

In short, the machine-learning core earns its cost twice: once as the opponent, once as the balancing system that makes infinite procedural content shippable.

---

## 14\. Aesthetic & Presentation

- **16-bit / SNES-era pixel art** — a blocky, readable style for robots, rooms, and UI, evoking the classic 2D era.  
- **Subterranean atmosphere** — distinct sector palettes (foundry, archive, reactor) signaling depth, hazard, and clan territory.  
- **Readability first.** Because robots are procedural and their powers initially hidden, presentation must let players *read intent*: silhouette telegraphs role, revealed movesets show clearly, fog and stealth states are unambiguous, and the cost/threat of an army is glanceable.  
- **The controller UI** centers the command fantasy: roster, deployment, the hand for drops, and the controller's abilities as a clear, limited toolkit.

---

## 15\. Roadmap & Dependencies

SRW depends on Botboard/Robot Wars maturity. The engine phases (*Botboard Spec §13*) are the hard prerequisites:

 BOTBOARD prereqs ───────────────►  SRW build order

 P0 engine core            ┐

 P1 cost \+ embeddings       │  enable army-building \+ procedural balance

 P2 AI (oracle→belief→FFA)  │  enable PvE controllers \+ clans

 P3 fog / stealth / terrain ┘  enable discovery \+ gating \+ hazard rooms

**SRW's own build order:**

1. **Vertical slice.** One sector, skirmish \+ elite rooms, a small fixed robot pool, salvage→forge→deploy, a controller with two abilities, win/lose a short run. Proves the moment+run loops on real battles.  
2. **Full run loop.** Sector graph generation, room archetypes, drafts/shops, capture-the-controller, basic meta-unlocks.  
3. **Information layer.** Fog/darkness rooms, spy/stealth, the codex/bestiary. (Requires Botboard P3.)  
4. **Factions.** Multiple clans with signature palettes, wandering/three-way encounters, alignment. (Requires Botboard P2 multiplayer.)  
5. **Procedural content \+ self-play balancing.** Procedural robots/dungeons/visuals with self-play vetting; ascension ladder; meta-progression breadth.  
6. **Polish & economy tuning.** Telemetry-driven difficulty, rarity, and pacing.

Everything from step 3 onward is gated on the corresponding engine capability already existing and tested.

---

## 16\. Open Questions & Risks

- **Pacing of fog.** Discovery is the signature pleasure but can frustrate; how much should be hidden, and how cheap should revealing be? Needs playtesting.  
- **Collection power-creep vs the cost model.** As the unlock pool grows, can the cost budget \+ self-play balancing hold the line, or will dominant builds emerge? The balancing pipeline (*§13*) is the mitigation but must be proven.  
- **Procedural robot readability.** Can generated sprites reliably telegraph Bit profiles well enough for players to reason under fog?  
- **Run length & extraction.** What is the right run length, and should extraction (banking progress) exist, or is death the only exit? Affects the meta loop's feel.  
- **Three-way encounter clarity.** Multiplayer brawls are emergent and exciting but hard to read; UI and AI behavior must make them legible, not chaotic.  
- **Difficulty legibility.** Tuning by cost budget \+ controller competence is elegant but invisible to players; the game must communicate threat without exposing the machinery.  
- **Engine dependency risk.** SRW's most distinctive systems (fog-as-play, factions, procedural balancing) sit on the engine's hardest, latest capabilities; slips in Botboard P2/P3 directly gate SRW.

---

## Appendix A — Leverage Map

Every major SRW system and the engine capability it is built on. This table is the dependency spine of the design.

| SRW system | Leverages (Botboard) | Notes |
| :---- | :---- | :---- |
| A battle | a Botboard game | SRW never re-implements rules |
| Robots / cards | Bots \= Bit-sets (§3) | content \= engine vocabulary |
| Forge / synthesis | the cost model (§4) | prices bespoke builds live |
| Army-building budget, rarity | cost prior \+ self-play (§4, §9) | fairness without hand-balancing |
| Controller & abilities | royalty (§5) \+ Axis-B effects (§3.2) | hack/spy/resurrect/wall/pit |
| Capture-the-controller | royal capture \+ capture semantics | run power spikes |
| Terrain rooms & metroidvania gates | terrain layer (§7.1), path Bits (§3.1) | drill/flight/hack as keys |
| Fog, stealth, discovery | hidden move-sets / PBS (§8) | the signature system |
| Codex / knowledge meta | belief collapse via spy (§8) | knowledge as progression |
| Factions / clans / three-way | multiplayer belief AI (§8) | living, self-interested world |
| PvE difficulty tiers | belief-state AI competence (§8) | one principled dial |
| Procedural robots | Bit generator \+ cost model (§4) | stats \= priced Bits |
| Content vetting & balancing | self-play farm (§9) | QA for infinite content |

---

## Appendix B — Example: a Generated Robot Card

A worked example of how a single generated robot is fully described by engine primitives — no bespoke stats.

  ╔══════════════════════════════════════╗

  ║  SCUTTLE-LANCE          \[Rare\]        ║   Clan:  Foundry Siege

  ║  ────────────────────────────────────║   Cost:  \~4.1  (cost model)

  ║  Bits:                                ║   Role:  ranged skirmisher

  ║   • rider(0,1)        move-only       ║

  ║   • hopper(0,1)       capture-only    ║   Sprite telegraphs: long, low,

  ║   • laser  (range 2, coupled retreat) ║   barrel-forward  →  reads "ranged"

  ║   • armor  (2 hits)                   ║

  ║  Hidden until observed in play.       ║   Belief narrows as it moves/fires.

  ╚══════════════════════════════════════╝

- Its **stats are its Bits**; its **cost** is the engine's valuation of that set; its **rarity** follows from cost/complexity (*§8*); its **AI** is the shared belief-state controller; its **balance** was vetted by self-play before it entered the pool (*§13*); and to the player it begins **hidden**, revealed only through the fog system (*§10*).

---

## Appendix C — Glossary

| Term | Meaning |
| :---- | :---- |
| Robot Wars | the robot-combat game; one battle \= one Botboard game between robot armies. |
| Subterranean Robot Wars (SRW) | the roguelike/metroidvania campaign wrapping Robot Wars battles. |
| Controller | the commanding robot (a royal piece); the player is one. |
| Run | a single descent; the unit of escalating, then reset, power. |
| Salvage | Bits dropped by a defeated robot, attachable at a Forge. |
| Forge | a room/system for combining Bits onto robots. |
| Clan | a faction with a signature Bit palette, driven by the multiplayer AI. |
| Fog / discovery | the hidden-moveset system surfaced as gameplay (engine PBS). |
| Codex | the persistent bestiary of identified robots and revealed movesets. |
| Cost budget | the cap on deployable army strength, from the engine's cost model. |
| Ascension | optional escalating difficulty modifiers for replay. |

---

*Subterranean Robot Wars — Game & Systems Specification, Working Draft v0.1. Complementary to the Botboard Technical Specification; consumes the engine's Bit model, cost model, imperfect-information AI, and self-play farm as the foundations of a roguelike campaign.*  
