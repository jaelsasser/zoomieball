//! Fixed-substep arena, actuator, sphere-contact, and scoring physics.

use crate::controller::MotorCommandBatch;
use crate::fixed::{Fx, Vec3Fx};
use crate::playbook::OracleIntentBatch;
use crate::world::{ActionCharges, ContactFrame, Role, Team, World};
use crate::{PHYSICS_SUBSTEPS, TICK_HZ};

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
    /// Per-substep velocity retention.
    pub drag: Fx,
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

/// Sparse physics events from one authoritative tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicsEvents {
    /// Team that scored, if any.
    pub scorer: Option<Team>,
}

/// Apply commands and advance exactly two fixed substeps.
pub fn step(
    world: &mut World,
    intents: &OracleIntentBatch,
    commands: &MotorCommandBatch,
    config: &PhysicsConfig,
) -> PhysicsEvents {
    assert_eq!(world.ids.len(), commands.commands.len());
    apply_actuators(world, intents, commands, config);
    let mut events = PhysicsEvents::default();
    for _ in 0..PHYSICS_SUBSTEPS {
        integrate(world, config);
        for _ in 0..config.collision_iterations {
            collide_spheres(world, config.restitution);
        }
        events.scorer = events.scorer.or_else(|| resolve_arena(world, config));
    }
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
        let right = normal.cross(forward).normalized();
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
            let cue_direction =
                (right * command.cue_hit[0] + normal * command.cue_hit[1] - forward).normalized();
            world.velocities[body] += cue_direction * config.air_impulse;
            world.charges[body].air = false;
        }
    }
}

fn integrate(world: &mut World, config: &PhysicsConfig) {
    let dt = Fx::from_raw(
        Fx::ONE_RAW / i32::try_from(TICK_HZ * PHYSICS_SUBSTEPS).expect("tick divisor fits i32"),
    );
    for body in 0..world.ids.len() {
        let magnus = world.spins[body].cross(world.velocities[body]) * config.magnus;
        world.velocities[body] += (magnus - Vec3Fx::Z * config.gravity) * dt;
        world.velocities[body] = world.velocities[body] * config.drag;
        cap_speed(&mut world.velocities[body], config.speed_cap);
        world.positions[body] += world.velocities[body] * dt;
        world.contacts[body] = ContactFrame {
            touching: false,
            normal: world.contacts[body].normal,
        };
    }
}

fn resolve_arena(world: &mut World, config: &PhysicsConfig) -> Option<Team> {
    let mut scorer = None;
    for body in 0..world.ids.len() {
        let position = world.positions[body];
        let radius = world.radii[body];
        if world.roles[body] == Role::Objective
            && position.y.abs() <= config.arena.goal_half_width
            && position.z <= config.arena.goal_height
        {
            if position.x > config.arena.half_length + radius {
                scorer = Some(Team::Zero);
            } else if position.x < -config.arena.half_length - radius {
                scorer = Some(Team::One);
            }
            if let Some(team) = scorer {
                world.scores[team.index()] = world.scores[team.index()].saturating_add(1);
                world.positions[body] = Vec3Fx::new(Fx::ZERO, Fx::ZERO, radius);
                world.velocities[body] = Vec3Fx::ZERO;
                world.spins[body] = Vec3Fx::ZERO;
                world.contacts[body] = ContactFrame::default();
                world.charges[body] = ActionCharges::default();
                continue;
            }
        }
        resolve_body_arena(world, body, config);
    }
    scorer
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
    let in_goal = world.roles[body] == Role::Objective
        && position.y.abs() <= config.arena.goal_half_width
        && position.z <= config.arena.goal_height;
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
        normal_velocity + (world.velocities[body] - normal_velocity) * Fx::from_raw(65_208);
    world.contacts[body] = ContactFrame {
        touching: true,
        normal,
    };
    world.charges[body] = ActionCharges::default();
}

fn collide_spheres(world: &mut World, restitution: Fx) {
    for first in 0..world.ids.len() {
        for second in first + 1..world.ids.len() {
            let offset = world.positions[second] - world.positions[first];
            let distance = offset.length();
            let minimum = world.radii[first] + world.radii[second];
            if distance >= minimum {
                continue;
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
        first.positions[0] = Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::ONE);
        first.positions[1] = Vec3Fx::new(Fx::HALF, Fx::ZERO, Fx::ONE);
        first.velocities[0] = Vec3Fx::X;
        first.velocities[1] = -Vec3Fx::X;
        let mut second = first.clone();
        for _ in 0..PhysicsConfig::default().collision_iterations {
            collide_spheres(&mut first, PhysicsConfig::default().restitution);
            collide_spheres(&mut second, PhysicsConfig::default().restitution);
        }
        assert_eq!(first, second);
        assert!(first.positions[0].x < first.positions[1].x);
    }
}
