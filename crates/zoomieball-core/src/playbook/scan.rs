//! The step-2 port scan: trigger predicates over latched tick input, world state, and match
//! metadata, in fixed eight-iteration first-true-wins order.

use crate::COACH_INTERVAL_TICKS;
use crate::fixed::Fx;
use crate::world::{Team, World};

use super::{GraphState, PORT_COUNT, PlayNode, Trigger, mirror};

/// Scan one node's eight ports and return the cursor the resolving team holds this tick.
///
/// The scan is a fixed eight iterations whatever the declared edge count, ports past that count
/// are false, and the first true port wins, so at most one transition occurs per team per tick.
#[must_use]
pub fn next_cursor(
    node: &PlayNode,
    cursor: usize,
    team: Team,
    state: &GraphState,
    world: &World,
    edge_logits: [Fx; PORT_COUNT],
) -> usize {
    let fired: [bool; PORT_COUNT] =
        std::array::from_fn(|port| port_fires(node, port, team, state, world, edge_logits));
    fired
        .iter()
        .position(|&fired| fired)
        .map_or(cursor, |port| node.edges[port].to)
}

fn port_fires(
    node: &PlayNode,
    port: usize,
    team: Team,
    state: &GraphState,
    world: &World,
    edge_logits: [Fx; PORT_COUNT],
) -> bool {
    let Some(edge) = node.edges.get(port) else {
        return false;
    };
    let ball = world.positions[world.objective_index()];
    let attacking_x = ball.x * mirror(team);
    match edge.trigger {
        Trigger::Always => true,
        Trigger::Elapsed(ticks) => {
            world.tick().saturating_sub(state.entered[team.index()]) >= u64::from(ticks)
        }
        Trigger::BallPast(x) => attacking_x >= x,
        Trigger::BallBehind(x) => attacking_x <= x,
        Trigger::BallAloft(z) => ball.z >= z,
        Trigger::Possession(relation) => state.possession(team, world.tick()) == relation,
        Trigger::Lead(goals) => {
            let scores = world.scores();
            i32::from(scores[team.index()]) - i32::from(scores[team.opponent().index()]) >= goals
        }
        // Spending each pulse once stops one pulse from driving four transitions.
        Trigger::CoachEdge => {
            world.tick() % u64::from(COACH_INTERVAL_TICKS) == 1
                && edge_logits[port] > node.coach_gate
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Playbook;
    use super::super::fixtures::{SHIPPED, shipped};
    use super::*;
    use crate::fixed::Vec3Fx;

    #[test]
    fn the_port_scan_takes_the_first_true_port_and_otherwise_holds() {
        let playbook = shipped();
        let recover = &playbook.nodes()[1];
        let mut world = World::new(10);
        let state = GraphState::default();
        let quiet = [Fx::ZERO; PORT_COUNT];

        // Port 0 is `BallPast(0.0)`; a ball behind halfway leaves ports 0 and 1 false.
        world.set_position(
            world.objective_index(),
            Vec3Fx::new(Fx::from_i32(-4), Fx::ZERO, Fx::ONE),
        );
        assert_eq!(
            next_cursor(recover, 1, Team::Zero, &state, &world, quiet),
            1
        );

        // The same ball is past halfway in team one's attacking frame, so its port 0 fires.
        assert_eq!(next_cursor(recover, 1, Team::One, &state, &world, quiet), 0);

        // Port 2 is `Always`, which is only reached because the two ahead of it are false.
        let press = &playbook.nodes()[0];
        assert_eq!(next_cursor(press, 0, Team::Zero, &state, &world, quiet), 0);
    }

    #[test]
    fn a_coach_edge_spends_one_pulse_on_the_tick_after_it() {
        let source = SHIPPED.replacen(
            "(to: 1, trigger: BallBehind(-8.0))",
            "(to: 1, trigger: CoachEdge)",
            1,
        );
        let playbook = Playbook::compile_ron(&source).unwrap();
        let press = &playbook.nodes()[0];
        let mut world = World::new(10);
        let state = GraphState::default();
        let clearing: [Fx; PORT_COUNT] =
            std::array::from_fn(|port| if port == 0 { Fx::ONE } else { Fx::ZERO });

        // Tick zero is a coach-due tick, so its logits are not readable until tick one.
        assert_eq!(
            next_cursor(press, 0, Team::Zero, &state, &world, clearing),
            0
        );
        world.tick = 1;
        assert_eq!(
            next_cursor(press, 0, Team::Zero, &state, &world, clearing),
            1
        );
        world.tick = 2;
        assert_eq!(
            next_cursor(press, 0, Team::Zero, &state, &world, clearing),
            0
        );
    }
}
