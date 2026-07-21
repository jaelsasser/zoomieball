# Controller ABI

The controller ABI is a batch-oriented boundary between deterministic match state and one statically dispatched controller backend. It contains typed simulation inputs, latched controller outputs, rewards, checkpoints, and witnesses. Presentation data is not part of this ABI.

## Batch contracts

| Type | Storage | Contract |
|---|---|---|
| `WorldView` | Borrowed structure-of-arrays slices | Canonical physical-body order; coaches have no body entry; match metadata remains typed rather than packed into controller strings |
| `ObservationBatch` | CSR offsets plus `RayObservation` records | One range per body; deterministic target order; complete forward 180-degree field before encoding |
| `OracleIntentBatch` | Caller-owned reusable vector | One initial intent per physical body, refreshed into steering before each physics step |
| `MotorCommandBatch` | Caller-owned reusable vector | Three spin residual lanes, three cue gates, and two cue-hit coordinates per physical body |
| `RewardBatch` | Caller-owned reusable vector | Continuous progress and sparse event terms accumulated until scheduled learning |

`ControllerBackend::act`, `learn`, `checkpoint`, and `restore` reuse caller-owned buffers. The match is generic over the backend, so hot dispatch is static. Allocation and parsing are permitted at initialization and checkpoint boundaries, not as an incidental part of a controller pulse.

The CPU `RenderSnapshot` is owned by `zoomieball-render`; `RenderSnapshot::publish(&World)` converts typed core state to packed cosmetic `f32` instances once per CPU publication. A GPU source implements `ResidentStateSource`, and `Renderer::render_resident()` performs no authoritative-state upload or readback. Neither representation appears in `ControllerBackend`.

## Schedule

| Work | Rate | Ordering contract |
|---|---:|---|
| Match/play input latch | 60 Hz | Precedes graph resolution |
| Graph assignment and perception | 60 Hz | Produces the body frame for the current tick |
| Coach populations | 15 Hz | Pulse on every fourth body tick before embodied populations |
| Squad mailbox publication | 15 Hz | A due coach write is visible to bodies in the same 60 Hz tick |
| Fielder and goalie populations | 60 Hz | Produce residuals and cue gates latched across both physics steps |
| Oracle steering and motor combination | 120 Hz | Refresh immediately before each physics step |
| Physics | 120 Hz | Two ordered steps per body tick |
| Learning | Scheduled | Runs after reward accumulation and before final witness publication |

The core exports this identity as `BODY_HZ = 60`, `COACH_HZ = 15`, `PHYSICS_HZ = 120`, `PHYSICS_SUBSTEPS = 2`, and `COACH_INTERVAL_TICKS = 4`; `TICK_HZ` aliases `BODY_HZ`. Controller rates are part of conformance. A device tier may change workload placement, not pulse frequency.

## Population lanes

| Population | Topology | Input layout | Output layout |
|---|---:|---|---|
| Fielder | 64/48/8 | 40 retina, 8 oracle, 8 mailbox, 8 proprioception | Motor command |
| Goalie | 96/64/8 | Fielder frame, three 8-lane foveae, 8 corridor lanes | Motor command |
| Coach | 128/96/72 | 80 union retina, 16 formation, 8 threat, 8 node, 8 edges, 8 match | Eight 8-lane squad mailboxes, then 8 edge logits |

Coach output lanes `0..64` are eight independent eight-lane mailboxes. Lanes `64..72` are logits corresponding by ordinal to the node's eight possible edge ports. Ports absent from the current node are masked. Coach output is advisory; latched match/play input remains authoritative over graph traversal.

Body output lanes `0..=2` are signed spin residuals. Lanes `3`, `4`, and `5` gate jump, boost, and the air cue. Lanes `6` and `7` locate the cue hit. Inputs are midpoint-biased into Zoomie's unsigned unit-lane domain; outputs remain bipolar. Oracle steering is recomputed for each physics step and combined with these latched residuals rather than being learned into a device-specific update rate.

## Graph-v0 playbook

### Live tracer subset

The current parser accepts this fixed RON subset, represented by [`assets/default-playbook.ron`](../assets/default-playbook.ron):

```ron
(
  version: 1,
  nodes: [
    (
      name: "shape",
      edges: [0],
      squad_cycle: [0, 1, 2, 3, 4, 5, 6, 7],
      goalie: (position: [-14.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
      fielder: (position: [0.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
    ),
  ],
)
```

Nodes may be cyclic. `edges` contains one to eight ordered target-node indices. `squad_cycle` is nonempty, contains only squad indices `0..=7`, and assigns a player by `local_id % squad_cycle.len()`. The role templates supply position and spin; team one mirrors the attack-axis components, and local IDs receive a deterministic formation spread. The tracer solver emits no cue gates.

The compiler rejects an unsupported `version`, empty graph, duplicate node names, dangling edges, invalid edge or squad counts, malformed syntax, and decimals that do not fit Q16.16. Numeric literals convert directly to Q16.16 without an intermediate float. Zoomieball-local playbooks are v0 artifacts: the fixture and parser change together without a migration reader.

### Required graph-v0 growth

One schema, rather than a parallel playbook representation, will grow to express these semantics:

| Node concern | Semantic contract | Encoding status |
|---|---|---|
| Triggers | Determine which ordered outgoing ports are eligible from latched match state | Concrete trigger vocabulary and RON shape blocked |
| Per-ball actions | Select a typed verb and typed target for each canonical ball/objective entry | Verb set, target variants, and table shape blocked |
| Squad assignments | Map embodied local IDs to one of eight coach mailboxes | Live as `squad_cycle`; richer assignment form not selected |
| Oracle intent | Supply the playbook component recomputed before each physics step | Live role templates are a tracer; per-ball form follows the action-table decision |
| Coach edges | Associate the eight coach logits with ordered graph ports, masking absent or ineligible ports | Port association fixed; transition policy remains match/play-input authoritative |

No trigger or verb syntax is implied by the current fixture. Those forms require explicit acknowledgement before the parser or asset grows.

## Witnesses

| `TickHash` field | Type and source | Use |
|---|---|---|
| `physics` | `u32`, `World::physics_hash()` | Commutative hash of canonical per-body physics words; normative CPU/WGSL comparison |
| `controller` | `u64`, `ControllerBackend::controller_hash()` | Sibling Zoomie inference and transient-state parity |
| `learning` | `u64`, `ControllerBackend::learning_hash()` | Eligibility, rewards, gates, and parameter-update parity |
| `pipeline` | `u64`, diagnostic fold | ABI words, play node, `World::diagnostic_hash()`, and the three component witnesses for replay localization |

The physics hash is schedule-independent because per-body hashes combine with wrapping addition. Controller and learning checksums retain sibling Zoomie's established arithmetic and wire semantics. The pipeline fold is diagnostic; equality of that fold alone does not replace component comparison.

Mirrored-state conformance does not compare raw hash values. A future transformed comparison must first specify mappings for polar vectors, axial vectors, team labels, commands, IDs, and event records.

## Checkpoints

Zoomieball checkpoints have a local envelope around sibling Zoomie population payloads. The local envelope is a v0 artifact and may change in place; sibling Zoomie's established population formats do not.

Restore validates the complete local header and payload shape before mutating a backend. The common header is 18 bytes:

| Header offset | Type | Field |
|---:|---|---|
| 0 | little-endian `u32` | `lane_abi` |
| 4 | little-endian `u32` | `physics_abi` |
| 8 | little-endian `u32` | `reward_abi` |
| 12 | little-endian `u32` | `schedule_abi` |
| 16 | little-endian `u16` | `active_per_team` |

`schedule_abi = 1` identifies the fixed 60 Hz body, 15 Hz coach, and 120 Hz oracle/motor/physics contract. `ZoomieBackend` prefixes the common header with the four bytes `ZBCT`, so its backend payload starts at byte 22. That payload retains the three populations, mailboxes, edge logits, accumulated team rewards, and the body-pulse, coach-pulse, and learning-pass counters needed for a bit-identical controller resume.

The checkpoint envelope is local test data rather than a stable import contract. No migration reader is required for a Zoomieball-local header change.

## Version scope

| Contract | Current word | Compatibility rule |
|---|---:|---|
| Playbook (`PLAYBOOK_ABI_VERSION`) | 1 | Zoomieball-local v0; update parser, fixture, and tests together |
| Controller lanes (`LANE_ABI_VERSION`) | 1 | Zoomieball-local v0 |
| Physics (`PHYSICS_ABI_VERSION`) | 1 | Zoomieball-local v0 |
| Reward (`REWARD_ABI_VERSION`) | 1 | Zoomieball-local v0 |
| Fixed schedule (`SCHEDULE_ABI_VERSION`) | 1 | Required identity: 60 Hz body, 15 Hz coach, 120 Hz oracle/motor/physics |
| Replay fold (`REPLAY_ABI_VERSION`) | 1 | Zoomieball-local v0 |
| Sibling Zoomie arithmetic and persistence | Established by Zoomie | Preserve its wire formats |
