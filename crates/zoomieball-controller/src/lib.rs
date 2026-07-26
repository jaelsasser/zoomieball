#![warn(missing_docs)]

//! Allocation-stable Zoomie population adapter for Zoomieball.

use std::collections::BTreeSet;

use zoomie_core::{Gate, LaneRows, NetId, NetMode};
use zoomie_math::fixed::ONE;
use zoomie_math::rng::Seed;
use zoomie_pop::{
    ChunkedSchedule, EdgeTag, ExploratoryHebbParams, ExploratoryHebbRule, Population, PulseEnv,
    Schedule, SparseCtrnn, SparseCtrnnConfig, SparseCtrnnMember, SparseCtrnnSpec, world_checksum,
};
use zoomieball_core::controller::{
    ActRequest, CheckpointError, CheckpointHeader, ControllerBackend, MotorCommand,
    MotorCommandBatch, RewardBatch, decode_header,
};
use zoomieball_core::fixed::{Fx, Vec3Fx};
use zoomieball_core::hash::{OFFSET_BASIS, fold_i32, fold_u64};
use zoomieball_core::perception::{RayObservation, Relation, SemanticTag};
use zoomieball_core::world::{Role, Team};
use zoomieball_core::{BODY_HZ, COACH_HZ, COACH_INTERVAL_TICKS};

const BODY_DT: i32 = ONE / BODY_HZ.cast_signed();
const COACH_DT: i32 = ONE / COACH_HZ.cast_signed();
const OUTPUT_GATE: i32 = ONE / 4;
const WEIGHT_SCALE: i32 = ONE / 2;
const CHECKPOINT_MAGIC: &[u8; 4] = b"ZBCT";

/// Zoomie-backed body and coach populations for one match roster.
pub struct ZoomieBackend {
    header: CheckpointHeader,
    fielders: RolePool,
    goalies: RolePool,
    coaches: RolePool,
    mailboxes: [[[i32; 8]; 8]; 2],
    edge_logits: [[i32; 8]; 2],
    team_rewards: [[i32; 2]; 2],
    body_pulses: u64,
    coach_pulses: u64,
    learn_passes: u64,
}

impl std::fmt::Debug for ZoomieBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ZoomieBackend")
            .field("header", &self.header)
            .field("fielders", &self.fielders.pop.len())
            .field("goalies", &self.goalies.pop.len())
            .field("coaches", &self.coaches.pop.len())
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

    /// Signed coach logits for the current node's eight ordered edge ports.
    #[must_use]
    pub const fn edge_logits(&self, team: Team) -> [i32; 8] {
        self.edge_logits[team.index()]
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
        write_population(output, &self.fielders.pop);
        write_population(output, &self.goalies.pop);
        write_population(output, &self.coaches.pop);
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
        let mut reader = Reader::new(input, 22);
        let fielders = read_population(&mut reader, &self.fielders)?;
        let goalies = read_population(&mut reader, &self.goalies)?;
        let coaches = read_population(&mut reader, &self.coaches)?;
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
        self.body_pulses = body_pulses;
        self.coach_pulses = coach_pulses;
        self.learn_passes = learn_passes;
        Ok(())
    }

    fn controller_hash(&self) -> u64 {
        world_checksum(&[
            self.fielders.pop.checksum_pair(),
            self.goalies.pop.checksum_pair(),
            self.coaches.pop.checksum_pair(),
        ])
    }

    // Folds learning state alone: a witness that absorbed `controller_hash` would
    // move both layers on any inference divergence and localize nothing.
    fn learning_hash(&self) -> u64 {
        let hash = fold_u64(OFFSET_BASIS, self.learn_passes);
        self.team_rewards.into_iter().flatten().fold(hash, fold_i32)
    }
}

struct RolePool {
    pop: Population<SparseCtrnn>,
    inputs: LaneRows,
    outputs: LaneRows,
    body_indices: Vec<usize>,
    dt: i32,
}

impl RolePool {
    fn new(
        spec: SparseCtrnnSpec,
        config: SparseCtrnnConfig,
        ids: &[NetId],
        body_indices: Vec<usize>,
        seed: u64,
        dt: i32,
    ) -> Self {
        let params = ExploratoryHebbParams {
            exploration_seed: seed ^ 0x004c_4541_524e,
            ..ExploratoryHebbParams::default()
        };
        let rule =
            ExploratoryHebbRule::new(params, &config).expect("the shipped learning rule is valid");
        let mut pop = Population::new(spec, config, Some(rule));
        let members = ids
            .iter()
            .copied()
            .map(|id| {
                (
                    id,
                    SparseCtrnnMember::seeded(Seed(seed), id, WEIGHT_SCALE, pop.spec(), &config),
                )
            })
            .collect();
        pop.insert_batch(members)
            .expect("role identities and seeded members are valid");
        let inputs = LaneRows::new(pop.input_lanes(), pop.len());
        let outputs = LaneRows::new(pop.output_lanes(), pop.len());
        Self {
            pop,
            inputs,
            outputs,
            body_indices,
            dt,
        }
    }
}

fn topology(spec: &SparseCtrnnSpec) -> (usize, usize, usize) {
    (
        spec.input_count(),
        spec.recurrent_count(),
        spec.output_count(),
    )
}

fn coach_spec() -> SparseCtrnnSpec {
    const INPUTS: usize = 128;
    const RECURRENT: usize = 96;
    const OUTPUTS: usize = 72;
    const FAN_IN: usize = 64;
    let dynamic = RECURRENT + OUTPUTS;
    let mut offsets = Vec::with_capacity(dynamic + 1);
    let mut sources = Vec::with_capacity(dynamic * FAN_IN);
    let mut tags = Vec::with_capacity(dynamic * FAN_IN);
    offsets.push(0);
    for target in 0..dynamic {
        let mut row = BTreeSet::new();
        for slot in 0..32 {
            row.insert((target * 37 + slot * 17) % INPUTS);
            row.insert(INPUTS + (target * 29 + slot * 11) % RECURRENT);
        }
        sources.extend(row);
        tags.resize(
            sources.len(),
            if target < RECURRENT {
                EdgeTag::Reservoir
            } else {
                EdgeTag::Head
            },
        );
        offsets.push(sources.len());
    }
    let leaks = (0..dynamic)
        .map(|target| {
            if target < RECURRENT {
                ONE / 32
            } else {
                ONE / 8
            }
        })
        .collect();
    SparseCtrnnSpec::new(INPUTS, RECURRENT, OUTPUTS, offsets, sources, leaks, tags)
        .expect("the 128/96/72 coach fixture is statically valid")
}

fn player_ids(active: usize, locals: std::ops::Range<usize>) -> Vec<NetId> {
    let ids: Vec<_> = Team::ALL
        .into_iter()
        .flat_map(|team| locals.clone().map(move |local| net_id(team, local)))
        .collect();
    assert_eq!(ids.len(), 2 * (active - 1));
    ids
}

fn player_indices(active: usize, locals: std::ops::Range<usize>) -> Vec<usize> {
    Team::ALL
        .into_iter()
        .flat_map(|team| {
            locals
                .clone()
                .map(move |local| team.index() * active + local)
        })
        .collect()
}

fn net_id(team: Team, local: usize) -> NetId {
    NetId::from_raw(u64::try_from(team.index() * 101 + local).expect("roster identity fits u64"))
}

fn encode_coaches(
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

fn encode_bodies(
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

fn clear_column(rows: &mut LaneRows, column: usize) {
    for lane in 0..rows.lanes() {
        rows.row_mut(lane)[column] = 0;
    }
}

fn write_signed(rows: &mut LaneRows, lane: usize, column: usize, raw: i32) {
    rows.row_mut(lane)[column] = i32::midpoint(raw.clamp(-ONE, ONE), ONE);
}

fn body_group(tag: SemanticTag) -> usize {
    match (tag.relation, tag.role) {
        (Relation::Neutral, _) => 0,
        (Relation::Teammate, Role::Goalie) => 1,
        (Relation::Teammate, _) => 2,
        (Relation::Opponent, _) => 3,
        (Relation::Arena | Relation::Goal, _) => 4,
    }
}

fn coach_group(tag: SemanticTag) -> usize {
    match tag.relation {
        Relation::Neutral => 0,
        Relation::Arena => 1,
        Relation::Goal => 2,
        Relation::Opponent => 3 + usize::from(tag.role == Role::Goalie),
        Relation::Teammate => 5 + usize::from(tag.squad % 5),
    }
}

fn receptor(direction: Vec3Fx) -> usize {
    usize::from(direction.x.raw() >= 0) << 2
        | usize::from(direction.z.raw() >= 0) << 1
        | usize::from(direction.y.raw() >= 0)
}

fn inverse_depth(depth: Fx) -> i32 {
    if depth.raw() <= 0 {
        return ONE;
    }
    clamp_i64(i64::from(ONE) * i64::from(ONE) / i64::from(depth.raw())).clamp(0, ONE)
}

fn signed_weight(ray: &RayObservation) -> i32 {
    let weight = inverse_depth(ray.depth);
    match ray.tag.relation {
        Relation::Teammate | Relation::Goal | Relation::Neutral => weight,
        Relation::Opponent | Relation::Arena => -weight,
    }
}

const fn charge_code(surface: bool, air: bool) -> i32 {
    match (surface, air) {
        (false, false) => -ONE,
        (false, true) => -ONE / 3,
        (true, false) => ONE / 3,
        (true, true) => ONE,
    }
}

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

fn clamp_i64(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

fn write_header(output: &mut Vec<u8>, header: CheckpointHeader) {
    output.extend_from_slice(&header.lane_abi.to_le_bytes());
    output.extend_from_slice(&header.physics_abi.to_le_bytes());
    output.extend_from_slice(&header.reward_abi.to_le_bytes());
    output.extend_from_slice(&header.schedule_abi.to_le_bytes());
    output.extend_from_slice(&header.active_per_team.to_le_bytes());
}

fn write_population(output: &mut Vec<u8>, population: &Population<SparseCtrnn>) {
    output.extend_from_slice(
        &u32::try_from(population.len())
            .expect("population count fits u32")
            .to_le_bytes(),
    );
    for &id in population.ids() {
        let member = population
            .extract(id)
            .expect("an iterated population identity exists");
        output.extend_from_slice(&id.bits().to_le_bytes());
        for values in [
            &member.weights,
            &member.reference_weights,
            &member.state,
            &member.eligibility,
        ] {
            output.extend_from_slice(
                &u32::try_from(values.len())
                    .expect("member vector fits u32")
                    .to_le_bytes(),
            );
            for &value in values {
                output.extend_from_slice(&value.to_le_bytes());
            }
        }
        output.extend_from_slice(&member.credit_age.to_le_bytes());
        output.extend_from_slice(&member.exploration_key.to_le_bytes());
    }
}

fn read_population(
    reader: &mut Reader<'_>,
    current: &RolePool,
) -> Result<Population<SparseCtrnn>, CheckpointError> {
    let count = usize::try_from(reader.u32()?).map_err(|_| CheckpointError::Malformed)?;
    if count != current.pop.len() {
        return Err(CheckpointError::Payload(
            "checkpoint roster differs".to_owned(),
        ));
    }
    let spec = current.pop.spec().clone();
    let expected_edges = spec.edge_count();
    let expected_state = spec.dynamic_count();
    let mut members = Vec::with_capacity(count);
    for &expected_id in current.pop.ids() {
        let id = NetId::from_raw(reader.u64()?);
        if id != expected_id {
            return Err(CheckpointError::Payload(
                "checkpoint controller identities differ".to_owned(),
            ));
        }
        members.push((
            id,
            SparseCtrnnMember {
                weights: reader.i32_vec(expected_edges)?,
                reference_weights: reader.i32_vec(expected_edges)?,
                state: reader.i32_vec(expected_state)?,
                eligibility: reader.i32_vec(expected_edges)?,
                credit_age: reader.u32()?,
                exploration_key: reader.u64()?,
            },
        ));
    }
    let mut population = Population::new(spec, *current.pop.config(), current.pop.rule().copied());
    population
        .insert_batch(members)
        .map_err(|error| CheckpointError::Payload(error.to_string()))?;
    Ok(population)
}

struct Reader<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    const fn new(input: &'a [u8], cursor: usize) -> Self {
        Self { input, cursor }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], CheckpointError> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(CheckpointError::Malformed)?;
        let bytes = self
            .input
            .get(self.cursor..end)
            .ok_or(CheckpointError::Malformed)?;
        self.cursor = end;
        bytes.try_into().map_err(|_| CheckpointError::Malformed)
    }

    fn u32(&mut self) -> Result<u32, CheckpointError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn i32(&mut self) -> Result<i32, CheckpointError> {
        Ok(i32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, CheckpointError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn i32_vec(&mut self, expected: usize) -> Result<Vec<i32>, CheckpointError> {
        let length = usize::try_from(self.u32()?).map_err(|_| CheckpointError::Malformed)?;
        if length != expected {
            return Err(CheckpointError::Malformed);
        }
        (0..length).map(|_| self.i32()).collect()
    }

    fn finished(&self) -> bool {
        self.cursor == self.input.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zoomieball_core::{Match, MatchConfig, Playbook};

    fn playbook() -> Playbook {
        Playbook::compile_ron(include_str!("../../../assets/default-playbook.ron")).unwrap()
    }

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

        let controller = game.controller();
        let mailbox = controller.mailboxes[Team::Zero.index()][1];
        for (lane, value) in mailbox.into_iter().enumerate() {
            assert_eq!(
                controller.fielders.inputs.row(48 + lane)[0],
                i32::midpoint(value.clamp(-ONE, ONE), ONE)
            );
        }
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

    #[test]
    fn checkpoint_round_trip_restores_all_population_witnesses() {
        let controller = ZoomieBackend::new(10, 11);
        let mut game = Match::new(MatchConfig::default(), playbook(), controller);
        for _ in 0..4 {
            game.tick();
        }
        let expected = (
            game.controller().controller_hash(),
            game.controller().learning_hash(),
        );
        let mut bytes = Vec::new();
        game.controller().checkpoint(&mut bytes);
        for _ in 0..3 {
            game.tick();
        }
        game.controller_mut().restore(&bytes).unwrap();
        assert_eq!(
            (
                game.controller().controller_hash(),
                game.controller().learning_hash(),
            ),
            expected
        );
    }

    #[test]
    fn checkpoint_abi_mismatch_fails_before_mutation() {
        let mut controller = ZoomieBackend::new(10, 13);
        let original = controller.controller_hash();
        let mut bytes = Vec::new();
        controller.checkpoint(&mut bytes);
        bytes[4] ^= 1;
        assert!(matches!(
            controller.restore(&bytes),
            Err(CheckpointError::AbiMismatch { .. })
        ));
        assert_eq!(controller.controller_hash(), original);
    }

    /// Layered witnesses localize a divergence only while each folds its own layer, so an
    /// inference pulse with no learning pass must leave the learning witness untouched.
    #[test]
    fn an_inference_only_tick_moves_the_controller_witness_alone() {
        let controller = ZoomieBackend::new(10, 19);
        let mut game = Match::new(MatchConfig::default(), playbook(), controller);
        game.tick();
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();
        game.tick();

        assert_eq!(
            game.controller().learn_passes,
            0,
            "the fixture must not reach a learning pass"
        );
        assert_ne!(
            game.controller().controller_hash(),
            controller,
            "an inference pulse must move the controller witness"
        );
        assert_eq!(
            game.controller().learning_hash(),
            learning,
            "no learning pass ran, so the learning witness must not move"
        );
    }
}
