//! Statically dispatched controller boundary and caller-owned batches.

use std::fmt;

use crate::fixed::{Fx, Vec3Fx};
use crate::perception::ObservationBatch;
use crate::playbook::OracleIntentBatch;
use crate::world::{Team, WorldView};
use crate::{LANE_ABI_VERSION, PHYSICS_ABI_VERSION, REWARD_ABI_VERSION, SCHEDULE_ABI_VERSION};

/// One decoded body motor command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MotorCommand {
    /// Signed body-frame angular-velocity residual.
    pub spin_residual: Vec3Fx,
    /// Surface cue vertical component gate.
    pub jump: bool,
    /// Surface cue forward component gate.
    pub boost: bool,
    /// Air cue gate.
    pub air_cue: bool,
    /// Bipolar cue-hit coordinates.
    pub cue_hit: [Fx; 2],
}

/// Reusable motor-command buffer in canonical body order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MotorCommandBatch {
    /// One initialized command per sphere; the objective command remains zero.
    pub commands: Vec<MotorCommand>,
}

impl MotorCommandBatch {
    /// Allocate one zero command per body.
    #[must_use]
    pub fn with_len(body_count: usize) -> Self {
        Self {
            commands: vec![MotorCommand::default(); body_count],
        }
    }

    /// Zero every command without changing capacity.
    pub fn clear(&mut self) {
        self.commands.fill(MotorCommand::default());
    }
}

/// One deterministically accumulated learning reward.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reward {
    /// Continuous signed objective progress.
    pub progress: Fx,
    /// Sparse match-event term.
    pub event: Fx,
}

/// Reusable player reward buffer in canonical body order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewardBatch {
    /// Accumulated per-body rewards.
    pub rewards: Vec<Reward>,
}

impl RewardBatch {
    /// Allocate one initialized reward per body.
    #[must_use]
    pub fn with_len(body_count: usize) -> Self {
        Self {
            rewards: vec![Reward::default(); body_count],
        }
    }

    /// Reset after a learning pass.
    pub fn clear(&mut self) {
        self.rewards.fill(Reward::default());
    }
}

/// Borrowed inputs to one controller act pass.
#[derive(Debug, Clone, Copy)]
pub struct ActRequest<'a> {
    /// Authoritative tick before physics.
    pub tick: u64,
    /// Current physical state.
    pub world: WorldView<'a>,
    /// Complete current observations.
    pub observations: &'a ObservationBatch,
    /// Naive play solver intents.
    pub intents: &'a OracleIntentBatch,
    /// Current play-node index.
    pub play_node: usize,
    /// Low eight enabled outgoing graph ports.
    pub enabled_edges: u8,
    /// Whether coaches must publish mailboxes before body evaluation.
    pub coach_due: bool,
}

/// Checkpoint preamble shared by every backend implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointHeader {
    /// Controller lane ABI.
    pub lane_abi: u32,
    /// Physics ABI recorded with the controller family.
    pub physics_abi: u32,
    /// Reward ABI recorded with learned state.
    pub reward_abi: u32,
    /// Fixed controller and physics schedule ABI.
    pub schedule_abi: u32,
    /// Active physical bodies per team.
    pub active_per_team: u16,
}

impl CheckpointHeader {
    /// Current expected preamble for one roster size.
    #[must_use]
    pub const fn current(active_per_team: u16) -> Self {
        Self {
            lane_abi: LANE_ABI_VERSION,
            physics_abi: PHYSICS_ABI_VERSION,
            reward_abi: REWARD_ABI_VERSION,
            schedule_abi: SCHEDULE_ABI_VERSION,
            active_per_team,
        }
    }
}

/// Checkpoint import failure before backend mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointError {
    /// Payload is truncated or malformed.
    Malformed,
    /// One version word differs from the runtime contract.
    AbiMismatch {
        /// Imported header.
        actual: CheckpointHeader,
        /// Runtime header.
        expected: CheckpointHeader,
    },
    /// Backend-specific payload failure.
    Payload(String),
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => write!(formatter, "malformed controller checkpoint"),
            Self::AbiMismatch { actual, expected } => {
                write!(
                    formatter,
                    "checkpoint ABI {actual:?} does not match {expected:?}"
                )
            }
            Self::Payload(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CheckpointError {}

/// Monomorphized controller implementation over caller-owned hot-loop buffers.
pub trait ControllerBackend {
    /// Evaluate due coaches first, then every embodied body population.
    fn act(&mut self, request: ActRequest<'_>, commands: &mut MotorCommandBatch);

    /// Apply accumulated deterministic rewards after physics.
    fn learn(&mut self, tick: u64, rewards: &RewardBatch);

    /// Write a complete versioned checkpoint, reusing `output` capacity.
    fn checkpoint(&self, output: &mut Vec<u8>);

    /// Validate and restore a checkpoint without partial mutation.
    fn restore(&mut self, input: &[u8]) -> Result<(), CheckpointError>;

    /// Current controller parameter and transient-state witness.
    fn controller_hash(&self) -> u64;

    /// Current learning-state witness.
    fn learning_hash(&self) -> u64;
}

/// Deterministic zero-output backend used to isolate the engine in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleController {
    header: CheckpointHeader,
}

impl IdleController {
    /// Construct for one active roster size.
    #[must_use]
    pub fn new(active_per_team: u16) -> Self {
        Self {
            header: CheckpointHeader::current(active_per_team),
        }
    }
}

impl ControllerBackend for IdleController {
    fn act(&mut self, _request: ActRequest<'_>, commands: &mut MotorCommandBatch) {
        commands.clear();
    }

    fn learn(&mut self, _tick: u64, _rewards: &RewardBatch) {}

    fn checkpoint(&self, output: &mut Vec<u8>) {
        output.clear();
        output.extend_from_slice(&self.header.lane_abi.to_le_bytes());
        output.extend_from_slice(&self.header.physics_abi.to_le_bytes());
        output.extend_from_slice(&self.header.reward_abi.to_le_bytes());
        output.extend_from_slice(&self.header.schedule_abi.to_le_bytes());
        output.extend_from_slice(&self.header.active_per_team.to_le_bytes());
    }

    fn restore(&mut self, input: &[u8]) -> Result<(), CheckpointError> {
        if input.len() != 18 {
            return Err(CheckpointError::Malformed);
        }
        let actual = decode_header(input)?;
        if actual != self.header {
            return Err(CheckpointError::AbiMismatch {
                actual,
                expected: self.header,
            });
        }
        Ok(())
    }

    fn controller_hash(&self) -> u64 {
        0
    }

    fn learning_hash(&self) -> u64 {
        0
    }
}

/// Decode the common 18-byte checkpoint header.
pub fn decode_header(input: &[u8]) -> Result<CheckpointHeader, CheckpointError> {
    if input.len() < 18 {
        return Err(CheckpointError::Malformed);
    }
    let word = |offset| {
        u32::from_le_bytes(
            input[offset..offset + 4]
                .try_into()
                .expect("bounds checked by the checkpoint length guard"),
        )
    };
    Ok(CheckpointHeader {
        lane_abi: word(0),
        physics_abi: word(4),
        reward_abi: word(8),
        schedule_abi: word(12),
        active_per_team: u16::from_le_bytes(
            input[16..18]
                .try_into()
                .expect("bounds checked by the checkpoint length guard"),
        ),
    })
}

pub(crate) fn accumulate_team_rewards(
    rewards: &mut RewardBatch,
    teams: &[Option<Team>],
    progress: Fx,
    scorer: Option<Team>,
) {
    for (reward, team) in rewards.rewards.iter_mut().zip(teams) {
        let Some(team) = team else {
            *reward = Reward::default();
            continue;
        };
        let signed_progress = if *team == Team::Zero {
            progress
        } else {
            -progress
        };
        reward.progress += signed_progress;
        if let Some(scorer) = scorer {
            reward.event += if *team == scorer { Fx::ONE } else { -Fx::ONE };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_checkpoint_header_binds_the_fixed_schedule() {
        let controller = IdleController::new(10);
        let mut checkpoint = Vec::new();
        controller.checkpoint(&mut checkpoint);
        assert_eq!(checkpoint.len(), 18);
        assert_eq!(decode_header(&checkpoint).unwrap().schedule_abi, 1);
    }

    #[test]
    fn idle_checkpoint_rejects_trailing_payload() {
        let mut controller = IdleController::new(10);
        let mut checkpoint = Vec::new();
        controller.checkpoint(&mut checkpoint);
        checkpoint.push(0);
        assert_eq!(
            controller.restore(&checkpoint),
            Err(CheckpointError::Malformed)
        );
    }
}
