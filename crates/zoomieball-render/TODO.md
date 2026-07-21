# `zoomieball-render` ownership

This crate owns cosmetic snapshots and the controller-independent raw renderer.
It accepts CPU publications or GPU-resident state without becoming simulation
authority.

- [x] **Alignment: move cosmetic snapshot construction out of core.**
  - Prerequisite: core exposes a borrowed authoritative presentation view without cosmetic `f32` storage.
  - Normative anchor: renderer ownership and authority boundaries in `../../docs/architecture.md`.
  - Boundary test: the CPU path builds and publishes exactly one packed snapshot per presentation frame, and core owns no cosmetic snapshot type.
  - Completion command: `cargo test -p zoomieball-render snapshot`.

- [ ] **M1: stabilize the CPU snapshot source for Canvas2D.**
  - Prerequisite: render-owned snapshot construction passes the Alignment publication test.
  - Normative anchor: permanent CPU presentation path in `../../DESIGN.md`.
  - Boundary test: a 10v10 snapshot carries bodies, balls, labels, and HUD-facing state without exposing controller internals or requiring per-object publication.
  - Completion command: `cargo test -p zoomieball-render cpu_snapshot_source`.

- [ ] **M3: split raw rendering over CPU-snapshot and GPU-resident state sources.**
  - Prerequisite: M2a establishes resident canonical buffers and the CPU snapshot source is stable.
  - Normative anchor: dual renderer input contract in `../../docs/architecture.md`.
  - Boundary test: the CPU source performs one packed upload, the GPU source performs zero authoritative-state uploads and zero readbacks, and both feed the same raw draw path.
  - Completion command: `cargo test -p zoomieball-render state_source`.

- [ ] **M3: retain camera, DPR, contours, and perception inspection across both sources.**
  - Prerequisite: the dual state-source seam drives the raw renderer.
  - Normative anchor: inspection and presentation responsibilities in `../../docs/architecture.md`.
  - Boundary test: broadcast/free/first-person cameras, full-DPR sizing, independent arena/goal contours, and perception overlays behave identically for CPU and resident inputs.
  - Completion command: `cargo test -p zoomieball-render`.

- [ ] **M3: implement the hard-light arena after cosmetic choices are acknowledged.**
  - Prerequisite: palette and font assets are acknowledged in the root roadmap.
  - Normative anchor: non-authoritative presentation contract in `../../DESIGN.md` and `../../docs/architecture.md`.
  - Boundary test: cosmetic changes leave all physics, controller, learning, and pipeline witnesses unchanged.
  - Completion command: `cargo test -p zoomieball-render --test presentation_contract`.
