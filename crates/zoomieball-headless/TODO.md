# `zoomieball-headless` ownership

The CPU runner and WASI entrypoint remain live. This package grows into the
replay debugger, benchmark driver, and later CPU/GPU parity frontend.

- [x] **Alignment: report the 60/15/120 schedule and layered witnesses honestly.**
  - Prerequisite: core and controller publish the aligned clocks and four witness layers.
  - Normative anchor: tick schedule in `../../GAME_TICK.md` and witness contract in `../../DESIGN.md`.
  - Boundary test: CLI output distinguishes physics, controller, learning, and diagnostic folds and contains no 64 Hz benchmark wording.
  - Completion command: `cargo test -p zoomieball-headless --test cli`.

- [ ] **M0: add deterministic replay recording and first-divergence debugging.**
  - Prerequisite: the public CPU tracer and v0 golden fixture schema are stable.
  - Normative anchor: replay witness policy in `../../DESIGN.md`.
  - Boundary test: replaying one input stream reproduces every witness, while one corrupted tick reports the first differing layer and tick.
  - Completion command: `cargo test -p zoomieball-headless --test replay_debugger`.

- [ ] **M0: certify native/WASI execution against the same golden replays.**
  - Prerequisite: replay recording and all CPU/controller witness fixtures pass natively.
  - Normative anchor: permanent headless/WASI contract in `../../docs/architecture.md`.
  - Boundary test: native and WASI runs consume the same fixture and emit identical physics, controller, learning, and pipeline witnesses.
  - Completion command: `cargo test -p zoomieball-headless && cargo build --release -p zoomieball-headless --target wasm32-wasip1 && node --no-warnings scripts/run-wasi.mjs target/wasm32-wasip1/release/zoomieball-headless.wasm 10 60 --hashes`.

- [ ] **M1: benchmark the permanent 10v10 CPU compatibility tier.**
  - Prerequisite: M0 conformance and the Canvas2D workload shape are stable.
  - Normative anchor: 10v10 real-time target and 60/120 schedule in `../../DESIGN.md`.
  - Boundary test: the benchmark runs physics, perception, inference, scheduled learning, rewards, and publication rather than a reduced simulation loop.
  - Completion command: `cargo run --release -p zoomieball-headless -- 10 7200`.

- [ ] **M2a/M2b: drive CPU/GPU parity and stage bisection.**
  - Prerequisite: the GPU crate exposes physics, controller, and learning witness streams.
  - Normative anchor: CPU-shadow bring-up and shadow-removal gates in `../../docs/architecture.md`.
  - Boundary test: the runner locates first divergence by tick, witness layer, and physics stage, and refuses primary-GPU status until every layer passes.
  - Completion command: `cargo test -p zoomieball-headless --test gpu_parity_driver`.

- [ ] **M3–M5: retain the 100v100 WebGPU performance target in the benchmark frontend.**
  - Prerequisite: GPU-resident controller and renderer paths are integrated.
  - Normative anchor: 100v100 WebGPU target in `../../DESIGN.md`.
  - Boundary test: the benchmark includes graph, perception, controller learning, physics, and resident presentation accounting with no authoritative readback.
  - Completion command: `cargo run --release -p zoomieball-headless -- 100 7200`.
