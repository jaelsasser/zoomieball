# Zoomieball — design

Status: v0, 2026-07-20. This document records the architecture and decisions
acknowledged to date. Undecided surfaces remain under [Open items](#open-items);
working values are marked `provisional`. [GAME_TICK.md](GAME_TICK.md) is normative
for runtime ordering and deterministic simulation behavior. This document is
normative for design intent.

## Problem

Zoomieball combines three systems that are easy to make independently and hard to
keep coherent together: a deterministic fixed-point sport, recurrent learned
controllers, and a GPU-resident renderer. A GPU-only implementation lacks a portable
oracle and excludes machines without usable WebGPU. A CPU-only implementation cannot
meet the 100v100 target. A playbook-only controller omits the Zoomie Net control path
that turns formation intent into embodied behavior.

The repository therefore retains the existing CPU simulation, perception,
controller, playbook, renderer seam, and headless runner as production architecture.
The CPU path is the conformance oracle, the permanent 10v10 compatibility tier, and
the lockstep shadow used to bring up the WebGPU path. GPU authority replaces the
shadow only after physics, controller inference, and learning witnesses agree.

The current implementation is an alignment/M0 tracer. It exercises the CPU path from
play selection through perception, controller output, physics, rewards, witnesses,
and presentation, but it does not yet conform to every arithmetic, arena, collision,
playbook, or GPU requirement below. Unimplemented ownership is tracked by the root
roadmap and one `TODO.md` in each workspace package. No current GPU feature is implied
by this document merely because it is part of the final architecture.

## System shape

Zoomieball is a Rocket-League-shaped team sport played by billiard balls. The player
authors a playbook; two teams of 10 or 100 uniform spheres (stretch: 1000 per side)
execute it, moving only through spin and cue-strike impulses, trying to put one
neutral game ball into the opposing goal.

The controller and simulation share one fixed schedule on every execution tier:

```text
60 Hz match/play input
          │
          ▼
graph-v0 assignment + oracle intent ───────────────────────────────┐
          │                                                        │ refresh
          ▼                                                        │ at 120 Hz
180° perception ──► 15 Hz coaches ──► squad mailboxes/edge logits │
          │                    │                                   │
          └────────────────────┴──► 60 Hz fielders + goalies       │
                                      │ residuals + cue gates       │
                                      ▼                             ▼
                               motor decode ◄────────────── oracle steering
                                      │
                                      ▼
                           2 × 120 Hz physics substeps
                                      │
                         rewards / learning / witnesses
                                      │
                         ┌────────────┴─────────────┐
                         ▼                          ▼
              packed CPU snapshot          GPU-resident buffers
                         │                          │
                 Canvas2D/raw render          raw WebGPU render
```

Perception and embodied networks pulse at 60 Hz. Coaches pulse every fourth body
tick, at 15 Hz, and publish mailboxes early enough for the same tick's body networks
to consume them. Oracle steering, motor combination, and physics run at 120 Hz. A
device may not select a different controller rate. The exact order is specified in
[GAME_TICK.md](GAME_TICK.md#tick-order).

## Vision

Tone anchor, acknowledged verbatim:

> Zoomieball is a broadcast from a white infinity cove — an arena made of light instead
> of lines, where two teams of small glossy jersey-striped spheres spell out their
> physics in visible spin and cast shadow beneath floating Helvetica, and the only true
> blacks on the field are the goal mouths and the one ball that belongs in them.

“Glossy” predates the flat-two-tone finish decision; the sentence otherwise stands.

What the ball sees and feels, acknowledged verbatim:

> You are a hand's-width of lacquer on a plain that outruns your horizon. The floor is
> the one certainty — white, endless, faintly warm — and your entire vocabulary is
> torque against it: grip translates your spin into travel, and you know the world
> mostly through that handshake. Distance arrives as shading, never as edges: the walls
> don't approach so much as the floor begins to lean, a slow rising pressure with no
> seam you could point to, until *down* has quietly rotated and you find you are
> climbing. Teammates are far small domes wearing your colors, readable by the tilt of
> their stripes; the enemy goal is a black slit lying on the horizon like a held
> breath. When you jump, the handshake ends — contact goes silent, gravity takes the
> wheel, and the only steering you have left is whatever curve your own spin can buy
> from the air, until the floor (or a wall that has become a floor) takes your hand
> again.

What the person sees, acknowledged verbatim:

> A high orthographic vantage with a slight, fixed shear — enough to stand the goal
> mouths up as dark slits and give the cove its lean, never enough to cost the
> composition its poster flatness. The arena fills the frame: a white field defined
> entirely by tangent shadows and corner pooling, marked in ink only at the center. The
> balls are small — a fortieth of the arena's width, crisp anti-aliased discs — so the
> person reads the *swarm* where the ball reads the *surface*: formations as
> constellations, plays as dashed intentions, aerials as shadows detaching from their
> owners, spin as bands swimming across beads of color. Numbers exist only when
> summoned, projected above the field in team ink.

The person-view text predates the hard-light revision: cast-shadow creases and
contour loops replace gradient bands. The reading distances and scale ratios stand.

## Game and movement

- Teams contain 10 or 100 active bodies per side; 1000 per side is a stretch goal.
  All balls share one radius and mass. One neutral game ball scores.
- Movement consists of a continuous commanded spin target plus cue-strike impulses.
  A cue hit has impulse direction **d**, surface hit normal **n**, and magnitude J,
  giving Δv = J·**d** and Δω = (5/2)·J·(**n**×**d**) at r = m = 1.
- Jump is the surface-normal cue preset. An airborne body has one configured air cue.
  One surface charge and one air charge are restored on any arena contact. Magnus
  force k(ω×v) provides airborne steering.
- Goals are dugouts: recessed boxes subtracted from the arena SDF behind each end
  wall. The scoring predicate—game-ball center crossing the mouth plane or the whole
  ball passing it—remains open and belongs to the RON match rules.
- Ball numbers are HUD-space projections, never paint on a ball. Hover shows one;
  alt-hold or a persistent setting shows all. A label flips below its ball when the
  normal position intersects another label.

## Zoomie Net control

The playbook supplies a deterministic oracle and Zoomie supplies bounded learned
residuals. Neither can silently replace the other. This split keeps a match legible
as authored tactics while allowing local adjustment learned from perception and
reward.

### Populations and perception

The deterministic perception builder covers the full 180° field and emits
target-directed CSR observations in canonical body order. Its spatial-grid
implementation and brute-force oracle must remain equivalent; occlusion,
distant-target inclusion, fovea behavior, and lane layout are conformance surfaces.
The same observation contract feeds three network families:

| Population | Physical members | Pulse rate | Responsibility |
|---|---:|---:|---|
| fielder | team bodies with fielder roles | 60 Hz | local steering residuals and cue gates |
| goalie | team bodies with goalie roles | 60 Hz | goal-oriented steering residuals and cue gates |
| coach | nonphysical team coaches | 15 Hz | squad mailboxes and graph edge logits |

Coaches run before embodied populations on due ticks. Their squad mailboxes are
published and consumed in that same body tick; an implicit one-tick delay is a
conformance failure. Edge logits participate only through graph-v0's defined coach
edge semantics.

### Playbook and motor combination

There is one cyclic graph-v0 schema, not a prototype and production pair. Each node
will be extended in place to hold triggers, per-ball verb/target tables, squad
assignments, oracle intent, and coach edge semantics. RON is the at-rest form and
flat tables are the execution form. Fixture plays use the same schema that the later
editor writes. No migration reader is required for Zoomieball-local v0 artifacts.

At the start of a body tick, graph-v0 resolves assignments and initial oracle intent.
Body networks receive that intent with perception and the current squad mailbox.
Their outputs latch one learned steering residual and cue gates for the body tick.
Before each 120 Hz physics substep, the oracle refreshes steering from current state;
the motor decoder combines that fresh oracle value with the latched learned output.
The learned term is therefore not recomputed at physics frequency, and the oracle is
not held stale for both substeps.

Trigger vocabulary and per-verb parameter shapes are still open. Checked-in plays
beyond fixtures remain blocked until those shapes are acknowledged.

### Rewards, learning, checkpoints, and inspection

Physics progress and match events accumulate typed rewards after each substep.
Learning runs only at its declared deterministic schedule, after reward accumulation;
its state has a witness separate from inference state. Checkpoints contain the
fielder, goalie, and coach populations plus schedule and learning state needed for an
exact continuation. The current `CheckpointHeader` carries `schedule_abi = 1` so a
checkpoint cannot silently cross the 60/15/120 schedule boundary. Zoomieball-local
checkpoint headers change in place during v0. The established sibling Zoomie wire
formats remain authoritative and are not revised by this repository.

Inspection is part of the controller boundary rather than a render dependency. The
headless runner exposes replay witnesses and first divergence; the perception
inspector exposes the exact CSR observations; controller inspection exposes outputs,
mailboxes, edge logits, population checksums, learning checksums, and checkpoint
round-trips.

## Execution tiers

| Tier | Authority | Presentation | Scale and purpose |
|---|---|---|---|
| CPU compatibility | scalar CPU simulation, perception, Zoomie inference, and learning | one packed cosmetic snapshot per body tick; Canvas2D or raw renderer | permanent 10v10 fallback, headless execution, conformance oracle, replay debugging |
| GPU bring-up | CPU shadow supplies commands while GPU physics advances in lockstep | GPU-resident renderer; diagnostic hash readback | stage-by-stage WGSL physics parity and first-divergence bisection |
| GPU primary | GPU physics plus GPU Zoomie schedules and Zoomieball-specific control | resident state buffers, with no authoritative-state upload or readback | shipping WebGPU path and 100v100 target |

The GPU Zoomie schedule belongs in a generic sibling `zoomie-gpu` crate. It implements
Zoomie families, gates, stepping, learning, outputs, and checksums, and remains
bit-identical to sibling Zoomie's serial/population oracle. `zoomieball-gpu` retains
game-specific perception, topology selection, rewards, squad mailboxes, graph
integration, and motor decoding. This boundary preserves Zoomie's established wire
formats while avoiding a game-specific fork of its arithmetic.

The primary WebGPU path loses the CPU shadow only after physics, controller, and
learning parity all pass. It then evaluates game state in resident buffers and gives
those buffers directly to the renderer. Small event and witness readbacks remain
diagnostic or application-facing; authoritative state is neither uploaded for
rendering nor read back for control.

## Workspace components

The workspace contains seven green Zoomieball packages. Dependency-heavy integrations
remain unpinned until their milestone starts.

| Package | Responsibility |
|---|---|
| `zoomieball-core` | CPU scalar simulation, fixed math, play graph, perception oracle, typed controller batches, rewards, replay witnesses |
| `zoomieball-controller` | CPU Zoomie populations, input/output encoding, learning, squad mailboxes, checkpoints |
| `zoomieball-gpu` | Zoomieball-specific GPU physics, perception, generic GPU-controller integration, and CPU/GPU parity harness |
| `zoomieball-render` | controller-independent raw renderer with CPU-snapshot and GPU-resident input paths |
| `bevy-zoomieball` | published Bevy wrapper and example application |
| `zoomieball-web` | WebGPU application, Canvas2D compatibility presenter, and DOM HUD |
| `zoomieball-headless` | native/WASI CPU runner, replay debugger, benchmark, and later CPU/GPU parity driver |

`zoomieball-core` does not own cosmetic `f32` snapshots. The render layer owns their
packing and publication. A CPU frame performs one packed snapshot publication. A GPU
frame consumes resident simulation buffers without an authoritative-state upload or
readback. Render-only orientation quaternions remain `f32`, non-authoritative, and
excluded from replay witnesses.

## Visual language

- One hard directional light drives cast-shadow azimuth, each ball's shading
  terminator, and its specular-dot position.
- Shadows are flat, hard-edged, single-tone ellipses drawn as a union. Under parallel
  light a sphere's shadow neither shrinks nor fades with altitude; height appears as
  lateral offset h·cot(elevation). A shadow folds, compresses, and changes heading
  where it crosses the floor-wall tangent. The walls are not drawn.
- The arena is a white rounded-box SDF with a large cove fillet. Ink appears only as
  the center line, center circle, two black goal apertures, and up to two contour
  loops. Tangent and waterline loops have a four-way runtime toggle:
  `none | tangent | waterline | both`. The center line ends in cove hooks.
- Ball finish is flat two-tone: body-lit, body-shade, band-lit, and band-shade. Both
  teams are jersey-striped: home uses red body and paper band; away uses paper body
  and blue band. The game ball alone is solid ink, with one white spin telltale and
  the specular dot.
- The palette is a token table. Dark-mode inversion swaps tables and introduces no
  ramps. Working values remain provisional:

  | Token | Value | Token | Value |
  |---|---|---|---|
  | paper | `#FFFFFF` | ink | `#111111` |
  | shadow | `#E7E5DE` | — | — |
  | home.body.lit | `#7A2222` | home.body.shade | `#4E1414` |
  | home.band.lit | `#F5F2EA` | home.band.shade | `#C7C2B2` |
  | away.body.lit | `#F7F5EF` | away.body.shade | `#D2CFC4` |
  | away.band.lit | `#1E3A66` | away.band.shade | `#132242` |
  | game.lit | `#1E1E1D` | game.shade | `#050505` |

- Type is Helvetica-class throughout, with a lowercase-leaning HUD. Helvetica needs
  an embedding license; Inter, Neue Haas Grotesk, and Arimo remain shipping
  candidates. HUD text uses HTML wherever a DOM overlay exists.

## Camera and projection

The compatibility camera uses the provisional affine projection
`sx = x − 0.12·y`, `sy = 0.62·y − 0.785·z`. The near-orthonormal 0.62/0.785 pair
keeps projected spheres circular within about 2%. The shear stands goal apertures up
as slits and gives shadows a consistent stage. WebGPU adds a free 3D camera; Canvas2D
keeps this fixed oblique view. Sphere-to-circle invariance makes cached appearance a
function of team and orientation rather than screen position.

## Why it is shaped this way

- **Fixed-point integers make conformance algebraic.** Q16.16 with exact widened
  intermediates avoids floating-point reassociation and contraction differences.
  Native, WASM, and WGSL replays can agree by value rather than compiler discipline.
- **The CPU implementation is permanent infrastructure.** It is the scalar oracle,
  headless implementation, fallback tier, and source of golden replays. Retaining it
  makes GPU bring-up observable and gives 10v10 users a complete game without WebGPU.
- **The oracle/residual split preserves authored intent.** Graph-v0 and refreshed
  oracle steering express the play; the learned term reacts locally. Latching only
  the learned output gives bodies 60 Hz perception without making 120 Hz physics use
  stale geometric steering.
- **Jacobi plus integer atomics makes pair scatter deterministic.** Wrapping addition
  is commutative. Atomically built cell lists still have nondeterministic order, so
  broadphase output is consumed as a set and cells are never capacity-capped. A later
  grid uses counting sort and prefix scan.
- **Layered witnesses localize divergence.** A commutative physics hash answers
  whether canonical body state differs; controller and learning checksums identify
  network divergence; a whole-pipeline fold aids diagnostics without pretending all
  layers have the same conformance semantics.
- **GPU residency avoids the expensive boundary.** Work that consumes current body
  state remains on the GPU in the primary tier. The raw renderer accepts those
  buffers directly, while the CPU tier pays for exactly one cosmetic publication.
- **The arena SDF is shared physics and lighting geometry.** Collision, cove contact,
  goal volumes, and shadow creases derive from one shape and cannot drift apart.
- **Hard flat shadows preserve the poster grammar at low cost.** Unioned single-tone
  ellipses work on both render tiers. Parallel-light offset encodes altitude without
  a small-versus-high ambiguity.
- **Small balls leave most pixels static.** At roughly 1/40 arena width per ball,
  dirty-rectangle Canvas2D painting and DPR-bucket sprite caching are viable. Below
  roughly 10 px radius, vector arcs may beat sprite blits and must be measured.

Known numeric limits remain: WGSL has no i64 or integer `mulhi`, so wide products use
16-bit limbs. The compatibility tier omits the free camera and large team sizes.
Render-side orientation may drift visually between clients because it is deliberately
non-authoritative.

## Runtime flow

One 60 Hz body tick latches match and play input, resolves graph-v0, builds perception,
optionally runs coaches, runs body networks, and latches learned outputs. Each of its
two 120 Hz substeps refreshes oracle steering, combines it with the latched output,
and executes the fixed physics stages. Afterward the runtime accumulates rewards,
runs due learning, publishes all witness layers, and exposes presentation state.
[GAME_TICK.md](GAME_TICK.md#tick-order) defines the complete ordering.

The witness layers are:

| Witness | Semantics |
|---|---|
| `TickHash.physics: u32` | normative `World::physics_hash()` over the ten canonical GPU/physics words per body |
| `TickHash.controller: u64` | Zoomie inference, parameter, and transient-state checksum |
| `TickHash.learning: u64` | Zoomie eligibility and learning-rule checksum |
| `TickHash.pipeline: u64` | diagnostic fold including `World::diagnostic_hash()`, ABI/version inputs, play state, and the three preceding witnesses |

## Milestones

| Milestone | Completion state and boundary |
|---|---|
| Alignment | Seven green packages, ownership TODOs, reconciled docs, 60/15/120 clocks, render-owned cosmetic snapshots, and honest tracer labels |
| M0 — conforming CPU | Public tracer from graph selection through presentation; exact arithmetic and constants; canonical words; arena, motor, contacts, Jacobi pairs, caps, events; extended graph-v0; native/WASM/WASI golden witnesses |
| M1 — CPU compatibility | Fixed-camera Canvas2D over CPU snapshots; complete 10v10 physics, perception, Zoomie inference/learning, HUD, labels, feel tuning, and realtime benchmark |
| M2a — GPU physics shadow | WGSL fixed helpers and physics stages; CPU-produced commands; per-step physics parity; first-divergence and stage bisection |
| M2b — GPU Zoomie | Generic sibling GPU schedules bit-identical to Zoomie's serial oracle; Zoomieball integration; controller and learning parity; shadow removal only after all witnesses pass |
| M3 — rendering | Raw renderer, GPU-resident source, hard-light arena, Bevy wrapper, free camera, contours, and perception inspector |
| M4 — playbook | GPU graph-v0 evaluation and checked-in plays after trigger and verb shapes are acknowledged |
| M5 — application | WebGPU shell, tier selection, DOM HUD, Canvas2D fallback, import/export, and device profiling |

The CPU target is realtime 10v10 including Canvas2D, perception, inference, and
learning. The WebGPU target remains 100v100. Formatting, workspace tests, Clippy with
warnings denied, native builds, and WASM checks gate each completed bite.

## Settled constraints

1. CPU and GPU implementations use deterministic Q16.16 state and one fixed update
   schedule: 60 Hz perception/body, 15 Hz coaches, and 120 Hz oracle/motor/physics.
2. The scalar CPU implementation remains a supported conformance, headless, and
   compatibility path. WebGPU is the primary large-match path.
3. Playbook oracle steering and learned Zoomie residuals both participate in control.
   Fielders, goalies, and coaches are distinct populations.
4. Coaches publish same-tick squad mailboxes and graph edge logits before body
   evaluation.
5. One graph-v0 schema is extended in place. Zoomieball-local v0 formats and fixtures
   have no migration requirement; sibling Zoomie's established formats do.
6. Physics uses fixed stage order and Jacobi pair solving with integer commutative
   accumulation.
7. The renderer is controller-independent. CPU presentation consumes one cosmetic
   packed snapshot; GPU presentation consumes resident buffers.
8. Conformance uses separate physics, controller, and learning witnesses plus a
   diagnostic pipeline fold.
9. The cove arena, dugout goals, aerial cue, Magnus force, hard light, flat shadows,
   contour toggles, striped teams, solid ink game ball, and HUD-only numbers remain
   the visual and physical direction.
10. RON playbooks compile to flat execution tables; a later editor writes the same
    schema.

## Open items

- Scoring predicate: center crossing versus the whole game ball passing, expressed as
  a RON match rule.
- Arena dimensions by team size, dugout interior dimensions, and ceiling height.
- Default contour-loop mode.
- Final physics constants after M1 feel tuning.
- Final palette tokens and shipped typeface.
- Bevy/wgpu version pins and publication policy; scaffolding chooses neither.
- Software-adapter and no-WebGPU user messaging.
- Graph-v0 trigger vocabulary, per-ball verb parameter shapes, formation definitions,
  and the exact set of checked-in plays.
- Whether a rust-gpu-to-naga single-source spike is viable. Hand port plus parity
  remains the critical-path plan.
