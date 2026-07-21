# `zoomieball-gpu` ownership

This crate owns Zoomieball-specific GPU physics, perception, controller
integration, and parity diagnostics. Generic Zoomie family schedules belong in
sibling Zoomie's future `zoomie-gpu` crate.

- [ ] **M2a: port fixed helpers and canonical physics words to WGSL.**
  - Prerequisite: M0 CPU arithmetic and ten-word state fixtures are stable.
  - Normative anchor: fixed arithmetic and canonical state in `../../GAME_TICK.md`.
  - Boundary test: every helper and packed word agrees bit-for-bit with CPU fixtures at zero, extrema, rounding boundaries, and overflow-policy edges.
  - Completion command: `cargo test -p zoomieball-gpu --test fixed_parity`.

- [ ] **M2a: port physics stages individually under CPU-shadow commands.**
  - Prerequisite: WGSL fixed helpers and canonical state packing pass.
  - Normative anchor: impulses-through-events stage order in `../../GAME_TICK.md`.
  - Boundary test: cue, motor, forces, contact, Jacobi pairs, caps, and events each match the CPU stage witness before the next stage lands.
  - Completion command: `cargo test -p zoomieball-gpu --test stage_parity`.

- [ ] **M2a: compare per-step physics hashes and bisect first divergence.**
  - Prerequisite: all GPU physics stages execute in normative order from CPU-produced commands.
  - Normative anchor: commutative `u32` physics witness and shadow bring-up contract in `../../DESIGN.md`.
  - Boundary test: a clean replay agrees at every 120 Hz step and an injected mismatch reports its first tick and stage.
  - Completion command: `cargo test -p zoomieball-gpu --test physics_parity`.

- [ ] **M2b: integrate sibling `zoomie-gpu` without duplicating generic family execution.**
  - Prerequisite: sibling Zoomie's GPU schedules are bit-identical to its serial/population inference and learning oracle.
  - Normative anchor: generic-versus-game-specific ownership in `../../docs/architecture.md` and wire compatibility in `../../docs/controller-abi.md`.
  - Boundary test: family stepping, gates, learning, outputs, and checksums come from sibling `zoomie-gpu`; this crate owns none of their generic wire format.
  - Completion command: `cargo test -p zoomieball-gpu --test zoomie_schedule_parity`.

- [ ] **M2b: port Zoomieball perception, topology selection, rewards, mailboxes, and motor decoding.**
  - Prerequisite: generic GPU family execution and M0 CPU fixtures are available.
  - Normative anchor: control path and same-tick mailbox order in `../../DESIGN.md`, `../../GAME_TICK.md`, and `../../docs/controller-abi.md`.
  - Boundary test: 60 Hz perception/body, 15 Hz coach, and 120 Hz oracle/motor schedules match CPU controller and learning witnesses over a golden replay.
  - Completion command: `cargo test -p zoomieball-gpu --test controller_parity`.

- [ ] **M2b: remove the CPU shadow only from the primary WebGPU path after parity.**
  - Prerequisite: physics, controller, and learning parity suites all pass on the supported device tier.
  - Normative anchor: final GPU residency and authority transition in `../../docs/architecture.md`.
  - Boundary test: the primary path advances resident state with no CPU simulation authority while the shadow remains selectable for diagnostics.
  - Completion command: `cargo test -p zoomieball-gpu --test residency_gate`.

- [ ] **M3: expose a renderer-facing resident state source with no authoritative transfer.**
  - Prerequisite: the primary GPU path owns canonical simulation buffers.
  - Normative anchor: GPU-resident renderer seam in `../../docs/architecture.md`.
  - Boundary test: frame instrumentation reports zero authoritative uploads and zero readbacks while preserving camera and inspector access through explicit debug paths.
  - Completion command: `cargo test -p zoomieball-gpu --test render_residency`.

- [ ] **M4: evaluate acknowledged graph-v0 plays on the GPU.**
  - Prerequisite: root graph trigger/verb decisions are acknowledged and M2b controller parity passes.
  - Normative anchor: graph selection, oracle intent, assignments, and coach edges in `../../DESIGN.md` and `../../GAME_TICK.md`.
  - Boundary test: every checked-in play produces CPU-identical assignments, initial intent, edge logits, and transitions.
  - Completion command: `cargo test -p zoomieball-gpu --test graph_parity`.

- [ ] **M3–M5: meet the 100v100 resident WebGPU target without device-specific rates.**
  - Prerequisite: M2b residency and M3 renderer-source tests pass.
  - Normative anchor: fixed update schedule and WebGPU performance target in `../../DESIGN.md`.
  - Boundary test: supported devices retain 60/15/120 rates at 100v100; profiling may select a tier but never changes controller cadence.
  - Completion command: `cargo test -p zoomieball-gpu --release gpu_100v100_realtime`.
