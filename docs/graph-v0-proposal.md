# graph-v0 trigger, verb, target, and coach-edge proposal

Status: proposal, 2026-07-25. This document is the artifact named by the
*Acknowledge graph triggers and verb/target shapes* gate in [`../TODO.md`](../TODO.md).
It is not normative. Every table below is a concrete candidate to acknowledge or
reject, not a survey of possibilities. Acknowledgment moves these shapes into
[`../DESIGN.md`](../DESIGN.md) and [`../GAME_TICK.md`](../GAME_TICK.md) and unblocks the
*extend the single graph-v0 schema in place* bite in
[`../crates/zoomieball-core/TODO.md`](../crates/zoomieball-core/TODO.md).

This document exists so that bite is not its own prerequisite. It describes a schema
that does not exist yet, using only vocabulary the settled documents already establish.

## What is already settled

These constrain every proposal below and are not reopened here.

- There is one cyclic graph-v0 schema, extended in place. RON is the at-rest form and
  flat tables are the execution form. Zoomieball-local v0 artifacts need no migration
  reader. (`DESIGN.md` settled constraints 5 and 10.)
- Step 2 of the tick order resolves triggers, per-ball verb/target entries, squad
  assignments, enabled coach edges, and the initial deterministic intent, once per
  60 Hz body tick. (`GAME_TICK.md` §Tick order.)
- Coaches pulse at 15 Hz and publish squad mailboxes and edge logits at step 4, before
  the same tick's body evaluation. Graph traversal consumes edge logits only through
  graph-v0's defined edge semantics. (`GAME_TICK.md` §Control state.)
- Graph decisions are integer-only. No `f32` or `f64` value may feed one.
  (`GAME_TICK.md` determinism rule 1.)
- graph-v0 supplies an oracle intent per embodied body. Cue gates and the steering
  residual are the learned body output. The motor combines both. (`GAME_TICK.md`
  §Control state; `DESIGN.md` §Playbook and motor combination.)

The live schema a node already carries: `name`, ordered `edges` (1..=8 ports),
`squad_cycle` (values 0..=7), and `goalie`/`fielder` role intents of `position` and
`spin`. The live controller surface a node must line up with: `edge_logits[team][0..8]`,
`mailboxes[team][squad][lane]`, and `ActRequest::enabled_edges: u8`. The live perception
vocabulary a target must line up with: `Relation` of teammate, opponent, neutral, arena,
or goal; `Role` of fielder, goalie, or objective; and squad `0..=7`.

## Frame convention

Proposed: every operand and parameter in a play file is written in the team-zero
attacking frame and mirrored on `x` by the resolving team's direction, exactly as
`RoleIntent.position.x` is mirrored today. One authored play therefore reads the same
for both teams, and a threshold like "past the halfway line" needs one spelling.

## Triggers

Proposed vocabulary. A trigger is a predicate on latched tick input, world state, and
match metadata as they stand at step 2. It reads no perception and no controller
output, because neither exists yet at step 2. `CoachEdge` is the single exception and
is defined in [Coach edge semantics](#coach-edge-semantics).

| Trigger | Operand | True when |
|---|---|---|
| `Always` | — | unconditionally; required on a node's last port |
| `Elapsed(t)` | `u32` body ticks | body ticks since the cursor entered this node ≥ `t` |
| `BallPast(x)` | Q16.16 | game-ball `pos.x` in the attacking frame ≥ `x` |
| `BallBehind(x)` | Q16.16 | game-ball `pos.x` in the attacking frame ≤ `x` |
| `BallAloft(z)` | Q16.16 | game-ball `pos.z` ≥ `z` |
| `Possession(r)` | `Teammate`, `Opponent`, `Neutral` | the most recent game-ball touch event is within `POSSESSION_TICKS` and has relation `r` to the resolving team; `Neutral` also covers an empty window |
| `Lead(n)` | `i32` goals | own score − opposing score ≥ `n` |
| `CoachEdge` | — | the resolving team's edge logit for this port clears the node's `coach_gate` on the tick after a coach pulse |

`Possession` needs one new `prov` constant, `POSSESSION_TICKS`, in the `GAME_TICK.md`
constants table. Touch events already exist: substep stage 9 evaluates goals, game-ball
touches, and pick queries.

### Evaluation rule

Proposed. At step 2, before intent resolution, the resolving cursor scans ports `0..8`
in index order. Ports beyond the node's declared edge count evaluate false. The first
true port moves the cursor to its target node for this tick; if no port is true the
cursor holds. The loop count is a fixed eight regardless of edge count, satisfying
determinism rules 2 and 7, and at most one transition occurs per team per body tick.

First-true-wins in port order, not highest-priority-wins, is the proposal: port order is
already the authored order in the file and in an editor, and it needs no reduction on
the GPU. A node whose ports are all false is legal and static; the `Always` requirement
on the last port is what makes a node deliberately leaving.

The existing human overrides, `Match::select_play_node` and `Match::traverse_play`,
latch at the start of the tick and outrank trigger evaluation for that tick. The play
node stays human-authoritative; triggers are the authored automation beneath it.

## Targets

Proposed vocabulary. A target resolves at step 2 from world state and match metadata
alone.

| Target | Resolves to |
|---|---|
| `GameBall` | the objective body's current position |
| `OwnGoal` | the resolving team's goal-mouth center |
| `OpponentGoal` | the opposing goal-mouth center |
| `Squad(n)` | the centroid of the resolving team's bodies currently assigned squad `n`, as a widened component sum divided by the member count with `qdiv`; `Slot` when the squad is empty |
| `NearestOpponent` | the opposing player body at minimum squared distance, ties broken by lowest canonical body index |
| `Slot` | the role's node template position after the existing per-local spread |

`Squad(n)` deliberately reuses the squad numbering that `squad_cycle` and the coach
mailboxes already use, so one number names a mailbox, an assignment, a perception tag,
and a target.

## Ball verbs

Proposed vocabulary. A verb resolves a target into one `OracleIntent`. **A verb emits no
cue gate.** Jump, boost, and air cue remain the learned body output, and a verb that
gated them would collapse the oracle/residual split that `DESIGN.md` keeps.

| Verb | Aim point | Reading |
|---|---|---|
| `Hold` | the role's node slot with the existing per-local spread | the current behavior; the only verb that consumes the `goalie`/`fielder` templates |
| `Chase` | the target's position | go get it |
| `Strike` | one radius behind the target on the line from the opposing goal mouth through the target | hit it that way |
| `Clear` | one radius behind the target on the line from the own goal mouth through the target | hit it away from us |
| `Mark` | `MARK_GAP` from the target toward the own goal mouth | stand goal-side of it |
| `Screen` | the midpoint between the target and the own goal mouth | stand between |
| `Post` | the own goal-mouth plane, `y` tracking the target's `y` clamped to the mouth half-width | the goalie verb |

> [!REVIEW] The verbs as is LGTM but I think this probably needs to read as something closer to football terminology applied to soccer teams -- it's full contact and the ball's physicality against other balls is part of what the 100v100 is meant to exploit. Compared to Rocket League our field isn't nearly as sparse

`Mark` needs one new `prov` constant, `MARK_GAP`.

Proposed resolution: for a body with local ID `L` on the resolving team, the squad is
`squad_cycle[L % squad_cycle.len()]` as today; a fielder-role body takes `verbs[squad]`
and a goalie-role body takes `goalie_verb`. The verb table is exactly eight entries
indexed by squad, matching the mailbox and edge-logit array widths. A per-local table
was considered and rejected: it does not survive contact with a 100v100 roster, and the
squad indirection is the authoring unit the coaches already address.

Proposed spin: every verb except `Hold` emits a zero spin target, and `Hold` emits the
node template's spin. A geometry-derived spin target is not legible on screen, and local
spin is what the learned residual is for.

Proposed degenerate case: where an aim-point construction normalizes a zero-length
direction, `qnorm` returns zero per its normative total definition and the verb falls
back to the target's position. No epsilon and no precondition are introduced.

## Coach edge semantics

Coaches publish `edge_logits[team][0..8]` as the coach family's bipolar output lanes
`64..72`, one per port ordinal. Lanes at or beyond the node's edge count are ignored,
matching the `enabled_edges` mask the node already sends in `ActRequest`.

Proposed:

1. A `CoachEdge` port fires when the resolving team's logit for that port index exceeds
   the node's `coach_gate`, a Q16.16 field defaulting to zero.
2. A `CoachEdge` port is evaluated only on the body tick immediately after a coach
   pulse, i.e. where `tick mod 4 == 1`, and reads that pulse's logits. On every other
   tick it is false.
3. A `CoachEdge` port is one port among eight and is scanned in the same first-true-wins
   order as every other trigger. Coaches cannot bypass a node's authored triggers.
4. Where several ports clear their gate, port order decides.

Rule 2 is the load-bearing one, and it is the clause a reviewer should press hardest.
Tick-order step 4 makes a one-tick delay nonconforming for *mailboxes*, which bodies
consume at step 5 of the same tick. Edge logits are not bound by that clause: both
`GAME_TICK.md` §Control state and `DESIGN.md` §Populations and perception route them
through "graph-v0's defined coach edge semantics", and this section is that definition.
The asymmetry is structural, not a concession — coaches publish at step 4 and the graph
resolves at step 2, so step 2 can never see a same-tick logit at all. On a coach-due
tick the freshest logits available to step 2 are four ticks old. Rule 2 reads them on
the one tick where they are freshest instead, and spending each pulse once stops a
single pulse from driving four transitions.

Rejecting rule 2 means evaluating `CoachEdge` on every tick from logits up to four ticks
old, which is legible only if a coach is expected to hold a lane steady across its whole
interval.

Rule 4 rejects an argmax over enabled ports. Argmax needs a reduction, and it makes one
lane's meaning depend on the other seven, so a coach author cannot reason about a port
locally.

## Forks that need an explicit verdict

1. **Cursor ownership.** `Match` holds one `play_node` for the whole match, but edge
   logits are per team. Proposed: one graph, two cursors, one per team; a team's coach
   logits gate only that team's transitions; the human overrides drive the player's
   team. The alternative — one shared cursor fed by combined logits — makes a team's
   tactical transition depend on the opponent's coach, and is rejected here. This is a
   data-model change and needs a verdict before the core bite starts.
2. **New per-match graph state.** `Elapsed` needs the tick each cursor entered its
   current node, and `Possession` needs the tick and relation of the most recent
   game-ball touch. Neither exists today, and both are replay and checkpoint state under
   the tick order's latching rule, so both land in `CheckpointHeader` alongside the
   learning-schedule binding. Rejecting either trigger removes its word; accepting them
   sizes the header change.
3. **Schema version.** Proposed: `PLAYBOOK_ABI_VERSION` moves 1 → 2 and
   `assets/default-playbook.ron` is rewritten in the same commit. No migration reader,
   per settled constraint 5. Version-1 files stop compiling; that is the intended cost.
4. **New constants.** Acknowledgment adds `POSSESSION_TICKS` and `MARK_GAP` as `prov`
   rows in the `GAME_TICK.md` constants table, subject to M1 feel tuning.
5. **Vocabulary closure.** Eight triggers, seven verbs, six targets. A rejection that
   trims the list is cheaper now than an extension after the WGSL port; the GPU
   evaluator in M4 pays for each variant in dispatch width.

## Worked example A — triggers and verbs over the checked-in default

The current `assets/default-playbook.ron` press/recover pair, extended with the proposed
fields. Nothing else about the file changes.

```ron
(
  version: 2,
  nodes: [
    (
      name: "press",
      edges: [
        (to: 1, trigger: BallBehind(-8.0)),
        (to: 0, trigger: Always),
      ],
      squad_cycle: [0, 0, 1, 1, 2, 2, 3, 3],
      coach_gate: 0.0,
      goalie_verb: (verb: Post, target: GameBall),
      verbs: [
        (verb: Strike, target: GameBall),
        (verb: Chase, target: GameBall),
        (verb: Mark, target: NearestOpponent),
        (verb: Screen, target: GameBall),
        (verb: Hold, target: Slot),
        (verb: Hold, target: Slot),
        (verb: Hold, target: Slot),
        (verb: Hold, target: Slot),
      ],
      goalie: (position: [-14.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
      fielder: (position: [2.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
    ),
    (
      name: "recover",
      edges: [
        (to: 0, trigger: BallPast(0.0)),
        (to: 0, trigger: Elapsed(180)),
        (to: 1, trigger: Always),
      ],
      squad_cycle: [3, 3, 2, 2, 1, 1, 0, 0],
      coach_gate: 0.0,
      goalie_verb: (verb: Post, target: GameBall),
      verbs: [
        (verb: Clear, target: GameBall),
        (verb: Mark, target: NearestOpponent),
        (verb: Mark, target: NearestOpponent),
        (verb: Screen, target: GameBall),
        (verb: Hold, target: Slot),
        (verb: Hold, target: Slot),
        (verb: Hold, target: Slot),
        (verb: Hold, target: Slot),
      ],
      goalie: (position: [-14.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
      fielder: (position: [-2.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
    ),
  ],
)
```

Reading of `press` for a team-zero body with local ID `5`: `squad_cycle[5] = 2`, so the
verb is `Mark` on `NearestOpponent` and the aim point is `MARK_GAP` goal-side of the
closest opposing body. Local ID `0` is the goalie and takes `Post` on `GameBall`
regardless of its squad. Local ID `1` gets `squad_cycle[1] = 0` and drives `Strike`, so
exactly two bodies per team are on the ball at any time in this node.

Transition of `press`: port 0 fires the first body tick the game ball is at or behind
`x = −8` in that team's attacking frame; otherwise port 1 holds the cursor. `recover`
returns on the ball crossing halfway, or unconditionally after 180 body ticks — three
seconds — so a stalled ball cannot strand a team in its own half.

## Worked example B — a coach-gated edge

Edge blocks only; the remaining fields of each node match Example A's shape.

```ron
(
  name: "shape",
  edges: [
    (to: 1, trigger: CoachEdge),
    (to: 2, trigger: Possession(Opponent)),
    (to: 0, trigger: Always),
  ],
  coach_gate: 0.25,
  // ... squad_cycle, goalie_verb, verbs, goalie, fielder
)
(
  name: "commit",
  edges: [
    (to: 0, trigger: Elapsed(90)),
    (to: 1, trigger: Always),
  ],
  coach_gate: 0.0,
  // ...
)
(
  name: "collapse",
  edges: [
    (to: 0, trigger: Possession(Teammate)),
    (to: 2, trigger: Always),
  ],
  coach_gate: 0.0,
  // ...
)
```

Trace with the cursor on `shape`. The coach pulses at tick `T = 8` and publishes
`edge_logits[team][0] = 0.31`. At step 2 of tick `T = 9`, port 0 is scanned first, `0.31`
clears `coach_gate = 0.25`, and the cursor moves to `commit`; the opponent's cursor is
unaffected because it reads its own team's lanes. `commit` returns to `shape` after 90
body ticks whatever the coach wants, so a coach cannot hold a team committed
indefinitely. Had the logit been `0.11`, port 0 would have been false, port 1 would have
been scanned, and losing possession would have sent the cursor to `collapse` instead.

This is the whole coach-edge surface: eight signed lanes, a per-node threshold, one
tick of latency, and port order for ties. A coach that never learns a useful lane leaves
the graph exactly as authored.

## Evidence

Legibility scratch work for these tables lives in
[`../spikes/playbook/`](../spikes/playbook/README.md), which is non-normative, is not a
schema source, is not a prerequisite of the gate, and cannot pre-empt its verdict.
