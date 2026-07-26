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

Coach output lanes `0..64` are eight independent eight-lane mailboxes. Lanes `64..72` are logits corresponding by ordinal to the node's eight possible edge ports. Ports absent from the current node are masked. Coach output is advisory; latched match/play input remains authoritative over graph traversal. `ActRequest` carries `play_node` and `enabled_edges` as pairs indexed by `Team::index`, and each coach column encodes its own team's entries: a team's logits gate only that team's transitions, and human overrides reach only the player's team. `ControllerBackend::edge_logits(team)` is how step 2 reads the previous pulse's lanes, since `act` runs at step 4 and cannot answer a step-2 question.

Body output lanes `0..=2` are signed spin residuals. Lanes `3`, `4`, and `5` gate jump, boost, and the air cue. Lanes `6` and `7` locate the cue hit. Inputs are midpoint-biased into Zoomie's unsigned unit-lane domain; outputs remain bipolar. Oracle steering is recomputed for each physics step and combined with these latched residuals rather than being learned into a device-specific update rate.

## Graph-v0 playbook

### Accepted RON

Keys are positional: a node reads `name`, `edges`, `squad_cycle`, `coach_gate`, `goalie_verb`, `verbs`, `goalie`, `fielder` in that order. This is the first node of [`assets/default-playbook.ron`](../assets/default-playbook.ron) verbatim:

```ron
(
  name: "press",
  edges: [
    (to: 1, trigger: BallBehind(-8.0)),
    (to: 0, trigger: Always),
  ],
  squad_cycle: [0, 1, 2, 3, 4, 5, 6, 7],
  coach_gate: 0.0,
  goalie_verb: (verb: Guard, target: GameBall, form: Point),
  verbs: [
    (verb: Drive,  target: GameBall,        form: Point),
    (verb: Pursue, target: GameBall,        form: Wedge(1.5)),
    (verb: Block,  target: NearestOpponent, form: Pod(2, 3, 1.5)),
    (verb: Lead,   target: GameBall,        form: Arc(2.0)),
    (verb: Cover,  target: NearestOpponent, form: Point),
    (verb: Zone,   target: GameBall,        form: Pod(1, 4, 2.5)),
    (verb: Sweep,  target: GameBall,        form: Point),
    (verb: Align,  target: Slot,            form: Pod(1, 11, 0.5)),
  ],
  goalie:  (position: [-14.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
  fielder: (position: [2.0, 0.0, 1.0],   spin: [0.0, 0.0, 0.0]),
)
```

Nodes may be cyclic. `edges` is one to eight ordered ports of `(to, trigger)`, and the last must carry `Always` so the fixed eight-iteration scan cannot fall off the end. `verbs` is exactly eight `(verb, target, form)` entries, one per squad, with `goalie_verb` as the ninth. `squad_cycle` is nonempty, contains only squad indices `0..=7`, and assigns a player by `local_id % squad_cycle.len()`; the quotient is that body's ordinal within its squad and chooses its formation slot. The role templates are `Align`'s aim point rather than a term every verb carries, and team one resolves them by a half turn about `+z`. The solver emits no cue gates: a verb supplies oracle intent alone, so the extension leaves the oracle/residual split intact.

The compiler rejects an unsupported `version`, empty graph, duplicate node names, dangling edges, a node whose last port is not `Always`, a `verbs` table that is not eight entries, squad indices above seven, pod extents or form gaps outside their authored bounds, any word outside the acknowledged vocabulary, malformed syntax, and decimals that do not fit Q16.16. Numeric literals convert directly to Q16.16 without an intermediate float. Zoomieball-local playbooks are v0 artifacts: the fixture and parser change together without a migration reader.

### Graph-v0 semantics

One schema carries all of these, rather than a parallel playbook representation:

| Node concern | Semantic contract | Encoding status |
|---|---|---|
| Triggers | Determine which ordered outgoing ports are eligible from latched match state | Live over the [graph-v0](graph-v0-proposal.md) vocabulary; the port scan is a fixed eight iterations and the last port must be `Always` |
| Ball actions | Select a typed verb, target, and formation for each of the eight squads, plus the goalie | Live as a squad-indexed `(verb, target, form)` table; the table is ABI-width eight and `squad_cycle` chooses which entries a roster reaches |
| Squad assignments | Map embodied local IDs to one of eight coach mailboxes | Live as `squad_cycle`; a body's ordinal within its squad now also chooses its formation slot |
| Oracle intent | Supply the playbook component recomputed before each physics step | Live role templates are a tracer; the acknowledged verb table supplies each body's aim point and formation slot |
| Coach edges | Associate the eight coach logits with ordered graph ports, masking absent or ineligible ports | Port association fixed; `CoachEdge` gating acknowledged, and transition policy remains match/play-input authoritative |

`PLAYBOOK_ABI_VERSION` is 2. Parser, fixture, and tests moved together; a version-1 file is rejected rather than migrated.

## Witnesses

| `TickHash` field | Type and source | Use |
|---|---|---|
| `physics` | `u32`, `World::physics_hash()` | Commutative hash of canonical per-body physics words; normative CPU/WGSL comparison |
| `controller` | `u64`, `ControllerBackend::controller_hash()` | Sibling Zoomie `inference_pair` parity — live weights, node states — plus the coach mailboxes and edge logits |
| `learning` | `u64`, `ControllerBackend::learning_hash()` | Sibling Zoomie `learning_pair` parity — anchors, exploration keys, eligibility, credit ages — plus each pool's `ExploratoryHebbRule` dials, the accumulated team rewards, and the learning-pass counter |
| `pipeline` | `u64`, diagnostic fold | ABI words, play node, `World::diagnostic_hash()`, and the three component witnesses for replay localization |

The physics hash is schedule-independent because per-body hashes combine with wrapping addition. Controller and learning checksums retain sibling Zoomie's established arithmetic and wire semantics. Each component witness folds its own layer and no other, so a divergence localizes to the single word that moved.

Localization is unconditional, but read-set coverage is not: every pool arms a learning rule, so an armed `step` reads the exploration key and dials and writes eligibility and the credit age before any learn pass runs. `controller` alone therefore never answers "will these two step alike" — that question needs both witnesses, and the `learning` witness is where the update rule's own parameters live. Two backends agreeing on `controller` may still be about to diverge.

`REPLAY_ABI_VERSION = 3` identifies the fold in which the component witnesses moved onto sibling Zoomie's split population witnesses and `learning` took each pool's rule dials. The word names the fold, not the state: records carrying different words are incomparable, and a mismatch there is an ABI change rather than state drift. The pipeline fold is diagnostic; equality of that fold alone does not replace component comparison.

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

`schedule_abi = 1` identifies the fixed 60 Hz body, 15 Hz coach, and 120 Hz oracle/motor/physics contract. `ZoomieBackend` prefixes the common header with the four bytes `ZBCT`, so its backend payload starts at byte 22. The header owes two more graph fields at offsets not yet assigned — each cursor's node-entry tick, and the most recent game-ball touch's tick and touching team — under the *bind the learning schedule and physics configuration to replay and checkpoint state* bite in `crates/zoomieball-core/TODO.md`.

That payload is a little-endian `u32` length prefix, then one `zoomie-wire` `ZNETLIVE` pack, then the backend-local transient tail. The wire pack owns the three populations with their specs, configs, learning rules, and capability manifests, plus the resume cursor; `zoomie-wire` recomputes each expected manifest on decode, so a capability divergence is rejected at the boundary instead of resuming into a silent desync. The length prefix is load-bearing: `decode_live` rejects trailing bytes and so must be handed an exact slice, which the fixed-width tail behind it would otherwise deny it. The tail carries the mailboxes, edge logits, accumulated team rewards, and the body-pulse, coach-pulse, and learning-pass counters. Together they are a bit-identical controller resume, including the rule dials — which the restoring backend adopts from the checkpoint rather than keeping its own, since those dials steer the trajectory and belong to the persisted team.

The checkpoint envelope is local test data rather than a stable import contract. No migration reader is required for a Zoomieball-local header change.

## Version scope

| Contract | Current word | Compatibility rule |
|---|---:|---|
| Playbook (`PLAYBOOK_ABI_VERSION`) | 2 | Zoomieball-local v0; update parser, fixture, and tests together |
| Controller lanes (`LANE_ABI_VERSION`) | 1 | Zoomieball-local v0 |
| Physics (`PHYSICS_ABI_VERSION`) | 1 | Zoomieball-local v0 |
| Reward (`REWARD_ABI_VERSION`) | 1 | Zoomieball-local v0 |
| Fixed schedule (`SCHEDULE_ABI_VERSION`) | 1 | Required identity: 60 Hz body, 15 Hz coach, 120 Hz oracle/motor/physics |
| Replay fold (`REPLAY_ABI_VERSION`) | 3 | Zoomieball-local v0; bump on any change to a component witness fold |
| Sibling Zoomie arithmetic and persistence | Established by Zoomie | Preserve its wire formats |
