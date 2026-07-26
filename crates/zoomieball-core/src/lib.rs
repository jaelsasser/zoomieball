#![warn(missing_docs)]

//! Deterministic CPU reference contracts, conformance oracle, and headless match pipeline.

pub mod controller;
pub mod fixed;
pub mod hash;
pub mod perception;
pub mod physics;
pub mod pipeline;
pub mod playbook;
pub mod world;

pub use controller::{
    ActRequest, CheckpointError, ControllerBackend, MotorCommand, MotorCommandBatch, Reward,
    RewardBatch,
};
pub use fixed::{Fx, Vec3Fx};
pub use perception::{ObservationBatch, RayObservation, SemanticTag, SpatialIndex};
pub use physics::{Arena, PHYSICS_DT, PhysicsConfig};
pub use pipeline::{Match, MatchConfig, TickHash};
pub use playbook::{OracleIntent, OracleIntentBatch, PlayNode, Playbook, PlaybookError};
pub use world::{ActionCharges, BodyId, ContactFrame, LocalId, Role, Team, World, WorldView};

/// Perception and embodied-controller pulses per second.
pub const BODY_HZ: u32 = 60;
/// Coach-controller pulses per second.
pub const COACH_HZ: u32 = 15;
/// Simulation frequency in authoritative body ticks per second.
pub const TICK_HZ: u32 = BODY_HZ;
/// Fixed physics substeps per authoritative tick.
pub const PHYSICS_SUBSTEPS: u32 = 2;
/// Oracle, motor, and physics updates per second.
pub const PHYSICS_HZ: u32 = TICK_HZ * PHYSICS_SUBSTEPS;
/// Body ticks between coach pulses.
pub const COACH_INTERVAL_TICKS: u32 = BODY_HZ / COACH_HZ;
/// Version of the controller lane layout.
pub const LANE_ABI_VERSION: u32 = 1;
/// Version of the authoritative physics arithmetic.
pub const PHYSICS_ABI_VERSION: u32 = 1;
/// Version of reward accumulation.
pub const REWARD_ABI_VERSION: u32 = 1;
/// Version of the fixed controller and physics schedule.
pub const SCHEDULE_ABI_VERSION: u32 = 1;
/// Version of the replay hash fold: bumped whenever a component witness changes value, so two
/// records carrying different words are known to be incomparable rather than read as state drift.
pub const REPLAY_ABI_VERSION: u32 = 3;
