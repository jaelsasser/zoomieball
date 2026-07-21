# Architecture

## Missing boundary in conventional game loops

GPU physics, render-time interpolation, and per-entity controller calls allow scheduling and device differences to affect gameplay. They also make native-to-WASM replay parity impossible to diagnose: a render frame, controller allocation, or broad-phase iteration can silently change the authoritative order.

Zoomieball instead admits one state transition at 64 Hz. Rendering consumes a converted snapshot and has no return edge.

## Components

```text
play node -> intent --+                         +-> rewards -> learning
world -> visibility -+-> controller -> command +-> physics -> hash -> snapshot
teammate observations -> 16 Hz coach -> mailboxes --^
```

| Component | Owns | Does not own |
|---|---|---|
| `zoomieball-core` | World order, Q16.16 arithmetic, perception, actuation, physics, rewards, hashes | Learned network implementation, GPU objects |
| `zoomieball-controller` | Zoomie populations, lane encoding, mailboxes, controller learning state | Physics policy, snapshots |
| `zoomieball-render` | Packed cosmetic frame and one-upload invariant | Authoritative state or controller dependency |
| `zoomieball-headless` | Match construction and reporting | Alternate simulation rules |

Only `zoomieball-controller` depends on `~/Projects/zoomie`. The core can be tested with a typed deterministic backend; renderer publication therefore cannot pull a controller or GPU dependency into replay tests.

## Tick flow

The order is part of the replay ABI:

1. Resolve the current cyclic play node and produce one oracle intent per physical body.
2. Rebuild the deterministic spatial index and enumerate every unoccluded sphere in each forward hemisphere.
3. On ticks divisible by four, run both coaches and publish eight squad mailboxes per team.
4. Run goalie and fielder populations; the bodies consume the mailboxes from step 3 in the same tick.
5. Decode spin residuals and gates, then consume surface and air cue charges.
6. Run two fixed physics substeps and two canonical collision sweeps per substep.
7. Generate rewards, learn, fold world/controller hashes, and replace the render snapshot.

The loop reuses caller-owned observation, intent, command, reward, and snapshot buffers. Initialization and playbook compilation may allocate.

## Deterministic geometry

Positions, velocities, angular velocity, normals, depths, and controller lanes are Q16.16. Products widen to `i64` or `i128`; normalization uses an integer square root. Arena contact combines simultaneous inward plane normals, producing rounded corner and cove normals without a traversal-dependent manifold order. Sphere pairs are visited in ascending body index for a fixed number of sweeps.

Perception casts a target-directed ray for every candidate sphere. The hemisphere boundary and physical occlusion are the only filters. The grid counting-sorts each sphere into every cell touched by its radius-expanded AABB; integer voxel traversal then tests the uncapped contents of only the cells crossed by a target ray. The grid remains an accelerator rather than a semantic authority: brute-force tests pin default, boundary, occlusion, distant-target, and 16 diagonal fixture outputs.

## Limits

The current renderer crate pins the packed upload contract but does not create a WebGPU device or shaders. Goal mouths are physical openings in the end walls, but their surrounding cove is represented by combined plane contacts rather than a higher-order analytic fillet. Registry transport, multiplayer, coach-authored play traversal, and GPU-authoritative physics are outside this architecture.
