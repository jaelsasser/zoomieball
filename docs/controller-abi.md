# Controller ABI

The controller ABI is a batch-oriented set of typed borrowed views passed through a statically dispatched backend.

## Batch contracts

| Type | Storage | Contract |
|---|---|---|
| `WorldView` | SoA borrowed slices | Canonical physical-body order; no coach entries |
| `ObservationBatch` | CSR offsets plus `RayObservation` records | One range per body; canonical target order |
| `OracleIntentBatch` | Preallocated vector | Desired world position and spin only |
| `MotorCommandBatch` | Preallocated vector | Three spin residuals, three gates, two cue coordinates |
| `RewardBatch` | Preallocated vector | Continuous progress and sparse event terms |
| `RenderSnapshot` | Packed cosmetic instances | Only `f32` conversion boundary |

`ControllerBackend::act`, `learn`, `checkpoint`, and `restore` operate on caller-owned buffers. The match is generic over the backend, so hot dispatch is static. `controller_hash` and `learning_hash` are included in every tick witness.

## Lane layouts

| Population | Topology | Layout |
|---|---:|---|
| Fielder | 64/48/8 | 40 retina, 8 oracle, 8 mailbox, 8 proprioception |
| Goalie | 96/64/8 | Fielder frame plus three 8-lane foveae and 8 corridor lanes |
| Coach | 128/96/72 | 80 union retina, 16 formation, 8 threat, 8 node, 8 edges, 8 match |

Coach output lanes `0..64` are eight independent eight-lane mailboxes. Lanes `64..72` are graph-edge logits. Human play selection remains authoritative.

Body outputs `0..3` are signed spin residuals. Lanes 3, 4, and 5 gate jump, boost, and the air cue. Lanes 6 and 7 locate the cue hit. Inputs are midpoint-biased into Zoomie's unsigned unit lane domain; outputs remain bipolar.

## Playbook schema

The accepted RON subset is:

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

Nodes may be cyclic. Each node has one to eight ordered edge ports and a nonempty squad cycle whose values are `0..7`. The compiler rejects version mismatches, dangling edges, duplicate node names, invalid fixed decimals, and malformed schemas. A role template is mirrored along the attack axis for team one, then spread deterministically by local ID; the play solver never emits cue gates.

## Version words

| ABI | Version |
|---|---:|
| Playbook | 1 |
| Controller lanes | 1 |
| Physics | 1 |
| Reward | 1 |
| Replay hash | 1 |

Checkpoint restore fails before mutation when any word differs.

