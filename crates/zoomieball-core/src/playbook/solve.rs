//! Oracle-intent resolution: each verb's aim point and construction axis, each form's member
//! slot, and the squad-shared anchors both are built from once per tick.

use crate::fixed::{Fx, Vec3Fx, mul64};
use crate::physics::Arena;
use crate::world::{BodyId, LocalId, Role, Team, World};

use super::{
    COVER_GAP, Form, OracleIntent, OracleIntentBatch, PlayNode, Playbook, SQUAD_COUNT, Target,
    Verb, half_turn, mirror,
};

impl Playbook {
    /// Resolve both teams' cursors into caller-owned intent and squad buffers.
    ///
    /// Squad assignment is latched across the whole roster before any aim point is built, so a
    /// `Squad(n)` centroid reads this tick's assignments rather than a mixture of two ticks'.
    pub fn resolve(
        &self,
        cursors: [usize; 2],
        arena: &Arena,
        world: &mut World,
        output: &mut OracleIntentBatch,
    ) {
        output
            .intents
            .resize(world.view().len(), OracleIntent::default());
        for index in 0..world.ids.len() {
            if let BodyId::Player { team, local } = world.ids[index] {
                world.squads[index] = self.nodes[cursors[team.index()]].squad_for(local.get());
            }
        }

        // The assignment pass is over, so the rest of the tick only reads: one shared borrow lets
        // `Squad(n)` and the per-body `NearestToMe` scan the same latched roster.
        let world = &*world;
        let anchors = Anchors::build(world, arena);
        for index in 0..world.ids.len() {
            let BodyId::Player { team, local } = world.ids[index] else {
                output.intents[index] = OracleIntent {
                    position: world.positions[index],
                    spin: Vec3Fx::ZERO,
                };
                continue;
            };
            output.intents[index] = resolve_body(
                &self.nodes[cursors[team.index()]],
                Body {
                    team,
                    role: world.roles[index],
                    local,
                    position: world.positions[index],
                },
                &anchors,
            );
        }
    }
}

/// The resolving body's own facts: its role and local ID pick the assignment, and the position is
/// read by the one verb and the one target that have no squad-shared answer, `Jam` and
/// `NearestToMe`.
#[derive(Debug, Clone, Copy)]
struct Body {
    team: Team,
    role: Role,
    local: LocalId,
    position: Vec3Fx,
}

/// The world references every verb resolves against: the squad-shared anchors built once per tick,
/// and the roster behind them, which the one per-body target rescans.
struct Anchors<'a> {
    /// Latched roster, held for `NearestToMe` alone — every other target is already reduced below.
    world: &'a World,
    ball: Vec3Fx,
    /// Each team's own goal-mouth center; the opposing entry is that team's attacking goal.
    goals: [Vec3Fx; 2],
    mouth_half_width: Fx,
    /// Each team's body nearest the game ball, which is the opposing team's `NearestOpponent`.
    nearest: [Vec3Fx; 2],
    /// Centroid per team and squad, absent where the squad holds nobody.
    squad_centroids: [[Option<Vec3Fx>; SQUAD_COUNT]; 2],
}

impl<'a> Anchors<'a> {
    fn build(world: &'a World, arena: &Arena) -> Self {
        let ball = world.positions[world.objective_index()];
        Self {
            world,
            ball,
            goals: Team::ALL.map(|team| {
                Vec3Fx::new(
                    -arena.half_length * mirror(team),
                    Fx::ZERO,
                    arena.goal_height * Fx::HALF,
                )
            }),
            mouth_half_width: arena.goal_half_width,
            nearest: Team::ALL.map(|team| nearest_to(world, team, ball)),
            squad_centroids: Team::ALL.map(|team| {
                std::array::from_fn(|squad| {
                    squad_centroid(
                        world,
                        team,
                        u8::try_from(squad).expect("a squad index fits u8"),
                    )
                })
            }),
        }
    }

    fn target(&self, target: Target, body: Body, slot: Vec3Fx) -> Vec3Fx {
        let team = body.team;
        match target {
            Target::GameBall => self.ball,
            Target::OwnGoal => self.goals[team.index()],
            Target::OpponentGoal => self.goals[team.opponent().index()],
            Target::Squad(squad) => {
                self.squad_centroids[team.index()][usize::from(squad)].unwrap_or(slot)
            }
            Target::NearestOpponent => self.nearest[team.opponent().index()],
            // The one target whose minimum is per body rather than per team, so it cannot be
            // reduced to an anchor: a squad on it has no shared aim point and each member's form is
            // built about its own, which is the asymmetry `Jam` already carries.
            Target::NearestToMe => nearest_to(self.world, team.opponent(), body.position),
            Target::Slot => slot,
        }
    }
}

/// The body of `team` at minimum squared distance to `reference`.
///
/// The ranking key is the exact widened square, not `length_squared`: the Q16.16 renormalization
/// there leaves `i32` for separations past ~181 r, and a comparison needs no renormalization at
/// all. `min_by_key` keeps the first minimum, which is the spec's lowest-canonical-index tie-break.
fn nearest_to(world: &World, team: Team, reference: Vec3Fx) -> Vec3Fx {
    (0..world.ids.len())
        .filter(|&index| world.teams[index] == Some(team))
        .min_by_key(|&index| {
            let offset = world.positions[index] - reference;
            [offset.x, offset.y, offset.z]
                .into_iter()
                .map(|component| {
                    u64::try_from(mul64(component.raw(), component.raw()))
                        .expect("a squared component is nonnegative")
                })
                .sum::<u64>()
        })
        .map(|index| world.positions[index])
        .expect("both teams hold at least one physical body")
}

/// The proposal's widened component sum: raw coordinates accumulate in `i64` so a full squad near
/// the coordinate boundary cannot wrap, and the truncating `i64` quotient of a raw Q16.16 sum by a
/// plain count is exactly `qdiv` of the sum by the count as Q16.16. A mean of in-domain positions
/// is itself in domain, so the narrowing cannot fail.
fn squad_centroid(world: &World, team: Team, squad: u8) -> Option<Vec3Fx> {
    let (sums, members) = (0..world.ids.len())
        .filter(|&index| world.teams[index] == Some(team) && world.squads[index] == squad)
        .fold(([0i64; 3], 0i64), |(sums, members), index| {
            let position = world.positions[index];
            (
                [
                    sums[0] + i64::from(position.x.raw()),
                    sums[1] + i64::from(position.y.raw()),
                    sums[2] + i64::from(position.z.raw()),
                ],
                members + 1,
            )
        });
    (members > 0).then(|| {
        let [x, y, z] = sums.map(|sum| {
            Fx::from_raw(i32::try_from(sum / members).expect("a mean of in-domain positions fits"))
        });
        Vec3Fx::new(x, y, z)
    })
}

fn resolve_body(node: &PlayNode, body: Body, anchors: &Anchors<'_>) -> OracleIntent {
    let (entry, template, ordinal) = match body.role {
        Role::Goalie => (node.goalie_verb, node.goalie, 0),
        Role::Fielder => (
            node.verbs[usize::from(node.squad_for(body.local.get()))],
            node.fielder,
            node.squad_ordinal(body.local.get()),
        ),
        Role::Objective => unreachable!("a player identity cannot have objective role"),
    };
    // The template is the only authored operand a body resolves, so it is the only value the play
    // frame turns; everything downstream of it is world geometry already.
    let slot = half_turn(template.position, body.team);
    let target = anchors.target(entry.target, body, slot);
    let (aim, axis) = aim_point(entry.verb, target, slot, body, anchors);
    OracleIntent {
        position: form_slot(
            entry.form,
            FormFrame::new(axis, body.team),
            aim,
            target,
            ordinal,
        ),
        spin: match entry.verb {
            Verb::Align => half_turn(template.spin, body.team),
            _ => Vec3Fx::ZERO,
        },
    }
}

/// One verb's aim point and construction axis, both in world coordinates.
///
/// A `None` axis is the spec's "no construction axis" and takes the attack axis downstream; a
/// `Some` axis that happens to be zero-length is a degenerate construction, and the difference is
/// what separates the `Align`/`Pursue`/`Guard` fallback from the collapse to `Point`.
fn aim_point(
    verb: Verb,
    target: Vec3Fx,
    slot: Vec3Fx,
    body: Body,
    anchors: &Anchors<'_>,
) -> (Vec3Fx, Option<Vec3Fx>) {
    let own_goal = anchors.goals[body.team.index()];
    let opponent_goal = anchors.goals[body.team.opponent().index()];
    match verb {
        Verb::Align => (slot, None),
        Verb::Pursue => (target, None),
        Verb::Drive => {
            let axis = opponent_goal - target;
            (target - axis.normalized(), Some(axis))
        }
        Verb::Clear => {
            let axis = target - own_goal;
            (target - axis.normalized(), Some(axis))
        }
        Verb::Cover => {
            let axis = own_goal - target;
            (target + axis.normalized() * COVER_GAP, Some(axis))
        }
        Verb::Zone => ((target + own_goal) * Fx::HALF, Some(own_goal - target)),
        Verb::Sweep => (
            Vec3Fx::new((target.x + own_goal.x) * Fx::HALF, target.y, target.z),
            Some(own_goal - target),
        ),
        Verb::Block => {
            let axis = anchors.ball - target;
            (target + axis.normalized() * COVER_GAP, Some(axis))
        }
        Verb::Lead => {
            let axis = opponent_goal - target;
            (target + axis.normalized() * COVER_GAP, Some(axis))
        }
        Verb::Jam => {
            let axis = target - body.position;
            (target + axis.normalized(), Some(axis))
        }
        // The mouth plane is unconditional: a target behind the goal line still projects.
        Verb::Guard => (
            Vec3Fx::new(
                own_goal.x,
                target
                    .y
                    .clamp(-anchors.mouth_half_width, anchors.mouth_half_width),
                own_goal.z,
            ),
            None,
        ),
    }
}

/// Floor-plane basis a formation lays out in.
#[derive(Debug, Clone, Copy)]
struct FormFrame {
    forward: Vec3Fx,
    lateral: Vec3Fx,
}

impl FormFrame {
    /// Lateral is forward rotated −90° about `+z`, already unit whenever forward is, so it is
    /// never renormalized. A zero forward leaves a zero frame and collapses every form to `Point`.
    ///
    /// Nothing here is turned. Forward is either a world-frame construction axis or the resolving
    /// team's own attack axis, and the −90° rotation commutes with the play frame's half turn, so a
    /// formation comes out turned with its team without carrying a sign of its own.
    fn new(axis: Option<Vec3Fx>, team: Team) -> Self {
        let forward = axis.map_or_else(
            || team.attack_axis(),
            |axis| Vec3Fx::new(axis.x, axis.y, Fx::ZERO).normalized(),
        );
        Self {
            forward,
            lateral: Vec3Fx::new(forward.y, -forward.x, Fx::ZERO),
        }
    }
}

/// Place one squad member for its ordinal.
///
/// `Arc` is built about the target rather than the aim point, so both are parameters. No form
/// displaces vertically, which is why the aim point's `z` is restored last.
fn form_slot(form: Form, frame: FormFrame, aim: Vec3Fx, target: Vec3Fx, ordinal: u32) -> Vec3Fx {
    let placed = match form {
        Form::Point => aim,
        Form::Pod { rank, file, gap } => {
            let cell = rank * file;
            let pod = ordinal / cell;
            let within = ordinal % cell;
            let width = i32::try_from(file).expect("a validated pod file fits i32");
            let column = i32::try_from(within % file).expect("a pod file index fits i32");
            let row = i32::try_from(within / file).expect("a pod rank index fits i32");
            let across = Fx::from_i32(alternating_step(pod) * (width + 1)) * gap
                + Fx::from_i32(2 * column - (width - 1)) * gap * Fx::HALF;
            aim + frame.lateral * across - frame.forward * (Fx::from_i32(row) * gap)
        }
        Form::Wedge(gap) => {
            let step = alternating_step(ordinal);
            aim + frame.lateral * (Fx::from_i32(step) * gap)
                - frame.forward * (Fx::from_i32(step.abs()) * gap)
        }
        Form::Arc(gap) => {
            let spoke = aim - target;
            let chord = frame.lateral * (Fx::from_i32(alternating_step(ordinal)) * gap);
            target + (spoke + chord).normalized() * spoke.length()
        }
    };
    Vec3Fx::new(placed.x, placed.y, aim.z)
}

/// `0, +1, −1, +2, −2, …`: the one primitive every form centers on without a member count.
fn alternating_step(ordinal: u32) -> i32 {
    let magnitude = i32::try_from(ordinal.div_ceil(2)).expect("a squad ordinal fits i32");
    if ordinal % 2 == 1 {
        magnitude
    } else {
        -magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::cyclic;
    use super::*;

    /// forward `+x`, so lateral is `−y` and a positive step walks toward `−y`.
    fn attacking_frame() -> FormFrame {
        FormFrame::new(None, Team::Zero)
    }

    #[test]
    fn each_form_places_its_ordinals_about_the_aim_point() {
        let frame = attacking_frame();
        let aim = Vec3Fx::new(Fx::from_i32(4), Fx::ZERO, Fx::ONE);
        let gap = Fx::from_i32(2);

        // `Point` converges the whole squad.
        for ordinal in 0..5 {
            assert_eq!(form_slot(Form::Point, frame, aim, aim, ordinal), aim);
        }

        // `Wedge` alternates laterally and sets each step back by its own magnitude.
        let wedge: Vec<_> = (0..3)
            .map(|ordinal| form_slot(Form::Wedge(gap), frame, aim, aim, ordinal))
            .collect();
        assert_eq!(wedge[0], aim);
        assert_eq!(
            wedge[1],
            Vec3Fx::new(Fx::from_i32(2), Fx::from_i32(-2), Fx::ONE)
        );
        assert_eq!(
            wedge[2],
            Vec3Fx::new(Fx::from_i32(2), Fx::from_i32(2), Fx::ONE)
        );

        // `Pod(2, 3, 2)` is three across and two deep, and the fourth pod member drops a rank.
        let pod = Form::Pod {
            rank: 2,
            file: 3,
            gap,
        };
        let across: Vec<_> = (0..4)
            .map(|ordinal| form_slot(pod, frame, aim, aim, ordinal))
            .collect();
        assert_eq!(
            across[0],
            Vec3Fx::new(Fx::from_i32(4), Fx::from_i32(2), Fx::ONE)
        );
        assert_eq!(across[1], aim);
        assert_eq!(
            across[2],
            Vec3Fx::new(Fx::from_i32(4), Fx::from_i32(-2), Fx::ONE)
        );
        assert_eq!(
            across[3],
            Vec3Fx::new(Fx::from_i32(2), Fx::from_i32(2), Fx::ONE)
        );

        // The next pod strides `(file + 1) * gap` clear of the first, leaving one empty slot.
        let second = form_slot(pod, frame, aim, aim, 6);
        assert_eq!(second.y, Fx::from_i32(-6));

        // `Arc` steps along the chord and renormalizes back onto the circle through the aim.
        let target = Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::ONE);
        let arc = form_slot(Form::Arc(gap), frame, aim, target, 1);
        assert_eq!(arc.z, aim.z);
        assert!(arc.y < Fx::ZERO);
        assert!(((arc - target).length() - Fx::from_i32(4)).abs() < Fx::from_raw(64));

        // A spoke with a vertical component renormalizes off the floor plane, and the final slot
        // still holds the aim's `z`: no form displaces vertically.
        let lifted = Vec3Fx::new(Fx::from_i32(4), Fx::ZERO, Fx::from_i32(2));
        let tilted = form_slot(Form::Arc(gap), frame, lifted, target, 1);
        assert_eq!(tilted.z, lifted.z);
        assert!(tilted.y < Fx::ZERO);
    }

    #[test]
    fn a_degenerate_axis_or_a_vanished_spoke_collapses_every_form_to_point() {
        // A construction axis with no floor-plane direction leaves a zero frame.
        let collapsed = FormFrame::new(Some(Vec3Fx::Z), Team::Zero);
        assert_eq!(collapsed.forward, Vec3Fx::ZERO);
        assert_eq!(collapsed.lateral, Vec3Fx::ZERO);

        let aim = Vec3Fx::new(Fx::from_i32(3), Fx::ZERO, Fx::ONE);
        let target = Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::ONE);
        let gap = Fx::from_i32(2);
        for form in [
            Form::Point,
            Form::Pod {
                rank: 2,
                file: 3,
                gap,
            },
            Form::Wedge(gap),
            Form::Arc(gap),
        ] {
            for ordinal in 0..4 {
                assert_eq!(form_slot(form, collapsed, aim, target, ordinal), aim);
            }
        }

        // An `Arc` whose aim already sits on its target has no circle to spread along.
        let frame = attacking_frame();
        for ordinal in 0..4 {
            assert_eq!(form_slot(Form::Arc(gap), frame, aim, aim, ordinal), aim);
        }
    }

    #[test]
    fn team_one_intents_mirror_team_zero_and_node_changes_reassign_squads() {
        let playbook = Playbook::compile_ron(&cyclic()).unwrap();
        let arena = Arena::default();
        let mut world = World::new(10);
        let mut intents = OracleIntentBatch::with_len(world.view().len());
        playbook.resolve([0, 0], &arena, &mut world, &mut intents);
        let zero = world
            .player_index(Team::Zero, LocalId::new(1).unwrap())
            .unwrap();
        let one = world
            .player_index(Team::One, LocalId::new(1).unwrap())
            .unwrap();
        // The play frame is a half turn about `+z`, so the authored template's `y` negates with its
        // `x`. The fixture's template is off the centre line, which is what makes that visible.
        let authored = intents.intents[zero].position;
        assert_ne!(authored.y, Fx::ZERO);
        assert_eq!(
            intents.intents[one].position,
            Vec3Fx::new(-authored.x, -authored.y, authored.z)
        );
        assert_eq!(world.view().squads[zero], 1);
        playbook.resolve([1, 1], &arena, &mut world, &mut intents);
        assert_eq!(world.view().squads[zero], 7);
    }

    #[test]
    fn each_team_resolves_against_its_own_cursor_in_one_pass() {
        let playbook = Playbook::compile_ron(&cyclic()).unwrap();
        let mut world = World::new(10);
        let mut intents = OracleIntentBatch::with_len(world.view().len());
        playbook.resolve([0, 1], &Arena::default(), &mut world, &mut intents);
        let zero = world
            .player_index(Team::Zero, LocalId::new(1).unwrap())
            .unwrap();
        let one = world
            .player_index(Team::One, LocalId::new(1).unwrap())
            .unwrap();
        assert_eq!(world.view().squads[zero], 1);
        assert_eq!(world.view().squads[one], 7);
    }
}
