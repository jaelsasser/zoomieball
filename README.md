# Zoomieball

A deterministic headless ball-sport simulation that drives bulk Zoomie controller populations and publishes one-way render snapshots.

## Quickstart

```sh
cargo run -p zoomieball-headless -- 10 256
cargo test --workspace
```

The first argument selects the active roster size per team (`10` or `100`); the second selects the number of 64 Hz ticks. Team-local ID `00` is the goalie, IDs `01..99` are fielders, and ID `100` is the nonphysical coach.

Native/WASI parity can be witnessed with the same roster and tick arguments:

```sh
cargo build --release -p zoomieball-headless
cargo build --release -p zoomieball-headless --target wasm32-wasip1
target/release/zoomieball-headless 10 64 --hashes
node scripts/run-wasi.mjs target/wasm32-wasip1/release/zoomieball-headless.wasm 10 64 --hashes
```

The `--hashes` outputs world, controller, learning, and combined witnesses for every tick and omits wall-clock fields, so the native and WASI streams can be compared byte-for-byte.

## Crates

| Crate | Responsibility |
|---|---|
| `zoomieball-core` | Fixed-point world, play compilation, perception, physics, rewards, replay hashes, and snapshots |
| `zoomieball-controller` | The only dependency edge to the sibling Zoomie workspace; owns body and coach populations |
| `zoomieball-render` | Controller-agnostic packed-frame and one-upload rendering contract |
| `zoomieball-headless` | Native match runner for deterministic smoke and performance checks |

## Determinism

The authoritative path uses Q16.16 values, widened integer intermediates, canonical body order, two fixed physics substeps, fixed collision iterations, and FNV-1a witnesses. Floating point first appears in `RenderSnapshot` publication. A tick never reads rendering state back into the world.

The RON playbook reader accepts the checked-in fixed-schema subset documented in [Controller ABI](docs/controller-abi.md#playbook-schema). Decimal literals are converted directly to Q16.16 without an intermediate float.

## Current delivery boundary

The workspace implements the authoritative match pipeline and the renderer upload seam. Raw WebGPU shaders, the Bevy 0.19 wrapper, browser shell, and OCI persistence remain later delivery layers; their boundaries do not feed data back into simulation.

## See also

- [Architecture](docs/architecture.md)
- [Controller ABI](docs/controller-abi.md)
- [Default cyclic playbook](assets/default-playbook.ron)
