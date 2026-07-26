//! Deterministic CPU tracer for fixed-substep actuation, contact, and scoring.

use crate::PHYSICS_SUBSTEPS;
use crate::controller::MotorCommandBatch;
use crate::fixed::{Fx, Vec3Fx};
use crate::playbook::OracleIntentBatch;
use crate::world::{ActionCharges, ContactFrame, Role, Team, World};

/// Baked Q16.16 duration of one nominal 120 Hz physics substep.
pub const PHYSICS_DT: Fx = Fx::from_raw(546);

/// Lateral basis for a contact whose normal is parallel to `forward`, which happens when the
/// tangent projection vanishes and `forward` falls back to the ±X attack axis against an end
/// wall. Y is normal to both attack axes, so it completes every reachable degenerate frame.
const DEGENERATE_RIGHT: Vec3Fx = Vec3Fx::Y;

/// White-cove interior and goal-mouth dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arena {
    /// Half-length along X.
    pub half_length: Fx,
    /// Half-width along Y.
    pub half_width: Fx,
    /// Ceiling height above the floor.
    pub height: Fx,
    /// Half-width of each goal mouth.
    pub goal_half_width: Fx,
    /// Goal mouth height.
    pub goal_height: Fx,
}

impl Default for Arena {
    fn default() -> Self {
        Self {
            half_length: Fx::from_i32(16),
            half_width: Fx::from_i32(9),
            height: Fx::from_i32(6),
            goal_half_width: Fx::from_i32(2),
            goal_height: Fx::from_i32(2),
        }
    }
}

/// Versioned fixed-point physics constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsConfig {
    /// Arena dimensions.
    pub arena: Arena,
    /// Downward acceleration.
    pub gravity: Fx,
    /// Magnus coefficient.
    pub magnus: Fx,
    /// Per-substep whole-velocity retention, i.e. air drag.
    pub drag: Fx,
    /// Per-contact retention of the tangential velocity component, i.e. surface friction.
    pub tangential_retention: Fx,
    /// Contact spin response.
    pub traction: Fx,
    /// Spin residual gain.
    pub residual_gain: Fx,
    /// Surface jump impulse.
    pub jump_impulse: Fx,
    /// Surface boost impulse.
    pub boost_impulse: Fx,
    /// Air cue impulse.
    pub air_impulse: Fx,
    /// Contact restitution.
    pub restitution: Fx,
    /// Maximum linear speed.
    pub speed_cap: Fx,
    /// Canonical sphere-collision sweeps per substep.
    pub collision_iterations: u8,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            arena: Arena::default(),
            gravity: Fx::from_raw(642_253), // 9.8
            magnus: Fx::from_raw(328),      // 0.005
            drag: Fx::from_raw(65_208),     // 0.995
            // Numerically equal to `drag` today, independently tunable by design.
            tangential_retention: Fx::from_raw(65_208), // 0.995
            traction: Fx::from_raw(16_384),
            residual_gain: Fx::HALF,
            jump_impulse: Fx::from_raw(294_912),
            boost_impulse: Fx::from_i32(3),
            air_impulse: Fx::from_i32(2),
            restitution: Fx::HALF,
            speed_cap: Fx::from_i32(20),
            collision_iterations: 2,
        }
    }
}

/// Which side touched the game ball during one tick's pair stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallTouch {
    /// Exactly one team's bodies touched the ball.
    Team(Team),
    /// Bodies from both teams touched the ball, which is honestly nobody's possession.
    ///
    /// The graph-v0 proposal's Triggers section rejects the tempting alternative — a
    /// canonical-index tie-break — as structurally biased: team zero owns the low indices
    /// and would win every contested touch in the match.
    Contested,
}

/// Sparse physics events from one authoritative tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicsEvents {
    /// Team that scored, if any.
    pub scorer: Option<Team>,
    /// Signed objective travel along X, summed per substep over continuous motion only.
    pub objective_progress: Fx,
    /// Who touched the game ball during this step, if anyone.
    ///
    /// Populated from the pair stage only: a player/ball contact yields `Some`, and
    /// `None` covers both no observed touch and an untouched-by-a-player arena contact.
    /// The caller pairs this with the tick to maintain the `Possession` trigger's replay
    /// state (`GAME_TICK.md` substep stage 9); physics.rs does not stamp a tick itself.
    pub ball_touch: Option<BallTouch>,
}

/// One goal resolved within a substep.
#[derive(Debug, Clone, Copy)]
struct Goal {
    team: Team,
    /// Signed X of the centre reposition, which is not motion and earns no progress.
    reposition_x: Fx,
}

/// Refresh oracle steering from latched commands and advance two fixed substeps.
pub fn step(
    world: &mut World,
    intents: &OracleIntentBatch,
    commands: &MotorCommandBatch,
    config: &PhysicsConfig,
) -> PhysicsEvents {
    assert_eq!(world.ids.len(), commands.commands.len());
    let objective = world.objective_index();
    let mut events = PhysicsEvents::default();
    let mut touched = [false; 2];
    for _ in 0..PHYSICS_SUBSTEPS {
        let entry_x = world.positions[objective].x;
        apply_actuators(world, intents, commands, config);
        integrate(world, config);
        for _ in 0..config.collision_iterations {
            collide_spheres(world, config.restitution, objective, &mut touched);
        }
        let goal = resolve_arena(world, config);
        let reposition_x = goal.map_or(Fx::ZERO, |goal| goal.reposition_x);
        events.objective_progress += world.positions[objective].x - entry_x - reposition_x;
        events.scorer = events.scorer.or(goal.map(|goal| goal.team));
    }
    events.ball_touch = match touched {
        [false, false] => None,
        [true, false] => Some(BallTouch::Team(Team::Zero)),
        [false, true] => Some(BallTouch::Team(Team::One)),
        [true, true] => Some(BallTouch::Contested),
    };
    events
}

fn apply_actuators(
    world: &mut World,
    intents: &OracleIntentBatch,
    commands: &MotorCommandBatch,
    config: &PhysicsConfig,
) {
    for body in 0..world.ids.len() {
        if world.roles[body] == Role::Objective {
            continue;
        }
        let command = commands.commands[body];
        let normal = world.contacts[body].normal;
        let desired_forward = intents.intents[body].position - world.positions[body];
        let tangent = desired_forward - normal * desired_forward.dot(normal);
        let forward = if tangent == Vec3Fx::ZERO {
            world.teams[body]
                .expect("player bodies have teams")
                .attack_axis()
        } else {
            tangent.normalized()
        };
        let right = direction_or(normal.cross(forward), DEGENERATE_RIGHT);
        let residual = forward * command.spin_residual.x
            + right * command.spin_residual.y
            + normal * command.spin_residual.z;
        let desired_spin = intents.intents[body].spin + residual * config.residual_gain;
        if world.contacts[body].touching {
            let current_spin = world.spins[body];
            world.spins[body] += (desired_spin - current_spin) * config.traction;
        }

        if world.contacts[body].touching
            && world.charges[body].surface
            && (command.jump || command.boost)
        {
            if command.jump {
                world.velocities[body] += normal * config.jump_impulse;
            }
            if command.boost {
                world.velocities[body] += forward * config.boost_impulse;
            }
            world.charges[body].surface = false;
            world.contacts[body].touching = false;
        }
        if world.charges[body].air && command.air_cue {
            // A hit offset that exactly cancels `-forward` still spends the charge, so fall
            // back to the zero-offset cue rather than to a no-op impulse.
            let cue_direction = direction_or(
                right * command.cue_hit[0] + normal * command.cue_hit[1] - forward,
                -forward,
            );
            world.velocities[body] += cue_direction * config.air_impulse;
            world.charges[body].air = false;
        }
    }
}

fn integrate(world: &mut World, config: &PhysicsConfig) {
    for body in 0..world.ids.len() {
        let magnus = world.spins[body].cross(world.velocities[body]) * config.magnus;
        world.velocities[body] += (magnus - Vec3Fx::Z * config.gravity) * PHYSICS_DT;
        world.velocities[body] = world.velocities[body] * config.drag;
        cap_speed(&mut world.velocities[body], config.speed_cap);
        world.positions[body] += world.velocities[body] * PHYSICS_DT;
        world.contacts[body] = ContactFrame {
            touching: false,
            normal: world.contacts[body].normal,
        };
    }
}

fn resolve_arena(world: &mut World, config: &PhysicsConfig) -> Option<Goal> {
    let mut goal = None;
    for body in 0..world.ids.len() {
        let position = world.positions[body];
        let radius = world.radii[body];
        let scorer = in_goal_mouth(world, body, &config.arena)
            .then(|| scoring_team(position.x, radius, &config.arena))
            .flatten();
        let Some(team) = scorer else {
            resolve_body_arena(world, body, config);
            continue;
        };
        world.scores[team.index()] = world.scores[team.index()].saturating_add(1);
        world.positions[body] = Vec3Fx::new(Fx::ZERO, Fx::ZERO, radius);
        world.velocities[body] = Vec3Fx::ZERO;
        world.spins[body] = Vec3Fx::ZERO;
        world.contacts[body] = ContactFrame::default();
        world.charges[body] = ActionCharges::default();
        goal = Some(Goal {
            team,
            reposition_x: -position.x,
        });
    }
    goal
}

/// Whether the objective sits in the goal cross-section, where the end walls do not apply.
fn in_goal_mouth(world: &World, body: usize, arena: &Arena) -> bool {
    let position = world.positions[body];
    world.roles[body] == Role::Objective
        && position.y.abs() <= arena.goal_half_width
        && position.z <= arena.goal_height
}

fn scoring_team(x: Fx, radius: Fx, arena: &Arena) -> Option<Team> {
    if x > arena.half_length + radius {
        return Some(Team::Zero);
    }
    if x < -arena.half_length - radius {
        return Some(Team::One);
    }
    None
}

fn resolve_body_arena(world: &mut World, body: usize, config: &PhysicsConfig) {
    let radius = world.radii[body];
    let mut correction = Vec3Fx::ZERO;
    let mut normal = Vec3Fx::ZERO;
    let position = world.positions[body];
    if position.z < radius {
        correction.z += radius - position.z;
        normal += Vec3Fx::Z;
    }
    if position.z > config.arena.height - radius {
        correction.z -= position.z - (config.arena.height - radius);
        normal -= Vec3Fx::Z;
    }
    if position.y < -config.arena.half_width + radius {
        correction.y += -config.arena.half_width + radius - position.y;
        normal += Vec3Fx::Y;
    }
    if position.y > config.arena.half_width - radius {
        correction.y -= position.y - (config.arena.half_width - radius);
        normal -= Vec3Fx::Y;
    }
    let in_goal = in_goal_mouth(world, body, &config.arena);
    if !in_goal && position.x < -config.arena.half_length + radius {
        correction.x += -config.arena.half_length + radius - position.x;
        normal += Vec3Fx::X;
    }
    if !in_goal && position.x > config.arena.half_length - radius {
        correction.x -= position.x - (config.arena.half_length - radius);
        normal -= Vec3Fx::X;
    }
    if normal == Vec3Fx::ZERO {
        return;
    }
    world.positions[body] += correction;
    let normal = normal.normalized();
    let outward_speed = world.velocities[body].dot(normal);
    if outward_speed < Fx::ZERO {
        world.velocities[body] -= normal * outward_speed * (Fx::ONE + config.restitution);
    }
    let normal_velocity = normal * world.velocities[body].dot(normal);
    world.velocities[body] =
        normal_velocity + (world.velocities[body] - normal_velocity) * config.tangential_retention;
    world.contacts[body] = ContactFrame {
        touching: true,
        normal,
    };
    world.charges[body] = ActionCharges::default();
}

/// Resolve overlapping pairs and accumulate which teams touched the game ball.
///
/// `touched` flags each team seen touching `objective` across every call this step (both
/// substeps, every collision iteration): a pure set union over the touching pairs, so the
/// result is the same however that set is built up, per determinism rule 6's ban on
/// order-dependent consumption of an unordered contact set.
fn collide_spheres(world: &mut World, restitution: Fx, objective: usize, touched: &mut [bool; 2]) {
    for first in 0..world.ids.len() {
        for second in first + 1..world.ids.len() {
            let offset = world.positions[second] - world.positions[first];
            let distance = offset.length();
            let minimum = world.radii[first] + world.radii[second];
            if distance >= minimum {
                continue;
            }
            if first == objective || second == objective {
                let player = if first == objective { second } else { first };
                let team = world.teams[player].expect("ball touchers are players");
                touched[team.index()] = true;
            }
            let normal = if distance == Fx::ZERO {
                Vec3Fx::X
            } else {
                offset / distance
            };
            let correction = normal * ((minimum - distance) * Fx::HALF);
            world.positions[first] -= correction;
            world.positions[second] += correction;
            let relative_speed = (world.velocities[second] - world.velocities[first]).dot(normal);
            if relative_speed >= Fx::ZERO {
                continue;
            }
            let impulse = -relative_speed * (Fx::ONE + restitution) * Fx::HALF;
            world.velocities[first] -= normal * impulse;
            world.velocities[second] += normal * impulse;
        }
    }
}

/// Unit vector along `vector`, or `fallback` when it has no direction.
///
/// Normalization is total and answers zero for a zero input, which is arithmetically defined
/// but not a physical direction: a zero basis vector silently deletes whatever rides on it.
fn direction_or(vector: Vec3Fx, fallback: Vec3Fx) -> Vec3Fx {
    let direction = vector.normalized();
    if direction == Vec3Fx::ZERO {
        return fallback;
    }
    direction
}

fn cap_speed(velocity: &mut Vec3Fx, cap: Fx) {
    let length = velocity.length();
    if length > cap {
        *velocity = *velocity * (cap / length);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{MotorCommand, MotorCommandBatch};
    use crate::playbook::{OracleIntent, OracleIntentBatch};

    fn still_intents(world: &World) -> OracleIntentBatch {
        OracleIntentBatch {
            intents: world
                .positions
                .iter()
                .copied()
                .map(|position| OracleIntent {
                    position: position + Vec3Fx::X,
                    spin: Vec3Fx::ZERO,
                })
                .collect(),
        }
    }

    #[test]
    fn jump_and_boost_share_one_surface_charge_and_can_fire_together() {
        let mut world = World::new(10);
        let intents = still_intents(&world);
        let mut commands = MotorCommandBatch::with_len(world.ids.len());
        commands.commands[0] = MotorCommand {
            jump: true,
            boost: true,
            ..MotorCommand::default()
        };
        apply_actuators(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert!(!world.charges[0].surface);
        assert!(world.velocities[0].z > Fx::ZERO);
        assert!(world.velocities[0].x > Fx::ZERO);
    }

    #[test]
    fn one_air_cue_is_consumed_until_arena_contact_restores_both_charges() {
        let mut world = World::new(10);
        world.contacts[0].touching = false;
        let intents = still_intents(&world);
        let mut commands = MotorCommandBatch::with_len(world.ids.len());
        commands.commands[0].air_cue = true;
        apply_actuators(&mut world, &intents, &commands, &PhysicsConfig::default());
        let first_velocity = world.velocities[0];
        apply_actuators(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert_eq!(world.velocities[0], first_velocity);
        world.positions[0].z = Fx::ZERO;
        resolve_body_arena(&mut world, 0, &PhysicsConfig::default());
        assert_eq!(world.charges[0], ActionCharges::default());
    }

    /// Against an end wall the forward fallback is the attack axis, i.e. the contact normal.
    fn wall_pinned_world() -> World {
        let mut world = World::new(10);
        world.contacts[0] = ContactFrame {
            touching: true,
            normal: Vec3Fx::X,
        };
        world
    }

    #[test]
    fn a_lateral_residual_survives_a_forward_parallel_to_the_contact_normal() {
        let mut world = wall_pinned_world();
        let intents = still_intents(&world);
        let mut commands = MotorCommandBatch::with_len(world.ids.len());
        commands.commands[0].spin_residual = Vec3Fx::Y;
        apply_actuators(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert!(world.spins[0].y > Fx::ZERO);
    }

    #[test]
    fn an_air_cue_that_cancels_its_own_offset_still_pushes() {
        let mut world = wall_pinned_world();
        let intents = still_intents(&world);
        let mut commands = MotorCommandBatch::with_len(world.ids.len());
        commands.commands[0] = MotorCommand {
            air_cue: true,
            cue_hit: [Fx::ZERO, Fx::ONE],
            ..MotorCommand::default()
        };
        apply_actuators(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert!(!world.charges[0].air);
        assert!(world.velocities[0].x < Fx::ZERO);
    }

    #[test]
    fn combined_floor_and_wall_contact_produces_a_cove_normal() {
        let mut world = World::new(10);
        world.positions[0] = Vec3Fx::new(Fx::from_i32(16), Fx::ZERO, Fx::ZERO);
        resolve_body_arena(&mut world, 0, &PhysicsConfig::default());
        let normal = world.contacts[0].normal;
        assert!(normal.x < Fx::ZERO);
        assert!(normal.z > Fx::ZERO);
    }

    #[test]
    fn speed_is_bounded_after_integration() {
        let mut world = World::new(10);
        world.velocities[0] = Vec3Fx::new(Fx::from_i32(100), Fx::ZERO, Fx::ZERO);
        integrate(&mut world, &PhysicsConfig::default());
        assert!(world.velocities[0].length() <= PhysicsConfig::default().speed_cap);
    }

    #[test]
    fn magnus_acceleration_is_mirrored_with_spin() {
        let mut positive = World::new(10);
        let mut negative = positive.clone();
        positive.velocities[0] = Vec3Fx::X * Fx::from_i32(4);
        negative.velocities[0] = positive.velocities[0];
        positive.spins[0] = Vec3Fx::Z * Fx::from_i32(2);
        negative.spins[0] = -positive.spins[0];
        integrate(&mut positive, &PhysicsConfig::default());
        integrate(&mut negative, &PhysicsConfig::default());
        assert_eq!(positive.velocities[0].y, -negative.velocities[0].y);
    }

    #[test]
    fn canonical_collision_sweeps_are_replay_stable() {
        let mut first = World::new(10);
        let objective = first.objective_index();
        first.positions[0] = Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::ONE);
        first.positions[1] = Vec3Fx::new(Fx::HALF, Fx::ZERO, Fx::ONE);
        first.velocities[0] = Vec3Fx::X;
        first.velocities[1] = -Vec3Fx::X;
        let mut second = first.clone();
        let mut first_touched = [false; 2];
        let mut second_touched = [false; 2];
        for _ in 0..PhysicsConfig::default().collision_iterations {
            collide_spheres(
                &mut first,
                PhysicsConfig::default().restitution,
                objective,
                &mut first_touched,
            );
            collide_spheres(
                &mut second,
                PhysicsConfig::default().restitution,
                objective,
                &mut second_touched,
            );
        }
        assert_eq!(first, second);
        assert_eq!(first_touched, second_touched);
        assert!(first.positions[0].x < first.positions[1].x);
    }

    #[test]
    fn a_goal_reposition_keeps_its_approach_progress_and_discards_the_jump() {
        let mut world = World::new(10);
        let objective = world.objective_index();
        let radius = world.radii[objective];
        let entry_x = Fx::from_i32(16) + Fx::from_raw(Fx::ONE_RAW / 4);
        world.positions[objective] = Vec3Fx::new(entry_x, Fx::ZERO, radius);
        world.velocities[objective] = Vec3Fx::X * Fx::from_i32(20);
        let intents = still_intents(&world);
        let commands = MotorCommandBatch::with_len(world.ids.len());
        let events = step(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert_eq!(events.scorer, Some(Team::Zero));
        assert_eq!(world.positions[objective].x, Fx::ZERO);
        assert!(events.objective_progress > Fx::ZERO);
        assert!(events.objective_progress < entry_x);
    }

    #[test]
    fn motor_refreshes_before_each_physics_substep() {
        let mut world = World::new(10);
        let mut intents = still_intents(&world);
        intents.intents[0].spin = Vec3Fx::X * Fx::from_i32(4);
        let commands = MotorCommandBatch::with_len(world.ids.len());
        step(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert!(world.spins[0].x > Fx::ONE);
    }

    #[test]
    fn game_ball_touch_is_none_without_a_player_contact() {
        let mut world = World::new(10);
        let intents = still_intents(&world);
        let commands = MotorCommandBatch::with_len(world.ids.len());
        let events = step(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert_eq!(events.ball_touch, None);
    }

    #[test]
    fn game_ball_touch_records_the_touching_players_team() {
        let mut world = World::new(10);
        let objective = world.objective_index();
        world.positions[3] = world.positions[objective];
        let intents = still_intents(&world);
        let commands = MotorCommandBatch::with_len(world.ids.len());
        let events = step(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert_eq!(events.ball_touch, Some(BallTouch::Team(Team::Zero)));
    }

    /// The proposal's Triggers section: a step in which bodies from both teams touch the
    /// game ball records a neutral touch. A canonical-index tie-break would hand team zero
    /// (`lower` here) every contested touch in the match, which is the rejected alternative.
    #[test]
    fn a_game_ball_touch_by_both_teams_in_one_step_is_contested() {
        let mut world = World::new(10);
        let objective = world.objective_index();
        let lower = 3;
        let higher = world.active_per_team() + 2;
        world.positions[lower] = world.positions[objective];
        world.positions[higher] = world.positions[objective];
        let intents = still_intents(&world);
        let commands = MotorCommandBatch::with_len(world.ids.len());
        let events = step(&mut world, &intents, &commands, &PhysicsConfig::default());
        assert_eq!(events.ball_touch, Some(BallTouch::Contested));
    }
}
