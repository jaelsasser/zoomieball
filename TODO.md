# Zoomieball roadmap

This is the cross-package delivery order. The package-local `TODO.md` files own
the independently reviewable bites. The CPU simulation, controller, renderer,
and headless runner stay live throughout the queue.

## Alignment and milestones

- [x] **Alignment — reconcile the live tracer and its contracts.**
  - Prerequisite: the existing four-package CPU/controller/render/headless tracer builds.
  - Normative anchor: `DESIGN.md`, `GAME_TICK.md`, `docs/architecture.md`, and `docs/controller-abi.md` after the in-place reconciliation.
  - Boundary test: the workspace contains seven green packages, the clocks are 60 Hz body/perception, 15 Hz coach, and 120 Hz physics, and cosmetic snapshots are render-owned.
  - Completion command: `cargo fmt --all -- --check && cargo test --workspace`.

- [ ] **M0 — make the permanent CPU path conforming.**
  - Prerequisite: Alignment is complete and graph-v0's CPU-facing shapes are acknowledged.
  - Normative anchor: the tick order and fixed arithmetic in `GAME_TICK.md`, the CPU reference contract and M0 milestone row in `DESIGN.md`, and the `M0` bite lists those govern in `crates/zoomieball-core/TODO.md`, `crates/zoomieball-controller/TODO.md`, and `crates/zoomieball-headless/TODO.md`.
  - Boundary test: every bite prefixed `M0` or `M0/M1` in those three package roadmaps is checked, and no other package roadmap carries an `M0` prefix.
  - Completion command: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **M1 — ship the permanent CPU/Canvas2D compatibility tier.**
  - Prerequisite: M0 golden replays pass natively and under WASM.
  - Normative anchor: the CPU fallback and fixed-camera presentation contracts in `DESIGN.md` and `docs/architecture.md`.
  - Boundary test: a 10v10 match runs CPU physics, perception, Zoomie inference/learning, HUD, labels, and one packed snapshot publication at real time, and a headless 100v100 match holds real time on native and WASM without presentation.
  - Completion command: `cargo test -p zoomieball-web canvas2d_10v10_realtime && cargo test -p zoomieball-headless realtime_100v100`.

- [ ] **M2a — bring up GPU physics against the CPU shadow.**
  - Prerequisite: M0 physics witnesses and stage-level CPU fixtures are stable, and M1's 100v100 realtime measurement has set the margin GPU physics is asked to buy.
  - Normative anchor: the impulses-through-events stages in `GAME_TICK.md` and lockstep-shadow contract in `docs/architecture.md`.
  - Boundary test: commands from the CPU shadow drive each WGSL stage, per-step physics hashes agree, and first-divergence stage bisection identifies a deliberately injected mismatch.
  - Completion command: `cargo test -p zoomieball-gpu --test physics_parity`.

- [ ] **M2b — move Zoomie family execution onto the GPU.**
  - Prerequisite: M2a passes and sibling Zoomie exposes a generic `zoomie-gpu` schedule crate without changing its established wire formats.
  - Normative anchor: the controller residency, schedule, and ownership split in `DESIGN.md`, `docs/architecture.md`, and `docs/controller-abi.md`.
  - Boundary test: inference, gates, learning, outputs, controller checksum, and learning checksum are bit-identical to sibling Zoomie's serial/population oracle.
  - Completion command: `cargo test -p zoomieball-gpu --test controller_parity`.

- [ ] **M3 — complete raw rendering and engine integration.**
  - Prerequisite: the render state-source seam is stable and the visual decisions below are acknowledged.
  - Normative anchor: the CPU-snapshot/GPU-resident renderer contract in `docs/architecture.md`.
  - Boundary test: CPU rendering publishes one packed snapshot, GPU rendering performs no authoritative-state upload or readback, and the Bevy example exercises free camera, contours, and perception inspection.
  - Completion command: `cargo test -p zoomieball-render && cargo test -p bevy-zoomieball`.

- [ ] **M4 — evaluate graph-v0 on the GPU.**
  - Prerequisite: M2b parity passes and graph triggers plus verb/target shapes are acknowledged.
  - Normative anchor: graph-v0 evaluation and coach edge semantics in `DESIGN.md` and `GAME_TICK.md`.
  - Boundary test: checked-in plays choose the same assignments, initial oracle intent, edge logits, and transitions on CPU and GPU.
  - Completion command: `cargo test -p zoomieball-gpu --test graph_parity`.

- [ ] **M5a — assemble the application shell over the CPU tier.**
  - Prerequisite: M1 ships the Canvas2D compatibility tier.
  - Normative anchor: tier selection, HUD, and persistence contracts in `DESIGN.md` and `docs/architecture.md`.
  - Boundary test: the shell runs a match end to end on Canvas2D, import/export preserves v0 local state, and tier selection resolves deterministically with WebGPU absent.
  - Completion command: `cargo test -p zoomieball-web --all-targets`.

- [ ] **M5b — light up the WebGPU primary path in the shell.**
  - Prerequisite: M5a ships and M2b/M3 primary paths pass their parity and residency gates.
  - Normative anchor: tier selection and profiling contracts in `DESIGN.md` and `docs/architecture.md`.
  - Boundary test: device profiling selects WebGPU or Canvas2D deterministically and both tiers expose the same match/HUD semantics.
  - Completion command: `cargo test -p zoomieball-web --all-targets`.

## Blocked decisions

These are decision gates, not invitations to install provisional policy.

- [ ] **Acknowledge scoring and final arena parameters.**
  - Prerequisite: the M0 arena SDF and event vocabulary are available for review.
  - Normative anchor: scoring and arena sections in `DESIGN.md` and `GAME_TICK.md`.
  - Boundary test: conformance fixtures cover every acknowledged scoring event and arena boundary without hidden constants.
  - Completion command: `cargo test -p zoomieball-core --test conformance`.

- [ ] **Acknowledge palette and font assets.**
  - Prerequisite: the M3 raw renderer can consume externally selected cosmetic resources.
  - Normative anchor: presentation ownership in `docs/architecture.md`.
  - Boundary test: renderer fixtures distinguish cosmetic resources from authoritative simulation state and both presentation tiers use the selected assets.
  - Completion command: `cargo test -p zoomieball-render --test presentation_contract`.

- [ ] **Select Bevy and wgpu versions.**
  - Prerequisite: M2a establishes the required GPU feature surface and M3 establishes the engine wrapper boundary.
  - Normative anchor: dependency ownership in `docs/architecture.md`.
  - Boundary test: the selected versions build native and WASM targets without introducing a second simulation or controller authority.
  - Completion command: `cargo tree -p bevy-zoomieball && cargo tree -p zoomieball-gpu`.

- [x] **Acknowledge graph triggers and verb/target shapes.**
  - Prerequisite: `docs/graph-v0-proposal.md` is checked in with worked trigger, ball-verb, target, and coach-edge examples.
  - Normative anchor: playbook oracle, squad assignment, and edge semantics in `DESIGN.md` and `GAME_TICK.md`.
  - Boundary test: every trigger, ball verb, target, form, and coach-edge shape carries a verdict in `docs/graph-v0-proposal.md` and no other document defines one; the RON fixtures that exercise them are the schema bite's boundary in `crates/zoomieball-core/TODO.md`.
  - Completion command: `cargo test -p zoomieball-core playbook`.

- [ ] **Acknowledge the observation encoding frame.**
  - Prerequisite: the M0 lane fixtures expose every direction-bearing lane of the fielder, goalie, and coach populations for review.
  - Normative anchor: lane layout as a conformance surface in `DESIGN.md` and the blocked semantic-mirror mapping list in `GAME_TICK.md`.
  - Boundary test: one acknowledged frame, attacking or world, governs every direction-bearing lane in all three populations, and the semantic-mirror mapping list records which mappings that choice still owes.
  - Completion command: `cargo test -p zoomieball-controller encoding`.
