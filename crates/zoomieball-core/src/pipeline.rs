//! Normative fixed-order match pipeline and per-tick replay witnesses.

use crate::controller::{
    ActRequest, ControllerBackend, MotorCommandBatch, RewardBatch, accumulate_team_rewards,
};
use crate::hash::{OFFSET_BASIS, fold_u64};
use crate::perception::{ObservationBatch, SpatialIndex};
use crate::physics::{PhysicsConfig, step as physics_step};
use crate::playbook::{OracleIntentBatch, Playbook};
use crate::world::{RenderSnapshot, World};
use crate::{LANE_ABI_VERSION, PHYSICS_ABI_VERSION, REPLAY_ABI_VERSION, REWARD_ABI_VERSION};

/// Runtime choices that do not alter hot-loop ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchConfig {
    /// Active physical bodies per team (`10` or `100`).
    pub active_per_team: usize,
    /// Fixed physics parameters.
    pub physics: PhysicsConfig,
    /// Ticks accumulated between controller learning passes.
    pub learning_interval: u32,
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            active_per_team: 10,
            physics: PhysicsConfig::default(),
            learning_interval: 4,
        }
    }
}

/// Authoritative witnesses published after one complete tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickHash {
    /// Fixed-point world witness.
    pub world: u64,
    /// Controller parameter/transient witness.
    pub controller: u64,
    /// Learning eligibility and rule-state witness.
    pub learning: u64,
    /// Replay ABI fold over the component witnesses and play node.
    pub combined: u64,
}

/// Complete deterministic match parameterized by one concrete controller backend.
#[derive(Debug)]
pub struct Match<B: ControllerBackend> {
    world: World,
    controller: B,
    playbook: Playbook,
    play_node: usize,
    physics: PhysicsConfig,
    learning_interval: u32,
    spatial: SpatialIndex,
    observations: ObservationBatch,
    intents: OracleIntentBatch,
    commands: MotorCommandBatch,
    rewards: RewardBatch,
    snapshot: RenderSnapshot,
    last_hash: TickHash,
}

impl<B: ControllerBackend> Match<B> {
    /// Allocate all hot-loop buffers and publish the initial snapshot.
    #[must_use]
    pub fn new(config: MatchConfig, playbook: Playbook, controller: B) -> Self {
        assert!(
            config.learning_interval > 0,
            "learning interval must be positive"
        );
        let world = World::new(config.active_per_team);
        let body_count = world.view().len();
        let mut snapshot = RenderSnapshot::with_capacity(body_count);
        snapshot.publish(&world);
        Self {
            world,
            controller,
            playbook,
            play_node: 0,
            physics: config.physics,
            learning_interval: config.learning_interval,
            spatial: SpatialIndex::new(body_count),
            observations: ObservationBatch::with_capacity(body_count),
            intents: OracleIntentBatch::with_len(body_count),
            commands: MotorCommandBatch::with_len(body_count),
            rewards: RewardBatch::with_len(body_count),
            snapshot,
            last_hash: TickHash::default(),
        }
    }

    /// Advance one complete normative 64 Hz tick.
    pub fn tick(&mut self) -> TickHash {
        self.playbook
            .resolve(self.play_node, &mut self.world, &mut self.intents);
        self.spatial.rebuild(self.world.view());
        self.observations
            .build(self.world.view(), &self.intents, &self.spatial);
        let tick = self.world.tick;
        self.controller.act(
            ActRequest {
                tick,
                world: self.world.view(),
                observations: &self.observations,
                intents: &self.intents,
                play_node: self.play_node,
                enabled_edges: if self.playbook.nodes()[self.play_node].edges().len() == 8 {
                    u8::MAX
                } else {
                    (1u8 << self.playbook.nodes()[self.play_node].edges().len()) - 1
                },
                coach_due: tick.is_multiple_of(4),
            },
            &mut self.commands,
        );

        let objective = self.world.objective_index();
        let previous_objective_x = self.world.positions[objective].x;
        let events = physics_step(
            &mut self.world,
            &self.intents,
            &self.commands,
            &self.physics,
        );
        let progress = self.world.positions[objective].x - previous_objective_x;
        accumulate_team_rewards(
            &mut self.rewards,
            &self.world.teams,
            progress,
            events.scorer,
        );

        self.world.tick += 1;
        if self
            .world
            .tick
            .is_multiple_of(u64::from(self.learning_interval))
        {
            self.controller.learn(self.world.tick, &self.rewards);
            self.rewards.clear();
        }
        self.last_hash = self.fold_hashes();
        self.snapshot.publish(&self.world);
        self.last_hash
    }

    /// Borrow the authoritative world.
    #[must_use]
    pub const fn world(&self) -> &World {
        &self.world
    }

    /// Mutably borrow the world for deterministic fixtures or local tools.
    #[must_use]
    pub const fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Borrow the controller backend.
    #[must_use]
    pub const fn controller(&self) -> &B {
        &self.controller
    }

    /// Mutably borrow the controller for explicit checkpoint operations.
    #[must_use]
    pub const fn controller_mut(&mut self) -> &mut B {
        &mut self.controller
    }

    /// Current human-authoritative play node.
    #[must_use]
    pub const fn play_node(&self) -> usize {
        self.play_node
    }

    /// Select a valid play node directly.
    pub fn select_play_node(&mut self, node: usize) {
        assert!(node < self.playbook.nodes().len(), "play node out of range");
        self.play_node = node;
    }

    /// Follow one outgoing port from the current node.
    pub fn traverse_play(&mut self, port: usize) -> bool {
        let Some(next) = self.playbook.traverse(self.play_node, port) else {
            return false;
        };
        self.play_node = next;
        true
    }

    /// Current immutable render snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &RenderSnapshot {
        &self.snapshot
    }

    /// Current observations, useful for the first-person inspector.
    #[must_use]
    pub const fn observations(&self) -> &ObservationBatch {
        &self.observations
    }

    /// Most recently published replay witnesses.
    #[must_use]
    pub const fn last_hash(&self) -> TickHash {
        self.last_hash
    }

    fn fold_hashes(&self) -> TickHash {
        let world = self.world.hash();
        let controller = self.controller.controller_hash();
        let learning = self.controller.learning_hash();
        let combined = [
            u64::from(REPLAY_ABI_VERSION),
            u64::from(LANE_ABI_VERSION),
            u64::from(PHYSICS_ABI_VERSION),
            u64::from(REWARD_ABI_VERSION),
            u64::try_from(self.play_node).expect("play node fits u64"),
            world,
            controller,
            learning,
        ]
        .into_iter()
        .fold(OFFSET_BASIS, fold_u64);
        TickHash {
            world,
            controller,
            learning,
            combined,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{CheckpointError, IdleController, MotorCommand};
    use crate::fixed::Vec3Fx;

    fn playbook() -> Playbook {
        Playbook::compile_ron(include_str!("../../../assets/default-playbook.ron")).unwrap()
    }

    #[derive(Debug, Clone)]
    struct TracerController {
        idle: IdleController,
        acts: u64,
        coaches: u64,
        learns: u64,
    }

    impl TracerController {
        fn new(active: u16) -> Self {
            Self {
                idle: IdleController::new(active),
                acts: 0,
                coaches: 0,
                learns: 0,
            }
        }
    }

    impl ControllerBackend for TracerController {
        fn act(&mut self, request: ActRequest<'_>, commands: &mut MotorCommandBatch) {
            self.acts += 1;
            self.coaches += u64::from(request.coach_due);
            commands.clear();
            for (body, team) in request.world.teams.iter().enumerate() {
                if team.is_some() {
                    commands.commands[body] = MotorCommand {
                        spin_residual: Vec3Fx::X,
                        boost: true,
                        ..MotorCommand::default()
                    };
                }
            }
        }

        fn learn(&mut self, _tick: u64, _rewards: &RewardBatch) {
            self.learns += 1;
        }

        fn checkpoint(&self, output: &mut Vec<u8>) {
            self.idle.checkpoint(output);
        }

        fn restore(&mut self, input: &[u8]) -> Result<(), CheckpointError> {
            self.idle.restore(input)
        }

        fn controller_hash(&self) -> u64 {
            self.acts
        }

        fn learning_hash(&self) -> u64 {
            self.learns
        }
    }

    #[test]
    fn tracer_spans_intent_perception_actuation_physics_hash_and_snapshot() {
        let config = MatchConfig::default();
        let mut a = Match::new(config, playbook(), TracerController::new(10));
        let mut b = Match::new(config, playbook(), TracerController::new(10));
        for expected_tick in 1..=8 {
            let hash_a = a.tick();
            let hash_b = b.tick();
            assert_eq!(hash_a, hash_b);
            assert_ne!(hash_a.world, 0);
            assert_eq!(a.snapshot().tick, expected_tick);
            assert_eq!(a.snapshot().instances.len(), 21);
        }
        assert_eq!(a.controller().acts, 8);
        assert_eq!(a.controller().coaches, 2);
        assert_eq!(a.controller().learns, 2);
    }

    #[test]
    fn cyclic_human_traversal_changes_and_returns_to_the_play_node() {
        let mut game = Match::new(MatchConfig::default(), playbook(), IdleController::new(10));
        assert!(game.traverse_play(0));
        assert_eq!(game.play_node(), 1);
        assert!(game.traverse_play(0));
        assert_eq!(game.play_node(), 0);
    }
}
