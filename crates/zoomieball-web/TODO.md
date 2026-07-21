# `zoomieball-web` ownership

This package owns the browser application, its DOM HUD, and the permanent
Canvas2D fallback. It does not own simulation or controller semantics.

- [ ] **M1: build the fixed-camera Canvas2D presenter over CPU snapshots.**
  - Prerequisite: `zoomieball-render` publishes the stable packed CPU snapshot source.
  - Normative anchor: Canvas2D compatibility tier in `../../DESIGN.md` and `../../docs/architecture.md`.
  - Boundary test: a 10v10 match presents bodies, balls, labels, and arena at full DPR from one snapshot publication per frame.
  - Completion command: `cargo test -p zoomieball-web --test canvas2d_presenter`.

- [ ] **M1: connect CPU physics, perception, Zoomie inference/learning, and HUD without reduced semantics.**
  - Prerequisite: the Canvas2D presenter and M0 CPU golden path pass under WASM.
  - Normative anchor: permanent CPU fallback control path in `../../DESIGN.md`.
  - Boundary test: the browser tier preserves 60 Hz perception/body, 15 Hz same-tick coach mailboxes, 120 Hz oracle/motor/physics, rewards, and all witness layers.
  - Completion command: `cargo test -p zoomieball-web cpu_fallback && cargo check -p zoomieball-web --target wasm32-unknown-unknown`.

- [ ] **M1: meet the 10v10 Canvas2D real-time target.**
  - Prerequisite: the full compatibility path is connected and feel-tuning inputs are explicit match/play inputs.
  - Normative anchor: compatibility performance target in `../../DESIGN.md`.
  - Boundary test: the benchmark includes the full CPU simulation and one presentation publication without changing fixed controller rates.
  - Completion command: `cargo test -p zoomieball-web --release canvas2d_10v10_realtime`.

- [ ] **M5: select WebGPU or Canvas2D through device profiling.**
  - Prerequisite: M1 fallback and M2b/M3 resident WebGPU paths pass independently.
  - Normative anchor: tier selection and fixed-rate policy in `../../docs/architecture.md`.
  - Boundary test: a capability/profile fixture chooses the same tier deterministically, retains the fallback, and never chooses device-specific controller rates.
  - Completion command: `cargo test -p zoomieball-web --test tier_selection`.

- [ ] **M5: connect the WebGPU shell to resident simulation and rendering.**
  - Prerequisite: GPU residency gate, raw renderer seam, and dependency versions are acknowledged.
  - Normative anchor: final GPU residency contract in `../../docs/architecture.md`.
  - Boundary test: the primary path performs no authoritative state upload/readback and exposes the CPU shadow only as an explicit diagnostic mode.
  - Completion command: `cargo test -p zoomieball-web --test webgpu_residency`.

- [ ] **M5: implement the DOM HUD with parity across presentation tiers.**
  - Prerequisite: both tiers expose the same presentation and witness views.
  - Normative anchor: HUD and inspection responsibilities in `../../DESIGN.md`.
  - Boundary test: score, play, tier, timing, witness, label, and diagnostic views report identical match semantics for a shared replay.
  - Completion command: `cargo test -p zoomieball-web --test hud_parity`.

- [ ] **M5: import and export v0-local plays, checkpoints, and replay witnesses.**
  - Prerequisite: graph-v0, local controller checkpoint, and replay fixture shapes are stable.
  - Normative anchor: local-v0 mutability and sibling-wire preservation in `../../DESIGN.md` and `../../docs/controller-abi.md`.
  - Boundary test: browser round-trips current local artifacts, rejects malformed inputs at typed boundaries, and leaves sibling Zoomie wire bytes unchanged.
  - Completion command: `cargo test -p zoomieball-web --test import_export`.
