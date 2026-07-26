//! Coach encoding at 15 Hz: `encode_coaches` fills one input column per team from the union
//! retina, formation-error, and reward lanes `encode_coach_column` writes, steps the coach pool
//! one pulse a quarter as often as the body pools, then fans its 64 mailbox lanes and 8 edge-port
//! logits back out per team. Split from `body.rs` because a coach pulse publishes state
//! (`mailboxes`, `edge_logits`) the same-tick body pulse reads — an ordering `ZoomieBackend::act`
//! owns — and neither encoder needs to know about the other to keep it.

use zoomie_core::{LaneRows, NetMode};
use zoomie_math::fixed::ONE;
use zoomie_pop::{ChunkedSchedule, PulseEnv, Schedule};
use zoomieball_core::COACH_INTERVAL_TICKS;
use zoomieball_core::controller::ActRequest;
use zoomieball_core::perception::Relation;
use zoomieball_core::world::Team;

use super::sense::{
    clamp_i64, clear_column, coach_group, inverse_depth, receptor, signed_weight, write_signed,
};
use crate::pool::RolePool;

/// Fill both teams' input columns, step the coach pool one pulse, and fan mailboxes and
/// edge-port logits back out per team.
pub(crate) fn encode_coaches(
    pool: &mut RolePool,
    request: ActRequest<'_>,
    team_rewards: &[[i32; 2]; 2],
    mailboxes: &mut [[[i32; 8]; 8]; 2],
    edge_logits: &mut [[i32; 8]; 2],
) {
    for team in Team::ALL {
        encode_coach_column(&mut pool.inputs, team.index(), team, request, team_rewards);
    }
    pool.pop.fill_input_lanes(0, &pool.inputs);
    ChunkedSchedule.step(
        &mut pool.pop,
        NetMode::Deterministic,
        PulseEnv {
            dt: pool.dt,
            tick: request.tick / u64::from(COACH_INTERVAL_TICKS),
        },
    );
    pool.pop.read_output_lanes(&mut pool.outputs);
    for team in Team::ALL {
        let column = team.index();
        for (squad, mailbox) in mailboxes[column].iter_mut().enumerate() {
            for (lane, value) in mailbox.iter_mut().enumerate() {
                *value = pool.outputs.row(squad * 8 + lane)[column];
            }
        }
        for (lane, value) in edge_logits[column].iter_mut().enumerate() {
            *value = pool.outputs.row(64 + lane)[column];
        }
    }
}

fn encode_coach_column(
    inputs: &mut LaneRows,
    column: usize,
    team: Team,
    request: ActRequest<'_>,
    team_rewards: &[[i32; 2]; 2],
) {
    clear_column(inputs, column);
    let mut retina = [0i64; 80];
    let mut formation = [0i64; 16];
    let mut formation_counts = [0i64; 8];
    let mut threat = [0i64; 8];
    for body in 0..request.world.len() {
        if request.world.teams[body] != Some(team) {
            continue;
        }
        let squad = usize::from(request.world.squads[body]);
        let error = request.intents.intents[body].position - request.world.positions[body];
        formation[squad * 2] += i64::from(error.x.raw());
        formation[squad * 2 + 1] += i64::from(error.y.raw());
        formation_counts[squad] += 1;
        for ray in request.observations.for_body(body) {
            let receptor = receptor(ray.direction);
            let group = coach_group(ray.tag);
            retina[group * 8 + receptor] =
                retina[group * 8 + receptor].saturating_add(i64::from(signed_weight(ray)));
            if ray.tag.relation == Relation::Opponent {
                threat[receptor] =
                    threat[receptor].saturating_add(i64::from(inverse_depth(ray.depth)));
            }
        }
    }
    for (lane, value) in retina.into_iter().enumerate() {
        write_signed(inputs, lane, column, clamp_i64(value).clamp(-ONE, ONE));
    }
    for squad in 0..8 {
        let count = formation_counts[squad].max(1);
        for axis in 0..2 {
            let raw = clamp_i64(formation[squad * 2 + axis] / count).clamp(-8 * ONE, 8 * ONE) / 8;
            write_signed(inputs, 80 + squad * 2 + axis, column, raw);
        }
    }
    for (lane, value) in threat.into_iter().enumerate() {
        write_signed(inputs, 96 + lane, column, clamp_i64(value).clamp(-ONE, ONE));
    }
    inputs.row_mut(104 + request.play_node % 8)[column] = ONE;
    for lane in 0..8 {
        inputs.row_mut(112 + lane)[column] = if request.enabled_edges & (1 << lane) == 0 {
            0
        } else {
            ONE
        };
    }
    write_signed(inputs, 120, column, team_rewards[team.index()][0]);
    write_signed(inputs, 121, column, team_rewards[team.index()][1]);
    for lane in 122..128 {
        inputs.row_mut(lane)[column] = ONE / 2;
    }
}
