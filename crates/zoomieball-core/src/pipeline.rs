//! Fixed-order CPU tracer pipeline and layered per-tick replay witnesses.

use crate::controller::{
    ActRequest, ControllerBackend, MotorCommandBatch, RewardBatch, accumulate_team_rewards,
};
use crate::hash::{OFFSET_BASIS, fold_u64};
use crate::perception::{ObservationBatch, SpatialIndex};
use crate::physics::{BallTouch, PhysicsConfig, step as physics_step};
use crate::playbook::{GraphState, OracleIntentBatch, PlayNode, Playbook, next_cursor};
use crate::world::{Team, World};
use crate::{
    COACH_INTERVAL_TICKS, LANE_ABI_VERSION, PHYSICS_ABI_VERSION, REPLAY_ABI_VERSION,
    REWARD_ABI_VERSION, SCHEDULE_ABI_VERSION,
};

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

/// Layered witnesses published after one complete tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickHash {
    /// Normative commutative fixed-point physics-state witness.
    pub physics: u32,
    /// Controller parameter/transient witness.
    pub controller: u64,
    /// Learning eligibility and rule-state witness.
    pub learning: u64,
    /// Diagnostic whole-pipeline fold, including match metadata and both teams' play nodes.
    pub pipeline: u64,
}

/// Complete deterministic match parameterized by one concrete controller backend.
#[derive(Debug)]
pub struct Match<B: ControllerBackend> {
    world: World,
    controller: B,
    playbook: Playbook,
    cursors: [usize; 2],
    graph_state: GraphState,
    pending_play_node: Option<usize>,
    physics: PhysicsConfig,
    learning_interval: u32,
    spatial: SpatialIndex,
    observations: ObservationBatch,
    intents: OracleIntentBatch,
    commands: MotorCommandBatch,
    rewards: RewardBatch,
    last_hash: TickHash,
}

impl<B: ControllerBackend> Match<B> {
    /// Allocate all hot-loop buffers.
    #[must_use]
    pub fn new(config: MatchConfig, playbook: Playbook, controller: B) -> Self {
        assert!(
            config.learning_interval > 0,
            "learning interval must be positive"
        );
        let world = World::new(config.active_per_team);
        let body_count = world.view().len();
        Self {
            world,
            controller,
            playbook,
            cursors: [0; 2],
            graph_state: GraphState::default(),
            pending_play_node: None,
            physics: config.physics,
            learning_interval: config.learning_interval,
            spatial: SpatialIndex::new(body_count),
            observations: ObservationBatch::with_capacity(body_count),
            intents: OracleIntentBatch::with_len(body_count),
            commands: MotorCommandBatch::with_len(body_count),
            rewards: RewardBatch::with_len(body_count),
            last_hash: TickHash::default(),
        }
    }

    /// Advance one complete 60 Hz body tick through the current CPU tracer.
    pub fn tick(&mut self) -> TickHash {
        let tick = self.world.tick;

        // Step 2. Each cursor scans its own node against its own team's logits. A latched human
        // override outranks the scan for the player's team: the scan is skipped outright, not run
        // and overwritten, because a fired port would restamp the node-entry tick even when the
        // override holds the cursor exactly where it already is — a spurious `Elapsed` reset.
        let overridden = self.pending_play_node.take();
        for team in Team::ALL {
            let node = overridden
                .filter(|_| team == Team::Zero)
                .unwrap_or_else(|| {
                    let cursor = self.cursors[team.index()];
                    next_cursor(
                        &self.playbook.nodes()[cursor],
                        cursor,
                        team,
                        &self.graph_state,
                        &self.world,
                        self.controller.edge_logits(team),
                    )
                });
            self.enter(team, node);
        }
        self.playbook.resolve(
            self.cursors,
            &self.physics.arena,
            &mut self.world,
            &mut self.intents,
        );

        self.spatial.rebuild(self.world.view());
        self.observations.build(
            self.world.view(),
            &self.intents,
            &self.physics.arena,
            &self.spatial,
        );
        self.controller.act(
            ActRequest {
                tick,
                world: self.world.view(),
                observations: &self.observations,
                intents: &self.intents,
                play_node: self.cursors,
                enabled_edges: self
                    .cursors
                    .map(|cursor| enabled_edges(&self.playbook.nodes()[cursor])),
                coach_due: tick.is_multiple_of(u64::from(COACH_INTERVAL_TICKS)),
            },
            &mut self.commands,
        );

        let events = physics_step(
            &mut self.world,
            &self.intents,
            &self.commands,
            &self.physics,
        );
        accumulate_team_rewards(
            &mut self.rewards,
            &self.world.teams,
            events.objective_progress,
            events.scorer,
        );
        // Substep stage 9 reports who touched the game ball; the tick it happened on is this
        // driver's to stamp, and the pair is what `Possession` reads at the next step 2. A
        // contested touch is honestly nobody's and lands as `None`, which `possession` already
        // reads as `Neutral` — the same word an empty window gets — overwriting any standing
        // single-team touch.
        if let Some(touch) = events.ball_touch {
            self.graph_state.touched = tick;
            self.graph_state.toucher = match touch {
                BallTouch::Team(team) => Some(team),
                BallTouch::Contested => None,
            };
        }

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

    /// One team's current play node.
    #[must_use]
    pub const fn play_node(&self, team: Team) -> usize {
        self.cursors[team.index()]
    }

    /// Current graph traversal and possession state, which is per-match replay state.
    #[must_use]
    pub const fn graph_state(&self) -> GraphState {
        self.graph_state
    }

    /// Queue a valid play node to latch on the player's team at the start of the next body tick.
    pub fn select_play_node(&mut self, node: usize) {
        assert!(node < self.playbook.nodes().len(), "play node out of range");
        self.pending_play_node = Some(node);
    }

    /// Queue one outgoing port on the player's team, from its latest queued or active node.
    pub fn traverse_play(&mut self, port: usize) -> bool {
        let source = self
            .pending_play_node
            .unwrap_or(self.cursors[Team::Zero.index()]);
        let Some(next) = self.playbook.traverse(source, port) else {
            return false;
        };
        self.pending_play_node = Some(next);
        true
    }

    /// Move one cursor, restarting the node-entry clock only where the node actually changed.
    ///
    /// A node's last port is `Always` back to some node, frequently itself; treating that as a
    /// fresh entry would reset `entered` every tick and strand every `Elapsed` port behind it.
    fn enter(&mut self, team: Team, node: usize) {
        if self.cursors[team.index()] == node {
            return;
        }
        self.cursors[team.index()] = node;
        self.graph_state.entered[team.index()] = self.world.tick;
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
        let physics = self.world.physics_hash();
        let world = self.world.diagnostic_hash();
        let controller = self.controller.controller_hash();
        let learning = self.controller.learning_hash();
        let [player_cursor, opponent_cursor] = self
            .cursors
            .map(|cursor| u64::try_from(cursor).expect("play node fits u64"));
        let pipeline = [
            u64::from(REPLAY_ABI_VERSION),
            u64::from(LANE_ABI_VERSION),
            u64::from(PHYSICS_ABI_VERSION),
            u64::from(REWARD_ABI_VERSION),
            u64::from(SCHEDULE_ABI_VERSION),
            player_cursor,
            opponent_cursor,
            u64::from(physics),
            world,
            controller,
            learning,
        ]
        .into_iter()
        .fold(OFFSET_BASIS, fold_u64);
        TickHash {
            physics,
            controller,
            learning,
            pipeline,
        }
    }
}

/// One low bit per declared outgoing port; the compiler caps a node at eight, which saturates.
fn enabled_edges(node: &PlayNode) -> u8 {
    u8::try_from((1u16 << node.edges().len()) - 1).expect("a node declares at most eight ports")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{CheckpointError, IdleController, MotorCommand};
    use crate::fixed::{Fx, Vec3Fx};
    use crate::playbook::PORT_COUNT;
    use crate::world::LocalId;
    use crate::{BODY_HZ, COACH_HZ, PHYSICS_HZ};

    const SHIPPED: &str = include_str!("../../../assets/default-playbook.ron");

    fn playbook() -> Playbook {
        Playbook::compile_ron(SHIPPED).unwrap()
    }

    #[derive(Debug, Clone)]
    struct TracerController {
        idle: IdleController,
        acts: u64,
        coaches: u64,
        learns: u64,
        /// Each team's enabled-edge mask as the last `act` received it.
        masks: [u8; 2],
        /// Standing coach publication, so a fixture can gate one team's ports without a pool.
        logits: [[Fx; PORT_COUNT]; 2],
    }

    impl TracerController {
        fn new(active: u16) -> Self {
            Self {
                idle: IdleController::new(active),
                acts: 0,
                coaches: 0,
                learns: 0,
                masks: [0; 2],
                logits: [[Fx::ZERO; PORT_COUNT]; 2],
            }
        }
    }

    impl ControllerBackend for TracerController {
        fn act(&mut self, request: ActRequest<'_>, commands: &mut MotorCommandBatch) {
            self.acts += 1;
            self.coaches += u64::from(request.coach_due);
            self.masks = request.enabled_edges;
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

        fn edge_logits(&self, team: Team) -> [Fx; PORT_COUNT] {
            self.logits[team.index()]
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
    fn tracer_spans_intent_perception_actuation_physics_and_hashes() {
        let config = MatchConfig::default();
        let mut a = Match::new(config, playbook(), TracerController::new(10));
        let mut b = Match::new(config, playbook(), TracerController::new(10));
        for expected_tick in 1..=8 {
            let hash_a = a.tick();
            let hash_b = b.tick();
            assert_eq!(hash_a, hash_b);
            assert_ne!(hash_a.physics, 0);
            assert_eq!(a.world().tick(), expected_tick);
        }
        assert_eq!(a.controller().acts, 8);
        assert_eq!(a.controller().coaches, 2);
        assert_eq!(a.controller().learns, 2);
    }

    /// The mask is one low bit per *declared* port, per team, from that team's own node — never a
    /// fixed eight. `press` declares two ports and `recover` three, so moving only the player's
    /// team onto `recover` parts the two masks.
    #[test]
    fn enabled_edges_carry_each_nodes_declared_port_count_per_team() {
        let mut game = Match::new(
            MatchConfig::default(),
            playbook(),
            TracerController::new(10),
        );
        game.tick();
        assert_eq!(game.controller().masks, [0b11, 0b11]);

        game.select_play_node(1);
        game.tick();
        assert_eq!(game.controller().masks, [0b111, 0b11]);
    }

    /// Both cursors start on `press` and scan the same ports against the same world, so team one
    /// standing still while team zero moves is the override — and nothing else — being read.
    #[test]
    fn cyclic_human_traversal_drives_the_players_team_alone() {
        let mut game = Match::new(MatchConfig::default(), playbook(), IdleController::new(10));
        assert!(game.traverse_play(0));
        assert_eq!(
            game.play_node(Team::Zero),
            0,
            "an override latches, it does not apply early"
        );
        game.tick();
        assert_eq!(
            [game.play_node(Team::Zero), game.play_node(Team::One)],
            [1, 0]
        );
        assert!(game.traverse_play(0));
        assert_eq!(game.play_node(Team::Zero), 1);
        game.tick();
        assert_eq!(
            [game.play_node(Team::Zero), game.play_node(Team::One)],
            [0, 0]
        );
    }

    /// Verdict 1: a team's coach logits gate only that team's transitions. Both cursors sit on the
    /// same node with the same `CoachEdge` port, and only the team whose lane cleared the gate
    /// leaves — on the tick after the pulse, not the pulse's own tick.
    #[test]
    fn a_coach_edge_moves_only_the_team_whose_logit_cleared_the_gate() {
        let source = SHIPPED.replacen(
            "(to: 1, trigger: BallBehind(-8.0))",
            "(to: 1, trigger: CoachEdge)",
            1,
        );
        let mut controller = TracerController::new(10);
        controller.logits[Team::Zero.index()][0] = Fx::ONE;
        let mut game = Match::new(
            MatchConfig::default(),
            Playbook::compile_ron(&source).unwrap(),
            controller,
        );

        game.tick();
        assert_eq!(
            [game.play_node(Team::Zero), game.play_node(Team::One)],
            [0, 0],
            "tick zero is the coach's own pulse, where its logits are not yet readable"
        );
        game.tick();
        assert_eq!(
            [game.play_node(Team::Zero), game.play_node(Team::One)],
            [1, 0]
        );
    }

    /// The three owners of one `Possession` decision meet here: physics names the touching team,
    /// this driver stamps the tick, and the next tick's port scan reads the pair back.
    #[test]
    fn a_game_ball_touch_latches_possession_for_the_touching_team_alone() {
        let source = SHIPPED.replacen(
            "(to: 1, trigger: BallBehind(-8.0))",
            "(to: 1, trigger: Possession(Teammate))",
            1,
        );
        let mut game = Match::new(
            MatchConfig::default(),
            Playbook::compile_ron(&source).unwrap(),
            IdleController::new(10),
        );
        let objective = game.world().objective_index();
        let toucher = game
            .world()
            .player_index(Team::Zero, LocalId::new(1).unwrap())
            .unwrap();
        let contact = game.world().view().positions[toucher];
        game.world_mut().set_position(objective, contact);

        game.tick();
        assert_eq!(game.graph_state().toucher, Some(Team::Zero));
        assert_eq!(game.graph_state().touched, 0);
        assert_eq!(
            [game.play_node(Team::Zero), game.play_node(Team::One)],
            [0, 0],
            "the touch happens in this tick's physics, after this tick's port scan"
        );

        game.tick();
        assert_eq!(
            [game.play_node(Team::Zero), game.play_node(Team::One)],
            [1, 0]
        );
    }

    /// A tick in which both teams touch the game ball is honestly nobody's: the record lands as
    /// the neutral relation, overwriting a standing possession. Under the canonical-index
    /// tie-break the proposal rejects, team zero would keep the ball here instead.
    #[test]
    fn a_contested_touch_overwrites_possession_with_neutral() {
        let mut game = Match::new(MatchConfig::default(), playbook(), IdleController::new(10));
        let objective = game.world().objective_index();
        let zero = game
            .world()
            .player_index(Team::Zero, LocalId::new(1).unwrap())
            .unwrap();
        let one = game
            .world()
            .player_index(Team::One, LocalId::new(1).unwrap())
            .unwrap();

        let contact = game.world().view().positions[zero];
        game.world_mut().set_position(objective, contact);
        game.tick();
        assert_eq!(game.graph_state().toucher, Some(Team::Zero));

        let contact = game.world().view().positions[zero];
        game.world_mut().set_position(one, contact);
        game.world_mut().set_position(objective, contact);
        game.tick();
        assert_eq!(game.graph_state().toucher, None);
        assert_eq!(game.graph_state().touched, 1);
    }

    /// A node whose last port is `Always` back to itself is scanned as true every tick. Were that
    /// counted as entering the node, the `Elapsed` port ahead of it could never come due — which is
    /// exactly the stall `recover`'s three-second escape hatch exists to prevent.
    #[test]
    fn holding_a_node_through_its_own_always_port_does_not_restart_the_elapsed_clock() {
        let source = SHIPPED
            .replacen(
                "(to: 0, trigger: BallPast(0.0))",
                "(to: 0, trigger: BallPast(100.0))",
                1,
            )
            .replacen(
                "(to: 0, trigger: Elapsed(180))",
                "(to: 0, trigger: Elapsed(3))",
                1,
            );
        let mut game = Match::new(
            MatchConfig::default(),
            Playbook::compile_ron(&source).unwrap(),
            IdleController::new(10),
        );

        game.select_play_node(1);
        game.tick();
        assert_eq!(game.graph_state().entered[Team::Zero.index()], 0);
        for _ in 0..2 {
            game.tick();
        }
        assert_eq!(
            game.play_node(Team::Zero),
            1,
            "two ticks short of the operand, the cursor holds"
        );

        game.tick();
        assert_eq!(game.play_node(Team::Zero), 0);
        assert_eq!(
            game.play_node(Team::One),
            0,
            "the opponent never left `press`"
        );
    }

    /// An override outranks trigger evaluation rather than replacing its result. `recover`'s
    /// `BallPast(0.0)` port fires every tick against a ball resting on the halfway line, so
    /// re-selecting the node the cursor already holds is the case that separates the two: skipping
    /// the scan leaves the node-entry tick alone, while running it and overwriting the answer
    /// walks the cursor off the node and back, restamping `entered` and stranding every `Elapsed`
    /// port behind it.
    #[test]
    fn an_override_outranks_the_port_scan_rather_than_overwriting_its_result() {
        let mut game = Match::new(MatchConfig::default(), playbook(), IdleController::new(10));
        game.select_play_node(1);
        game.tick();
        assert_eq!(game.play_node(Team::Zero), 1);
        assert_eq!(game.graph_state().entered[Team::Zero.index()], 0);

        game.select_play_node(1);
        game.tick();
        assert_eq!(
            game.play_node(Team::Zero),
            1,
            "the override holds the cursor where it already is"
        );
        assert_eq!(
            game.graph_state().entered[Team::Zero.index()],
            0,
            "a scan that never ran cannot restamp the node-entry tick"
        );
    }

    #[test]
    fn fixed_schedule_is_sixty_fifteen_one_twenty() {
        assert_eq!(BODY_HZ, 60);
        assert_eq!(COACH_HZ, 15);
        assert_eq!(COACH_INTERVAL_TICKS, 4);
        assert_eq!(PHYSICS_HZ, 120);
    }
}
