# GAME_TICK.md — Zoomieball runtime contract

`GAME_TICK.md` specifies the deterministic update schedule, numeric model, canonical
physics words, substep stages, and witness semantics shared by every conforming
Zoomieball implementation. Rendering, cameras, HUD layout, and render-only orientation
are outside this contract except at the presentation boundary.

Status: v0, 2026-07-20. The schedule and ordering in this document are normative.
Constants marked `prov` are feel-provisional pending M1 tuning; changing a `prov`
value is a data change. `open` marks a value not yet decided. The current CPU runtime
is an M0 tracer through the complete public pipeline, not yet a claim of numeric or
physics conformance. GPU physics and GPU Zoomie execution remain later milestones.

## Problem

A deterministic physics loop is insufficient when control also has time. Perception,
fielder and goalie networks, coaches, playbook intent, motor decoding, rewards, and
learning must observe one order and one cadence or identical physics kernels still
produce different matches. GPU bring-up also needs witnesses that distinguish body
state divergence from controller or learning divergence.

This contract fixes those boundaries while retaining two implementations: a permanent
scalar CPU path and a WebGPU-primary path. The CPU path remains the native/WASM/WASI
oracle, headless runner, 10v10 compatibility tier, and lockstep shadow until all GPU
witness layers agree.

## Clocks

| Clock | Rate | Period and work |
|---|---:|---|
| match/play input | 60 Hz | latched once at the start of each body tick |
| perception | 60 Hz | rebuilt once from the latched tick state |
| fielder and goalie populations | 60 Hz | pulse once after any due coach pulse |
| coach populations | 15 Hz | pulse on body ticks divisible by four |
| oracle steering, motor combination, physics | 120 Hz | run twice per body tick against current substep state |

Let `T` be the zero-based 60 Hz body tick and `j ∈ {0, 1}` the substep within it.
The zero-based physics step is `S = 2T + j`. `coach_due(T)` is `T mod 4 == 0`, so
tick zero publishes a coach mailbox before the first body pulse. Every execution tier
uses this phase and these rates; device-specific controller rates are forbidden.

The normative substep is 1/120 s. Its baked Q16.16 timestep is `DT = 546`
(0.008331…). Two physics substeps complete one body tick. Learning has no implicit
wall-clock rate: the match's deterministic learning schedule declares when a pass is
due and is part of replay/checkpoint state.

## Tick order

One body tick executes the following steps in exactly this order:

1. **Latch 60 Hz match/play input.** Input for tick `T` becomes immutable for the
   tick. Delayed application or mid-tick mutation is nonconforming.
2. **Resolve graph-v0 assignments and initial oracle intent.** The current cyclic
   play node resolves triggers, per-ball verb/target entries, squad assignments,
   enabled coach edges, and the initial deterministic intent.
3. **Build perception.** The deterministic builder produces the full 180° CSR
   observation batch in canonical body order from the current world and resolved
   intent.
4. **Pulse due coaches.** When `coach_due(T)`, both coach populations consume the
   tick's state and perception, then publish squad mailboxes and edge logits. These
   values are visible to step 5 of the same tick. A one-tick mailbox delay is
   nonconforming.
5. **Pulse body networks.** Fielder and goalie populations consume the current
   observations, oracle intent, and squad mailboxes. Their steering residuals and cue
   gates latch for both substeps of tick `T`.
6. **Refresh and combine control before each substep.** Before physics steps `2T` and
   `2T + 1`, graph-v0's oracle steering is recomputed from the current substep state.
   Motor decoding combines that fresh value with the latched Zoomie residual and cue
   gates. Body networks do not repulse at 120 Hz; oracle steering is not held stale
   across both substeps.
7. **Run physics.** Each substep executes every stage from
   [impulses through events](#physics-substep-order), in order, with no controller or
   playbook stage inserted between them.
8. **Finalize the tick.** The runtime accumulates rewards from both substeps, runs a
   learning pass when scheduled, publishes physics/controller/learning witnesses and
   the diagnostic pipeline fold, then exposes presentation state.

All input, graph, perception, controller, reward, and learning batches are initialized
and canonically ordered. An implementation may fuse adjacent stages only when the
observable reads, writes, and witness values are identical to this order.

## Units and numeric model

- Length unit: one ball radius (r = 1). Time unit: seconds. Angular velocity: rad/s.
  Mass m = 1; sphere inertia I = 2/5.
- All authoritative physical state and every value feeding it use Q16.16 `i32`
  (65536 = 1.0). The range is two's-complement asymmetric: [−32768.0, +32767.99998],
  with resolution 2⁻¹⁶. The negative endpoint is representable and in-domain.
- No `f32` or `f64` value may feed simulation, controller input, rewards, graph
  decisions, or a conformance witness.
- Baked constants have one Zoomieball source and are emitted for WGSL. A shadow WGSL
  literal is a defect.

### Required arithmetic helpers

Each helper is an exact function of its inputs. Exact output, not the implementation
algorithm, is normative.

| Helper | Definition |
|---|---|
| `mul64(a: i32, b: i32) -> i64` | Exact signed 64-bit product. WGSL emulates it with 16-bit limbs. |
| `qmul(a, b)` | `sign(a)·sign(b)·((|a|·|b|) >> 16)`, with an exact magnitude product and truncation toward zero. The result must fit `i32`. This preserves `qmul(-a,b) == -qmul(a,b)`. |
| `qdiv(a, b)` | Sign-adjusted `trunc((|a| << 16) / |b|)`, exact through 64÷32 long division or an equivalent exact method. Callers guarantee `b != 0`. |
| `isqrt64(x: u64) -> u32` | `floor(sqrt(x))`, exact. |
| `qlen(v)` | `isqrt64(Σ mul64(vᵢ,vᵢ))`, producing a Q16.16 vec3 magnitude. |
| `qnorm(v)` | Total on every input: `qdiv(vᵢ, qlen(v))` per component when `qlen(v) != 0`, and the zero vector when `qlen(v) == 0`. Zero in, zero out is normative; there is no epsilon precondition. |

Cross and dot products accumulate exact component products in widened storage, then
divide the accumulated sum by 65536 with truncation toward zero, matching `qmul`. This
renormalization is not a shift: an arithmetic right shift rounds toward −∞ and differs
by one raw unit for every negative sum that is not a multiple of 65536. Caps bound all
state so those accumulators cannot overflow.

`qmul`, `qdiv`, `from_i32`, `sqrt`, and the cross/dot renormalization are defined only
where the exact result fits `i32`; `qdiv` additionally requires `b != 0`. `Add`, `Sub`,
and `Neg` are total and wrap two's-complement per determinism rule 3. Conforming
callers never drive the defined-domain helpers outside `i32`, and the state caps of
substep stage 8 are what bound their inputs.

A tier may trap on an out-of-range helper result. The CPU tier does: those five helpers
panic, as a defect detector for nonconforming input. WGSL has no trapping arithmetic,
so the GPU tier cannot trap and produces an unspecified value instead. The tiers agree
because the out-of-range case is unreachable for conforming input, not because the GPU
tier checks anything.

## Canonical physics words and match metadata

The normative GPU/physics state contains exactly ten `u32` words per physical body,
stored structure-of-arrays in canonical body order:

| Words | Source type | Meaning |
|---|---|---|
| `pos.x..pos.z` | i32 Q16.16 × 3 | center position |
| `vel.x..vel.z` | i32 Q16.16 × 3 | linear velocity |
| `spin.x..spin.z` | i32 Q16.16 × 3 | angular velocity ω |
| `flags` | u32 | team/objective class, grounded state, action charges, cooldown, and reserved zero bits |

The v0 `flags` allocation remains: bit 0 team, bit 1 game-ball, bit 2 grounded, bit 3
surface charge, bit 4 air charge, bits 8–15 cooldown ticks, and all remaining bits
reserved as zero. Ball count is `2·team_size + 1`.

`World::physics_hash()` is the normative witness over these words.

The ten-word boundary does not delete the rest of the world model. Stable typed IDs,
roles, squads, borrowed views, contact frames, radii, score, tick, graph assignments,
and other match metadata remain authoritative inputs to the appropriate stages. They
are stored separately from the ten physics words and are excluded from the normative
commutative physics hash. `World::diagnostic_hash()` covers the broader CPU world for
local diagnosis. Metadata needed to continue a replay belongs in replay and checkpoint
state; the diagnostic pipeline fold may cover it.

Render instances and orientation quaternions are cosmetic `f32` data owned by the
render layer. They are neither canonical words nor match metadata.

## Constants

| Name | Nominal | Status | Notes |
|---|---:|---|---|
| `BODY_HZ` | 60 | acked | perception and fielder/goalie pulses |
| `COACH_HZ` | 15 | acked | one pulse per four body ticks |
| `PHYSICS_HZ` | 120 | acked | oracle, motor, and physics rate |
| `SUBSTEPS` | 2 | acked | physics substeps per body tick |
| `DT` | 1/120 s | acked | baked Q16.16 value 546 |
| `G` | 28 r/s² | prov | gravity |
| `K_MAGNUS` | 0.045 | prov | `a = K_MAGNUS·(ω×v)` |
| `K_MOTOR` | 6 /s | prov | spin slew; per-substep factor baked |
| `K_DECAY` | 0.6 /s | prov | uncommanded spin decay; baked per-substep multiplier 0.99501 |
| `K_TRACTION` | 6 /s | prov | tangential blend rate toward rolling |
| `A_TRACTION_MAX` | 50 r/s² | prov | traction acceleration cap |
| `A_STICK` | 10 r/s² | prov | commanded-contact adhesion cap; not yet validated in the feel rig |
| `E_REST` | 0.55 | prov | arena and pair restitution |
| `V_REST_MIN` | 4 r/s | prov | below this normal speed, restitution is zero |
| `MU` | 0.5 | prov | arena and pair Coulomb friction |
| `M_TANG` | 2/7 | acked | effective tangential contact mass for a unit sphere |
| `J_JUMP` | 13 r/s | prov | surface jump Δv along contact normal |
| `J_AIRJUMP` | 11 r/s | prov | air jump Δv along +z |
| `N_ITER` | 4 | prov | fixed Jacobi iteration count |
| `V_MAX` | 40 r/s | prov | speed cap; `V_MAX·DT = 0.33 r < r` |
| `W_MAX` | 40 rad/s | prov | spin cap |
| `EPS_LEN` | 2⁻⁸ | prov | physics-level direction guard; not a `qnorm` precondition |

A cue preset is `(hit-offset angle θ from center toward the hit normal, azimuth
frame, J)`. It applies Δv = J·d and
Δω = (5/2)·J·sin(θ)·(n̂×d̂) using canonical hit geometry.

| Preset | d | θ | J | Status |
|---|---|---:|---:|---|
| jump | contact normal | 0 | `J_JUMP` | acknowledged form, provisional J |
| air jump | +z | 0 | `J_AIRJUMP` | provisional |
| boost | horizontal command/velocity direction | 15° above center | 10.7 r/s | provisional |

The 10v10 arena working set is provisional: half-extents `hx = 35`, `hy = 22`, XY
corner radius 10, cove fillet `R_f = 6.3`, goal-mouth half-width 5.7, and mouth
height 4.3. Scaling by team size, dugout depth, and ceiling height remain open.

## Arena SDF

The playable volume is a rounded box in XY, extruded in z from floor `z = 0`, with a
quarter-torus cove of radius `R_f` where floor meets wall. Two dugout boxes subtract
from the end walls behind the goal mouths. An invisible ceiling caps the volume.

Contact evaluation is normative:

1. Compute signed 2D distance `d2` to the rounded-rectangle wall and inward 2D normal
   `n2` using exact helpers. Distance is positive inside.
2. Select one region:
   - Inside a mouth corridor—`|y|` below mouth half-width, `z` below mouth height,
     beyond the mouth-side fillet—evaluate floor contact only.
   - Otherwise, when `d2 < R_f && z < R_f`, evaluate the fillet cross-section with
     `c = (d2 − R_f, z − R_f)` and `L = |c|`. Contact occurs when
     `L > R_f − 1`; the normal is `n2·(−c_d/L)` horizontally and `−c_z/L`
     vertically.
   - Otherwise evaluate wall contact when `z ≥ R_f && d2 < 1`, or floor contact
     when `z < 1 && d2 ≥ R_f`.
3. A game-ball crossing of the mouth plane emits a goal event. Center crossing versus
   whole-ball crossing is open. The dugout interior otherwise collides as wall.

## Control state

Graph-v0, perception, and controller data use typed caller-owned batches rather than
raw strings or a renderer-facing buffer. Their binary layout is versioned by the
controller ABI, but these semantics are part of the tick contract:

- Perception covers 180° and is target-directed. The spatial-grid result must equal
  the brute-force result, including occlusion, distant targets, fovea behavior, and
  lane layout.
- Fielder, goalie, and coach populations are distinct Zoomie families. The CPU
  implementation remains the semantic oracle. The later generic sibling
  `zoomie-gpu` implementation must reproduce Zoomie's serial/population inference and
  learning schedules bit-for-bit.
- Graph-v0 provides an oracle intent for each embodied body. A body output provides a
  learned steering residual and cue gates. The motor combines both; a zero residual
  preserves oracle behavior.
- Coaches publish squad mailboxes and enabled-edge logits before same-tick body
  evaluation. Graph traversal consumes edge logits only through graph-v0's defined
  edge semantics.
- Match/play input is latched at 60 Hz. The final GPU-primary path evaluates
  state-dependent control in resident buffers; it does not read authoritative state
  back to the CPU for control.

Zoomieball-local graph, controller, replay, and checkpoint artifacts are v0 and change
in place. The v1 `schedule_abi` word in `CheckpointHeader` binds checkpoints to the
60/15/120 schedule and tick-zero coach phase. No migration reader is required. Sibling
Zoomie's established persistence and wire formats remain unchanged and authoritative
for generic Zoomie state.

## Physics substep order

After step 6 has produced the current substep's decoded motor and impulse requests,
every physical body executes these stages in exactly this order. Per-body branches are
value-level selections and never change accumulation structure.

1. **Impulses.** Apply requested cue presets subject to latched cue gates, contact
   state, charges, and cooldown. Consume the applicable charge and set cooldown.
2. **Motor.** When commanded, slew `ω` toward the combined oracle-plus-residual spin
   target by the baked `K_MOTOR` factor. Otherwise apply `K_DECAY` independently to
   each component.
3. **Gravity.** Apply `vel.z −= qmul(G, DT)`.
4. **Magnus.** Apply `vel += K_MAGNUS·(ω × vel)·DT`, with widened accumulation.
5. **Integrate.** Apply `pos += vel·DT`.
6. **Arena contact.** Query the SDF and project penetration by `pos += n·pen`. For an
   approaching body (`v·n < 0`), apply normal impulse
   `j_n = −(1+e')·(v·n)`, where `e'` is `E_REST` above `V_REST_MIN` and zero below
   it. Compute contact slip `u = v + ω×(−n)` and apply tangential impulse
   `j_t = min(MU·j_n, |u_t|·M_TANG)` opposite `û_t`, including spin transfer
   `Δω = (5·j_t/2)·(n×û_t)`. Blend tangential velocity toward `ω×n` at
   `K_TRACTION`, capped by `A_TRACTION_MAX`; apply up to `A_STICK` along `−n` while
   commanded. Set grounded when `n.z > 0.1`. Any arena contact restores both action
   charges.
7. **Pairs.** Enumerate candidate pairs, then run exactly `N_ITER` Jacobi iterations.
   Each iteration reads a coherent snapshot, computes equal-mass normal and
   tangential impulses with the arena restitution gate and Coulomb clamp, accumulates
   per-body Δv and Δω through wrapping integer addition, then applies them. The
   iteration count does not depend on contact count. The initial broadphase is tiled
   all-pairs; a later grid uses counting sort and prefix scan.
8. **Caps.** Clamp velocity magnitude to `V_MAX` and spin magnitude to `W_MAX` through
   `qlen`/`qdiv` rescaling. Values below their cap are unchanged.
9. **Events.** Evaluate goals, game-ball touches, and pick-query results, then append
   event records to the event multiset/ring.

An atomically appended event ring has deterministic contents and nondeterministic
physical order. Consumers sort the records by the event ABI's canonical key before
match logic, replay comparison, or presentation uses them. No semantic consumer may
depend on append order.

## Rewards and learning

Each substep retains the deterministic progress deltas and events needed by the
reward model. Step 8 of the body tick accumulates them into typed body/team reward
batches. A scheduled learning pass observes the complete accumulator, mutates all due
Zoomie populations in their fixed schedule, updates the learning witness, and clears
only the rewards consumed by that pass.

Progress rewards measure continuous game-ball motion only. A substep's progress delta
is the game-ball displacement produced by that substep's integrate, arena-contact,
pairs, and caps stages. A discontinuous reposition is not motion: it contributes zero
progress, and the position it establishes is the baseline for the next substep's delta.
The post-goal return of the game ball to the arena centre is such a reposition. A goal
is paid by its event reward alone; the displacement the reposition causes is neither
rewarded nor penalized, on either team's sign.

Learning cannot run between the two physics substeps. Inspection, checkpointing, and
replay publication observe state after any due learning pass so a checkpoint resumes
at an unambiguous tick boundary.

## Witnesses

Four values are published after each complete body tick. Only the first three are
independent conformance layers; the fourth is a diagnostic index over the whole
pipeline.

| Witness | Required semantics |
|---|---|
| `TickHash.physics: u32` | normative `World::physics_hash()` over the ten canonical words and canonical body-index salt |
| `TickHash.controller: u64` | Zoomie checksum over inference parameters and transient controller state |
| `TickHash.learning: u64` | Zoomie checksum over eligibility and learning-rule state |
| `TickHash.pipeline: u64` | diagnostic fold including `World::diagnostic_hash()`, local ABI/version words, graph state, and the three component witnesses |

For body ID `b`, interpret its ten canonical words as `u32` bit patterns
`w₀…w₉` and compute:

```text
h = 0x811C9DC5
for w in w0..w9:  h = (h ^ w) * 0x01000193
h ^= b
h ^= h >> 16;  h *= 0x7FEB352D
h ^= h >> 15;  h *= 0x846CA68B
h ^= h >> 16
```

All arithmetic wraps modulo 2³². The body-tick physics witness `H` is the wrapping
`u32` sum of every body `h`. The sum is commutative and can be accumulated with
`atomicAdd` independent of GPU scheduling. CPU and GPU use exactly this algorithm;
the wider diagnostic pipeline fold does not replace it.

Controller and learning checksum algorithms follow sibling Zoomie's semantic oracle.
Zoomieball may wrap those values in its v0 replay format but may not redefine their
meaning for the GPU implementation.

## Presentation boundary

After witnesses publish, the CPU tier converts authoritative state once into one
packed cosmetic snapshot owned by `zoomieball-render`. Canvas2D and other CPU
presenters consume that publication. Snapshot `f32` values cannot feed a later tick.

The GPU-primary tier gives resident state buffers to the raw renderer directly. It
does not upload a CPU copy of authoritative state and does not read authoritative
state back for presentation. Event, witness, and explicit inspection readbacks do not
make the CPU an authority.

## Determinism rules

1. Integer-only arithmetic feeds physical state, controller state, rewards, learning,
   graph decisions, and conformance witnesses.
2. Dispatch, loop, and iteration counts are fixed wherever they affect accumulated
   values.
3. Signed overflow uses two's-complement wrapping and shifts use masked amounts.
   Conforming code never divides by zero.
4. Cross-thread accumulation uses only commutative-associative integer operations:
   wrapping add, min, max, and, or, and xor.
5. Every read observes initialized memory. Scratch buffers are cleared or fully
   overwritten before reuse.
6. Atomically built lists have deterministic contents and nondeterministic order.
   They are consumed as sets or canonically sorted. A grid cell may not drop entries
   at a fixed capacity because the dropped body would depend on scheduling.
7. Every implementation observes the tick order and physics substep order above. A
   reorder is a contract change.
8. Constants have one Rust source and are emitted for WGSL.
9. CPU, WASM, and GPU implementations share one schedule. Hardware capability cannot
   alter body, coach, oracle, motor, physics, reward, or learning cadence.

## Conformance

Conformance proceeds in layers so the first differing subsystem remains visible:

1. Exact kernel vectors cover `qmul`, `qdiv`, `isqrt64`, `qlen`, SDF contact, motor
   decoding, cue impulses, and pair impulses.
2. Native, WASM, and WASI CPU golden replays compare physics, controller, and learning
   streams, with the pipeline fold retained for diagnosis.
3. GPU physics consumes commands from the lockstep CPU shadow and compares the
   physics witness after every 120 Hz step. A first differing step is bisected with
   intermediate stage witnesses.
4. Generic GPU Zoomie inference and learning compare their checksums with sibling
   Zoomie's serial/population oracle.
5. The primary GPU path loses its CPU shadow only after physics, controller, and
   learning streams all agree for the golden corpus.

Raw mirrored-hash equality is not a conformance test. Reflection changes word bit
patterns and the hash includes stable body IDs, so equality between an original hash
and an untransformed mirrored hash is neither expected nor meaningful.

A semantic mirror test remains blocked until one normative mapping specifies all of:

- polar-vector reflection;
- axial-vector reflection, including spin handedness;
- team and scoring-side exchange;
- oracle, residual, cue-hit, and impulse command transformation;
- stable physical and controller ID permutation; and
- every event-kind and payload transformation.

Only after those mappings exist may a test transform the mirrored state back into the
original canonical frame and compare witnesses. Until then, implementations must not
substitute raw mirrored-state hash equality.

## Current implementation boundary

The live CPU code preserves fixed math, typed world state and metadata, two physics
substeps, CSR perception with a brute-force oracle, fielder/goalie/coach controller
populations, cyclic graph-v0, rewards, checkpoints, layered tracer hashes, rendering
seams, and the headless runner. These are the M0 tracer and the base for incremental
conformance work.

Exact GAME_TICK arithmetic throughout, the final arena SDF, full cue/motor/contact and
Jacobi behavior, graph-v0 extensions, golden native/WASM/WASI witness streams, WGSL
physics, generic GPU Zoomie schedules, GPU residency, and cross-tier parity remain
explicit roadmap work. Existing coverage is retained or rewritten around the behavior
it protects; tracer labels must not be promoted to conformance claims before the
relevant boundary tests pass.
