# `zoomieball-core` ownership

The deterministic CPU implementation remains the conformance oracle, headless
engine, and compatibility-tier simulation. Local v0 schemas change in place.

- [x] **M0: add the public end-to-end CPU tracer before conforming internals.**
  - Prerequisite: Alignment clocks and the render-owned presentation boundary are in place.
  - Normative anchor: steps 1–8 of the normative order in `../../GAME_TICK.md`.
  - Boundary test: one public call crosses graph selection, perception, coach/body batches, motor combination, two physics steps, rewards, four witnesses, and presentation publication.
  - Completion command: `cargo test -p zoomieball-headless --test pipeline`.

- [x] **M0: conform the exact fixed-point kernels.**
  - Prerequisite: the M0 tracer fails on the first nonconforming arithmetic fixture.
  - Normative anchor: required arithmetic helpers and determinism rules in `../../GAME_TICK.md`.
  - Boundary test: public vectors cover signed multiply and divide, integer square root, vector length and normalization, widened products, wrapping arithmetic, and rejected precondition violations.
  - Completion command: `cargo test -p zoomieball-core --test fixed_conformance`.

- [ ] **M0: centralize the baked physics and perception constants.**
  - Prerequisite: fixed-point kernels conform and the decimal-to-Q16 baking policy is acknowledged.
  - Normative anchor: the constants table and single-source rule in `../../GAME_TICK.md`.
  - Boundary test: the CPU defaults consume one typed Rust source of raw Q16.16 words suitable for later WGSL emission; tracer literals and shadow copies are absent, and `SpatialIndex::new` derives its `[33, 19, 7]` dimensions and `(-16, -9, 0)` origin from `Arena` rather than restating them.
  - Completion command: `cargo test -p zoomieball-core --test physics_constants`.

- [ ] **M0: conform the canonical physics words.**
  - Prerequisite: fixed-point kernels and baked constants conform.
  - Normative anchor: canonical physics words and match metadata in `../../GAME_TICK.md`, and the one-radius-and-mass rule in `../../DESIGN.md`.
  - Boundary test: exact position, velocity, spin, team, game-ball, grounded, charge, and cooldown words feed hard-coded body and world hashes under the normative `flags` allocation including its bits 8–15, the shared sphere radius is one constant rather than the per-body `World.radii`, and match metadata cannot enter the payload.
  - Completion command: `cargo test -p zoomieball-core --test canonical_state`.

- [ ] **M0: replace arena and physics stages in tested bites.**
  - Prerequisite: canonical words and fixed helpers conform.
  - Normative anchor: arena SDF and impulses-through-events stage order in `../../GAME_TICK.md`.
  - Boundary test: cue model, motor, forces, contacts, Jacobi pairs, caps, and events each have an order-sensitive fixture, including collision-order invariance where specified.
  - Completion command: `cargo test -p zoomieball-core physics`.

- [ ] **M0: bound every physics accumulator against the state caps.**
  - Prerequisite: the arena and physics stages are replaced in tested bites, so each stage's accumulator inputs are final.
  - Normative anchor: the caps-bound reachability argument for the trapping helpers in `../../GAME_TICK.md`.
  - Boundary test: one fixture per substep stage drives its inputs to the `V_MAX` and `W_MAX` caps at the arena extents and shows no `qmul`, `qdiv`, `from_i32`, `sqrt`, or cross/dot renormalization leaving `i32`.
  - Completion command: `cargo test -p zoomieball-core --test caps_bounds`.

- [ ] **M0: retain and certify perception as the CPU oracle.**
  - Prerequisite: conforming world geometry is available to target-directed rays and the spatial grid.
  - Normative anchor: full-180-degree perception contract in `../../DESIGN.md` and perception timing in `../../GAME_TICK.md`.
  - Boundary test: CSR/grid output equals brute force for occlusion, distant targets, fovea boundaries, and lane layout at each 60 Hz pulse.
  - Completion command: `cargo test -p zoomieball-core perception`.

- [ ] **M0: cast environment rays from the observer instead of emitting arena extents.**
  - Prerequisite: the arena SDF conforms and the perception oracle certifies target-directed rays.
  - Normative anchor: the arena SDF in `../../GAME_TICK.md` and the target-directed 180-degree perception contract in `../../DESIGN.md`.
  - Boundary test: `append_environment_rays` takes the observer position, wall, ceiling, floor, and goal depths are that observer's surface distances, the floor ray reads `position.z - radius` rather than the constant `FLOOR_RAY_DEPTH`, and the two goal rays carry distinct depths rather than a shared `arena.half_length`.
  - Completion command: `cargo test -p zoomieball-core --test environment_rays`.

- [ ] **M0: extend the single graph-v0 schema in place.**
  - Prerequisite: trigger and verb/target shapes are acknowledged in the root roadmap.
  - Normative anchor: playbook oracle, squad assignment, initial intent, and coach edge semantics in `../../DESIGN.md`.
  - Boundary test: the existing RON fixture round-trips nodes with triggers, per-ball verb/target tables, assignments, oracle intent, and edges; no alternate schema or migration reader exists.
  - Completion command: `cargo test -p zoomieball-core playbook`.

- [ ] **M0: bind the learning schedule and physics configuration to replay and checkpoint state.**
  - Prerequisite: the baked constants have one typed source, so a configuration digest folds a stable word set.
  - Normative anchor: the learning-schedule replay/checkpoint clause in `../../GAME_TICK.md` and the checkpoint header table in `../../docs/controller-abi.md`.
  - Boundary test: `CheckpointHeader` carries `learning_interval` and a `PhysicsConfig` digest that moves for any single field including `tangential_retention`, restore under a different schedule or a retuned constant fails before backend mutation, and two matches differing only in one of those inputs produce different tick-zero pipeline folds.
  - Completion command: `cargo test -p zoomieball-core --test schedule_binding`.

- [ ] **M0: publish layered witnesses and CPU golden replays.**
  - Prerequisite: conforming physics, graph, perception, rewards, and typed controller batches are connected by the tracer, and the baked constants have rescaled the tracer's `0.35` body radius to the normative r = 1 length unit, so no golden is cut while every spatial word still moves.
  - Normative anchor: witness layering and v0 replay policy in `../../DESIGN.md` and `../../docs/controller-abi.md`.
  - Boundary test: fixtures separately expose the commutative `u32` physics-state hash, controller checksum, learning checksum, and diagnostic pipeline fold; raw mirrored-hash equality is absent.
  - Completion command: `cargo test -p zoomieball-core --test golden_replays`.

- [ ] **M1: hold the 10v10 compatibility simulation to its real-time budget.**
  - Prerequisite: native/WASM M0 goldens agree and the Canvas2D consumer is wired.
  - Normative anchor: permanent CPU fallback contract in `../../DESIGN.md`.
  - Boundary test: 10v10 physics and perception sustain the declared 60/120 schedule without skipping controller or learning pulses.
  - Completion command: `cargo test -p zoomieball-core --release cpu_10v10_realtime`.
