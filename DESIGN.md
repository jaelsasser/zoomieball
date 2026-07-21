# Zoomieball — design

Status: v0.1, 2026-07-19. Records decisions acked to date; items not yet decided are
listed under [Open items](#open-items) and marked `provisional` where a working value
exists. `TICK.md` is the normative simulation spec; where the two disagree, TICK.md wins
for simulation behaviour and this document wins for intent.

## Overview

Zoomieball is a Rocket-League-shaped team sport played by billiard balls. The player
authors a playbook; two teams of 10 or 100 uniform spheres (stretch: 1000 per side)
execute it, moving only through spin and cue-strike impulses, trying to put a single
neutral game ball into the opposing goal. The simulation is deterministic fixed-point
integer arithmetic resident on the GPU; the renderer is a stark hard-light poster:
white void arena, flat unioned shadows, two jersey-striped teams, one black ball.

Tone anchor, acked verbatim:

> Zoomieball is a broadcast from a white infinity cove — an arena made of light instead
> of lines, where two teams of small glossy jersey-striped spheres spell out their
> physics in visible spin and cast shadow beneath floating Helvetica, and the only true
> blacks on the field are the goal mouths and the one ball that belongs in them.

("Glossy" predates the flat-two-tone finish decision; the sentence otherwise stands.)

```
CPU (wasm, single-thread)                GPU (WebGPU)
─────────────────────────                ─────────────────────────────
play selection, HUD, camera ─ mailbox ─▶ playbook eval ─▶ 2× physics substeps
        ▲                     (≤2 KB/tick)                     │
        │                                                      ▼
  DOM overlay (score, clock,             state buffers (SoA, ping-pong)
  play bar, number labels)                    │                │
        ▲                                     ▼                ▼
        └──── async map (~102–103 B) ── event ring        render pass
              2–3 frame latency         + pick pass       (vertex pull,
                                                           no readback)
```

## Vision

What the ball sees and feels (acked verbatim):

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

What the person sees (acked verbatim):

> A high orthographic vantage with a slight, fixed shear — enough to stand the goal
> mouths up as dark slits and give the cove its lean, never enough to cost the
> composition its poster flatness. The arena fills the frame: a white field defined
> entirely by tangent shadows and corner pooling, marked in ink only at the center. The
> balls are small — a fortieth of the arena's width, crisp anti-aliased discs — so the
> person reads the *swarm* where the ball reads the *surface*: formations as
> constellations, plays as dashed intentions, aerials as shadows detaching from their
> owners, spin as bands swimming across beads of color. Numbers exist only when
> summoned, projected above the field in team ink.

(The person-view text predates the hard-light revision: "tangent shadows and corner
pooling" is now produced by cast-shadow creases and contour loops rather than gradient
bands. The reading distances and scale ratios stand.)

## Game

- Teams of 10 or 100 per side; 1000 per side is a stretch goal. All balls share one
  radius and mass; a single neutral game ball scores.
- Control is a playbook: RON files at rest, compiled at load into flat GPU tables
  (per-ball `{verb_id, target_kind, target_idx, params[4]}` plus a phase-transition
  table). Verb set is provisional: seek, intercept, screen, shoot-at, clear; the set
  stays under a dozen. Plays are phases with roles assigned by ball index and
  triggers (zone entry, possession change, time). An in-game editor writes the same
  RON schema later; hardcoded RON fixtures drive development.
- Control is layered by data locality: per-ball steering runs on the GPU every tick
  (it reads ball positions); coarse play selection runs on the CPU off the delayed
  event stream, where ~50 ms of staleness is acceptable.
- Movement verbs per ball: continuous commanded spin (a slewed target angular
  velocity), plus cue-strike impulses parameterized as a billiards hit — impulse
  direction **d**, surface hit normal **n**, magnitude J, giving Δv = J·**d** and
  Δω = (5/2)·J·(**n**×**d**) at r = m = 1. Jump is the surface-normal preset; the
  air move is a second configured cue hit. One ground jump plus one air move, both
  restored on any surface contact. Magnus force k(ω×v) provides air steering.
- Goals are dugouts: recessed boxes subtracted from the arena SDF behind each end
  wall. The scoring predicate (game-ball center crosses the mouth plane vs. fully
  past) is open and will be a RON match rule.
- Ball numbers are not on the balls. They exist as HUD-space projections above each
  ball — screen-constant size, team-colored numeral on a hairline stem — shown on
  hover (single ball), alt-hold (all), or a persistent setting. Label placement uses
  one declutter rule: flip below the ball on intersection.

## Visual language

- One hard directional light. It drives three things that therefore always agree:
  cast-shadow azimuth, the shading terminator on every ball, and the specular dot
  position.
- Shadows are flat, hard-edged, single-tone ellipses, drawn as a union (one color, so
  overlap is invisible). Under parallel light a sphere's shadow neither shrinks nor
  fades with altitude — height reads purely as lateral offset h·cot(elevation) along
  the light azimuth. Where a shadow crosses the floor–wall tangent it creases: folds,
  compresses, changes heading. The walls are never drawn; they are the thing that
  bends shadows, so the arena's shape is traced by play itself.
- The arena is a white void: a rounded-box SDF with a large edge fillet (the cove),
  walls visually endless, ink present only as center line, center circle, two black
  goal apertures, and up to two contour loops. The loops — the tangent loop (where
  flat floor ends) and a waterline loop drawn on the cove at constant height — imply
  the bowl through their varying gap (wide at the far wall, near-touching at the near
  wall, flared at corners). Loop rendering is a 4-way runtime toggle:
  `none | tangent | waterline | both`. The center line terminates in cove hooks: a
  foreshortened riser at the far end, a J-hook at the near end.
- Ball finish is flat two-tone: body-lit, body-shade, band-lit, band-shade — four
  solid fills split by a hard terminator fixed to the global light. The band snaps to
  its shade tone where it crosses the terminator. Both teams are jersey-striped
  (home: red body, paper band; away: paper body, blue band); the game ball is the
  only solid: ink with one white surface dot (spin telltale) plus the specular. Spin
  legibility comes from band swim, terminator crossing, and (game ball) the dot.
- Palette is a token table; inversion (dark mode) is a table swap, no ramps. Working
  values, provisional:

  | Token | Value | Token | Value |
  |---|---|---|---|
  | paper | `#FFFFFF` | ink | `#111111` |
  | shadow | `#E7E5DE` | — | — |
  | home.body.lit | `#7A2222` | home.body.shade | `#4E1414` |
  | home.band.lit | `#F5F2EA` | home.band.shade | `#C7C2B2` |
  | away.body.lit | `#F7F5EF` | away.body.shade | `#D2CFC4` |
  | away.band.lit | `#1E3A66` | away.band.shade | `#132242` |
  | game.lit | `#1E1E1D` | game.shade | `#050505` |

- Type is Helvetica-class throughout, lowercase-leaning HUD. Helvetica itself
  requires an embedding license; Inter, Neue Haas Grotesk, or Arimo are the
  candidates for shipping. HUD (score, clock, play bar, number labels on the compat
  tier's DOM path) is real HTML text, not glyph atlases, wherever a DOM overlay
  exists.

## Camera and projection

World → screen is affine: `sx = x − 0.12·y`, `sy = 0.62·y − 0.785·z` (constants
provisional; the 0.62/0.785 pair is near-orthonormal so spheres project to circles
within ~2%). The shear stands the goal apertures up as slits and gives shadows a
consistent stage. The WebGPU tier adds a free 3D camera; the compat tier fixes this
oblique view. Sphere-to-circle invariance under the affine camera is load-bearing:
ball appearance is f(team, orientation) only, independent of screen position, which is
what makes sprite caching valid.

## Components

| Crate | Responsibility | Depends on |
|---|---|---|
| `zoomie_core` | Fixed-point math, deterministic scalar sim (reference impl), arena SDF, play compiler (RON → tables), replay format, state hash. `no_std`-friendly. | — |
| `zoomie_gpu` | WGSL kernels + pipeline/bind-group setup against raw wgpu; constants generated from `zoomie_core` via `build.rs`. | wgpu (version-matched to Bevy's) |
| `bevy_zoomie` | Published Bevy plugin: render-graph nodes wrapping `zoomie_gpu`; in-repo test app. Publishes in lockstep with Bevy releases. | bevy, `zoomie_gpu` |
| `zoomie_web` | wasm app: WebGPU canvas + DOM HUD, tier detection, Canvas2D compat presenter driven by the `zoomie_core` scalar sim. No Bevy. | `zoomie_core`, `zoomie_gpu` |

Tier selection at startup: no WebGPU adapter, or a software adapter reported via
`adapter.info` → CPU sim + Canvas2D, fixed camera, 10v10.

## Why it's shaped this way

- **Integer fixed-point because float is not portable.** Any legal compiler transform
  of integer arithmetic is value-preserving mod 2³²; float reassociation and FMA
  contraction are not. Q16.16 with emulated 64-bit intermediates gives bit-identical
  results across driver compilers and vendors, and makes replays, lockstep netplay,
  and cross-tier parity properties of the algebra rather than disciplines.
- **Jacobi + integer atomics because addition commutes.** Order-independent
  accumulation makes GPU scatter deterministic regardless of scheduling. The known
  trap: atomically built cell lists have deterministic contents but nondeterministic
  order, so broadphase output must be consumed as a set (sums), and capped-capacity
  cells are forbidden (which ball overflows is schedule-dependent). Grid, when added,
  is counting-sort + prefix scan.
- **GPU-resident with layered control because readback is the scarce resource.**
  Uploads (mailbox) are cheap; readbacks carry `mapAsync` latency and sync points.
  Everything that must read per-tick ball state (steering, hover picking) runs on the
  GPU; only bytes-per-tick events and an 8-byte debug hash come back, 2–3 frames
  late.
- **Two tiers, one core, because the reference sim must exist anyway.** The scalar
  Rust sim is the determinism oracle for the WGSL port; giving it a Canvas2D
  presenter converts a test asset into a compatibility tier for blocklisted
  Chromebooks at near-zero marginal cost. Physics lives twice (Rust, WGSL) and is
  held together by TICK.md, shared per-kernel test vectors, and per-tick hash parity
  in CI.
- **The arena SDF is shared physics and lighting geometry.** Collision, cove
  shading-by-shadow-crease (fixed-step march along the light direction), and goal
  volumes are one expression; they cannot desync.
- **Hard flat shadows because they are cheaper and clearer than gradients.** Unioned
  single-tone ellipses beat gradient fills on every tier; parallel-light offset
  encoding removes the small-vs-high ambiguity of soft blobs; the compat tier
  approximates a crease as two overlapping same-tone ellipses.
- **Small balls, large arena.** At ~1/40 arena width per ball, most pixels belong to
  the static layer: dirty-rect repainting pays again on the compat tier, the sprite
  atlas is baked per-DPR bucket, and below ~10 px radius direct vector arcs rival
  sprite blits (measure, don't assume). Crispness on retina comes from rendering at
  `devicePixelRatio` (capped ~1.25–2 by tier).

Known limits: WGSL has no i64 and no mulhi (products go through 16-bit limbs);
division is avoided in favour of Newton reciprocal/rsqrt; the compat tier gives up
the free camera and large team sizes; orientation quaternions are render-side f32 and
non-authoritative, so visual roll may drift between clients by design.

## Flow

One 60 Hz tick: CPU writes the mailbox (play selection, camera-independent commands,
≤2 KB) → GPU latches inputs → playbook eval emits per-ball spin targets and impulse
requests → two 120 Hz physics substeps run the TICK.md stage order (impulses, motor,
gravity, Magnus, integrate, arena contact with traction, Jacobi pair solve, caps) →
events (goals, game-ball touches, pick results) append to a ring the CPU maps
asynchronously and sorts by (tick, ball_id) → the render pass vertex-pulls the two
most recent state snapshots and interpolates; a small f32 compute pass advances
render-only orientation from deterministic spin. The DOM overlay draws score, clock,
play bar, and number labels.

## Milestones

- M0 `zoomie_core`: fixed-point module (`qmul`, `isqrt`, `rsqrt`), vec ops, arena
  SDF, scalar sim per TICK.md, replay format, headless tests.
- M1 Canvas2D presenter, early — it is the dev visualization and the feel-tuning
  rig; feel risk retires here.
- M2 GPU sim: WGSL ports, tiled n² collisions, parity harness green on golden
  replays.
- M3 impostor renderer + free camera as the Bevy plugin with the in-repo test app.
- M4 playbook: RON schema, compiler, GPU eval pass, two or three fixture plays.
- M5 `zoomie_web` shell: tier detection, match flow, DOM HUD.
- Post-M5: prefix-sum grid broadphase; 1000-per-side scaling; editor.

## Decision ledger

Acked as of 2026-07-19:

1. Rust/Bevy + wgpu → WebGPU; GPU-resident deterministic Q16.16 sim, no f32 in sim.
2. Jacobi collision solve with integer atomics; fixed dispatch and iteration counts.
3. Mailbox in, event ring out; readback async, small, sorted (tick, ball_id).
4. WebGPU primary tier + Canvas2D compat tier, both in v1, sharing `zoomie_core`.
5. TICK.md normative; per-kernel shared test vectors; commutative per-tick state hash.
6. Crate split `zoomie_core` / `zoomie_gpu` / `bevy_zoomie` (published) / `zoomie_web`.
7. Full aerial play: cove arena (large-fillet rounded-box SDF), Magnus + one air cue
   impulse, charges reset on contact; invisible sim ceiling.
8. Dugout goals via SDF box subtraction; black slit apertures.
9. Both teams jersey-striped (red/paper vs paper/blue); game ball solid ink with one
   white dot; numbers as HUD projections only (hover / alt-hold / setting).
10. Playbook: RON files first, compiled to flat tables; in-game editor later.
11. Oblique ortho camera with slight fixed shear; free camera on WebGPU tier only.
12. Hard single directional light; flat unioned single-tone shadows; height as
    lateral offset; shadow creases reveal the cove.
13. Contour loops (tangent + waterline) as a 4-way runtime toggle.
14. Flat two-tone ball finish, four tones, band snaps at the terminator.
15. Arena drawn by negative space: no boundary outline, no gradients in the arena;
    ink only for markings, apertures, loops.

## Open items

- Scoring predicate (center-cross vs fully-past), as a RON match rule.
- Arena dimensions as a function of team size (10v10 values are provisional; number
  legibility is expected to yield to solid-vs-band team reading at 100v100).
- Default loop mode out of the 4-way toggle.
- Physics constants: rig values are feel-provisional pending M1 tuning (see TICK.md
  tables).
- Palette finalization (tokens above are working values) and shipped typeface choice.
- Bevy/wgpu version-pinning policy for the published plugin.
- Software-adapter / no-WebGPU user messaging.
- RON schema details: verb parameter shapes, trigger vocabulary, formation
  definitions.
- Dugout interior dimensions and ceiling height.
- Whether rust-gpu → naga single-source of the sim is viable (spike; off the critical
  path — hand-port + parity harness is the plan of record).
