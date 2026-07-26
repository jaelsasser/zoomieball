//! [`ZoomieBackend`]: the crate's one public type, wiring the three role pools (fielders,
//! goalies, coaches) into zoomieball-core's `ControllerBackend` boundary — per-tick `act`
//! (encode -> step -> decode, coach mailboxes published before the same-tick body pulse),
//! per-interval `learn` (team-reward gates set, then one scheduled learn pass per pool), the
//! checkpoint round-trip, and the two-layer `controller_hash`/`learning_hash` witnesses.
//! Everything else in the crate — pool construction, per-population encoding, the checkpoint
//! envelope, the witness folds — is plumbing this file assembles but does not itself define.

use zoomie_core::Gate;
use zoomie_math::fixed::ONE;
use zoomie_math::rng::Seed;
use zoomie_pop::{ChunkedSchedule, PulseEnv, Schedule, SparseCtrnnConfig, SparseCtrnnSpec};
use zoomieball_core::controller::{
    ActRequest, CheckpointError, CheckpointHeader, ControllerBackend, MotorCommandBatch,
    RewardBatch, decode_header,
};
use zoomieball_core::fixed::Fx;
use zoomieball_core::playbook::PORT_COUNT;
use zoomieball_core::world::Team;
use zoomieball_core::{BODY_HZ, COACH_HZ, COACH_INTERVAL_TICKS};

use crate::checkpoint::{
    CHECKPOINT_MAGIC, PAYLOAD_OFFSET, Reader, read_payload, restore_pools, write_header,
    write_payload,
};
use crate::encode::{clamp_i64, encode_bodies, encode_coaches};
use crate::pool::{RolePool, coach_spec, net_id, player_ids, player_indices, topology};
use crate::witness::{controller_witness, learning_witness};

const BODY_DT: i32 = ONE / BODY_HZ.cast_signed();
const COACH_DT: i32 = ONE / COACH_HZ.cast_signed();

/// Zoomie-backed body and coach populations for one match roster.
pub struct ZoomieBackend {
    header: CheckpointHeader,
    /// The 60 Hz fielder pool, persisted and folded first.
    pub(crate) fielders: RolePool,
    /// The 60 Hz goalie pool.
    pub(crate) goalies: RolePool,
    /// The 15 Hz coach pool.
    pub(crate) coaches: RolePool,
    /// Per team, the eight squad mailboxes a coach pulse publishes for the same and later ticks.
    pub(crate) mailboxes: [[[i32; 8]; 8]; 2],
    /// Per team, the coach logits for the current play node's eight ordered edge ports.
    pub(crate) edge_logits: [[i32; 8]; 2],
    /// Per team, the `learn`-written reward the next coach pulse reads at input lanes 120 and 121.
    pub(crate) team_rewards: [[i32; 2]; 2],
    /// The next tick a restored backend must execute — the checkpoint's resume cursor.
    pub(crate) next_tick: u64,
    /// Completed body pulses.
    pub(crate) body_pulses: u64,
    /// Completed coach pulses.
    pub(crate) coach_pulses: u64,
    /// Completed scheduled learning passes.
    pub(crate) learn_passes: u64,
}

impl std::fmt::Debug for ZoomieBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ZoomieBackend")
            .field("header", &self.header)
            .field("fielders", &self.fielders.pop.len())
            .field("goalies", &self.goalies.pop.len())
            .field("coaches", &self.coaches.pop.len())
            .field("next_tick", &self.next_tick)
            .field("body_pulses", &self.body_pulses)
            .field("coach_pulses", &self.coach_pulses)
            .field("learn_passes", &self.learn_passes)
            .finish_non_exhaustive()
    }
}

impl ZoomieBackend {
    /// Seed exactly two active team rosters and two nonphysical coaches.
    #[must_use]
    pub fn new(active_per_team: usize, seed: u64) -> Self {
        assert!(matches!(active_per_team, 10 | 100));
        let config = SparseCtrnnConfig::default();
        let fielder_ids = player_ids(active_per_team, 1..active_per_team);
        let goalie_ids: Vec<_> = Team::ALL.into_iter().map(|team| net_id(team, 0)).collect();
        let coach_ids: Vec<_> = Team::ALL
            .into_iter()
            .map(|team| net_id(team, 100))
            .collect();
        let fielders = RolePool::new(
            SparseCtrnnSpec::reservoir_64_48_8(Seed(seed ^ 0xF1)),
            config,
            &fielder_ids,
            player_indices(active_per_team, 1..active_per_team),
            seed ^ 0xF1,
            BODY_DT,
        );
        let goalies = RolePool::new(
            SparseCtrnnSpec::reservoir_96_64_8(Seed(seed ^ 0x61)),
            config,
            &goalie_ids,
            vec![0, active_per_team],
            seed ^ 0x61,
            BODY_DT,
        );
        let coaches = RolePool::new(
            coach_spec(),
            config,
            &coach_ids,
            Vec::new(),
            seed ^ 0xC0,
            COACH_DT,
        );
        Self {
            header: CheckpointHeader::current(
                u16::try_from(active_per_team).expect("roster fits u16"),
            ),
            fielders,
            goalies,
            coaches,
            mailboxes: [[[0; 8]; 8]; 2],
            edge_logits: [[0; 8]; 2],
            team_rewards: [[0; 2]; 2],
            next_tick: 0,
            body_pulses: 0,
            coach_pulses: 0,
            learn_passes: 0,
        }
    }

    /// `(input, recurrent, output)` geometry for fielders, goalies, and coaches.
    #[must_use]
    pub fn topologies(&self) -> [(usize, usize, usize); 3] {
        [
            topology(self.fielders.pop.spec()),
            topology(self.goalies.pop.spec()),
            topology(self.coaches.pop.spec()),
        ]
    }

    /// `(fielder, goalie, coach)` member counts.
    #[must_use]
    pub fn population_counts(&self) -> (usize, usize, usize) {
        (
            self.fielders.pop.len(),
            self.goalies.pop.len(),
            self.coaches.pop.len(),
        )
    }

    /// Signed selected coach mailbox.
    #[must_use]
    pub const fn mailbox(&self, team: Team, squad: usize) -> [i32; 8] {
        self.mailboxes[team.index()][squad]
    }

    /// The tick a restored backend must execute next — where a resumed driver picks up.
    #[must_use]
    pub const fn next_tick(&self) -> u64 {
        self.next_tick
    }

    /// Completed body controller pulses.
    #[must_use]
    pub const fn body_pulses(&self) -> u64 {
        self.body_pulses
    }

    /// Completed 15 Hz coach pulses.
    #[must_use]
    pub const fn coach_pulses(&self) -> u64 {
        self.coach_pulses
    }
}

impl ControllerBackend for ZoomieBackend {
    fn act(&mut self, request: ActRequest<'_>, commands: &mut MotorCommandBatch) {
        commands.clear();
        if request.coach_due {
            encode_coaches(
                &mut self.coaches,
                request,
                &self.team_rewards,
                &mut self.mailboxes,
                &mut self.edge_logits,
            );
            self.coach_pulses += 1;
        }
        encode_bodies(
            &mut self.fielders,
            request,
            &self.mailboxes,
            false,
            commands,
        );
        encode_bodies(&mut self.goalies, request, &self.mailboxes, true, commands);
        self.body_pulses += 1;
        // The driver's tick is authoritative; deriving the resume coordinate from `body_pulses`
        // would assume `act` ran on every tick from zero, which the trait never promises.
        self.next_tick = request.tick + 1;
    }

    /// The coach lanes are already Q16.16 raw words, so the graph reads them without a rescale.
    fn edge_logits(&self, team: Team) -> [Fx; PORT_COUNT] {
        self.edge_logits[team.index()].map(Fx::from_raw)
    }

    fn learn(&mut self, tick: u64, rewards: &RewardBatch) {
        set_body_gates(&mut self.fielders, rewards);
        set_body_gates(&mut self.goalies, rewards);
        for team in Team::ALL {
            let (sum, count) = rewards
                .rewards
                .iter()
                .enumerate()
                .filter(|(body, _)| {
                    let active = usize::from(self.header.active_per_team);
                    (*body / active) == team.index() && *body < active * 2
                })
                .fold((0i64, 0i64), |(sum, count), (_, reward)| {
                    (
                        sum.saturating_add(i64::from((reward.progress + reward.event).raw())),
                        count + 1,
                    )
                });
            let average = if count == 0 { 0 } else { sum / count };
            let raw = clamp_i64(average).clamp(-ONE, ONE);
            self.team_rewards[team.index()] = [raw, if raw == 0 { 0 } else { raw.signum() * ONE }];
            assert!(
                self.coaches.pop.set_gate(net_id(team, 100), Gate::new(raw)),
                "a coach gate that does not land is silently absent learning"
            );
        }
        let body_env = PulseEnv { dt: BODY_DT, tick };
        let coach_env = PulseEnv {
            dt: COACH_DT,
            tick: tick / u64::from(COACH_INTERVAL_TICKS),
        };
        ChunkedSchedule.learn(&mut self.fielders.pop, body_env);
        ChunkedSchedule.learn(&mut self.goalies.pop, body_env);
        ChunkedSchedule.learn(&mut self.coaches.pop, coach_env);
        self.fielders.pop.clear_gates();
        self.goalies.pop.clear_gates();
        self.coaches.pop.clear_gates();
        self.learn_passes += 1;
    }

    fn checkpoint(&self, output: &mut Vec<u8>) {
        output.clear();
        output.extend_from_slice(CHECKPOINT_MAGIC);
        write_header(output, self.header);
        write_payload(
            output,
            self.next_tick,
            [&self.fielders.pop, &self.goalies.pop, &self.coaches.pop],
        );
        for team in 0..2 {
            for squad in 0..8 {
                for lane in self.mailboxes[team][squad] {
                    output.extend_from_slice(&lane.to_le_bytes());
                }
            }
            for lane in self.edge_logits[team] {
                output.extend_from_slice(&lane.to_le_bytes());
            }
            for lane in self.team_rewards[team] {
                output.extend_from_slice(&lane.to_le_bytes());
            }
        }
        for counter in [self.body_pulses, self.coach_pulses, self.learn_passes] {
            output.extend_from_slice(&counter.to_le_bytes());
        }
    }

    fn restore(&mut self, input: &[u8]) -> Result<(), CheckpointError> {
        if input.get(..4) != Some(CHECKPOINT_MAGIC) {
            return Err(CheckpointError::Malformed);
        }
        let actual = decode_header(input.get(4..).ok_or(CheckpointError::Malformed)?)?;
        if actual != self.header {
            return Err(CheckpointError::AbiMismatch {
                actual,
                expected: self.header,
            });
        }
        let mut reader = Reader::new(input, PAYLOAD_OFFSET);
        let checkpoint = read_payload(&mut reader)?;
        let [fielders, goalies, coaches] = restore_pools(
            &checkpoint,
            [&self.fielders.pop, &self.goalies.pop, &self.coaches.pop],
        )?;
        let mut mailboxes = [[[0; 8]; 8]; 2];
        let mut edge_logits = [[0; 8]; 2];
        let mut team_rewards = [[0; 2]; 2];
        for (team, team_mailboxes) in mailboxes.iter_mut().enumerate() {
            for squad in team_mailboxes {
                for lane in squad {
                    *lane = reader.i32()?;
                }
            }
            for lane in &mut edge_logits[team] {
                *lane = reader.i32()?;
            }
            for lane in &mut team_rewards[team] {
                *lane = reader.i32()?;
            }
        }
        let body_pulses = reader.u64()?;
        let coach_pulses = reader.u64()?;
        let learn_passes = reader.u64()?;
        if !reader.finished() {
            return Err(CheckpointError::Malformed);
        }
        self.fielders.pop = fielders;
        self.goalies.pop = goalies;
        self.coaches.pop = coaches;
        self.mailboxes = mailboxes;
        self.edge_logits = edge_logits;
        self.team_rewards = team_rewards;
        self.next_tick = checkpoint.cursor().next_tick();
        self.body_pulses = body_pulses;
        self.coach_pulses = coach_pulses;
        self.learn_passes = learn_passes;
        Ok(())
    }

    // Both witnesses, their fold order, and what a match does and does not prove: `witness.rs`.
    fn controller_hash(&self) -> u64 {
        controller_witness(self)
    }

    fn learning_hash(&self) -> u64 {
        learning_witness(self)
    }
}

/// Set every bound body's learning gate from its accumulated progress/event reward, ahead of
/// the pool's scheduled learn pass.
fn set_body_gates(pool: &mut RolePool, rewards: &RewardBatch) {
    for (column, &body) in pool.body_indices.iter().enumerate() {
        let raw = (rewards.rewards[body].progress + rewards.rewards[body].event)
            .raw()
            .clamp(-ONE, ONE);
        let id = pool.pop.ids()[column];
        assert!(
            pool.pop.set_gate(id, Gate::new(raw)),
            "a body gate that does not land is silently absent learning"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zoomieball_core::fixed::{Fx, Vec3Fx};
    use zoomieball_core::{Match, MatchConfig};

    use crate::fixture::playbook;

    #[test]
    fn roster_and_topologies_match_the_public_contract() {
        let small = ZoomieBackend::new(10, 7);
        assert_eq!(small.population_counts(), (18, 2, 2));
        assert_eq!(
            small.topologies(),
            [(64, 48, 8), (96, 64, 8), (128, 96, 72)]
        );
        let full = ZoomieBackend::new(100, 7);
        assert_eq!(full.population_counts(), (198, 2, 2));
    }

    #[test]
    fn timing_sixty_body_pulses_include_fifteen_coach_publications() {
        let controller = ZoomieBackend::new(10, 9);
        let mut game = Match::new(MatchConfig::default(), playbook(), controller);
        for _ in 0..60 {
            game.tick();
        }
        assert_eq!(game.controller().body_pulses(), 60);
        assert_eq!(game.controller().coach_pulses(), 15);
        assert_ne!(game.controller().mailbox(Team::Zero, 0), [0; 8]);
    }

    #[test]
    fn timing_fresh_mailbox_is_encoded_into_the_same_tick_body_input() {
        let controller = ZoomieBackend::new(10, 10);
        let mut game = Match::new(MatchConfig::default(), playbook(), controller);
        game.tick();

        // Fielder column zero is team zero's local ID 1, and `press` cycles `[0, 1, 2, 3, …]`, so
        // its mailbox is squad one's.
        let controller = game.controller();
        let mailbox = controller.mailboxes[Team::Zero.index()][1];
        for (lane, value) in mailbox.into_iter().enumerate() {
            assert_eq!(
                controller.fielders.inputs.row(48 + lane)[0],
                i32::midpoint(value.clamp(-ONE, ONE), ONE)
            );
        }
    }

    /// A coach advises one team. Both columns are filled in the same pass from the same
    /// `ActRequest`, so a shared cursor would show up here as two identical node/edge-mask spans
    /// even though the two teams are on different nodes.
    #[test]
    fn encoding_each_coach_column_reads_its_own_teams_cursor_and_edge_mask() {
        let controller = ZoomieBackend::new(10, 29);
        let mut game = Match::new(MatchConfig::default(), playbook(), controller);
        game.select_play_node(1);
        game.tick();

        assert_eq!(
            [game.play_node(Team::Zero), game.play_node(Team::One)],
            [1, 0],
            "the fixture must separate the two cursors"
        );
        let [player, opponent] = Team::ALL.map(Team::index);
        let inputs = &game.controller().coaches.inputs;
        // Lanes 104..112 are one-hot on the node index.
        assert_eq!([inputs.row(104)[player], inputs.row(105)[player]], [0, ONE]);
        assert_eq!(
            [inputs.row(104)[opponent], inputs.row(105)[opponent]],
            [ONE, 0]
        );
        // `recover` declares three ports to `press`'s two, so the masks part at port 2.
        assert_eq!(
            [inputs.row(114)[player], inputs.row(114)[opponent]],
            [ONE, 0]
        );
    }

    /// A goal repositions the objective to the arena centre, so a tick-spanning progress
    /// delta reads as a full-length run the wrong way and saturates the scoring team's
    /// gates at `-ONE` — teaching goal avoidance.
    #[test]
    fn a_scored_goal_never_punishes_the_scoring_team() {
        let controller = ZoomieBackend::new(10, 17);
        let mut game = Match::new(
            MatchConfig {
                learning_interval: 1,
                ..MatchConfig::default()
            },
            playbook(),
            controller,
        );
        let objective = game.world().objective_index();
        let radius = game.world().view().radii[objective];
        game.world_mut().set_position(
            objective,
            Vec3Fx::new(
                Fx::from_i32(16) + Fx::from_raw(Fx::ONE_RAW / 4),
                Fx::ZERO,
                radius,
            ),
        );
        game.world_mut()
            .set_velocity(objective, Vec3Fx::X * Fx::from_i32(20));
        game.tick();

        assert_eq!(game.world().scores(), [1, 0], "the fixture must score");
        let [scorer, conceder] =
            Team::ALL.map(|team| game.controller().team_rewards[team.index()][0]);
        assert!(scorer >= 0, "scoring team gate {scorer} is negative");
        assert!(conceder <= 0, "conceding team gate {conceder} is positive");
    }

    /// The resume cursor is the driver's tick, not a pulse count, so it stays one ahead of the
    /// last tick `act` saw whatever the counters read.
    #[test]
    fn timing_the_resume_cursor_tracks_the_authoritative_tick() {
        let controller = ZoomieBackend::new(10, 23);
        let mut game = Match::new(MatchConfig::default(), playbook(), controller);
        for _ in 0..7 {
            game.tick();
        }
        assert_eq!(game.controller().next_tick, game.world().tick());
    }
}
