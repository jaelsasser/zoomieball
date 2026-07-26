//! Graph-v0 call sheets: the closed vocabulary, compiled node tables, and per-team state.
//!
//! One sheet, three jobs, one file each: [`ron`] compiles the at-rest RON form into the flat
//! tables below, [`scan`] runs the tick-order step-2 port scan, and [`solve`] resolves each
//! node into per-body oracle intent. Everything schema-shaped — the vocabulary enums, the
//! compiled [`PlayNode`], and the replay-state [`GraphState`] — lives here.

mod ron;
mod scan;
mod solve;

pub use ron::PlaybookError;
pub use scan::next_cursor;

use crate::fixed::{Fx, Vec3Fx};
use crate::perception::Relation;
use crate::world::Team;

/// Version of the accepted playbook schema.
pub const PLAYBOOK_ABI_VERSION: u32 = 2;

/// Outgoing ports every node is scanned for, whatever its declared edge count.
pub const PORT_COUNT: usize = 8;

/// Squads a node assigns, matching the mailbox and edge-logit array widths.
pub const SQUAD_COUNT: usize = 8;

/// `Possession` window in body ticks since the most recent game-ball touch.
pub const POSSESSION_TICKS: u64 = 30;

/// `Cover`/`Block`/`Lead` standoff from the target: `GAME_TICK.md`'s `COVER_GAP`, 3 r.
///
/// A radius is the length unit (`GAME_TICK.md` r = 1), so this and the verbs' one-radius offsets
/// are absolute Q16.16 lengths. The live world's spheres are 0.35 wide — an M0 divergence from
/// the r = 1 premise — and these offsets deliberately do not scale by that: scaling by the
/// resolving body's radius would make every standoff verb's aim depend on the resolving body,
/// which the proposal reserves for `Jam` alone.
pub const COVER_GAP: Fx = Fx::from_i32(3);

/// Desired position and spin emitted by the naive play solver.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OracleIntent {
    /// Desired world-space position.
    pub position: Vec3Fx,
    /// Desired world-space angular velocity.
    pub spin: Vec3Fx,
}

/// Reusable oracle-intent buffer in canonical body order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OracleIntentBatch {
    /// One intent per physical sphere.
    pub intents: Vec<OracleIntent>,
}

impl OracleIntentBatch {
    /// Allocate one initialized intent per body.
    #[must_use]
    pub fn with_len(body_count: usize) -> Self {
        Self {
            intents: vec![OracleIntent::default(); body_count],
        }
    }
}

/// Role-wide formation template in one play node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleIntent {
    /// Center position, before the verb and form displace it.
    pub position: Vec3Fx,
    /// Desired angular velocity, emitted only by `Align`.
    pub spin: Vec3Fx,
}

/// Predicate on latched tick input, world state, and match metadata at tick-order step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Unconditionally true; required on a node's last port.
    Always,
    /// Body ticks since the cursor entered this node reached the operand.
    Elapsed(u32),
    /// Game-ball `pos.x` in the attacking frame is at or beyond the operand.
    BallPast(Fx),
    /// Game-ball `pos.x` in the attacking frame is at or behind the operand.
    BallBehind(Fx),
    /// Game-ball `pos.z` is at or above the operand.
    BallAloft(Fx),
    /// The most recent game-ball touch has this relation to the resolving team.
    Possession(Relation),
    /// Own score less opposing score reached the operand.
    Lead(i32),
    /// The resolving team's logit for this port cleared the node's gate after a coach pulse.
    CoachEdge,
}

/// World reference a verb builds its aim point against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The objective body's current position.
    GameBall,
    /// The resolving team's goal-mouth center.
    OwnGoal,
    /// The opposing goal-mouth center.
    OpponentGoal,
    /// Centroid of the resolving team's bodies in that squad; the slot when it is empty.
    Squad(u8),
    /// Opposing player body nearest the game ball, ties broken by lowest canonical index.
    NearestOpponent,
    /// Opposing player body nearest the resolving body, ties broken by lowest canonical index.
    NearestToMe,
    /// The role's node template position.
    Slot,
}

/// Contact assignment resolving one target into an aim point and a construction axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Hold your alignment.
    Align,
    /// Run it down.
    Pursue,
    /// Drive it — or drive them — downfield.
    Drive,
    /// Get it out of our end.
    Clear,
    /// Man coverage, goal-side of it.
    Cover,
    /// Hold the lane.
    Zone,
    /// Last one back, reading the whole field.
    Sweep,
    /// Get between them and the ball.
    Block,
    /// Lead blocker, out in front of it.
    Lead,
    /// Go hit it; do not stand near it.
    Jam,
    /// The goalie verb.
    Guard,
}

/// Offset a squad member takes from its squad's aim point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Zero; the whole squad converges.
    Point,
    /// Consecutive pods `file` across and `rank` deep, laid out laterally at one pod's gap.
    Pod {
        /// Bodies deep in one pod.
        rank: u32,
        /// Bodies across in one pod.
        file: u32,
        /// Spacing between adjacent bodies.
        gap: Fx,
    },
    /// Alternating lateral steps, each set back by its own magnitude.
    Wedge(Fx),
    /// Alternating chord steps renormalized onto the circle through the aim point.
    Arc(Fx),
}

/// One squad's written assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbEntry {
    /// Contact assignment.
    pub verb: Verb,
    /// Reference the verb resolves against.
    pub target: Target,
    /// Shape the squad takes about the aim point.
    pub form: Form,
}

/// One ordered outgoing port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayEdge {
    /// Destination node index.
    pub to: usize,
    /// Predicate that moves the cursor along this port.
    pub trigger: Trigger,
}

/// Per-match graph traversal state, which is replay and checkpoint state under the latching rule.
///
/// A later phase stores this on `Match` and records the touch from substep stage 9.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphState {
    /// Body tick each team's cursor entered its current node.
    pub entered: [u64; 2],
    /// Body tick of the most recent game-ball touch.
    pub touched: u64,
    /// Team of the most recent game-ball touch, absent before the first one.
    pub toucher: Option<Team>,
}

impl GraphState {
    /// Relation of the game ball's most recent toucher to `team`, `Neutral` outside the window.
    #[must_use]
    pub fn possession(&self, team: Team, tick: u64) -> Relation {
        let Some(toucher) = self.toucher else {
            return Relation::Neutral;
        };
        if tick.saturating_sub(self.touched) >= POSSESSION_TICKS {
            return Relation::Neutral;
        }
        if toucher == team {
            Relation::Teammate
        } else {
            Relation::Opponent
        }
    }
}

/// Validated play graph node, carrying the flat tables its RON source compiled to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayNode {
    name: String,
    edges: Vec<PlayEdge>,
    squad_cycle: Vec<u8>,
    coach_gate: Fx,
    goalie_verb: VerbEntry,
    verbs: [VerbEntry; SQUAD_COUNT],
    goalie: RoleIntent,
    fielder: RoleIntent,
    /// Cycle positions holding each squad.
    squad_count: [u32; SQUAD_COUNT],
    /// Positions before each cycle position holding that position's squad.
    cycle_prefix: Vec<u32>,
    /// One where the goalie's local ID takes that squad, zero otherwise.
    goalie_correction: [u32; SQUAD_COUNT],
}

impl PlayNode {
    /// Stable node name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ordered outgoing ports.
    #[must_use]
    pub fn edges(&self) -> &[PlayEdge] {
        &self.edges
    }

    /// Threshold a coach edge logit must exceed to fire.
    #[must_use]
    pub const fn coach_gate(&self) -> Fx {
        self.coach_gate
    }

    /// The eight fielder assignments, indexed by squad.
    #[must_use]
    pub const fn verbs(&self) -> &[VerbEntry; SQUAD_COUNT] {
        &self.verbs
    }

    /// The assignment every goalie-role body takes, whatever its squad.
    #[must_use]
    pub const fn goalie_verb(&self) -> VerbEntry {
        self.goalie_verb
    }

    /// Mailbox assigned to one local ID.
    #[must_use]
    pub fn squad_for(&self, local: u8) -> u8 {
        self.squad_cycle[usize::from(local) % self.squad_cycle.len()]
    }

    /// Ordinal of one fielder among its squad's fielders, in local-ID order.
    ///
    /// This is the spec's closed form over the compiled tables, so it needs no squad-size query.
    /// The goalie is excluded from fielder formations and has no ordinal.
    ///
    /// # Panics
    ///
    /// Panics when `local` names the goalie.
    #[must_use]
    pub fn squad_ordinal(&self, local: u8) -> u32 {
        let position = usize::from(local) % self.squad_cycle.len();
        let squad = usize::from(self.squad_cycle[position]);
        let cycles = u32::try_from(usize::from(local) / self.squad_cycle.len())
            .expect("a roster index fits u32");
        (cycles * self.squad_count[squad] + self.cycle_prefix[position])
            .checked_sub(self.goalie_correction[squad])
            .expect("the goalie is the only body excluded from a fielder formation")
    }
}

/// Validated cyclic play graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playbook {
    nodes: Vec<PlayNode>,
}

impl Playbook {
    /// Compile the fixed-schema RON subset without floating-point conversion.
    pub fn compile_ron(source: &str) -> Result<Self, PlaybookError> {
        ron::compile(source)
    }

    /// Validated graph nodes.
    #[must_use]
    pub fn nodes(&self) -> &[PlayNode] {
        &self.nodes
    }

    /// Follow one ordered outgoing port from a node.
    #[must_use]
    pub fn traverse(&self, node: usize, port: usize) -> Option<usize> {
        Some(self.nodes.get(node)?.edges.get(port)?.to)
    }
}

/// Sign carrying one team-zero attacking-frame axis into world coordinates.
///
/// The play frame is a half turn about `+z`, so this one sign serves `x` and `y` alike and `z`
/// never takes it. [`half_turn`] is the whole-operand form.
fn mirror(team: Team) -> Fx {
    team.attack_axis().x
}

/// Carry one authored play-file operand into the world frame.
///
/// A team-one resolution is a half turn about `+z`: `x` and `y` both negate, `z` is untouched. One
/// authored play therefore means one thing for either team, and an asymmetric one resolves to each
/// team's own left rather than to one absolute touchline. A rotation maps polar and axial vectors
/// identically, which is why `spin` takes this transform unchanged from `position` and no
/// handedness rule is owed here.
///
/// Only authored operands pass through. Every aim point built from world geometry — the game ball,
/// a goal mouth, an opponent body, a squad centroid — is already in the world frame, and turning it
/// a second time would run the play backwards.
fn half_turn(operand: Vec3Fx, team: Team) -> Vec3Fx {
    let direction = mirror(team);
    Vec3Fx::new(operand.x * direction, operand.y * direction, operand.z)
}

/// Shared sheet fixtures for this module tree's unit tests.
#[cfg(test)]
mod fixtures {
    use super::Playbook;

    pub(super) const SHIPPED: &str = include_str!("../../../../assets/default-playbook.ron");

    pub(super) const HOLD: &str = "(verb: Align, target: Slot, form: Point),
         (verb: Align, target: Slot, form: Point),
         (verb: Align, target: Slot, form: Point),
         (verb: Align, target: Slot, form: Point),
         (verb: Align, target: Slot, form: Point),
         (verb: Align, target: Slot, form: Point),
         (verb: Align, target: Slot, form: Point),
         (verb: Align, target: Slot, form: Point),";

    /// A two-node cyclic sheet whose fielders all hold their template slot, so a test can vary
    /// one field without restating the schema. Node `a`'s fielder template is off the centre line
    /// in `y` as well as `x`, so the play frame's half turn shows in both components.
    pub(super) fn sheet(first_cycle: &str) -> String {
        format!(
            r#"(
              version: 2,
              nodes: [
                (name: "a", edges: [(to: 1, trigger: Always)],
                 squad_cycle: {first_cycle}, coach_gate: 0.0,
                 goalie_verb: (verb: Guard, target: GameBall, form: Point),
                 verbs: [{HOLD}],
                 goalie: (position: [-14.0, 0, 1], spin: [0, 0, 0]),
                 fielder: (position: [1.25, 2.5, 1], spin: [0, 0, 0])),
                (name: "b", edges: [(to: 0, trigger: Always)],
                 squad_cycle: [7], coach_gate: 0.0,
                 goalie_verb: (verb: Guard, target: GameBall, form: Point),
                 verbs: [{HOLD}],
                 goalie: (position: [-14, 0, 1], spin: [0, 0, 0]),
                 fielder: (position: [-1.25, 0, 1], spin: [0, 0, 0])),
              ],
            )"#
        )
    }

    pub(super) fn cyclic() -> String {
        sheet("[0, 1]")
    }

    pub(super) fn shipped() -> Playbook {
        Playbook::compile_ron(SHIPPED).expect("the shipped playbook compiles")
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{cyclic, sheet, shipped};
    use super::*;
    use crate::physics::Arena;
    use crate::world::World;

    /// Closed form versus brute force for every fielder local the roster can hold, across every
    /// cycle of length 1..=3 exhaustively and a seeded spread of longer cycles through 16 —
    /// repeated squads, unnamed squads, and cycles both shorter and longer than the roster.
    #[test]
    fn the_compiled_squad_ordinal_matches_a_brute_force_count() {
        let check = |cycle: &[u8]| {
            let written = format!(
                "[{}]",
                cycle
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let playbook = Playbook::compile_ron(&sheet(&written)).unwrap();
            let node = &playbook.nodes()[0];
            for local in 1u8..100 {
                let squad = node.squad_for(local);
                let brute = (1..local)
                    .filter(|&other| node.squad_for(other) == squad)
                    .count();
                assert_eq!(
                    node.squad_ordinal(local),
                    u32::try_from(brute).unwrap(),
                    "cycle {written} local {local}"
                );
            }
        };

        for length in 1..=3usize {
            let combinations = 8u32.pow(u32::try_from(length).unwrap());
            for combination in 0..combinations {
                let cycle: Vec<u8> = (0..length)
                    .map(|position| {
                        u8::try_from((combination >> (3 * position)) & 7)
                            .expect("a three-bit squad fits u8")
                    })
                    .collect();
                check(&cycle);
            }
        }

        // Longer cycles, seeded rather than exhaustive, plus the degenerate all-one-squad shape.
        let mut state = 0x5EED_u64;
        for length in 4..=16usize {
            check(&vec![3u8; length]);
            for _ in 0..24 {
                let cycle: Vec<u8> = (0..length)
                    .map(|_| {
                        state = state
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1);
                        u8::try_from((state >> 33) & 7).expect("a three-bit squad fits u8")
                    })
                    .collect();
                check(&cycle);
            }
        }
    }

    #[test]
    #[should_panic(expected = "the goalie is the only body excluded")]
    fn the_goalie_has_no_squad_ordinal() {
        let playbook = Playbook::compile_ron(&cyclic()).unwrap();
        let _ = playbook.nodes()[0].squad_ordinal(0);
    }

    #[test]
    fn the_shipped_call_sheet_round_trips_and_reads_as_the_worked_example() {
        let playbook = shipped();
        assert_eq!(playbook.nodes().len(), 2);
        let press = &playbook.nodes()[0];
        assert_eq!(press.name(), "press");
        assert_eq!(press.edges().len(), 2);
        assert_eq!(press.edges()[1].trigger, Trigger::Always);
        assert_eq!(press.coach_gate(), Fx::ZERO);
        assert_eq!(press.goalie_verb().verb, Verb::Guard);

        // Local ID 2 blocks its nearest opponent from a three-across, two-deep pod.
        assert_eq!(press.squad_for(2), 2);
        assert_eq!(
            press.verbs()[2],
            VerbEntry {
                verb: Verb::Block,
                target: Target::NearestOpponent,
                form: Form::Pod {
                    rank: 2,
                    file: 3,
                    gap: Fx::from_raw(98_304),
                },
            }
        );

        // The whole sheet resolves for both rosters without leaving the arithmetic domain.
        for roster in [10, 100] {
            let mut world = World::new(roster);
            let mut intents = OracleIntentBatch::with_len(world.view().len());
            for cursors in [[0, 0], [0, 1], [1, 0], [1, 1]] {
                playbook.resolve(cursors, &Arena::default(), &mut world, &mut intents);
            }
        }
    }
}
