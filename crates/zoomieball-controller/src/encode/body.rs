//! Fielder and goalie body encoding: `encode_bodies` drives one role pool's per-tick pulse —
//! write each bound body's column, step, decode motor commands from the output lanes — and
//! `encode_body_column`/`encode_goalie_foveae` are its per-body writers, including the goalie's
//! four extra foveae lanes that widen only the columns flagged `goalie`. Split from `coach.rs`
//! because the two encoders share no per-tick state (bodies pulse at 60 Hz, coaches at 15 Hz) and
//! only their sensory primitives in common, which live in `sense.rs`.

use zoomie_core::{LaneRows, NetMode};
use zoomie_math::fixed::ONE;
use zoomie_pop::{ChunkedSchedule, PulseEnv, Schedule};
use zoomieball_core::controller::{ActRequest, MotorCommand, MotorCommandBatch};
use zoomieball_core::fixed::{Fx, Vec3Fx};
use zoomieball_core::perception::{RayObservation, Relation};
use zoomieball_core::world::{Role, Team};

use super::sense::{
    body_group, charge_code, clamp_i64, clear_column, inverse_depth, receptor, signed_weight,
    write_signed,
};
use crate::pool::RolePool;

const OUTPUT_GATE: i32 = ONE / 4;

/// Encode every bound body's column, step the pool one pulse, and decode motor commands back.
pub(crate) fn encode_bodies(
    pool: &mut RolePool,
    request: ActRequest<'_>,
    mailboxes: &[[[i32; 8]; 8]; 2],
    goalie: bool,
    commands: &mut MotorCommandBatch,
) {
    for (column, &body) in pool.body_indices.iter().enumerate() {
        encode_body_column(&mut pool.inputs, column, body, request, mailboxes, goalie);
    }
    pool.pop.fill_input_lanes(0, &pool.inputs);
    ChunkedSchedule.step(
        &mut pool.pop,
        NetMode::Deterministic,
        PulseEnv {
            dt: pool.dt,
            tick: request.tick,
        },
    );
    pool.pop.read_output_lanes(&mut pool.outputs);
    for (column, &body) in pool.body_indices.iter().enumerate() {
        commands.commands[body] = MotorCommand {
            spin_residual: Vec3Fx::new(
                Fx::from_raw(pool.outputs.row(0)[column]),
                Fx::from_raw(pool.outputs.row(1)[column]),
                Fx::from_raw(pool.outputs.row(2)[column]),
            ),
            jump: pool.outputs.row(3)[column] > OUTPUT_GATE,
            boost: pool.outputs.row(4)[column] > OUTPUT_GATE,
            air_cue: pool.outputs.row(5)[column] > OUTPUT_GATE,
            cue_hit: [
                Fx::from_raw(pool.outputs.row(6)[column]),
                Fx::from_raw(pool.outputs.row(7)[column]),
            ],
        };
    }
}

fn encode_body_column(
    inputs: &mut LaneRows,
    column: usize,
    body: usize,
    request: ActRequest<'_>,
    mailboxes: &[[[i32; 8]; 8]; 2],
    goalie: bool,
) {
    clear_column(inputs, column);
    let mut retina = [0i64; 40];
    for ray in request.observations.for_body(body) {
        let lane = body_group(ray.tag) * 8 + receptor(ray.direction);
        retina[lane] = retina[lane].saturating_add(i64::from(signed_weight(ray)));
    }
    for (lane, value) in retina.into_iter().enumerate() {
        write_signed(inputs, lane, column, clamp_i64(value).clamp(-ONE, ONE));
    }
    let delta = request.intents.intents[body].position - request.world.positions[body];
    let direction = delta.normalized();
    for (offset, raw) in [
        direction.x.raw(),
        direction.y.raw(),
        direction.z.raw(),
        (delta.length().raw() / 16).clamp(0, ONE),
        request.intents.intents[body].spin.x.raw().clamp(-ONE, ONE),
        request.intents.intents[body].spin.y.raw().clamp(-ONE, ONE),
        request.intents.intents[body].spin.z.raw().clamp(-ONE, ONE),
        ONE,
    ]
    .into_iter()
    .enumerate()
    {
        write_signed(inputs, 40 + offset, column, raw);
    }
    let team = request.world.teams[body].expect("body populations contain players");
    let squad = usize::from(request.world.squads[body]);
    for (lane, &value) in mailboxes[team.index()][squad].iter().enumerate() {
        write_signed(inputs, 48 + lane, column, value);
    }
    let velocity = request.world.velocities[body] / Fx::from_i32(20);
    let spin = request.world.spins[body] / Fx::from_i32(8);
    for (offset, raw) in [
        velocity.x.raw(),
        velocity.y.raw(),
        velocity.z.raw(),
        spin.x.raw(),
        spin.y.raw(),
        spin.z.raw(),
        if request.world.contacts[body].touching {
            ONE
        } else {
            -ONE
        },
        charge_code(
            request.world.charges[body].surface,
            request.world.charges[body].air,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        write_signed(inputs, 56 + offset, column, raw.clamp(-ONE, ONE));
    }
    if goalie {
        encode_goalie_foveae(inputs, column, request.observations.for_body(body), team);
    }
}

fn encode_goalie_foveae(
    inputs: &mut LaneRows,
    column: usize,
    rays: &[RayObservation],
    _team: Team,
) {
    let mut objective = [0i64; 8];
    let mut opponent = [0i64; 8];
    let mut goal = [0i64; 8];
    let mut corridor = [0i64; 8];
    for ray in rays {
        let receptor = receptor(ray.direction);
        let weight = i64::from(inverse_depth(ray.depth));
        if ray.tag.role == Role::Objective && ray.tag.relation == Relation::Neutral {
            objective[receptor] = objective[receptor].saturating_add(weight);
        }
        if ray.tag.relation == Relation::Opponent {
            opponent[receptor] = opponent[receptor].saturating_add(weight);
            if ray.direction.x.abs() > ray.direction.y.abs() {
                corridor[receptor] = corridor[receptor].saturating_add(weight);
            }
        }
        if ray.tag.relation == Relation::Goal {
            goal[receptor] = goal[receptor].saturating_add(weight);
        }
    }
    for (base, lanes) in [(64, objective), (72, opponent), (80, goal), (88, corridor)] {
        for (lane, value) in lanes.into_iter().enumerate() {
            write_signed(
                inputs,
                base + lane,
                column,
                clamp_i64(value).clamp(-ONE, ONE),
            );
        }
    }
}
