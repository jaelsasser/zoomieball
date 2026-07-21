# TICK.md — Zoomieball simulation step

TICK.md specifies the Zoomieball simulation step: the numeric model, canonical state,
constants, and per-substep stage order that every conforming implementation — the
`zoomie_core` scalar Rust sim and the `zoomie_gpu` WGSL sim — reproduces
bit-identically. Rendering, cameras, HUD, and orientation quaternions are outside
this document except where noted.

Status: v0.1, 2026-07-19. Structure and numeric model are normative. Constants marked
`prov` are feel-provisional pending M1 tuning; changing a `prov` value is a data
change, not a spec change. `open` marks values not yet decided.

## Units and numeric model

- Length unit: one ball radius (r = 1). Time unit: seconds. Angular velocity: rad/s.
  Mass m = 1; sphere inertia I = 2/5.
- All state and all state-feeding arithmetic is Q16.16 in `i32` (65536 = 1.0). Range
  ±32767.9999; resolution 2⁻¹⁶.
- The nominal substep is 1/120 s; the normative timestep is the baked constant
  `DT = 546` (Q16.16, = 0.008331…). Nominal decimals in this document are for
  reading; baked Q16.16 values are normative and single-sourced from `zoomie_core`
  into WGSL by `build.rs`.
- No `f32`/`f64` appears in any value that feeds simulation state.

### Required arithmetic helpers

All helpers are exact functions of their inputs; exactness, not algorithm, is
normative — any implementation producing these values bit-for-bit conforms.

| Helper | Definition |
|---|---|
| `mul64(a: i32, b: i32) -> i64` | Exact 64-bit product. WGSL emulates via 16-bit limbs (no i64, no mulhi). |
| `qmul(a, b)` | `sign(a)·sign(b)·((|a|·|b|) >> 16)`, magnitude product exact in 64 bits, truncation toward zero. Result must fit `i32`; callers bound operands (see caps). Sign-magnitude truncation makes negation exact: `qmul(-a,b) == -qmul(a,b)`. |
| `qdiv(a, b)` | `sign`-adjusted `trunc((|a| << 16) / |b|)`, exact via emulated 64÷32 long division (fixed 48-iteration restoring loop or equivalent exact method). `b == 0` is a caller error; conforming callers never divide by zero. |
| `isqrt64(x: u64) -> u32` | `floor(sqrt(x))`, exact (fixed 32-iteration restoring shift-subtract or equivalent). |
| `qlen(v)` | `isqrt64(Σ mul64(vᵢ,vᵢ)) ` — magnitude of a Q16.16 vec3, Q16.16 result. |
| `qnorm(v)` | `qdiv(vᵢ, qlen(v))` per component; caller guarantees `qlen(v) > EPS_LEN`. |

Cross and dot products use `mul64` accumulation in i64 before the `>> 16`
renormalization, so intermediate overflow cannot occur for capped state.

## Canonical state

Structure-of-arrays, per ball:

| Field | Type | Meaning |
|---|---|---|
| `pos[3]` | i32 Q16.16 | center position |
| `vel[3]` | i32 Q16.16 | linear velocity, r/s |
| `spin[3]` | i32 Q16.16 | angular velocity ω, rad/s |
| `flags` | u32 | bit 0 team, bit 1 is-game-ball, bit 2 grounded, bit 3 jump charge, bit 4 air-move charge, bits 8–15 cooldown ticks, rest reserved (zero) |

Ball count: 2·team_size + 1. Orientation quaternions are render-side `f32`,
non-authoritative, and excluded from state, hashing, and conformance.

## Constants

| Name | Nominal | Status | Notes |
|---|---|---|---|
| `TICK_HZ` | 60 | acked | mailbox latch rate |
| `SUBSTEPS` | 2 | acked | physics substeps per tick |
| `DT` | 1/120 s | acked | baked 546 |
| `G` | 28 r/s² | prov | gravity |
| `K_MAGNUS` | 0.045 | prov | a = K_MAGNUS·(ω×v) |
| `K_MOTOR` | 6 /s | prov | spin slew rate; per-substep factor baked |
| `K_DECAY` | 0.6 /s | prov | uncommanded spin decay; baked per-substep multiplier 0.99501 |
| `K_TRACTION` | 6 /s | prov | tangential blend rate toward rolling |
| `A_TRACTION_MAX` | 50 r/s² | prov | traction acceleration cap |
| `A_STICK` | 10 r/s² | prov | adhesion cap, commanded contact only; not in the feel rig — unvalidated |
| `E_REST` | 0.55 | prov | restitution, arena and pairs |
| `V_REST_MIN` | 4 r/s | prov | below this normal speed, restitution = 0 |
| `MU` | 0.5 | prov | Coulomb friction, arena and pairs |
| `M_TANG` | 2/7 | acked | effective tangential mass at contact, unit sphere |
| `J_JUMP` | 13 r/s | prov | ground jump Δv along contact normal |
| `J_AIRJUMP` | 11 r/s | prov | air jump Δv along +z |
| `N_ITER` | 4 | prov | Jacobi iterations, fixed |
| `V_MAX` | 40 r/s | prov | speed cap; guarantees V_MAX·DT = 0.33 r < r (no tunneling) |
| `W_MAX` | 40 rad/s | prov | spin cap |
| `EPS_LEN` | 2⁻⁸ | prov | normalization guard |

Cue-impulse presets (billiards model, acked): a preset is (hit-offset angle θ from
center toward the hit normal, azimuth frame, J). Applied as Δv = J·d,
Δω = (5/2)·J·sin(θ)·(n̂×d̂) with the canonical hit geometry. Preset table v0:

| Preset | d | θ | J | Status |
|---|---|---|---|---|
| jump | contact normal | 0 (center hit: pure translation) | J_JUMP | acked form, prov J |
| air jump | +z | 0 | J_AIRJUMP | prov |
| boost | horizontal command/velocity dir | 15° above center (topspin) | 10.7 r/s | prov |

Arena (10v10 working set, all `prov`; scaling with team size is `open`):
half-extents hx = 35, hy = 22, XY corner radius 10, cove fillet R_f = 6.3, goal
mouth half-width 5.7, mouth height 4.3, dugout depth `open`, ceiling z = 37 `open`.

## Arena SDF

The playable volume is a rounded box in XY extruded in z, floor z = 0, with a
quarter-torus cove of radius R_f where floor meets wall, minus two dugout boxes
behind the ±x mouths, capped by the ceiling plane. Contact evaluation (normative):

1. `d2, n2 = ` signed 2D distance to the rounded-rect wall (positive inside) and the
   inward 2D normal, from the standard rounded-box distance with exact helpers.
2. Region select: mouth corridor (|y| < mouth half-width, z < mouth height, beyond
   the mouth-side fillet) → floor-only contact; else fillet cross-section when
   `d2 < R_f && z < R_f`: with c = (d2 − R_f, z − R_f), L = |c|, contact when
   `L > R_f − 1`, normal = n2-scaled (−c_d/L) horizontally and (−c_z/L) vertically;
   else wall when `z ≥ R_f && d2 < 1`; else floor when `z < 1 && d2 ≥ R_f`.
3. Goal: game-ball crossing of the mouth plane (predicate `open`: center-cross vs
   fully-past) emits a GOAL event; dugout interior otherwise collides as walls.

## Stage order

Per substep, in exactly this order. Every stage runs for every ball; per-ball
branches are value-level (select), never accumulation-structural.

1. **latch** (substep 0 only): mailbox for tick T becomes the command source for
   both substeps.
2. **playbook**: evaluate per-ball verb → `ω_cmd[3]`, impulse requests.
3. **impulses**: apply requested cue presets gated by charges and cooldowns; consume
   charge bits; set cooldown.
4. **motor**: `ω += (ω_cmd − ω)·k_slew` when commanded; else `ω = qmul(ω,
   K_DECAY_SUB)` per component.
5. **gravity**: `vel.z −= qmul(G, DT)`.
6. **magnus**: `vel += K_MAGNUS·(ω × vel)·DT` (mul64 accumulation).
7. **integrate**: `pos += vel·DT`.
8. **arena contact**: SDF query; positional projection `pos += n·pen`; if
   approaching (`v·n < 0`): normal impulse `j_n = −(1+e')·(v·n)` with `e' = E_REST`
   when `−v·n > V_REST_MIN` else 0; contact slip `u = v + ω×(−n)`; tangential
   impulse `j_t = min(MU·j_n, |u_t|·M_TANG)` opposing `û_t`, with
   `Δω = (5·j_t/2)·(n×û_t)`. Then traction: blend the tangential velocity toward
   `ω×n` at K_TRACTION, acceleration capped at A_TRACTION_MAX; adhesion up to
   A_STICK along −n while commanded. Set grounded when `n.z > 0.1`; restore jump
   and air-move charges on any contact.
9. **pairs**: broadphase (v1: all-pairs, workgroup-tiled; later: counting-sort
   prefix-scan grid) → N_ITER Jacobi iterations: each iteration reads a coherent
   snapshot, computes per-pair equal-mass impulses (normal with the same restitution
   gate, tangential with the same Coulomb clamp and spin transfer, halved per
   ball), accumulates Δv/Δω into integer accumulators via wrapping `atomicAdd`,
   then applies. Iteration count is fixed regardless of contact count.
10. **caps**: clamp `|vel|` to V_MAX and `|ω|` to W_MAX by magnitude
    (`qlen`/`qdiv` rescale), skipped below the cap.
11. **events**: goal test, game-ball touch detection, pick-query results; append
    records to the event ring via an atomic cursor.

Per tick, after substep 2: publish the state snapshot (ping-pong) and, when
enabled, the state hash.

## Mailbox and events

Mailbox (CPU → GPU, once per tick, ≤ 2 KB, format v0 `prov`): header
`{tick: u32, play[2]: u32, n_cmds: u32}`; command
`{ball: u16, flags: u16, ω_cmd[3]: i32, preset: u8, pad[3]}`. Absent commands leave
playbook control in force.

Event record (GPU → CPU): `{tick: u32, kind: u8, ball: u16, pad: u8, payload: u32}`,
ring of 256, cursor via `atomicAdd`. Kinds v0: 0 GOAL (payload = side), 1
GAME_TOUCH (ball = toucher), 2 PHASE (payload = team·phase), 3 PICK (payload =
query id). Within a tick the ring's multiset is deterministic, its order is not; the
consumer sorts by (tick, ball, kind) before use.

## State hash

Per ball, over the ten state words `w₀…w₉` = pos, vel, spin (as u32 bit patterns),
flags:

```
h = 0x811C9DC5
for w in w0..w9:  h = (h ^ w) * 0x01000193        (wrapping, FNV-1a)
h ^= ball_id
h ^= h >> 16;  h *= 0x7FEB352D
h ^= h >> 15;  h *= 0x846CA68B
h ^= h >> 16                                       (wrapping)
```

Tick hash `H` = wrapping u32 sum of per-ball `h` — commutative, therefore
`atomicAdd`-accumulable and schedule-independent. Debug readback is 8 bytes/tick
(tick, H).

## Determinism rules

1. Integer-only in anything that feeds state (R7 restates: no float laundering
   through "temporary" values).
2. Fixed dispatch, loop, and iteration counts; no data-dependent trip counts that
   change what is accumulated.
3. Two's-complement wrapping on overflow, masked shift amounts. WGSL specifies
   these; the defined-division-by-zero behavior claim should be re-verified against
   the current WGSL spec before relying on it — conforming code never divides by
   zero regardless.
4. Cross-thread accumulation only through commutative-associative integer
   operations: wrapping add, min, max, and, or, xor.
5. No reads of uninitialized memory; buffers cleared or fully written each use.
6. Atomically built lists (grid cells, event ring) have deterministic contents and
   nondeterministic order: consume as sets, sort on the CPU, and never cap cell
   capacity (a dropped ball is schedule-dependent).
7. Identical stage order in every implementation; a reorder is a spec change.
8. All constants baked from `zoomie_core` (single source); a WGSL literal that
   shadows a Rust constant is a defect.

## Conformance

1. Kernel vectors: `qmul`, `qdiv`, `isqrt64`, `qlen`, SDF contact, pair impulse —
   vectors generated by `zoomie_core`, replayed through a WGSL compute harness in
   CI, byte-equal.
2. Golden replays: (initial state, seed, command stream) → per-tick hash streams
   from both implementations, equal for the full match.
3. Mirror test: reflecting initial state and commands across the center line yields
   the mirrored hash stream exactly (guaranteed by sign-magnitude truncation).
4. Divergence procedure: first differing tick from the hash streams, then bisect by
   stage with intermediate hashing to localize the kernel.
