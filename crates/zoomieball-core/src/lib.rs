#![warn(missing_docs)]

//! CPU-authoritative Zoomieball simulation contracts and deterministic world pipeline.

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
pub use physics::{Arena, PhysicsConfig};
pub use pipeline::{Match, MatchConfig, TickHash};
pub use playbook::{OracleIntent, OracleIntentBatch, PlayNode, Playbook, PlaybookError};
pub use world::{
    ActionCharges, BodyId, ContactFrame, LocalId, RenderInstance, RenderSnapshot, Role, Team,
    World, WorldView,
};

/// Simulation frequency in authoritative ticks per second.
pub const TICK_HZ: u32 = 64;
/// Fixed physics substeps per authoritative tick.
pub const PHYSICS_SUBSTEPS: u32 = 2;
/// Version of the controller lane layout.
pub const LANE_ABI_VERSION: u32 = 1;
/// Version of the authoritative physics arithmetic.
pub const PHYSICS_ABI_VERSION: u32 = 1;
/// Version of reward accumulation.
pub const REWARD_ABI_VERSION: u32 = 1;
/// Version of the replay hash fold.
pub const REPLAY_ABI_VERSION: u32 = 1;
