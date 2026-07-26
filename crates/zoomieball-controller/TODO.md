# `zoomieball-controller` ownership

The CPU Zoomie populations remain active as the compatibility implementation
and serial oracle for GPU bring-up.

- [x] **Alignment: change body and coach population clocks to 60 Hz and 15 Hz.**
  - Prerequisite: the core request batch exposes the 60 Hz body tick and every-fourth-tick coach schedule.
  - Normative anchor: controller pulse order in `../../GAME_TICK.md` and timing ABI in `../../docs/controller-abi.md`.
  - Boundary test: coaches publish mailboxes before the same-tick body pulse, while oracle/motor refreshes do not repulse either population at 120 Hz.
  - Completion command: `cargo test -p zoomieball-controller timing`.

- [ ] **M0: conform fielder, goalie, and coach encodings without bypassing the batch API.**
  - Prerequisite: core perception lanes, squad assignments, rewards, and typed requests are stable.
  - Normative anchor: population topology and mailbox/cue contracts in `../../DESIGN.md` and `../../docs/controller-abi.md`.
  - Boundary test: fovea, lane-layout, cue-charge, squad mailbox, and edge-logit fixtures pass for all three population families.
  - Completion command: `cargo test -p zoomieball-controller encoding`.

- [ ] **M0: settle the observation encoding frame across all three populations.**
  - Prerequisite: the observation encoding frame is acknowledged in the root roadmap.
  - Normative anchor: lane layout as a conformance surface in `../../DESIGN.md` and the blocked semantic-mirror mapping list in `../../GAME_TICK.md`.
  - Boundary test: one tactical situation and its team-exchanged mirror drive the fielder retina, goalie foveae, coach union retina, oracle-direction, and proprioception lanes to the acknowledged frame's mapping, `receptor` reads that frame rather than raw world-space sign octants, and `encode_goalie_foveae` takes no ignored `Team` parameter.
  - Completion command: `cargo test -p zoomieball-controller --test encoding_frame`.

- [ ] **M0: update the local checkpoint header in place.**
  - Prerequisite: final 60/15 timing fields and graph-v0 topology identifiers are known.
  - Normative anchor: v0 persistence and sibling-wire-format constraints in `../../docs/controller-abi.md`.
  - Boundary test: current checkpoints reject mismatched timing/topology cleanly, round-trip learning state and mailboxes, and no migration reader is introduced.
  - Completion command: `cargo test -p zoomieball-controller checkpoint`.
  - Progress: the population payload now rides `zoomie-wire`'s length-prefixed `ZNETLIVE` pack, which carries the specs, configs, rules, manifests, and resume cursor and validates the recomputed capability manifest on decode; only the graph-v0 topology identifiers in the local header remain, and they are still unacknowledged.

- [x] **M0: expose controller and learning checksums independently over their complete word sets.**
  - Prerequisite: population stepping and scheduled learning conform to sibling Zoomie's serial semantics, and the backend folds sibling `Population::<SparseCtrnn>::inference_pair` and `learning_pair` rather than `checksum_pair`, which reaches the state row alone.
  - Normative anchor: layered witness contract in `../../DESIGN.md` and the `controller`/`learning` witness rows in `../../docs/controller-abi.md`.
  - Boundary test: a mutated live weight changes the controller checksum, a mutated eligibility, anchor, or credit-age word changes the learning checksum, neither witness reaches the other's words or is inferred from the physics hash, and no act or learn pulse allocates.
  - Completion command: `cargo test -p zoomieball-controller checksum`.

- [ ] **M0/M1: produce native/WASM controller goldens and retain 10v10 learning.**
  - Prerequisite: timing, encodings, checkpoints, and checksum layers conform.
  - Normative anchor: CPU fallback and deterministic schedule in `../../DESIGN.md`.
  - Boundary test: native and WASM fixtures agree across inference and scheduled learning while a 10v10 compatibility match keeps all populations active.
  - Completion command: `cargo test -p zoomieball-controller --test golden_replays && cargo check -p zoomieball-controller --target wasm32-unknown-unknown`.
  - Progress: `tests/witness_golden.rs` pins both component witnesses and the checkpoint bytes for a fixed 10v10 match, reproduced identically by `zoomieball-headless 10 60 --hashes` on native and wasm32-wasip1; it deliberately asserts nothing about lane semantics, so the full replay goldens stay blocked behind the observation encoding frame decision and the two encoding bites above it.
