# Zoomieball

Zoomieball is a deterministic ball-sport simulation with batched Zoomie controllers, a permanent CPU execution tier, and a WebGPU-primary delivery architecture under construction.

## Quickstart

Run the live CPU tracer with 10 active bodies per team for 256 body ticks:

```sh
cargo run -p zoomieball-headless -- 10 256
cargo test --workspace
```

The first argument is the active roster size per team (`10` or `100`). The second is the number of 60 Hz body ticks. Each body tick rebuilds perception, pulses the body populations once, and runs two 120 Hz physics steps; coaches pulse every fourth body tick at 15 Hz. Team-local ID `00` is the goalie, IDs `01..99` are fielders, and ID `100` is the nonphysical coach.

Compare native and WASI tracer witnesses with identical roster and tick arguments:

```sh
cargo build --release -p zoomieball-headless
cargo build --release -p zoomieball-headless --target wasm32-wasip1
target/release/zoomieball-headless 10 60 --hashes
node --no-warnings scripts/run-wasi.mjs target/wasm32-wasip1/release/zoomieball-headless.wasm 10 60 --hashes
```

`--hashes` emits `physics`, `controller`, `learning`, and `pipeline` fields and omits wall-clock data, so the two streams can be compared byte-for-byte. Their layering and widths already match [the controller ABI](docs/controller-abi.md#witnesses); golden M0 conformance still depends on replacing the tracer physics and control behaviors beneath them.

## Building

Zoomieball path-depends on sibling [Zoomie](https://github.com/jaelsasser/zoomie) for `zoomie-core`, `zoomie-math`, and `zoomie-pop`, and expects that checkout beside this one:

```
zoomieball/
zoomienet/
```

Cargo builds whatever revision is checked out there, uncommitted edits included, and nothing here records which one. Witnesses and golden replays are therefore reproducible only against a sibling revision you have tracked by hand.

## Packages

| Package | Responsibility | Current state |
|---|---|---|
| `zoomieball-core` | CPU scalar simulation, play graph, perception oracle, typed controller batches, rewards, and replay witnesses | Live tracer; M0 conformance work remains |
| `zoomieball-controller` | CPU Zoomie populations, encoding, learning, mailboxes, and checkpoints | Live tracer at the fixed 60/15 Hz schedule |
| `zoomieball-gpu` | Zoomieball-specific GPU physics, perception, controller integration, and parity harness | Scaffold |
| `zoomieball-render` | Controller-independent renderer contracts and render-owned CPU presentation snapshots | CPU seam and resident-source contract live; raw GPU renderer pending |
| `bevy-zoomieball` | Published Bevy wrapper and example | Scaffold; Bevy version deliberately unselected |
| `zoomieball-web` | WebGPU application, Canvas2D compatibility presenter, and DOM HUD | Scaffold; wgpu version deliberately unselected |
| `zoomieball-headless` | CPU runner, replay debugger, benchmark, and later CPU/GPU parity driver | Runner live; parity tooling pending |

Each package owns a `TODO.md`; the [root roadmap](TODO.md) records cross-package milestones. Scaffold packages remain dependency-light until their milestone starts.

## Execution tiers

The deterministic CPU implementation remains permanent. It is the conformance oracle, headless implementation, replay debugger, and 10v10 Canvas2D fallback. During WebGPU bring-up it also runs as a lockstep shadow: CPU commands feed GPU physics and the two paths compare per-step witnesses. The primary WebGPU path becomes GPU-resident only after physics, controller, and learning parity pass; authoritative state is then neither uploaded for rendering nor read back for presentation.

The checked-in [default playbook](assets/default-playbook.ron) is the current cyclic graph-v0 tracer fixture. Trigger and per-ball verb/target forms are intentionally not encoded until those schema decisions are acknowledged.

## Determinism boundary

State-feeding work uses Q16.16 values, widened integer intermediates, canonical IDs, and a fixed update schedule. Floating point belongs only to render-owned presentation data, and rendering has no return edge into simulation. `TickHash` separates a commutative `u32` `physics` hash, `u64` Zoomie `controller` and `learning` checksums, and a diagnostic `u64` `pipeline` fold.

## See also

- [Normative tick contract](GAME_TICK.md)
- [System design](DESIGN.md)
- [Architecture](docs/architecture.md)
- [Controller ABI](docs/controller-abi.md)
- [Roadmap](TODO.md)
