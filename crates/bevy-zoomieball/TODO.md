# `bevy-zoomieball` ownership

This package becomes the published engine wrapper and example at M3. It stays
dependency-free until the raw renderer seam and version decision are ready.

- [ ] **M3: select Bevy only after the wrapper's required feature surface is known.**
  - Prerequisite: the root Bevy/wgpu version decision is acknowledged from M2a and renderer evidence.
  - Normative anchor: dependency and engine-boundary contract in `../../docs/architecture.md`.
  - Boundary test: the selected dependency builds native and WASM targets without introducing an engine-owned simulation or controller path.
  - Completion command: `cargo check -p bevy-zoomieball --all-targets && cargo check -p bevy-zoomieball --target wasm32-unknown-unknown`.

- [ ] **M3: wrap the controller-independent raw renderer and both state sources.**
  - Prerequisite: `zoomieball-render` exposes stable CPU-snapshot and GPU-resident input seams.
  - Normative anchor: Bevy wrapper role in `../../docs/architecture.md`.
  - Boundary test: wrapper systems schedule presentation only, CPU uses one packed upload, and GPU uses no authoritative upload or readback.
  - Completion command: `cargo test -p bevy-zoomieball --test wrapper_contract`.

- [ ] **M3: publish an example covering free camera, contours, and perception inspection.**
  - Prerequisite: the wrapper renders both input sources and acknowledged cosmetic assets are available.
  - Normative anchor: renderer inspection surface in `../../docs/architecture.md`.
  - Boundary test: the example switches broadcast/free/first-person cameras, toggles arena and goal contours, and inspects one body's perception without mutating authoritative state.
  - Completion command: `cargo check -p bevy-zoomieball --example zoomieball`.

- [ ] **M3: prepare the package as a published, documented wrapper.**
  - Prerequisite: wrapper and example boundary tests pass with selected dependencies.
  - Normative anchor: published Bevy wrapper responsibility in `../../DESIGN.md`.
  - Boundary test: public documentation identifies supported state sources and authority constraints, and the packaged crate contains the example and required assets.
  - Completion command: `cargo package -p bevy-zoomieball --allow-dirty`.
