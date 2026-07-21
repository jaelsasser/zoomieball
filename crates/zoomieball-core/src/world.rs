//! Canonically ordered physical world and cosmetic snapshot publication.

use crate::fixed::{Fx, Vec3Fx};
use crate::hash::{OFFSET_BASIS, fold_i32, fold_u64};

/// One of the two opposing teams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Team {
    /// Team attacking toward positive X.
    Zero = 0,
    /// Team attacking toward negative X.
    One = 1,
}

impl Team {
    /// Both teams in canonical order.
    pub const ALL: [Self; 2] = [Self::Zero, Self::One];

    /// Canonical array index.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Forward attack axis.
    #[must_use]
    pub const fn attack_axis(self) -> Vec3Fx {
        match self {
            Self::Zero => Vec3Fx::X,
            Self::One => Vec3Fx::new(Fx::from_raw(-Fx::ONE_RAW), Fx::ZERO, Fx::ZERO),
        }
    }

    /// Opposing team.
    #[must_use]
    pub const fn opponent(self) -> Self {
        match self {
            Self::Zero => Self::One,
            Self::One => Self::Zero,
        }
    }
}

/// Validated team-local identity in `00..=100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(u8);

impl LocalId {
    /// Goalie identity.
    pub const GOALIE: Self = Self(0);
    /// Nonphysical coach identity.
    pub const COACH: Self = Self(100);

    /// Parse a valid local identity.
    pub const fn new(value: u8) -> Option<Self> {
        if value <= 100 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Raw local number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Physical body role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Role {
    /// Goal-defending body at local ID `00`.
    Goalie = 0,
    /// Field body at local IDs `01..99`.
    Fielder = 1,
    /// Neutral objective sphere.
    Objective = 2,
}

/// Stable identity of one physical sphere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyId {
    /// Team-owned body.
    Player {
        /// Owning team.
        team: Team,
        /// Team-local physical identity.
        local: LocalId,
    },
    /// Neutral objective sphere.
    Objective,
}

/// Restorable action-charge state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionCharges {
    /// One combined surface jump/boost cue is available.
    pub surface: bool,
    /// One airborne cue is available.
    pub air: bool,
}

impl Default for ActionCharges {
    fn default() -> Self {
        Self {
            surface: true,
            air: true,
        }
    }
}

/// Last arena contact used to orient body-space commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContactFrame {
    /// Whether the body touched the arena during the last substep.
    pub touching: bool,
    /// Inward unit normal at the combined contact.
    pub normal: Vec3Fx,
}

impl Default for ContactFrame {
    fn default() -> Self {
        Self {
            touching: true,
            normal: Vec3Fx::Z,
        }
    }
}

/// Stable struct-of-arrays world view passed across the controller boundary.
#[derive(Debug, Clone, Copy)]
pub struct WorldView<'a> {
    /// Stable body identities.
    pub ids: &'a [BodyId],
    /// Positions.
    pub positions: &'a [Vec3Fx],
    /// Linear velocities.
    pub velocities: &'a [Vec3Fx],
    /// Angular velocities.
    pub spins: &'a [Vec3Fx],
    /// Arena contact frames.
    pub contacts: &'a [ContactFrame],
    /// Physical roles.
    pub roles: &'a [Role],
    /// Current play-assigned mailbox indices.
    pub squads: &'a [u8],
    /// Team ownership, absent for the objective.
    pub teams: &'a [Option<Team>],
    /// Action charges.
    pub charges: &'a [ActionCharges],
    /// Sphere radii.
    pub radii: &'a [Fx],
}

impl WorldView<'_> {
    /// Number of physical spheres, including the objective.
    #[must_use]
    pub fn len(self) -> usize {
        self.ids.len()
    }

    /// Whether the view contains no spheres.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.ids.is_empty()
    }
}

/// Authoritative fixed-point world state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct World {
    pub(crate) ids: Vec<BodyId>,
    pub(crate) positions: Vec<Vec3Fx>,
    pub(crate) velocities: Vec<Vec3Fx>,
    pub(crate) spins: Vec<Vec3Fx>,
    pub(crate) contacts: Vec<ContactFrame>,
    pub(crate) roles: Vec<Role>,
    pub(crate) squads: Vec<u8>,
    pub(crate) teams: Vec<Option<Team>>,
    pub(crate) charges: Vec<ActionCharges>,
    pub(crate) radii: Vec<Fx>,
    pub(crate) scores: [u16; 2],
    pub(crate) tick: u64,
    active_per_team: usize,
}

impl World {
    /// Construct `active_per_team` bodies for each team plus the objective.
    /// Supported match sizes are `10` and `100`.
    #[must_use]
    pub fn new(active_per_team: usize) -> Self {
        assert!(
            matches!(active_per_team, 10 | 100),
            "active roster must be 10 or 100 bodies per team"
        );
        let body_count = active_per_team * 2 + 1;
        let mut ids = Vec::with_capacity(body_count);
        let mut positions = Vec::with_capacity(body_count);
        let mut roles = Vec::with_capacity(body_count);
        let mut teams = Vec::with_capacity(body_count);
        let mut squads = Vec::with_capacity(body_count);
        let radius = Fx::from_raw(22_938); // 0.35, rounded once in the ABI fixture.

        for team in Team::ALL {
            for local in 0..active_per_team {
                let local = LocalId::new(u8::try_from(local).expect("roster fits u8"))
                    .expect("physical local id is in range");
                ids.push(BodyId::Player { team, local });
                positions.push(spawn_position(team, local, radius));
                roles.push(if local == LocalId::GOALIE {
                    Role::Goalie
                } else {
                    Role::Fielder
                });
                teams.push(Some(team));
                squads.push(local.get() % 8);
            }
        }
        ids.push(BodyId::Objective);
        positions.push(Vec3Fx::new(Fx::ZERO, Fx::ZERO, radius));
        roles.push(Role::Objective);
        teams.push(None);
        squads.push(0);

        Self {
            ids,
            positions,
            velocities: vec![Vec3Fx::ZERO; body_count],
            spins: vec![Vec3Fx::ZERO; body_count],
            contacts: vec![ContactFrame::default(); body_count],
            roles,
            squads,
            teams,
            charges: vec![ActionCharges::default(); body_count],
            radii: vec![radius; body_count],
            scores: [0; 2],
            tick: 0,
            active_per_team,
        }
    }

    /// Borrow the stable `SoA` controller view.
    #[must_use]
    pub fn view(&self) -> WorldView<'_> {
        WorldView {
            ids: &self.ids,
            positions: &self.positions,
            velocities: &self.velocities,
            spins: &self.spins,
            contacts: &self.contacts,
            roles: &self.roles,
            squads: &self.squads,
            teams: &self.teams,
            charges: &self.charges,
            radii: &self.radii,
        }
    }

    /// Active physical roster size per team.
    #[must_use]
    pub const fn active_per_team(&self) -> usize {
        self.active_per_team
    }

    /// Objective body index.
    #[must_use]
    pub const fn objective_index(&self) -> usize {
        self.active_per_team * 2
    }

    /// Resolve a team-local physical body index.
    #[must_use]
    pub fn player_index(&self, team: Team, local: LocalId) -> Option<usize> {
        let local = usize::from(local.get());
        (local < self.active_per_team).then_some(team.index() * self.active_per_team + local)
    }

    /// Completed authoritative ticks.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Match score in team order.
    #[must_use]
    pub const fn scores(&self) -> [u16; 2] {
        self.scores
    }

    /// Set one body position for fixtures and deterministic resets.
    pub fn set_position(&mut self, index: usize, position: Vec3Fx) {
        self.positions[index] = position;
    }

    /// Set one body velocity for fixtures and deterministic resets.
    pub fn set_velocity(&mut self, index: usize, velocity: Vec3Fx) {
        self.velocities[index] = velocity;
    }

    /// Canonical authoritative-state witness.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let hash = fold_u64(OFFSET_BASIS, self.tick);
        let hash = self
            .scores
            .into_iter()
            .fold(hash, |state, score| fold_u64(state, u64::from(score)));
        (0..self.ids.len()).fold(hash, |state, index| {
            let id_word = match self.ids[index] {
                BodyId::Player { team, local } => {
                    (u64::try_from(team.index()).expect("team fits u64") << 32)
                        | u64::from(local.get())
                }
                BodyId::Objective => u64::MAX,
            };
            let state = fold_u64(state, id_word);
            let state = [
                self.positions[index],
                self.velocities[index],
                self.spins[index],
                self.contacts[index].normal,
            ]
            .into_iter()
            .flat_map(|vector| [vector.x.raw(), vector.y.raw(), vector.z.raw()])
            .fold(state, fold_i32);
            let flags = u64::from(self.contacts[index].touching)
                | (u64::from(self.charges[index].surface) << 1)
                | (u64::from(self.charges[index].air) << 2)
                | (u64::from(self.squads[index]) << 8);
            fold_u64(state, flags)
        })
    }
}

fn spawn_position(team: Team, local: LocalId, radius: Fx) -> Vec3Fx {
    if local == LocalId::GOALIE {
        return Vec3Fx::new(
            Fx::from_i32(match team {
                Team::Zero => -14,
                Team::One => 14,
            }),
            Fx::ZERO,
            radius,
        );
    }
    let ordinal = i32::from(local.get()) - 1;
    let column = ordinal % 11;
    let row = ordinal / 11;
    let own_half = Fx::from_raw((column - 5) * 49_152); // 0.75 spacing.
    let x = match team {
        Team::Zero => Fx::from_i32(-5) + Fx::from_raw(row * 49_152),
        Team::One => Fx::from_i32(5) - Fx::from_raw(row * 49_152),
    };
    Vec3Fx::new(x, own_half, radius)
}

/// One cosmetic sphere instance in a render snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct RenderInstance {
    /// World position.
    pub position: [f32; 3],
    /// Linear velocity for render-only rolling orientation.
    pub velocity: [f32; 3],
    /// Radius.
    pub radius: f32,
    /// Team code (`0`, `1`, or `2` for neutral).
    pub team: u32,
    /// Local number, with `u32::MAX` for the objective.
    pub local_id: u32,
    /// Physical role code.
    pub role: u32,
}

/// Reusable one-way cosmetic state published after each tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderSnapshot {
    /// Completed authoritative tick represented by this frame.
    pub tick: u64,
    /// Packed sphere instances in canonical body order.
    pub instances: Vec<RenderInstance>,
}

impl RenderSnapshot {
    /// Allocate capacity for a world once.
    #[must_use]
    pub fn with_capacity(body_count: usize) -> Self {
        Self {
            tick: 0,
            instances: Vec::with_capacity(body_count),
        }
    }

    pub(crate) fn publish(&mut self, world: &World) {
        self.tick = world.tick;
        self.instances.clear();
        self.instances
            .extend((0..world.ids.len()).map(|index| RenderInstance {
                position: [
                    world.positions[index].x.to_f32(),
                    world.positions[index].y.to_f32(),
                    world.positions[index].z.to_f32(),
                ],
                velocity: [
                    world.velocities[index].x.to_f32(),
                    world.velocities[index].y.to_f32(),
                    world.velocities[index].z.to_f32(),
                ],
                radius: world.radii[index].to_f32(),
                team: world.teams[index].map_or(2, |team| team as u32),
                local_id: match world.ids[index] {
                    BodyId::Player { local, .. } => u32::from(local.get()),
                    BodyId::Objective => u32::MAX,
                },
                role: world.roles[index] as u32,
            }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_contains_two_complete_physical_teams_and_one_objective() {
        let world = World::new(100);
        assert_eq!(world.view().len(), 201);
        assert_eq!(world.player_index(Team::Zero, LocalId::GOALIE), Some(0));
        assert_eq!(
            world.player_index(Team::One, LocalId::new(99).unwrap()),
            Some(199)
        );
        assert_eq!(world.ids[world.objective_index()], BodyId::Objective);
    }

    #[test]
    fn identical_worlds_have_identical_hashes() {
        let a = World::new(10);
        let b = a.clone();
        assert_eq!(a.hash(), b.hash());
    }
}
