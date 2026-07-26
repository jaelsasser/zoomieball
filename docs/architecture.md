# Architecture

## Scheduling and authority problem

A game loop that mixes render interpolation, per-entity controller calls, and unordered GPU collision work cannot explain a replay divergence. Device-dependent controller rates and GPU-to-CPU presentation readback also make the simulation authority depend on the delivery tier.

Zoomieball instead fixes one update schedule across every tier and keeps a permanent scalar CPU implementation. The CPU path is both a useful runtime and the conformance oracle for GPU bring-up. GPU authority is earned in lockstep, one witness layer at a time; rendering never feeds state back into either implementation.

## Runtime flow

```text
                              one 60 Hz body tick

match/play input -> graph-v0 assignments -> 180-degree perception
                                               |
                         every fourth tick: 15 Hz coaches
                                               |
                         same-tick squad mailboxes + edge logits
                                               |
                              60 Hz body Zoomies
                                               |
                              latched residuals and cue gates
                                               |
                  +----------------------------+----------------------------+
                  |             two 120 Hz physics steps                   |
                  | refresh oracle steering -> combine -> physics stages   |
                  +----------------------------+----------------------------+
                                               |
                       rewards -> scheduled learning -> witnesses
                                               |
                 CPU: render-owned snapshot | GPU: resident buffers
```

The order is normative:

1. Latch 60 Hz match and play input.
2. Resolve graph-v0 squad assignments and the initial oracle intent.
3. Build the complete forward 180-degree perception frame.
4. Every fourth body tick, pulse both coaches and publish squad mailboxes for use later in the same tick.
5. Pulse goalie and fielder populations, then latch their residuals and cue gates.
6. Before each 120 Hz physics step, refresh oracle steering and combine it with the latched Zoomie output.
7. Run the impulses-through-events physics stages specified by [GAME_TICK.md](../GAME_TICK.md#physics-substep-order).
8. Accumulate rewards, run due learning, publish the layered witnesses, and expose presentation state.

The schedule is 60 Hz for perception and embodied networks, 15 Hz for coaches, and 120 Hz for oracle refresh, motor combination, and physics. Device capability never changes these rates.

## Packages

| Package | Owns | Neighbors | Current state |
|---|---|---|---|
| `zoomieball-core` | CPU scalar world, ten canonical physics words per body, separate match metadata, graph-v0, perception oracle, typed controller batches, rewards, replay witnesses | Controller backend and render publication boundary | Live tracer; normative physics bites pending |
| `zoomieball-controller` | CPU fielder, goalie, and coach Zoomie populations; lane encoding; learning; squad mailboxes; edge logits; local checkpoint envelope | Sibling Zoomie CPU crates and `zoomieball-core` ABI | Live tracer |
| `zoomieball-gpu` | Zoomieball WGSL physics and perception, topology selection, rewards, mailboxes, motor decoding, and parity diagnostics | `zoomie-gpu`, core oracle, renderer | Scaffold |
| `zoomieball-render` | Raw controller-independent renderer, CPU presentation snapshot, GPU-resident state-source contract, cameras, contours, and perception inspection | Core state on CPU; resident buffers on GPU | CPU seam live; raw GPU renderer pending |
| `bevy-zoomieball` | Published Bevy integration and example | Raw renderer | Scaffold; dependency pin blocked |
| `zoomieball-web` | WebGPU shell, tier selection, Canvas2D compatibility presenter, DOM HUD, and import/export | GPU and render packages | Scaffold; dependency pin blocked |
| `zoomieball-headless` | CPU match runner, golden replay debugger, benchmark, and CPU/GPU parity driver | Core, controller, later GPU | CPU runner live |

`zoomieball-controller` is the Zoomieball-owned CPU adapter. M2b adds a generic `zoomie-gpu` sibling crate beside the existing Zoomie workspace; it is not a Zoomieball package. That sibling owns generic GPU schedules for network families, gates, stepping, learning, outputs, and checksums. Zoomieball-specific perception, graph selection, rewards, mailboxes, and motor decoding remain in `zoomieball-gpu`. Existing sibling Zoomie persistence and arithmetic formats remain authoritative and are not versioned by this workspace.

## Execution tiers

### Permanent CPU tier

The scalar path supplies deterministic conformance, native and WASI headless runs, golden replay production, and the permanent 10v10 Canvas2D fallback. Its spatial grid is an accelerator only: target-directed brute force remains the perception oracle for equivalence, occlusion, boundary, fovea, and distant-target tests.

The live implementation is an end-to-end tracer through play selection, perception, CPU Zoomie inference and learning, physics, witnesses, and presentation publication. The witness types and layering are in place, but the arithmetic, arena SDF, collision stages, cue model, and golden replay corpus do not yet conform to M0. Those gaps stay visible in package `TODO.md` files rather than being hidden behind a prototype archive.

### Lockstep GPU bring-up

M2a keeps the CPU match authoritative and feeds its commands into GPU physics. Every physics step compares the normative commutative `u32` state hash. A first mismatch selects the earliest step, after which intermediate stage hashes bisect the divergence.

M2b runs GPU Zoomie inference and learning beside the sibling Zoomie serial/population oracle. Physics, controller, and learning witnesses must all match before the CPU shadow can leave the primary WebGPU path.

### GPU-resident primary tier

After parity, simulation, perception, Zoomie execution, learning, and raw rendering remain resident on the GPU. Presentation consumes resident buffers directly. Routine frames perform no authoritative-state upload and no state readback; small explicit diagnostics such as witnesses and sorted events cross the boundary when enabled. The CPU implementation remains available as fallback and oracle.

## State and witness boundaries

Each physical body has ten canonical physics words: three position, three velocity, three spin, and one flags word. The flags word carries only the canonical team, objective, contact, and charge bits. Full typed IDs, roles, squads, contact frames, scores, graph state, and other match metadata remain available to the CPU model without widening that ten-word GPU layout.

| `TickHash` field | Width | Source and purpose |
|---|---:|---|
| `physics` | `u32` | `World::physics_hash()`: wrapping sum of per-body hashes, commutative across GPU workgroups and normative for CPU/WGSL parity |
| `controller` | `u64` | `ControllerBackend::controller_hash()`: network parameters and transient controller state |
| `learning` | `u64` | `ControllerBackend::learning_hash()`: eligibility, reward, and learning-rule state |
| `pipeline` | `u64` | Diagnostic fold over ABI words, graph state, `World::diagnostic_hash()`, and the three component witnesses |

Raw equality between an original and mirrored hash stream is not a conformance rule. A transformed mirrored-state comparison remains blocked until polar and axial vectors, team labels, commands, IDs, and event records all have explicit mirror mappings.

## Presentation boundary

Cosmetic `f32` conversion belongs to `zoomieball-render`, not `zoomieball-core` and not the controller ABI. The CPU source calls `RenderSnapshot::publish(&World)` and `Renderer::render()` performs one packed upload per presentation update. The GPU source implements `ResidentStateSource`; `Renderer::render_resident()` records zero authoritative uploads and zero readbacks. Cameras, device-pixel ratio, interpolation, contours, labels, orientation quaternions, and inspector overlays are non-authoritative.

## Open decisions

Scoring details, final arena values, palette, font, and Bevy and wgpu versions remain explicit TODOs. Graph triggers and the per-squad verb/target/form shapes are acknowledged in [graph-v0](graph-v0-proposal.md); the checked-in playbook stays a cyclic graph-v0 tracer until the schema bite encodes them, and it is not a provisional encoding of any unresolved policy. The 10v10 CPU/Canvas2D realtime goal and 100v100 WebGPU target are performance requirements, not alternate update schedules.
