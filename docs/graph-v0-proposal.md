# graph-v0 trigger, verb, target, form, and coach-edge vocabulary

Status: acknowledged, 2026-07-26. This document is the artifact named by the
*Acknowledge graph triggers and verb/target shapes* gate in [`../TODO.md`](../TODO.md).
Every table below carries a verdict. Acknowledgment moves these shapes into
[`../DESIGN.md`](../DESIGN.md) and [`../GAME_TICK.md`](../GAME_TICK.md) and unblocks the
*extend the single graph-v0 schema in place* bite in
[`../crates/zoomieball-core/TODO.md`](../crates/zoomieball-core/TODO.md).

This document exists so that bite is not its own prerequisite. It describes a schema
that does not exist yet, using only vocabulary the settled documents already establish.

## What is already settled

These constrain every table below and are not reopened here.

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

Every operand and parameter in a play file is written in the team-zero attacking frame and
resolved into the world frame by a **half turn about `+z`** for team one: `x` and `y` both
negate, `z` is untouched. One authored play therefore reads the same for both teams, and a
threshold like "past the halfway line" needs one spelling.

The half turn, rather than the `x`-only reflection `RoleIntent.position` used through M0,
is what makes an asymmetric play mean one thing. The arena is symmetric under a half turn —
goals at `±x`, the same shape either side — so "overload the left" resolves to each team's
own left. Under an `x`-only mirror it resolves to one absolute touchline, and the away team
runs the play backwards. That the four acknowledged forms are all bilaterally symmetric
about their forward axis makes the two conventions currently indistinguishable in shape;
they differ only in which ordinal stands where, and the first asymmetric form proposed
would make the difference visible. Settling it now costs one sign.

A rotation also spares this document the axial-vector question that `GAME_TICK.md`
§Conformance still lists as blocked. A reflection maps polar and axial vectors differently
and would owe a spin rule; a rotation maps them identically, so `spin` mirrors exactly as
`position` does and no handedness is at stake inside a play file. The blocked semantic
mirror mapping is a separate obligation and this convention does not discharge it.

## Register

A play node is a **call sheet**: a per-squad written assignment, resolved fresh every
body tick. Only one team sport writes assignments down as a matter of culture, so the
sheet's grammar is American football's even though the game is soccer. Three donors,
each supplying what it alone names well:

| Layer | Donor | Contributes |
|---|---|---|
| the sheet | American football | the concept of a written per-squad assignment; `Align`, `Block`, `Lead`, `Jam` |
| the sport | soccer | the objective, the goal mouth, the arena; `Clear`, `Guard` |
| the contact | rugby | the vocabulary of a permanently live loose ball in contested space; `Pod`, `Sweep` |

Most of the set is Anglophone team-sport common stock — `Cover` is universal, `Zone` is
basketball's, `Drive` and `Pursue` belong to everyone. Only `Align`, `Block`, `Lead`, and
`Jam` are American-specific.

Australian rules' `shepherd` — legal body-blocking for a teammate, in a continuous sport,
on the largest field and roster in team sport — was considered for the blocking verb and
set aside. It is the single word that lexicon contributes; the rest of it is disposal
vocabulary (mark, handball, kick) and Zoomieball has no passing. The sheet already owns
the assignment word, so `Block` stays. This is recorded so it is not re-derived.

Rugby's `clear out` — hit the body over the ball and drive it off — is the purest
existing expression of the contact thesis below, but `Clear` is spent on the ball verb.
It survives as `Jam`'s reading rather than as a name.

## Triggers

A trigger is a predicate on latched tick input, world state, and match metadata as they
stand at step 2. It reads no perception and no controller output, because neither exists
yet at step 2. `CoachEdge` is the single exception and is defined in
[Coach edge semantics](#coach-edge-semantics).

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
constants table, and one new game-ball touch record from substep stage 9.

A step in which bodies from **both** teams touch the game ball records a neutral touch.
`Possession` then reads `Neutral`, which is already the relation it gives an empty window.
The alternative — a canonical-index tie-break — is deterministic but structurally biased,
because team zero owns the low indices and would win every contested touch in the match.
A contested touch is honestly nobody's possession, so the vocabulary already had the word.

### Evaluation rule

At step 2, before intent resolution, the resolving cursor scans ports `0..8` in index
order. Ports beyond the node's declared edge count evaluate false. The first true port
moves the cursor to its target node for this tick; if no port is true the cursor holds.
The loop count is a fixed eight regardless of edge count, satisfying determinism rules 2
and 7, and at most one transition occurs per team per body tick.

First-true-wins in port order, not highest-priority-wins: port order is already the
authored order in the file and in an editor, and it needs no reduction on the GPU. A node
whose ports are all false is legal and static; the `Always` requirement on the last port
is what makes a node deliberately leaving.

The existing human overrides, `Match::select_play_node` and `Match::traverse_play`, latch
at the start of the tick and outrank trigger evaluation for that tick. They drive the
player's team cursor. The play node stays human-authoritative; triggers are the authored
automation beneath it.

## Targets

A target resolves at step 2 from world state and match metadata alone.

| Target | Resolves to |
|---|---|
| `GameBall` | the objective body's current position |
| `OwnGoal` | the resolving team's goal-mouth center |
| `OpponentGoal` | the opposing goal-mouth center |
| `Squad(n)` | the centroid of the resolving team's bodies currently assigned squad `n`, as a widened component sum divided by the member count with `qdiv`; `Slot` when the squad is empty |
| `NearestOpponent` | the opposing player body at minimum squared distance **to the game ball**, ties broken by lowest canonical body index |
| `NearestToMe` | the opposing player body at minimum squared distance **to the resolving body**, ties broken by lowest canonical body index |
| `Slot` | the role's node template position |

`Squad(n)` deliberately reuses the squad numbering that `squad_cycle` and the coach
mailboxes already use, so one number names a mailbox, an assignment, a perception tag,
and a target.

`NearestOpponent` and `NearestToMe` are the same construction against two references, and
the pair exists because the verbs want different ones. `Block` and `Lead` are squad
assignments against a single threat: twelve bodies converging on the body nearest the ball,
spread by their form, is the shape those verbs are for. `Cover` is man coverage, and man
coverage in which twelve defenders take the same man is not coverage. One target cannot be
both.

`NearestToMe` is the expensive one and the only word in this vocabulary that is. Its
minimum is per body rather than per team, so a squad on it costs a scan per member —
roughly 10⁴ squared distances per team per body tick at 100 per side, against a measured
1.89x native realtime margin. The spatial grid cannot amortize it: `SpatialIndex` is
rebuilt at tick-order step 3 and the graph resolves at step 2, so a grid-assisted query
would read the previous tick's index or force a tick-order change, and neither is worth
buying here. A sheet that never authors `NearestToMe` never pays for it.

## Contact

Every verb below is a contact verb, and the target vocabulary is polymorphic: a target is
a body as readily as it is the ball. `Clear(NearestOpponent)` is a shove of that body away
from our end. `Drive(NearestOpponent)` is the same shove toward theirs. `Jam` exists only
to arrive with speed. The field is not sparse — at 100 per side the arena is crowded by
construction, and ball-on-ball physicality is the thing the roster size exists to exploit.
The verb table is written so that reading is available to a play author without a
comment.

**A verb emits no cue gate.** Jump, boost, and air cue remain the learned body output, and
a verb that gated them would collapse the oracle/residual split that `DESIGN.md` keeps.

## Verbs

A verb resolves a target into one aim point, plus the construction axis that orients its
squad's formation. Eleven verbs.

| Verb | Aim point | Construction axis | Reading |
|---|---|---|---|
| `Align` | the role's node template position | none | hold your alignment |
| `Pursue` | the target's position | none | run it down |
| `Drive` | one radius behind the target, along the axis | target → opposing goal | drive it — or drive *them* — downfield |
| `Clear` | one radius behind the target, along the axis | own goal → target | get it out of our end |
| `Cover` | `COVER_GAP` from the target toward the own goal | target → own goal | man coverage, goal-side of it |
| `Zone` | the midpoint between the target and the own goal | target → own goal | hold the lane |
| `Sweep` | the `Zone` depth in `x`, the target's `y` and `z` | target → own goal | last one back, reading the whole field |
| `Block` | `COVER_GAP` from the target toward the game ball | target → game ball | get between them and the ball |
| `Lead` | `COVER_GAP` from the target toward the opposing goal | target → opposing goal | lead blocker, out in front of it |
| `Jam` | one radius past the target, along the axis | resolving body → target | go hit it; do not stand near it |
| `Guard` | the own goal-mouth plane, `y` tracking the target's `y` clamped to the mouth half-width | none | the goalie verb |

`Cover`, `Block`, and `Lead` are one construction against three references — the own
goal, the game ball, and the opposing goal — and share one `prov` constant, `COVER_GAP`.
`Drive` and `Clear` are likewise one construction: the aim point is one radius back along
the direction of the intended hit. No verb introduces a constant beyond `COVER_GAP`; a
radius is the length unit.

`Sweep` is the only verb that mixes references by component. It holds the `Zone` depth so
it cannot be pulled forward by one opponent, and takes the target's lateral position so it
shades with the play — which is what distinguishes it from `Zone`, whose `y` is halved
along with its `x`, and from `Guard`, which is pinned to the mouth.

`Jam` is the only *verb* whose aim point depends on the resolving body, and `NearestToMe`
the only *target*. Either one costs a squad its shared anchor: each member converges from
wherever it already is, and its formation is built about its own aim point. That asymmetry
is the point, not a wart — it is how a squad arrives from several angles at once, and it is
why the two are separable. `Jam` on a shared target is a gang tackle; any verb on
`NearestToMe` is an assignment sheet.

`Guard`'s aim point takes its `x` from the goal-mouth plane unconditionally. A target
behind the goal line still projects, `y`-clamped; there is no special case and no
precondition.

**Verb assignment.** For a body with local ID `L` on the resolving team, the squad is
`squad_cycle[L % squad_cycle.len()]` as today; a fielder-role body takes `verbs[squad]`
and a goalie-role body takes `goalie_verb`. The verb table is exactly eight entries
indexed by squad, matching the mailbox and edge-logit array widths. A per-local table was
considered and rejected: it does not survive contact with a 100v100 roster, and the squad
indirection is the authoring unit the coaches already address.

**Spin.** Every verb except `Align` emits a zero spin target; `Align` emits the node
template's spin. A geometry-derived spin target is not legible on screen, and local spin
is what the learned residual is for.

**Degenerate case.** Where an aim-point or axis construction normalizes a zero-length
direction, `qnorm` returns zero per its normative total definition and the verb falls back
to the target's position. No epsilon and no precondition is introduced.

## Formations

A verb sends its whole squad to one aim point. At 100 per side with eight squads that is
roughly twelve bodies converging on one Q16.16 coordinate, and the play reads as a scrum
whatever it was authored to mean. Formation is therefore not decoration on top of the verb
table; it is what makes any verb other than `Align` survive a real roster.

**A verb entry is `(verb, target, form)`.** The verb and target give the squad's aim
point; the form and the body's ordinal within its squad give that body's offset from it.
The hard-coded `ordinal % 11 - 5` spread in today's `Playbook::resolve` is deleted:
`Align` with `Pod(1, 11, 0.5)` reproduces it, and every other verb gains the same
capability.

### Squad ordinal

A body's ordinal `k` is its position among the resolving team's **fielder-role** bodies
assigned the same squad, in local-ID order. The goalie is excluded: it is on `goalie_verb`
and its slot would otherwise be a hole in a fielder formation.

With cycle length `C` and squad `s = squad_cycle[L % C]`:

```text
k(L) = (L / C)·count[s] + prefix[L % C] − goalie_correction[s]
```

`count[s]` is the number of cycle positions holding squad `s`; `prefix[i]` is the number
of positions before `i` holding the same squad as position `i`; `goalie_correction[s]` is
one when `squad_cycle[0] == s` and zero otherwise. All three are compiled once per node at
RON compile time and are exactly the flat execution tables settled constraint 10 asks for.
No squad-size query is needed anywhere below.

### The alternating step

Every form places its members with one primitive, so the three shapes stay predictable
against one another:

```text
step(k) = ⌊(k+1)/2⌋ · (k odd ? +1 : −1)      →   0, +1, −1, +2, −2, …
```

Slot 0 is always the anchor and the shape grows symmetrically outward. Centering never
needs the member count, which is what keeps the whole mechanism a closed-form integer
function of one local ID.

### The form frame

Formations are a floor-plane concept.

- **forward** is the verb's construction axis, projected to `xy` and normalized. A verb
  with no construction axis — `Align`, `Pursue`, `Guard` — uses the attacking-frame `+x`.
- **lateral** is forward rotated −90° about `+z`: `(forward.y, −forward.x, 0)`.
- **z** is the aim point's `z`; no form displaces vertically.

When forward's `xy` components are both zero, `qnorm` returns zero per its total
definition, lateral is zero with it, and the form collapses to `Point`. That is the same
no-epsilon stance the verbs take.

### Shapes

| Form | Slot for ordinal `k`, relative to the aim point |
|---|---|
| `Point` | zero; the whole squad converges. The default. |
| `Pod(rank, file, gap)` | pod `p = k / (rank·file)`, within-pod `q = k mod (rank·file)`, file `f = q mod file`, rank `r = q / file`. lateral `= step(p)·(file+1)·gap + (2f − (file−1))·gap/2`; depth `= −r·gap`. |
| `Wedge(gap)` | lateral `= step(k)·gap`; depth `= −|step(k)|·gap`. |
| `Arc(gap)` | `target + qnorm((aim − target) + lateral·step(k)·gap) · qlen(aim − target)`. |

`Pod` is rugby's three-body unit rather than a lattice: a squad tiles into consecutive
pods of `file` bodies across and `rank` deep, and successive pods lay out laterally by the
same alternating step at a stride of `(file+1)·gap` — one empty gap between adjacent pods.
A 1-3-3-1 or 2-4-2 shape is `Pod` entries on different squads, not a form parameter.
`Pod(2, 3, 1.5)` is a three-across, two-deep blocking cluster; the same entry at 100v100
grows to a second and third pod beside it rather than silently dropping bodies.

`Arc` avoids trigonometry deliberately. There is no `sin`/`cos` helper in
`GAME_TICK.md` §Required arithmetic helpers and this document does not add one: members
step along the chord and are renormalized back onto the circle of radius
`qlen(aim − target)`. Spacing therefore compresses at wide angles, which is acceptable for
a cordon and is cheaper than a baked trigonometric table on both tiers. Where
`aim == target` the renormalization is zero-in-zero-out and `Arc` collapses to `Point`.

`Line` and `Column` were considered and cut: `Pod(rank, 1, gap)` is a column and
`Pod(1, file, gap)` is a line, so both were pure dispatch width.

### Authored bounds

A form's `gap` and a pod's extents are bounded at compile time: `gap` to `100` and `rank`
and `file` to `1..=100` each. These are not feel limits — they are what keeps the slot
arithmetic inside `i32`. A slot offset is the product of an extent, an ordinal, and a gap;
unbounded, an authored `Pod(1, 1, 5000.0)` drives `qmul` out of range at a ten-body roster.
The CPU tier panics there, as `GAME_TICK.md` says it may, but the GPU tier cannot trap and
produces an unspecified value instead, so the two tiers would disagree on a file that
compiled. Determinism rule 1 makes the bound part of the schema rather than a defensive
check, and the compiler rejects an out-of-range extent or gap with a typed error.

## Coach edge semantics

Coaches publish `edge_logits[team][0..8]` as the coach family's bipolar output lanes
`64..72`, one per port ordinal. Lanes at or beyond the node's edge count are ignored,
matching the `enabled_edges` mask the node already sends in `ActRequest`.

1. A `CoachEdge` port fires when the resolving team's logit for that port index exceeds
   the node's `coach_gate`, a Q16.16 field defaulting to zero.
2. A `CoachEdge` port is evaluated only on the body tick immediately after a coach pulse,
   i.e. where `tick mod 4 == 1`, and reads that pulse's logits. On every other tick it is
   false.
3. A `CoachEdge` port is one port among eight and is scanned in the same first-true-wins
   order as every other trigger. Coaches cannot bypass a node's authored triggers.
4. Where several ports clear their gate, port order decides.

Rule 2 is the load-bearing one. Tick-order step 4 makes a one-tick delay nonconforming for
*mailboxes*, which bodies consume at step 5 of the same tick. Edge logits are not bound by
that clause: both `GAME_TICK.md` §Control state and `DESIGN.md` §Populations and perception
route them through "graph-v0's defined coach edge semantics", and this section is that
definition. The asymmetry is structural, not a concession — coaches publish at step 4 and
the graph resolves at step 2, so step 2 can never see a same-tick logit at all. On a
coach-due tick the freshest logits available to step 2 are four ticks old. Rule 2 reads
them on the one tick where they are freshest instead, and spending each pulse once stops a
single pulse from driving four transitions. The rejected alternative — evaluating
`CoachEdge` every tick from logits up to four ticks old — is legible only if a coach is
expected to hold a lane steady across its whole interval.

Rule 4 rejects an argmax over enabled ports. Argmax needs a reduction, and it makes one
lane's meaning depend on the other seven, so a coach author cannot reason about a port
locally.

## Acknowledged verdicts

1. **Cursor ownership.** One graph, two cursors, one per team. A team's coach logits gate
   only that team's transitions; the human overrides drive the player's team. The
   alternative — one shared cursor fed by combined logits — makes a team's tactical
   transition depend on the opponent's coach, and is rejected. `Match::play_node` becomes
   per-team, and `ActRequest` carries a cursor and an enabled-edge mask per team.
2. **New per-match graph state.** `Elapsed` needs the tick each cursor entered its current
   node; `Possession` needs the tick and relation of the most recent game-ball touch, which
   substep stage 9 must begin recording. Both are replay and checkpoint state under the
   tick order's latching rule, so both land in `CheckpointHeader` alongside the
   learning-schedule binding — that wiring belongs to the *bind the learning schedule* bite,
   not to the schema bite.
3. **Schema version.** `PLAYBOOK_ABI_VERSION` moves 1 → 2 and `assets/default-playbook.ron`
   is rewritten in the same commit. No migration reader, per settled constraint 5.
   Version-1 files stop compiling; that is the intended cost.
4. **New constants.** `POSSESSION_TICKS` and `COVER_GAP` join the `GAME_TICK.md` constants
   table as `prov` rows, subject to M1 feel tuning. `COVER_GAP` is the acknowledged
   `MARK_GAP` under its non-dangling name: `Mark` became `Cover`, and the constant now
   serves `Cover`, `Block`, and `Lead`.
5. **Vocabulary closure.** Eight triggers, eleven verbs, seven targets, four forms. The
   verb count moved from the seven originally proposed: the register recast renamed seven,
   `Block`, `Lead`, `Jam`, and `Sweep` were added, and `Line` and `Column` were cut from the
   form list as expressible in `Pod`. `NearestToMe` is the seventh target. The M4 GPU
   evaluator pays for each variant in dispatch width, and this is the closed set it will be
   built against.
6. **Play frame.** A team-one resolution is a half turn about `+z`, not an `x`-only
   reflection. This moves `RoleIntent.position`, whose `y` was previously unmirrored, and
   re-baselines every golden that reads a team-one intent.

## Worked example A — a call sheet over the checked-in default

The current `assets/default-playbook.ron` press/recover pair at version 2.

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
    ),
    (
      name: "recover",
      edges: [
        (to: 0, trigger: BallPast(0.0)),
        (to: 0, trigger: Elapsed(180)),
        (to: 1, trigger: Always),
      ],
      squad_cycle: [7, 6, 5, 4, 3, 2, 1, 0],
      coach_gate: 0.0,
      goalie_verb: (verb: Guard, target: GameBall, form: Point),
      verbs: [
        (verb: Clear,  target: GameBall,        form: Point),
        (verb: Cover,  target: NearestOpponent, form: Point),
        (verb: Cover,  target: NearestOpponent, form: Point),
        (verb: Zone,   target: GameBall,        form: Pod(1, 3, 2.5)),
        (verb: Sweep,  target: GameBall,        form: Pod(1, 3, 3.0)),
        (verb: Jam,    target: NearestOpponent, form: Point),
        (verb: Align,  target: Slot,            form: Pod(1, 11, 0.5)),
        (verb: Align,  target: Slot,            form: Pod(1, 11, 0.5)),
      ],
      goalie:  (position: [-14.0, 0.0, 1.0], spin: [0.0, 0.0, 0.0]),
      fielder: (position: [-2.0, 0.0, 1.0],  spin: [0.0, 0.0, 0.0]),
    ),
  ],
)
```

Reading of `press` for a team-zero body with local ID `2`: `squad_cycle[2] = 2`, so the
entry is `Block` on `NearestOpponent` in a `Pod(2, 3, 1.5)`. The squad's aim point is
`COVER_GAP` from that opponent toward the game ball; forward is the opponent-to-ball axis
flattened to the floor, so the pod's three files spread across the opponent's route to the
ball and its second rank sits `1.5` behind. Local ID `0` is the goalie and takes `Guard` on
`GameBall` regardless of its squad. Local ID `1` gets `squad_cycle[1] = 1` and pursues in a
wedge behind whoever is on `Drive`.

The cycle names all eight squads, so every entry in the table runs. A cycle that names
fewer — `[0, 0, 1, 1, 2, 2, 3, 3]` is the tempting one, since it doubles up each squad at a
10-body roster — silently strands the entries it never indexes: the table is ABI-width
eight because the mailboxes are, and `squad_cycle` alone decides which of its rows a roster
can reach. An authored sheet whose last four rows never execute reads exactly like one
whose last four rows do.

At 10 per side each squad holds one or two fielders, so most forms resolve to their anchor
slot and the sheet reads as a formation of singletons. At 100 per side the same file grows
squad 2 to a second and third blocking pod and squad 3's arc into a real cordon, without
one edit — which is the property the form field exists to buy.

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
body ticks whatever the coach wants, so a coach cannot hold a team committed indefinitely.
Had the logit been `0.11`, port 0 would have been false, port 1 would have been scanned,
and losing possession would have sent the cursor to `collapse` instead.

This is the whole coach-edge surface: eight signed lanes, a per-node threshold, one tick of
latency, and port order for ties. A coach that never learns a useful lane leaves the graph
exactly as authored.

## Evidence

Legibility scratch work for these tables lives in
[`../spikes/playbook/`](../spikes/playbook/README.md), which is non-normative, is not a
schema source, is not a prerequisite of the gate, and cannot pre-empt its verdict.
